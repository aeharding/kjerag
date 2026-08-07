//! What a session's across-seam harvest actually refused, direction by
//! direction and moment by moment, and the picture each refusal was looking
//! at.
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin refusals -- <file.insv> \
//!   rows=refusals.csv sheets=scratch/sheets arc=93,125
//! ```
//!
//! The harvest on `feat/per-session-epi` prints three numbers and keeps none
//! of what is behind them: `Session` carries a per-direction mean and a
//! surviving-moment count, each moment's instant, correlation and excursion
//! are dropped inside two `filter` closures, and the four patch-level
//! refusals are accumulated into one `Refused` that nothing ever reads. So
//! "99 moment(s) refused as near content" cannot be attributed to a direction,
//! a time or a patch, and nothing on that branch can say what the refused
//! population is looking at.
//!
//! This reads the same rings with the same shipped correlator - `seam::ring`,
//! `seam::acquired`, `seam::read_ring_centred`, unchanged, off `main` - on the
//! same deterministic plan, and keeps every moment. The gate arithmetic is
//! reproduced here because it lives on a branch this does not build against;
//! it is **checked** rather than trusted, against `--bin epifield`'s own three
//! counts on the same capture.
//!
//! `sheets=` is the half that matters. Each place gets one contact sheet per
//! lens: the 128 directions of the ring, decoded and laid out in azimuth
//! order, each tile bordered by what the harvest did with that direction. A
//! claim about what a refused direction is looking at is a claim about pixels,
//! and pixels are what this writes down.

use std::path::{Path, PathBuf};

use kjerag_media::{Fallible, Plane, Size, Walk};
use kjerag_meta::{CalibrationSet, Lens};
use kjerag_render::seam::{Found, Probe, Refused, Where, acquired, mapped, read_ring_centred};
use kjerag_render::{AZIMUTHS, Reframe, Ring, band};

/// The harvest's plan, from `seam::EPI_PLACES` and `seam::EPI_FRAMES` on
/// `feat/per-session-epi`. Places are spread over the duration at
/// `(place + 0.5) / places`; the frames at each are consecutive.
const PLACES: usize = 6;
const FRAMES: usize = 4;

/// The gate constants the harvest applies, quoted from
/// `crates/render/src/seam.rs` on `feat/per-session-epi`
/// (`EPI_MOMENTS_NEEDED`, `EPI_FAR_M`, `GATE_MADS`, `GATE_FLOOR_DEG`,
/// `WILD_FLOOR_DEG`). Reproduced and not imported, because that branch is not
/// what this builds against; what proves the reproduction is the three counts
/// coming out the same.
const MOMENTS_NEEDED: usize = 3;
const FAR_M: f64 = 60.0;
const GATE_MADS: f64 = 4.0;
const GATE_FLOOR_DEG: f64 = 0.10;
const WILD_FLOOR_DEG: f64 = 0.50;

/// A contact sheet tile, and how many of them go across a sheet.
const TILE: usize = 96;
const EDGE: usize = 4;
const SHEET_COLS: usize = 16;

/// How wide a tile's crop is on the sphere, in degrees. `Probe::span` is 3.7,
/// so this is the patch the correlator reads with a little round it.
const CROP_DEG: f64 = 5.0;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let calibration = CalibrationSet::from_insv(&options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let probe = Probe {
        patches: AZIMUTHS,
        ..Probe::default()
    };
    let map = mapped(&calibration.lenses, frame);
    let ring = kjerag_render::seam::ring(probe.patches);

    println!(
        "file:   {}",
        options
            .input
            .file_name()
            .unwrap_or_default()
            .to_string_lossy(),
    );
    println!(
        "plan:   {PLACES} places x {FRAMES} frames, {} directions, probe span {:.2} step {:.2} \
         keep {:.2} contrast {:.1}",
        probe.patches, probe.span, probe.step, probe.keep, probe.contrast,
    );

    let mut read = gather(&options, frame, &map, &ring, &probe)?;
    let session = judge(&mut read, &ring, &calibration.lenses);
    verdict(&session, &read, &probe);
    per_direction(&session, &read, &ring, &options);
    if let Some(name) = &options.rows {
        write_rows(&read, &session, name)?;
    }
    if let Some(dir) = &options.sheets {
        sheets(&options, &read, &session, &map, &ring, dir)?;
    }
    Ok(())
}

