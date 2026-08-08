//! What a calibration can and cannot reach at one view: the same across-seam
//! reading `--bin crossing` takes, taken at one set of sites through several
//! calibrations at once.
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin ceiling -- <file.insv> \
//!   time=65.666 yaw=179.00 pitch=-36.97 fov=20.00 lock=1 bins=180 \
//!   arm=base=v3+pool arm=v6=v6+none arm=v6pool=v6+pool
//! ```
//!
//! `--bin crossing` answers "how far apart do the two lenses draw this, under
//! the pose the app draws". It cannot answer "and under a different
//! **calibration**", because the map it reads through comes off [`Scene`],
//! which builds it from the file's own `offset_v3` with at most a five knob
//! [`SeamFit`] on lens 1. A different lens table has no way in.
//!
//! This builds the map itself, from [`Reframe::new`] over lenses of its own
//! and the [`Held`] the pass would have built for the same frame, the way
//! `--bin arcs` does. What makes that legitimate is the first control below
//! and not the argument: run with the shipped arm alone, the table this prints
//! has to be `--bin crossing`'s table at the same line, site for site.
//!
//! **The sites are traced once and every arm is measured at those same
//! sites.** A calibration moves the crossover contour, so re-tracing per arm
//! would compare two sets of content as well as two calibrations; the plant
//! control in `--bin crossing` holds the site still for the same reason. The
//! seam's two axes come off the shipped baseline in every arm too, so `epi`
//! and `perp` name one pair of directions down the whole table.
//!
//! **Three controls, and the negatives do not count without them.**
//!
//! - `control=map` is the first: the base arm against the shipped
//!   `--bin crossing` path, which is checked by eye against a recorded run,
//!   and the `offset_v3` this file's own parser reads against
//!   `CalibrationSet::from_insv`'s lenses, field by field, to the bit.
//! - `control=null` reads lens 0 against its own picture through every arm.
//!   Every reading must be exactly zero, whatever the calibration.
//! - `control=plant=<knob>:<amount>` turns one knob by a known amount on top
//!   of each arm and reads the move back against what the map predicts.
//!
//! An arm is `<label>=<basis>+<pose>`. The basis is `v3` (what the app reads)
//! or `v6` (what the camera declares it was calibrated with, `src/offset.rs`),
//! and the pose is `none`, `pool`, or five knobs written out.
//!
//! **A `v6` arm is not the v6 calibration.** It is v6's eleven pose and
//! intrinsic tokens, which sit in the same slots in both grammars, with the
//! distortion left at v3's. Thirteen coefficients do not fit in the five
//! [`kjerag_meta::Distortion`] holds and the shader has no more, so the run
//! prints what the two radial polynomials do to each other over the seam's own
//! radius and then says plainly that eight coefficients are in no arm at all.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kjerag_media::{Cue, Fallible, Plane, Reader, Walk};
use kjerag_meta::{CalibrationSet, Filter, Lens, Quat};
use kjerag_render::{Camera, Held, Reframe, Rolling, Sampling, SeamFit, Size};
use kjerag_spike::crossing::{self, Axes, Floor, Reading, Refused, Site, Source, Support};
use kjerag_spike::offset::{self, Carry};

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let calibration = CalibrationSet::from_insv(&options.input)?;
    let written = offset::written(&options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);

    say_strings(&written);
    let v3 = control_v3(&written, &calibration)?;
    let v6 = say_v6(&written, &v3)?;

    let arms = options
        .arms
        .iter()
        .map(|arm| arm.build(&options.input, &v3, v6.as_deref()))
        .collect::<Fallible<Vec<_>>>()?;

    // The frame the container puts at that instant, asked for by its own
    // time rather than by the line's: a line a hair past a frame's timestamp
    // walks to the next one, and then this is a different picture from the
    // one `--bin crossing` read at the same line. Its own map control would
    // not have caught that; the assertion below is what does.
    let timing = Reader::open(&options.input)?.timing();
    let cue = Cue::Time(Duration::from_secs_f64(options.time.max(0.0)));
    let (index, at) = (cue.index(timing), cue.time(timing));
    let held = holding(&options, &calibration, index)?;
    let mut walk = Walk::open(&options.input, at.as_secs_f64(), frame)?;
    let pair = walk
        .next_pair()?
        .ok_or("no synchronized raw lens pair at that instant")?;
    if pair.index != index {
        return Err(format!(
            "refused: that instant is frame {index} and the raw walk landed on {}",
            pair.index,
        )
        .into());
    }
    let [front, back] = &pair.lenses[..] else {
        return Err(format!(
            "this file decodes {} lens streams, not 2",
            pair.lenses.len()
        )
        .into());
    };
    let taken = Taken {
        front,
        back,
        frame,
        held,
    };
    println!(
        "frame:  {} at {:.3} s, held {}",
        pair.index,
        pair.at.as_secs_f64(),
        match options.lock {
            true => "world fixed off the orientation track",
            false => "in the body's own frame",
        },
    );

    // One map, one contour, one set of axes, for every arm in the table.
    let base = map(&arms[0].lenses, frame, &options, held);
    let baseline = arms[0]
        .lenses
        .get(1)
        .map_or([0.0; 3], |lens| lens.pose.translation_m);
    let sites = crossing::trace(&base, options.raster(), baseline, options.bins);
    println!(
        "sites:  {} traced on {}'s crossover, {} arc bins ({:.2} deg each), axes off its baseline",
        sites.len(),
        arms[0].label,
        options.bins,
        360.0 / options.bins as f64,
    );
    if sites.is_empty() {
        println!("refused: this view shows no two-lens crossover contour at all");
        return Ok(());
    }

    let mut csv = String::from(
        "arm,site,arc_deg,view_x,view_y,epi_src_px,perp_src_px,epi_view_px,perp_view_px,ncc,status\n",
    );
    let mut summaries = Vec::new();
    for arm in &arms {
        let map = map(&arm.lenses, frame, &options, held);
        let rows = read(&map, &options, &taken, &sites, arm);
        let summary = summarize(arm, &rows, &sites, options.bins);
        write_rows(&mut csv, arm, &sites, &rows)?;
        summaries.push(summary);
    }
    table(&options, &summaries, &arms);
    write_csv(&options, &csv)?;
    Ok(())
}

