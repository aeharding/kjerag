//! **The null: what the app draws, frame by frame, as a checksum.**
//!
//! One question, and the whole of it: does this build draw the picture the
//! owner approved, byte for byte, on the film he approved it on?
//!
//! An A/B answered by an eye is answered about a build, and a build is a
//! commit plus everything that was deleted on the way to shipping it. This is
//! how a merge proves it did not move the picture while it was tidying up:
//! play a real stretch of a real file offscreen, render every frame through
//! the app's own pass, and print an md5 per frame and one over the run. Two
//! builds that print the same summary drew the same film.
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin null -- <file.insv> \
//!   from=63.5 to=69.5 yaw=179.00 pitch=-36.97 fov=20.00 lock=1 seam=factory
//! ```
//!
//! **It plays rather than seeking, and that is the point.** The seam band
//! warms over seconds of film and the handover line is held on world content
//! across frames ([`kjerag_render::SeamAnchor`]), so a single seeked frame
//! exercises neither. `from` is where the film is opened and `to` is where the
//! run stops; every frame between them is decoded, measured and drawn in
//! order, exactly as a player would.
//!
//! **One `prepare` per frame is the app's behaviour and not an approximation
//! of it.** The follow is charged in FILM: a redraw with no new frame behind
//! it advances the clock by nothing and the update law at a step of nothing is
//! exactly the identity. So a run at one draw per frame and a window at 60 Hz
//! over a 30 fps file hold the line in the same place, which is the property
//! that makes this instrument able to speak for the app at all.
//!
//! Nothing is written to disk unless `out=` names a directory, and a frame of
//! real footage is personal video: prefer the checksums.

use std::path::PathBuf;
use std::time::Duration;

use kjerag_media::Fallible;
use kjerag_render::{Camera, Cue, Horizon, Sampling, Scene, ScenePipeline, Size};
use kjerag_spike::{Gpu, Render, Seam};

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    // Empty and absent are one thing to `projection::anchoring` since
    // 2026-08-09 - `KJERAG_ANCHOR=` is a shell expanding a variable that is
    // itself unset, not a request to turn the anchor off - so this line
    // reports what the engine read and not what the shell wrote. The way to
    // ask for the default in an A/B arm is `env -u KJERAG_ANCHOR`.
    let anchor = std::env::var("KJERAG_ANCHOR").unwrap_or_default();
    println!(
        "anchor: KJERAG_ANCHOR={}",
        match anchor.is_empty() {
            true => "<unset>",
            false => &anchor,
        }
    );
    println!(
        "width:  KJERAG_HANDOVER_DEG={}",
        std::env::var("KJERAG_HANDOVER_DEG").unwrap_or_else(|_| "<unset>".to_owned())
    );
    println!(
        "file:   {} from {:.3} s to {:.3} s",
        options.input.display(),
        options.from,
        options.to
    );
    println!(
        "view:   yaw={} pitch={} fov={} lock={} seam={} {}x{}",
        options.yaw,
        options.pitch,
        options.fov,
        u8::from(options.lock),
        options.seam_name,
        options.w,
        options.h
    );

    let mut pipeline = ScenePipeline::new(&gpu.device, kjerag_spike::FORMAT);
    let mut scene = Scene::still(&options.input, Cue::Time(secs(options.from)))?;
    scene.set_horizon(match options.lock {
        true => Horizon::Locked,
        false => Horizon::Free,
    });
    options.seam.hold(&scene);

    let size = Size::new(options.w, options.h);
    let stop = secs(options.to);
    let camera = Camera {
        yaw: options.yaw.to_radians() as f32,
        pitch: options.pitch.to_radians() as f32,
        fov: options.fov.to_radians() as f32,
    };

    // One running digest over every frame in order, so the summary changes if
    // any frame changes OR if the frames arrive in a different order or in a
    // different number. A per-frame line as well, so a difference can be
    // pointed at rather than only detected.
    let mut whole = Md5::new();
    let mut frames = 0usize;
    let mut last = Duration::ZERO;
    println!("\n  frame        at   md5 of the drawn frame");
    while let Some((_, now)) = scene.frame() {
        if now > stop {
            break;
        }
        let picture = Render {
            gpu: &gpu,
            scene: &scene,
            pipeline: &mut pipeline,
        }
        .frame(camera, Sampling::default(), size)?;
        let digest = Md5::of(&picture.rgba);
        whole.eat(&picture.rgba);
        println!("{frames:>7} {:>9.3}s   {digest}", now.as_secs_f64());
        if let Some(dir) = &options.out {
            std::fs::create_dir_all(dir)?;
            picture.save(&gpu, &dir.join(format!("{frames:05}.png")))?;
        }
        frames += 1;
        last = now;
        if !scene.advance()? {
            break;
        }
    }
    if frames == 0 {
        return Err(format!(
            "no frames between {:.3} and {:.3} s",
            options.from, options.to
        )
        .into());
    }
    println!(
        "\nnull:   {frames} frames, {:.3} s to {:.3} s\nsummary {}",
        options.from,
        last.as_secs_f64(),
        whole.finish(),
    );
    Ok(())
}