// ------------------------------------------------------------ the readings

/// One direction read at one instant, and what became of it.
struct Moment {
    place: usize,
    frame: usize,
    seconds: f64,
    direction: usize,
    /// The reading, where one correlated above `Probe::keep`.
    found: Option<Found>,
    /// Which of the four patch-level refusals answered, where nothing did.
    refused: Refused,
    fate: Fate,
    /// How far above its direction's own middle this moment sat, in degrees,
    /// which is the quantity the far gate thresholds.
    excursion: f64,
}

/// What a moment's reading ended up as.
#[derive(Clone, Copy, PartialEq)]
enum Fate {
    Kept,
    /// Correlation under `Probe::keep`, dropped before any gate.
    Unlike,
    /// No patch at all: outside, flat, or pinned against the search.
    NoPatch,
    /// The far gate: this moment swung further than a thing at `FAR_M` would.
    Near,
    /// The trim gate: past four MADs of its direction's own middle.
    Trimmed,
    /// The direction never had `MOMENTS_NEEDED` of anything.
    TooFew,
    /// The direction was thrown out whole by the shape gate.
    Wild,
}

impl Fate {
    fn name(self) -> &'static str {
        match self {
            Self::Kept => "kept",
            Self::Unlike => "unlike",
            Self::NoPatch => "no-patch",
            Self::Near => "near",
            Self::Trimmed => "trimmed",
            Self::TooFew => "too-few",
            Self::Wild => "wild",
        }
    }
}

/// One place of the plan, kept whole so a sheet is drawn off the same decode
/// the readings came from.
struct Place {
    at: f64,
    seconds: f64,
    planes: Vec<Plane>,
}

struct Gathered {
    places: Vec<Place>,
    moments: Vec<Moment>,
    centre: isize,
}

impl Gathered {
    fn tally(&self, want: Fate) -> usize {
        self.moments.iter().filter(|m| m.fate == want).count()
    }

    fn of(&self, direction: usize) -> impl Iterator<Item = &Moment> {
        self.moments
            .iter()
            .filter(move |m| m.direction == direction)
    }
}

/// Every direction of every frame of the plan, read through the shipped
/// correlator one direction at a time so that a refusal has an owner.
///
/// `read_ring_centred` maps over the ring with no state between its elements
/// (`seam::one` takes one direction and nothing else), so a one-element slice
/// reads the number the whole ring would have read and answers with its own
/// `Refused`.
fn gather(
    options: &Options,
    frame: Size,
    map: &Reframe,
    ring: &[Where],
    probe: &Probe,
) -> Fallible<Gathered> {
    let mut walk = Walk::over(std::slice::from_ref(&options.input), 0.0, frame)?;
    if walk.streams() < 2 {
        return Err("this capture carries one lens stream, so it has no seam".into());
    }
    let duration = walk.duration().as_secs_f64();
    let mut gathered = Gathered {
        places: Vec::new(),
        moments: Vec::new(),
        centre: 0,
    };
    let mut centre = None;
    for place in 0..PLACES {
        let at = duration * (place as f64 + 0.5) / PLACES as f64;
        walk.jump(at)?;
        let mut first = None;
        for index in 0..FRAMES {
            let Some(pair) = walk.next_pair()? else {
                break;
            };
            let seconds = pair.at.as_secs_f64();
            if centre.is_none() {
                centre = acquired(map, &pair.lenses, ring, probe);
            }
            for direction in 0..ring.len() {
                let mut refused = Refused::default();
                let found = read_ring_centred(
                    map,
                    &pair.lenses,
                    &ring[direction..=direction],
                    probe,
                    centre.unwrap_or(0),
                    &mut refused,
                )[0];
                let found = found.filter(|found| found.r >= probe.keep);
                gathered.moments.push(Moment {
                    place,
                    frame: index,
                    seconds,
                    direction,
                    found,
                    refused,
                    // `seam::one` returns `None` only for outside, flat or
                    // pinned; a peak under `keep` comes back as a reading and
                    // is dropped above, so those are the two ways to be here
                    // with nothing.
                    fate: match found {
                        Some(_) => Fate::Kept,
                        None if refused.outside + refused.flat + refused.pinned > 0 => {
                            Fate::NoPatch
                        }
                        None => Fate::Unlike,
                    },
                    excursion: f64::NAN,
                });
            }
            if first.is_none() {
                first = Some(Place {
                    at,
                    seconds,
                    planes: pair.lenses,
                });
            }
        }
        let Some(first) = first else {
            return Err(format!("place {place} at {at:.1} s decoded no pair").into());
        };
        println!(
            "place:  {place} at {at:.1} s, first frame {:.3} s",
            first.seconds
        );
        gathered.places.push(first);
    }
    gathered.centre = centre.unwrap_or(0);
    println!(
        "centre: the along-seam search was centred {} grid step(s) off what the camera wrote",
        gathered.centre,
    );
    Ok(gathered)
}