// ------------------------------------------------------------ the strings

fn say_strings(written: &offset::Written) {
    let shape = |name: &str, text: &Option<String>| match text {
        None => println!("{name:<14}absent"),
        Some(text) => match offset::parse(text) {
            Err(why) => println!(
                "{name:<14}{} tokens, unreadable: {why}",
                text.split('_').count()
            ),
            Ok(read) => println!(
                "{name:<14}{} tokens, {} lenses x {} + 2, {} distortion coefficients, \
                 trailing 0x{:x} (version {}), read as v{}",
                text.split('_').count(),
                read.blocks.len(),
                read.per_lens,
                read.blocks.first().map_or(0, |b| b.distortion.len()),
                read.trailing,
                read.declared_version(),
                read.read_version()
                    .map_or("?".to_owned(), |v| v.to_string()),
            ),
        },
    };
    shape("offset_v2:", &written.v2);
    shape("offset_v3:", &written.v3);
    shape("offset_v6:", &written.v6);
    shape("orig_v6:", &written.original_v6);
    println!(
        "declared:     capture_offset_version = {}{}",
        written
            .declared
            .map_or("absent".to_owned(), |v| v.to_string()),
        match written.declared {
            Some(4) => " (OFFSET_V6, which kjerag does not read)",
            _ => "",
        },
    );
    println!(
        "canvas:       delivered {}x{}, crop window {}x{}",
        written.dimension.width, written.dimension.height, written.crop.width, written.crop.height,
    );
}

/// The first control: this module's own reader against `kjerag_meta`'s.
///
/// A v6 arm is only worth anything if the v3 arm built the same way is the
/// calibration the app already draws, so the parser and the canvas to
/// delivered conversion are checked against the shipped ones before any arm
/// is built on them, field by field and to the bit.
fn control_v3(written: &offset::Written, calibration: &CalibrationSet) -> Fallible<Vec<Lens>> {
    let text = written.v3.as_ref().ok_or("this file writes no offset_v3")?;
    let mine = offset::parse(text)?.lenses(written.dimension, written.crop, Carry::Written)?;
    let theirs = &calibration.lenses;
    if mine.len() != theirs.len() {
        return Err("control=map: the two readers disagree about the lens count".into());
    }
    for (index, (mine, theirs)) in mine.iter().zip(theirs).enumerate() {
        for (name, mine, theirs) in fields(mine, theirs) {
            if mine.to_bits() != theirs.to_bits() {
                return Err(format!(
                    "control=map FAILED: lens {index} {name} reads {mine} here and {theirs} \
                     through CalibrationSet, so nothing below is the app's calibration"
                )
                .into());
            }
        }
    }
    println!(
        "control=map:  offset_v3 through this reader equals CalibrationSet::from_insv's \
         {} lenses, {} fields each, to the bit",
        mine.len(),
        fields(&mine[0], &theirs[0]).len(),
    );
    Ok(mine)
}