fn secs(seconds: f64) -> Duration {
    Duration::from_secs_f64(seconds.max(0.0))
}

/// md5, written out here rather than taken as a dependency.
///
/// **A checksum and not a security primitive.** What it has to do is change
/// when a picture changes, and the number this prints has to be the number
/// `md5sum` prints for the same bytes, so that a reader can check one against
/// a file on disk. Both are true of the plain algorithm and neither needs a
/// crate: adding one to the lock file would mean regenerating the Flatpak
/// source list to ship an instrument that never leaves this box.
struct Md5 {
    state: [u32; 4],
    length: u64,
    tail: Vec<u8>,
}

impl Md5 {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];

    fn new() -> Self {
        Self {
            state: [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476],
            length: 0,
            tail: Vec::new(),
        }
    }

    fn of(bytes: &[u8]) -> String {
        let mut one = Self::new();
        one.eat(bytes);
        one.finish()
    }

    fn eat(&mut self, bytes: &[u8]) {
        self.length += bytes.len() as u64;
        self.tail.extend_from_slice(bytes);
        let whole = self.tail.len() / 64 * 64;
        let ready: Vec<u8> = self.tail.drain(..whole).collect();
        for block in ready.chunks_exact(64) {
            self.block(block);
        }
    }

    fn finish(mut self) -> String {
        let bits = self.length.wrapping_mul(8);
        self.tail.push(0x80);
        while self.tail.len() % 64 != 56 {
            self.tail.push(0);
        }
        self.tail.extend_from_slice(&bits.to_le_bytes());
        let tail = std::mem::take(&mut self.tail);
        for block in tail.chunks_exact(64) {
            self.block(block);
        }
        self.state
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn block(&mut self, block: &[u8]) {
        let words: [u32; 16] = std::array::from_fn(|i| {
            u32::from_le_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ])
        });
        let [mut a, mut b, mut c, mut d] = self.state;
        for i in 0..64 {
            let (mix, index) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            // sin-derived constants, computed rather than tabled.
            let k = ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32;
            let step = a
                .wrapping_add(mix)
                .wrapping_add(k)
                .wrapping_add(words[index]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(step.rotate_left(Self::S[i]));
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
    }
}

struct Options {
    input: PathBuf,
    from: f64,
    to: f64,
    yaw: f64,
    pitch: f64,
    fov: f64,
    w: u32,
    h: u32,
    lock: bool,
    seam: Seam,
    seam_name: String,
    out: Option<PathBuf>,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            input: PathBuf::new(),
            from: 0.0,
            to: 2.0,
            yaw: 90.0,
            pitch: 0.0,
            fov: 60.0,
            w: 960,
            h: 540,
            lock: true,
            seam: Seam::Factory,
            seam_name: String::from("factory"),
            out: None,
        };
        let mut seam = String::from("factory");
        for arg in args {
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("from", v)) => options.from = v.parse()?,
                Some(("to", v)) => options.to = v.parse()?,
                Some(("yaw", v)) => options.yaw = v.parse()?,
                Some(("pitch", v)) => options.pitch = v.parse()?,
                Some(("fov", v)) => options.fov = v.parse()?,
                Some(("w", v)) => options.w = v.parse()?,
                Some(("h", v)) => options.h = v.parse()?,
                Some(("lock", v)) => options.lock = v.parse::<u32>()? != 0,
                Some(("seam", v)) => seam = v.to_string(),
                Some(("out", v)) => options.out = Some(PathBuf::from(v)),
                Some((key, _)) => return Err(format!("no argument called {key}").into()),
            }
        }
        if options.input.as_os_str().is_empty() {
            return Err(USAGE.into());
        }
        options.seam = Seam::parse(&seam)?;
        options.seam_name = seam;
        Ok(options)
    }
}

const USAGE: &str = "usage: null <file.insv> from=s to=s yaw=deg pitch=deg fov=deg [w=px] \
     [h=px] [lock=0] [seam=factory|roll:0.8,yaw:-2.3,pitch:-0.9,cx:-3.3,cy:-11.9] [out=dir]";

#[cfg(test)]
mod tests {
    use super::Md5;

    /// The digest is `md5sum`'s, which is the whole reason it is here: a
    /// number a reader cannot check against a file on disk is a number they
    /// have to take on trust.
    #[test]
    fn the_digest_is_the_one_md5sum_prints() {
        assert_eq!(Md5::of(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(Md5::of(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            Md5::of(b"The quick brown fox jumps over the lazy dog"),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
        // Longer than one block and not a multiple of one, which is where a
        // padding mistake shows.
        let long: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let mut split = Md5::new();
        split.eat(&long[..37]);
        split.eat(&long[37..600]);
        split.eat(&long[600..]);
        assert_eq!(split.finish(), Md5::of(&long));
    }
}