// ------------------------------------------------------------ the gates

/// What the session read, per direction, after the gates.
struct Session {
    read: Vec<f64>,
    kept: Vec<usize>,
    near: usize,
    wild: usize,
    /// Per direction, the degrees of positive excursion the far gate allows.
    reach: Vec<f64>,
}

impl Session {
    fn covered(&self) -> usize {
        self.kept.iter().filter(|count| **count > 0).count()
    }
}

/// The harvest's three gates, in its order, with every moment's fate recorded
/// on the way through.
fn judge(read: &mut Gathered, ring: &[Where], lenses: &[Lens]) -> Session {
    let baseline = band::baseline(lenses);
    let mut session = Session {
        read: vec![0.0; ring.len()],
        kept: vec![0; ring.len()],
        near: 0,
        wild: 0,
        reach: vec![f64::NAN; ring.len()],
    };
    for direction in 0..ring.len() {
        // What a near thing at the gate's own distance would add here, by
        // `Cell::metres`' arithmetic run backwards on this capture's baseline.
        let reach = (f64::from(Ring::cell(direction, baseline).reach_m) / FAR_M).to_degrees();
        session.reach[direction] = reach;
        let alive: Vec<usize> = (0..read.moments.len())
            .filter(|at| read.moments[*at].direction == direction)
            .filter(|at| read.moments[*at].found.is_some())
            .collect();
        let across = |read: &Gathered, at: usize| read.moments[at].found.expect("alive").across;
        if alive.len() < MOMENTS_NEEDED {
            mark(read, &alive, Fate::TooFew);
            continue;
        }
        let all: Vec<f64> = alive.iter().map(|at| across(read, *at)).collect();
        let (middle, _) = tolerated(&all, GATE_MADS, GATE_FLOOR_DEG);
        for at in &alive {
            read.moments[*at].excursion = across(read, *at) - middle;
        }
        let (far, near): (Vec<usize>, Vec<usize>) = alive
            .iter()
            .partition(|at| across(read, **at) - middle <= reach);
        session.near += near.len();
        mark(read, &near, Fate::Near);
        if far.len() < MOMENTS_NEEDED {
            mark(read, &far, Fate::TooFew);
            continue;
        }
        // The same reduction one level in that every other ring here takes.
        let values: Vec<f64> = far.iter().map(|at| across(read, *at)).collect();
        let (middle, tolerance) = tolerated(&values, GATE_MADS, GATE_FLOOR_DEG);
        let (kept, trimmed): (Vec<usize>, Vec<usize>) = far
            .iter()
            .partition(|at| (across(read, **at) - middle).abs() <= tolerance);
        mark(read, &trimmed, Fate::Trimmed);
        if kept.len() < MOMENTS_NEEDED {
            mark(read, &kept, Fate::TooFew);
            continue;
        }
        session.read[direction] =
            kept.iter().map(|at| across(read, *at)).sum::<f64>() / kept.len() as f64;
        session.kept[direction] = kept.len();
    }
    for direction in wild(&session) {
        session.read[direction] = 0.0;
        session.kept[direction] = 0;
        session.wild += 1;
        for moment in read.moments.iter_mut() {
            if moment.direction == direction && moment.fate == Fate::Kept {
                moment.fate = Fate::Wild;
            }
        }
    }
    session
}