fn fields<'a>(mine: &Lens, theirs: &Lens) -> Vec<(&'a str, f64, f64)> {
    let one = |lens: &Lens| {
        [
            lens.intrinsics.xi,
            lens.intrinsics.fx,
            lens.intrinsics.fy,
            lens.intrinsics.cx,
            lens.intrinsics.cy,
            lens.distortion.k1,
            lens.distortion.k2,
            lens.distortion.k3,
            lens.distortion.p1,
            lens.distortion.p2,
            lens.pose.yaw_deg,
            lens.pose.pitch_deg,
            lens.pose.roll_deg,
            lens.pose.translation_m[0],
            lens.pose.translation_m[1],
            lens.pose.translation_m[2],
            f64::from(lens.lens_type),
        ]
    };
    const NAMES: [&str; 17] = [
        "xi",
        "fx",
        "fy",
        "cx",
        "cy",
        "k1",
        "k2",
        "k3",
        "p1",
        "p2",
        "yaw",
        "pitch",
        "roll",
        "tx",
        "ty",
        "tz",
        "lens_type",
    ];
    NAMES
        .into_iter()
        .zip(one(mine))
        .zip(one(theirs))
        .map(|((name, mine), theirs)| (name, mine, theirs))
        .collect()
}

/// What v6 says, what of it kjerag can hold, and what it cannot.
///
/// The radial line is the whole of the honesty here. v6 carries four leading
/// coefficients where v3 carries three, and the seam is one radius on the
/// normalized plane, so the two polynomials can be evaluated against each
/// other exactly where the question is and the difference quoted in the
/// delivered pixels a reader can compare with the residual. The nine
/// coefficients past the fourth have no names this branch could check, and
/// what they are worth is quoted as a bound and not as a reading.
fn say_v6(written: &offset::Written, v3: &[Lens]) -> Fallible<Option<Vec<Lens>>> {
    let Some(text) = &written.v6 else {
        println!("v6:     absent from this file");
        return Ok(None);
    };
    let read = offset::parse(text)?;
    let lenses = read.lenses(written.dimension, written.crop, Carry::From(v3))?;
    println!("\nv6 against v3, lens block by lens block (canvas px and degrees as written):");
    println!(
        "  lens         xi        fx        fy        cx        cy       yaw     pitch      roll"
    );
    let v3_read = offset::parse(written.v3.as_ref().expect("checked in control_v3"))?;
    for (index, (six, three)) in read.blocks.iter().zip(&v3_read.blocks).enumerate() {
        let delta = [
            six.xi - three.xi,
            six.fx - three.fx,
            six.fy - three.fy,
            six.cx - three.cx,
            six.cy - three.cy,
            six.yaw - three.yaw,
            six.pitch - three.pitch,
            six.roll - three.roll,
        ];
        print!("  {index:<4} v6-v3");
        for value in delta {
            print!(" {value:+9.3}");
        }
        println!();
    }
    for (index, (six, three)) in read.blocks.iter().zip(&v3_read.blocks).enumerate() {
        let r = offset::seam_radius(six.xi);
        let mine = offset::radial(&three.distortion[..3], r);
        let theirs = offset::radial(&six.distortion[..4], r);
        // A radial factor multiplies the radius, so a difference in it is a
        // radial displacement of `r * delta` on the normalized plane, which
        // the focal length turns into canvas pixels and the crop ratio into
        // delivered ones.
        let canvas = r * (theirs - mine) * six.fx;
        let delivered = canvas * f64::from(written.dimension.width) / f64::from(written.crop.width);
        println!(
            "  lens {index}: at the seam radius {r:.4}, v3's three radial terms read {mine:.6} \
             and v6's four read {theirs:.6}; the gap is {canvas:+.2} canvas px, \
             {delivered:+.2} delivered px"
        );
    }
    let widest = read
        .blocks
        .iter()
        .flat_map(|block| block.distortion[4..].iter().map(|c| c.abs()))
        .fold(0.0_f64, f64::max);
    let r = read.blocks[0].xi.recip();
    let bound = widest * r * r * r * read.blocks[0].fx * f64::from(written.dimension.width)
        / f64::from(written.crop.width);
    println!(
        "  NOT IN ANY ARM: coefficients 5 to 13 of each v6 block. kjerag holds five \
         (k1,k2,k3,p1,p2) and the shader holds the same five. The largest of the nine is \
         {widest:.5}; a coefficient that size on an r^2 term would be worth {bound:.1} \
         delivered px at the seam, so their absence is a bound of that order and not a \
         rounding.",
    );
    Ok(Some(lenses))
}

// ------------------------------------------------------------ the arms

/// One column of the table: a calibration, and the label it is reported under.
struct Arm {
    label: String,
    lenses: Vec<Lens>,
    /// The pose knobs on top of the basis, for the line that says what ran.
    pose: Option<SeamFit>,
    basis: String,
}

/// An arm as the command line names it, before a file has been read.
struct Asked {
    label: String,
    basis: String,
    pose: String,
}

impl Asked {
    fn parse(value: &str) -> Fallible<Self> {
        let (label, spec) = value
            .split_once('=')
            .ok_or("an arm is label=basis+pose, e.g. base=v3+pool")?;
        let (basis, pose) = spec
            .split_once('+')
            .ok_or("an arm is label=basis+pose, e.g. v6=v6+none")?;
        Ok(Self {
            label: label.to_owned(),
            basis: basis.to_owned(),
            pose: pose.to_owned(),
        })
    }

    fn build(&self, input: &Path, v3: &[Lens], v6: Option<&[Lens]>) -> Fallible<Arm> {
        let lenses = match self.basis.as_str() {
            "v3" => v3.to_vec(),
            "v6" => v6
                .ok_or("this file writes no offset_v6, so no v6 arm can be built")?
                .to_vec(),
            other => return Err(format!("no calibration basis called {other}").into()),
        };
        let pose = match self.pose.as_str() {
            "none" => None,
            knobs => Some(kjerag_spike::fit_arg(knobs, Some(input))?),
        };
        Ok(Arm {
            label: self.label.clone(),
            lenses: match pose {
                Some(fit) => fit.applied(&lenses),
                None => lenses,
            },
            pose,
            basis: self.basis.clone(),
        })
    }
}

// ------------------------------------------------------------ the map

/// The [`Held`] the pass would have built for this frame: the orientation at
/// the camera's own instant, inverted for the lock, and the turn the body
/// makes across the readout window.
///
/// **Both halves, and the rolling one is not optional here.** `--bin arcs`
/// leaves it out and `kjerag_render::seam::mapped` leaves it out, on the
/// measurement that an X4 reads down the delivered frame and so contributes
/// 0.000 degrees at the seam. That is about the two lenses' *disagreement*,
/// not about where the picture lands: dropped here, every site of the map
/// control came out two output pixels off `--bin crossing`'s and the tables
/// stopped comparing. `Scene::view` builds it from the exposure clock's
/// instant and the file's own readout, and this is that, reproduced.
fn holding(options: &Options, calibration: &CalibrationSet, index: u64) -> Fallible<Held> {
    let at_us = calibration.exposure[0]
        .frame_time_us(index)
        .ok_or("that instant is past this file's exposure record")?;
    let track = calibration.orientation(Filter::default());
    if track.is_empty() && options.lock {
        return Err("this file carries no IMU record, so a lock=1 view has no frame".into());
    }
    let readout = calibration.readout();
    let span = (readout.seconds * 1e6) as i64;
    let axis = readout.sweep.axis();
    Ok(Held {
        body_from_world: match options.lock {
            true => track.at(at_us).conjugate(),
            false => Quat::IDENTITY,
        },
        rolling: (!track.is_empty() && span > 0 && axis != [0.0; 2]).then(|| Rolling {
            turn: track.turn(at_us - span / 2, at_us + span / 2),
            axis,
        }),
    })
}

fn map(lenses: &[Lens], frame: Size, options: &Options, held: Held) -> Reframe {
    Reframe::new(
        lenses,
        frame,
        options.camera(),
        held,
        1.0,
        false,
        Sampling::default(),
    )
}