fn mark(read: &mut Gathered, which: &[usize], fate: Fate) {
    for at in which {
        read.moments[*at].fate = fate;
    }
}

/// Which directions do not belong to the ring's own five-term shape, which is
/// the harvest's `wild`.
fn wild(session: &Session) -> Vec<usize> {
    let phi = |index: usize| index as f64 / AZIMUTHS as f64 * std::f64::consts::TAU;
    let covered: Vec<usize> = (0..AZIMUTHS).filter(|at| session.kept[*at] > 0).collect();
    let rows: Vec<(Vec<f64>, f64)> = covered
        .iter()
        .map(|at| (band::basis(phi(*at)).to_vec(), session.read[*at]))
        .collect();
    let Some(fit) = kjerag_render::seam::least_squares(&rows) else {
        return Vec::new();
    };
    let left = |at: usize| {
        let basis = band::basis(phi(at));
        session.read[at]
            - (0..5)
                .map(|term| basis[term] * fit.params[term])
                .sum::<f64>()
    };
    let all: Vec<f64> = covered.iter().map(|at| left(*at)).collect();
    let (middle, tolerance) = tolerated(&all, GATE_MADS, WILD_FLOOR_DEG);
    covered
        .into_iter()
        .filter(|at| (left(*at) - middle).abs() > tolerance)
        .collect()
}

/// The middle of a population and how far off it a member may sit: a median
/// and four median absolute deviations, floored. `seam::tolerated`.
fn tolerated(values: &[f64], mads: f64, floor: f64) -> (f64, f64) {
    let middle = middle_of(values.iter().copied());
    let scatter = middle_of(values.iter().map(|value| (value - middle).abs()));
    (middle, (mads * scatter).max(floor))
}

fn middle_of(values: impl Iterator<Item = f64>) -> f64 {
    let mut all: Vec<f64> = values.collect();
    if all.is_empty() {
        return 0.0;
    }
    all.sort_by(f64::total_cmp);
    all[all.len() / 2]
}

// ------------------------------------------------------------ the report

/// The line the app prints, rebuilt here, beside what it does not print.
fn verdict(session: &Session, read: &Gathered, probe: &Probe) {
    println!(
        "\nread:   {} of {AZIMUTHS} directions, {} moment(s) refused as near content, {} \
         direction(s) as the wrong feature",
        session.covered(),
        session.near,
        session.wild,
    );
    println!(
        "moments: {} in all; kept {}, unlike {}, no-patch {}, near {}, trimmed {}, too-few {}, \
         wild {}",
        read.moments.len(),
        read.tally(Fate::Kept),
        read.tally(Fate::Unlike),
        read.tally(Fate::NoPatch),
        read.tally(Fate::Near),
        read.tally(Fate::Trimmed),
        read.tally(Fate::TooFew),
        read.tally(Fate::Wild),
    );
    let patches = read.moments.iter().fold(Refused::default(), |mut all, m| {
        all.outside += m.refused.outside;
        all.flat += m.refused.flat;
        all.unlike += m.refused.unlike;
        all.pinned += m.refused.pinned;
        all
    });
    println!(
        "patches: outside {}, flat {}, unlike {}, pinned {}, which are the four the harvest \
         accumulates and never reads",
        patches.outside, patches.flat, patches.unlike, patches.pinned,
    );
    let reach: Vec<f64> = session
        .reach
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    println!(
        "gate:   the far gate allows {:.4} to {:.4} deg of positive excursion, against a \
         correlation grid stepped {:.2} deg",
        reach.iter().copied().fold(f64::MAX, f64::min),
        reach.iter().copied().fold(f64::MIN, f64::max),
        probe.step,
    );
}