// ------------------------------------------------------------ the reading

/// One site's reading under one arm.
struct Row {
    reading: Result<Reading, Refused>,
    source: Result<Axes, crossing::NoScale>,
    view: Result<Axes, crossing::NoScale>,
    /// `(measured, predicted)` in radians, under `control=plant`.
    plant: Option<(Axes, Axes)>,
}

/// The two decoded lens pictures every arm is read against, and the frame and
/// the pose they were taken at. One value because they travel together and
/// nothing here ever has one of them without the others.
struct Taken<'a> {
    front: &'a Plane,
    back: &'a Plane,
    frame: Size,
    held: Held,
}

fn read(
    map: &Reframe,
    options: &Options,
    taken: &Taken<'_>,
    sites: &[Site],
    arm: &Arm,
) -> Vec<Row> {
    let planted = options.plant.map(|(knob, amount)| {
        let mut fit = SeamFit::default();
        turn(&mut fit, knob, amount);
        self::map(&fit.applied(&arm.lenses), taken.frame, options, taken.held)
    });
    let (front, back) = (taken.front, taken.back);
    let lanes = std::thread::available_parallelism().map_or(1, |count| count.get());
    let chunk = sites.len().div_ceil(lanes).max(1);
    std::thread::scope(|scope| {
        let workers: Vec<_> = sites
            .chunks(chunk)
            .map(|chunk| {
                let planted = planted.as_ref();
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|site| one(map, options, front, back, *site, planted))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("a measuring lane panicked"))
            .collect()
    })
}

fn one(
    map: &Reframe,
    options: &Options,
    front: &Plane,
    back: &Plane,
    site: Site,
    planted: Option<&Reframe>,
) -> Row {
    let reference = Source {
        plane: front,
        lens: 0,
    };
    let target = match options.null {
        true => reference,
        false => Source {
            plane: back,
            lens: 1,
        },
    };
    let reading = crossing::measure(
        map,
        reference,
        target,
        site,
        options.support(),
        options.floor(),
    );
    Row {
        source: crossing::source_scale(map, target.lens, site),
        view: crossing::view_scale(map, site, options.raster()),
        plant: planted.and_then(|planted| {
            let (_, amount) = options.plant?;
            let held = reading.as_ref().ok()?;
            let moved = crossing::measure(
                planted,
                reference,
                target,
                site,
                options.support(),
                options.floor(),
            )
            .ok()?;
            let predicted =
                crossing::response(map, map, planted, target.lens, site, amount / 2.0).ok()?;
            Some((
                Axes {
                    perp: moved.shift_rad.perp - held.shift_rad.perp,
                    epi: moved.shift_rad.epi - held.shift_rad.epi,
                },
                Axes {
                    perp: predicted.perp * amount,
                    epi: predicted.epi * amount,
                },
            ))
        }),
        reading,
    }
}

// ------------------------------------------------------------ the reduction

/// One arm's whole answer at one view, reduced the way `--bin crossing`
/// reduces a run: the along-seam gate per crossing, then a median and a
/// median absolute deviation over what it left.
struct Summary {
    label: String,
    accepted: usize,
    sites: usize,
    epi_src: f64,
    epi_view: f64,
    epi_spread_view: f64,
    perp_src: f64,
    perp_view: f64,
    plant: Option<(f64, f64)>,
}

fn summarize(arm: &Arm, rows: &[Row], sites: &[Site], bins: usize) -> Summary {
    let mut rows: Vec<Row> = rows.iter().map(clone_row).collect();
    for run in crossings(sites, bins) {
        gate(&mut rows, &run);
    }
    let read: Vec<Pixels> = rows.iter().filter_map(pixels).collect();
    let planted: Vec<(Axes, Axes)> = rows
        .iter()
        .filter(|row| row.reading.is_ok())
        .filter_map(|row| row.plant)
        .collect();
    Summary {
        label: arm.label.clone(),
        accepted: read.len(),
        sites: sites.len(),
        epi_src: median(&read.iter().map(|p| p.source.epi).collect::<Vec<_>>()),
        epi_view: median(&read.iter().map(|p| p.view.epi).collect::<Vec<_>>()),
        epi_spread_view: deviation(&read.iter().map(|p| p.view.epi).collect::<Vec<_>>()),
        perp_src: median(&read.iter().map(|p| p.source.perp).collect::<Vec<_>>()),
        perp_view: median(&read.iter().map(|p| p.view.perp).collect::<Vec<_>>()),
        plant: (!planted.is_empty()).then(|| {
            (
                median(
                    &planted
                        .iter()
                        .map(|(m, _)| m.epi.to_degrees())
                        .collect::<Vec<_>>(),
                ),
                median(
                    &planted
                        .iter()
                        .map(|(_, p)| p.epi.to_degrees())
                        .collect::<Vec<_>>(),
                ),
            )
        }),
    }
}

fn clone_row(row: &Row) -> Row {
    Row {
        reading: row.reading,
        source: row.source,
        view: row.view,
        plant: row.plant,
    }
}

/// The along-seam gate, per crossing, exactly as `--bin crossing` runs it.
fn gate(rows: &mut [Row], run: &[usize]) {
    let tolerance = GATE_DEG.to_radians();
    let readings: Vec<Reading> = run
        .iter()
        .filter_map(|at| rows[*at].reading.as_ref().ok().copied())
        .collect();
    let Some(plausible) = crossing::Plausible::measured(&readings, tolerance) else {
        return;
    };
    let mut results: Vec<Result<Reading, Refused>> =
        run.iter().map(|at| rows[*at].reading).collect();
    crossing::gate(&mut results, plausible);
    for (at, result) in run.iter().zip(results) {
        rows[*at].reading = result;
    }
}

/// `--bin crossing`'s default `perpgate`, which every reading in this
/// branch's recorded tables was taken under.
const GATE_DEG: f64 = 0.40;

/// Which sites belong to the same crossing, `--bin crossing`'s rule: the
/// sorted circle turned so its widest gap is at the ends, then cut wherever a
/// gap runs past four bins.
fn crossings(sites: &[Site], bins: usize) -> Vec<Vec<usize>> {
    if sites.is_empty() {
        return Vec::new();
    }
    let gap = |at: usize| {
        (sites[at].node.phi - sites[(at + sites.len() - 1) % sites.len()].node.phi)
            .rem_euclid(std::f64::consts::TAU)
    };
    let start = (0..sites.len())
        .max_by(|one, other| gap(*one).total_cmp(&gap(*other)))
        .unwrap_or(0);
    let apart = 4.0 * std::f64::consts::TAU / bins as f64;
    let mut out = vec![Vec::new()];
    for step in 0..sites.len() {
        let at = (start + step) % sites.len();
        if step > 0 && gap(at) > apart {
            out.push(Vec::new());
        }
        out.last_mut().expect("a run was pushed above").push(at);
    }
    out
}

struct Pixels {
    source: Axes,
    view: Axes,
}