/// Every direction, with what the gates did to it and what it read.
fn per_direction(session: &Session, read: &Gathered, ring: &[Where], options: &Options) {
    println!(
        "\n{:>6}{:>9}{:>7}{:>7}{:>7}{:>7}{:>8}{:>8}{:>11}{:>9}{:>8}",
        "cell",
        "phi deg",
        "alive",
        "kept",
        "near",
        "trim",
        "nopatch",
        "unlike",
        "across deg",
        "spread",
        "in arc",
    );
    for (direction, at) in ring.iter().enumerate() {
        let tally = |want: Fate| read.of(direction).filter(|m| m.fate == want).count();
        let alive = read.of(direction).filter(|m| m.found.is_some()).count();
        let phi = wrapped(at.phi.to_degrees());
        println!(
            "{direction:>6}{phi:>9.1}{alive:>7}{:>7}{:>8}{:>7}{:>8}{:>8}{:>11.3}{:>9.3}{:>8}",
            session.kept[direction],
            tally(Fate::Near),
            tally(Fate::Trimmed),
            tally(Fate::NoPatch),
            tally(Fate::Unlike),
            session.read[direction],
            spread(read, direction),
            match options.marked(phi) {
                true => "yes",
                false => "",
            },
        );
    }
    let Some((from, to)) = options.arc else {
        return;
    };
    let named: Vec<usize> = (0..ring.len())
        .filter(|at| options.marked(wrapped(ring[*at].phi.to_degrees())))
        .collect();
    let covered = named.iter().filter(|at| session.kept[**at] > 0).count();
    println!(
        "\narc:    {from:.0} to {to:.0} deg is cells {} to {}, of which {covered} of {} are read.",
        named.first().copied().unwrap_or(0),
        named.last().copied().unwrap_or(0),
        named.len(),
    );
    for fate in [
        Fate::Near,
        Fate::Trimmed,
        Fate::NoPatch,
        Fate::Unlike,
        Fate::TooFew,
    ] {
        let count: usize = named
            .iter()
            .map(|at| read.of(*at).filter(|m| m.fate == fate).count())
            .sum();
        println!("        {:>9}: {count} moment(s) in that arc", fate.name());
    }
}

/// The range of a direction's across-seam readings over every moment that
/// correlated at all, which is the population the far gate cuts.
fn spread(read: &Gathered, direction: usize) -> f64 {
    let values: Vec<f64> = read
        .of(direction)
        .filter_map(|m| m.found.map(|found| found.across))
        .collect();
    match values.is_empty() {
        true => f64::NAN,
        false => {
            values.iter().copied().fold(f64::MIN, f64::max)
                - values.iter().copied().fold(f64::MAX, f64::min)
        }
    }
}

fn wrapped(degrees: f64) -> f64 {
    (degrees + 180.0).rem_euclid(360.0) - 180.0
}