fn pixels(row: &Row) -> Option<Pixels> {
    let reading = row.reading.as_ref().ok()?;
    let (source, view) = (row.source.ok()?, row.view.ok()?);
    Some(Pixels {
        source: Axes {
            perp: reading.shift_rad.perp * source.perp,
            epi: reading.shift_rad.epi * source.epi,
        },
        view: Axes {
            perp: reading.shift_rad.perp * view.perp,
            epi: reading.shift_rad.epi * view.epi,
        },
    })
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    match sorted.len() % 2 {
        0 => (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0,
        _ => sorted[sorted.len() / 2],
    }
}

fn deviation(values: &[f64]) -> f64 {
    let middle = median(values);
    median(
        &values
            .iter()
            .map(|v| (v - middle).abs())
            .collect::<Vec<_>>(),
    )
}

// ------------------------------------------------------------ the report

fn table(options: &Options, summaries: &[Summary], arms: &[Arm]) {
    println!(
        "\nview:   yaw {:.2}, pitch {:.2}, fov {:.2}, lock {}, raster {} px{}",
        options.yaw,
        options.pitch,
        options.fov,
        u8::from(options.lock),
        options.size,
        match options.null {
            true => "   NULL CONTROL: lens 0 against its own picture",
            false => "",
        },
    );
    println!(
        "  {:<12} {:>6} {:>10} {:>10} {:>9} {:>10} {:>10}  calibration",
        "arm", "sites", "epi src", "epi view", "spread", "perp src", "perp view",
    );
    for (summary, arm) in summaries.iter().zip(arms) {
        println!(
            "  {:<12} {:>2}/{:<3} {:>10.2} {:>10.2} {:>9.2} {:>10.2} {:>10.2}  {}",
            summary.label,
            summary.accepted,
            summary.sites,
            summary.epi_src,
            summary.epi_view,
            summary.epi_spread_view,
            summary.perp_src,
            summary.perp_view,
            describe(arm),
        );
        if let Some((measured, predicted)) = summary.plant {
            println!(
                "  {:<12} plant read {measured:+.4} deg on epi against {predicted:+.4} predicted",
                "",
            );
        }
    }
}

fn describe(arm: &Arm) -> String {
    match arm.pose {
        None => format!("{}, no learned pose", arm.basis),
        Some(fit) => format!(
            "{} + roll:{:.3},yaw:{:.3},pitch:{:.3},cx:{:.2},cy:{:.2}",
            arm.basis, fit.roll_deg, fit.yaw_deg, fit.pitch_deg, fit.cx_px, fit.cy_px,
        ),
    }
}

fn write_rows(csv: &mut String, arm: &Arm, sites: &[Site], rows: &[Row]) -> Fallible<()> {
    for (index, (site, row)) in sites.iter().zip(rows).enumerate() {
        let peak = match &row.reading {
            Ok(reading) => format!("{:.5}", reading.correlation),
            Err(why) => why
                .correlation()
                .map_or(String::new(), |p| format!("{p:.5}")),
        };
        let status = match &row.reading {
            Ok(_) if row.source.is_err() || row.view.is_err() => "no-scale",
            Ok(_) => "accepted",
            Err(why) => why.label(),
        };
        let head = format!(
            "{},{index},{:.4},{:.2},{:.2}",
            arm.label,
            site.node.phi.to_degrees(),
            site.view_pixel[0],
            site.view_pixel[1],
        );
        match pixels(row) {
            None => writeln!(csv, "{head},,,,,{peak},{status}")?,
            Some(p) => writeln!(
                csv,
                "{head},{:.4},{:.4},{:.4},{:.4},{peak},{status}",
                p.source.epi, p.source.perp, p.view.epi, p.view.perp,
            )?,
        }
    }
    Ok(())
}

fn write_csv(options: &Options, csv: &str) -> Fallible<()> {
    let out = PathBuf::from("scratch").join(options.out.clone().unwrap_or_else(|| {
        format!(
            "ceiling-t{:.3}-yaw{:.0}-fov{:.0}.csv",
            options.time, options.yaw, options.fov,
        )
    }));
    std::fs::create_dir_all(out.parent().unwrap_or(Path::new(".")))?;
    let stamp = format!(
        "# instrument: kjerag-spike --bin ceiling\n# source: {}\n# args: {}\n\
         # reduction: sites traced once on the first arm, every arm measured at those same \
         sites through its own map; per crossing along-seam gate at {GATE_DEG} deg, then a \
         median over what it left\n",
        options.input.display(),
        std::env::args().skip(1).collect::<Vec<_>>().join(" "),
    );
    std::fs::write(&out, format!("{stamp}{csv}"))?;
    println!("\nwrote {}", out.display());
    Ok(())
}

// ------------------------------------------------------------ the options

fn turn(fit: &mut SeamFit, knob: usize, amount: f64) {
    match knob {
        0 => fit.roll_deg += amount,
        1 => fit.yaw_deg += amount,
        2 => fit.pitch_deg += amount,
        3 => fit.cx_px += amount,
        _ => fit.cy_px += amount,
    }
}

const KNOBS: [&str; 5] = ["roll", "yaw", "pitch", "cx", "cy"];

struct Options {
    input: PathBuf,
    time: f64,
    yaw: f64,
    pitch: f64,
    fov: f64,
    lock: bool,
    size: u32,
    bins: usize,
    span: f64,
    search: f64,
    step: f64,
    contrast: f64,
    agreement: f64,
    null: bool,
    plant: Option<(usize, f64)>,
    arms: Vec<Asked>,
    out: Option<String>,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut out = Self {
            input: PathBuf::new(),
            time: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            fov: 55.0,
            lock: true,
            size: 1024,
            bins: 360,
            // `--bin crossing`'s own defaults, unchanged, because a reading
            // here is only worth anything beside one of its readings.
            span: 2.20,
            search: 1.40,
            step: 0.07,
            contrast: 2.0,
            agreement: 0.5,
            null: false,
            plant: None,
            arms: Vec::new(),
            out: None,
        };
        for arg in args {
            match arg.split_once('=') {
                None => out.input = PathBuf::from(arg),
                Some(("time", value)) => out.time = value.parse()?,
                Some(("yaw", value)) => out.yaw = value.parse()?,
                Some(("pitch", value)) => out.pitch = value.parse()?,
                Some(("fov", value)) => out.fov = value.parse()?,
                Some(("lock", value)) => out.lock = value.parse::<u32>()? != 0,
                Some(("size", value)) => out.size = value.parse()?,
                Some(("bins", value)) => out.bins = value.parse()?,
                Some(("span", value)) => out.span = value.parse()?,
                Some(("search", value)) => out.search = value.parse()?,
                Some(("step", value)) => out.step = value.parse()?,
                Some(("contrast", value)) => out.contrast = value.parse()?,
                Some(("ncc", value)) => out.agreement = value.parse()?,
                Some(("arm", value)) => out.arms.push(Asked::parse(value)?),
                Some(("out", value)) => out.out = Some(value.to_owned()),
                Some(("control", value)) => match value.split_once('=') {
                    None if value == "null" => out.null = true,
                    None if value == "map" => {}
                    Some(("plant", knobs)) => {
                        let (name, amount) = knobs
                            .split_once(':')
                            .ok_or("a plant is knob:amount, e.g. yaw:0.10")?;
                        let index = KNOBS
                            .iter()
                            .position(|knob| *knob == name)
                            .ok_or_else(|| format!("no calibration knob called {name}"))?;
                        out.plant = Some((index, amount.parse()?));
                    }
                    _ => return Err(format!("no control called {value}. {USAGE}").into()),
                },
                Some((key, _)) => return Err(format!("no argument called {key}. {USAGE}").into()),
            }
        }
        if out.input.as_os_str().is_empty() {
            return Err(USAGE.into());
        }
        if out.arms.is_empty() {
            out.arms.push(Asked::parse("base=v3+pool")?);
        }
        if out.bins == 0 {
            return Err("bins= is how many arc bins the crossing is cut into".into());
        }
        if out.null && out.plant.is_some() {
            return Err(
                "the null and the plant are two controls, and each is a run of its own".into(),
            );
        }
        Ok(out)
    }

    fn camera(&self) -> Camera {
        Camera {
            yaw: self.yaw.to_radians() as f32,
            pitch: self.pitch.to_radians() as f32,
            fov: self.fov.to_radians() as f32,
        }
    }

    fn raster(&self) -> Size {
        Size::new(self.size, self.size)
    }

    fn support(&self) -> Support {
        Support {
            span_deg: self.span,
            search_deg: self.search,
            step_deg: self.step,
        }
    }

    fn floor(&self) -> Floor {
        Floor {
            contrast: self.contrast,
            agreement: self.agreement,
        }
    }
}

const USAGE: &str = "usage: ceiling <file.insv> [time=seconds] [yaw=deg] [pitch=deg] [fov=deg] \
     [lock=1] [size=px] [bins=n] [span=deg] [search=deg] [step=deg] [contrast=codes] [ncc=score] \
     [arm=label=v3|v6+none|pool|roll:..,yaw:..,pitch:..,cx:..,cy:..] \
     [control=null | control=plant=knob:amount] [out=name.csv]";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_arm_is_a_label_a_basis_and_a_pose() {
        let asked = Asked::parse("v6pool=v6+pool").expect("a well formed arm");
        assert_eq!(asked.label, "v6pool");
        assert_eq!(asked.basis, "v6");
        assert_eq!(asked.pose, "pool");
    }

    #[test]
    fn an_arm_without_a_pose_is_refused_rather_than_guessed_at() {
        assert!(Asked::parse("v6=v6").is_err());
        assert!(Asked::parse("v6+none").is_err());
    }

    /// The two controls are separate runs, because the null holds both lens
    /// sources equal and the plant needs them different.
    #[test]
    fn the_null_and_the_plant_do_not_run_together() {
        let line = "f.insv control=null control=plant=yaw:0.1";
        assert!(Options::parse(line.split_whitespace().map(str::to_owned)).is_err());
    }
}