fn write_rows(read: &Gathered, session: &Session, name: &str) -> Fallible<()> {
    let out = PathBuf::from("scratch").join(name);
    std::fs::create_dir_all("scratch")?;
    let mut text = String::from(
        "place,frame,time_s,cell,phi_deg,along_deg,across_deg,r,contrast,excursion_deg,gate_deg,\
         fate,outside,flat,pinned\n",
    );
    let number = |value: Option<f64>| value.map_or(String::new(), |v| format!("{v:.4}"));
    for moment in &read.moments {
        text.push_str(&format!(
            "{},{},{:.3},{},{:.2},{},{},{},{},{},{:.4},{},{},{},{}\n",
            moment.place,
            moment.frame,
            moment.seconds,
            moment.direction,
            wrapped(moment.direction as f64 / AZIMUTHS as f64 * 360.0),
            number(moment.found.map(|f| f.along)),
            number(moment.found.map(|f| f.across)),
            number(moment.found.map(|f| f.r)),
            number(moment.found.map(|f| f.contrast)),
            number(moment.excursion.is_finite().then_some(moment.excursion)),
            session.reach[moment.direction],
            moment.fate.name(),
            moment.refused.outside,
            moment.refused.flat,
            moment.refused.pinned,
        ));
    }
    std::fs::write(&out, text)?;
    println!("\nwrote {}", out.display());
    Ok(())
}

// ------------------------------------------------------------ the pictures

/// One contact sheet per place per lens: the ring's directions, decoded, in
/// azimuth order, each bordered by what the harvest did with it.
fn sheets(
    options: &Options,
    read: &Gathered,
    session: &Session,
    map: &Reframe,
    ring: &[Where],
    dir: &str,
) -> Fallible<()> {
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir)?;
    for (index, place) in read.places.iter().enumerate() {
        for lens in 0..place.planes.len().min(2) {
            let rows = ring.len().div_ceil(SHEET_COLS);
            let sheet = draw(options, session, map, ring, &place.planes[lens], lens);
            let path = dir.join(format!(
                "place{index}-t{:.0}s-lens{lens}.png",
                place.seconds,
            ));
            write_png(&sheet, SHEET_COLS * TILE, rows * TILE, &path)?;
            println!(
                "wrote {} (place {index}, {:.1} s)",
                path.display(),
                place.at
            );
        }
    }
    Ok(())
}

/// The sheet itself: RGB, `SHEET_COLS` tiles across, one tile per direction.
fn draw(
    options: &Options,
    session: &Session,
    map: &Reframe,
    ring: &[Where],
    plane: &Plane,
    lens: usize,
) -> Vec<u8> {
    let rows = ring.len().div_ceil(SHEET_COLS);
    let width = SHEET_COLS * TILE;
    let mut pixels = vec![0u8; width * rows * TILE * 3];
    for direction in 0..ring.len() {
        let (col, row) = (direction % SHEET_COLS, direction / SHEET_COLS);
        let scale = per_degree(map, ring, direction, lens);
        let centre = map
            .project(
                lens,
                map.view_ray_from_body(ring[direction].centre.map(|c| c as f32)),
            )
            .pixel;
        let border = border(options, session, ring, direction);
        for y in 0..TILE {
            for x in 0..TILE {
                let at = ((row * TILE + y) * width + col * TILE + x) * 3;
                let edge = x < EDGE || y < EDGE || x >= TILE - EDGE || y >= TILE - EDGE;
                let colour = match edge {
                    true => border,
                    false => picture(plane, centre, scale, x, y),
                };
                pixels[at..at + 3].copy_from_slice(&colour);
            }
        }
    }
    pixels
}

/// Green read, red unread; the named arc's own cells are blue, so a claim
/// about them can be checked by eye against the ring either side.
fn border(options: &Options, session: &Session, ring: &[Where], direction: usize) -> [u8; 3] {
    let marked = options.marked(wrapped(ring[direction].phi.to_degrees()));
    match (session.kept[direction] > 0, marked) {
        (true, true) => [90, 200, 255],
        (true, false) => [40, 200, 60],
        (false, true) => [60, 90, 255],
        (false, false) => [220, 40, 40],
    }
}

/// One pixel of a tile's middle: this lens's own picture of that direction.
fn picture(plane: &Plane, centre: [f32; 2], scale: f64, x: usize, y: usize) -> [u8; 3] {
    let span = (TILE - 2 * EDGE) as f64;
    let source =
        |offset: usize| (offset as f64 - EDGE as f64 - span / 2.0) / span * CROP_DEG * scale;
    let code = plane
        .at(
            f64::from(centre[0]) + source(x),
            f64::from(centre[1]) + source(y),
        )
        .unwrap_or(0.0) as u8;
    [code, code, code]
}

/// Source pixels per degree at one direction, measured off the map rather than
/// assumed: two directions a degree apart on the ring, projected, differenced.
fn per_degree(map: &Reframe, ring: &[Where], direction: usize, lens: usize) -> f64 {
    let phi = ring[direction].phi;
    let at = |phi: f64| {
        let body = [phi.cos() as f32, phi.sin() as f32, 0.0];
        map.project(lens, map.view_ray_from_body(body)).pixel
    };
    let half = 0.5_f64.to_radians();
    let (before, after) = (at(phi - half), at(phi + half));
    f64::from((before[0] - after[0]).hypot(before[1] - after[1])).max(1.0)
}

fn write_png(pixels: &[u8], width: usize, height: usize, path: &Path) -> Fallible<()> {
    let mut png = png::Encoder::new(
        std::io::BufWriter::new(std::fs::File::create(path)?),
        width as u32,
        height as u32,
    );
    png.set_color(png::ColorType::Rgb);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()?.write_image_data(pixels)?;
    Ok(())
}

// ------------------------------------------------------------ the arguments

struct Options {
    input: PathBuf,
    rows: Option<String>,
    sheets: Option<String>,
    /// An arc of azimuths to mark on the sheets and in the table, which is how
    /// one view's own directions are picked out of the ring.
    arc: Option<(f64, f64)>,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            input: PathBuf::new(),
            rows: None,
            sheets: None,
            arc: None,
        };
        for arg in args {
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("rows", value)) => options.rows = Some(value.to_string()),
                Some(("sheets", value)) => options.sheets = Some(value.to_string()),
                Some(("arc", value)) => {
                    let (from, to) = value
                        .split_once(',')
                        .ok_or("an arc is from,to in degrees")?;
                    options.arc = Some((from.parse()?, to.parse()?));
                }
                Some((key, _)) => return Err(format!("no argument called {key}").into()),
            }
        }
        match options.input.as_os_str().is_empty() {
            true => Err(USAGE.into()),
            false => Ok(options),
        }
    }

    /// Whether an azimuth is inside the named arc, the long way round
    /// included.
    fn marked(&self, phi: f64) -> bool {
        let Some((from, to)) = self.arc else {
            return false;
        };
        let (phi, from, to) = (
            phi.rem_euclid(360.0),
            from.rem_euclid(360.0),
            to.rem_euclid(360.0),
        );
        match from <= to {
            true => (from..=to).contains(&phi),
            false => phi >= from || phi <= to,
        }
    }
}

const USAGE: &str = "usage: refusals <file.insv> [rows=name.csv] [sheets=dir] [arc=from,to]";

#[cfg(test)]
mod tests {
    use super::*;

    /// `tolerated` is a median and four median absolute deviations, floored,
    /// and the floor is what answers on a population that agrees.
    #[test]
    fn the_floor_answers_where_a_population_agrees() {
        let (middle, tolerance) = tolerated(&[1.0, 1.0, 1.0, 1.0], GATE_MADS, GATE_FLOOR_DEG);
        assert_eq!(middle, 1.0);
        assert_eq!(tolerance, GATE_FLOOR_DEG);
    }

    /// An arc that crosses the back of the circle holds the azimuths either
    /// side of it and not the ones between its two numbers.
    #[test]
    fn an_arc_over_the_back_of_the_circle_holds_its_own_ends() {
        let options = Options {
            input: PathBuf::new(),
            rows: None,
            sheets: None,
            arc: Some((350.0, 10.0)),
        };
        assert!(options.marked(0.0));
        assert!(options.marked(-5.0));
        assert!(options.marked(355.0));
        assert!(!options.marked(180.0));
    }
}
