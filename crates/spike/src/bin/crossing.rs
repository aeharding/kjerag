//! How far apart the two lenses draw the same thing, along the seam crossing
//! a named view shows (issue #103's successor question).
//!
//! `step` measures a horizon's step across the seam and needs a horizon to
//! measure. On the owner's 2026-05-01 views it fits scenery at 51 to 86 px
//! rms, so a seam-fix candidate could not be screened there before it reached
//! his eyes. This measures the seam itself instead: the contour where the pass
//! hands the picture over is traced, and at fixed points along it the two raw
//! lens pictures are asked how far apart they draw the same content.
//!
//! **`bins=` is part of a reading and every command here states it.** It sets
//! how many arc bins the crossing is cut into, and two runs at different
//! `bins` are two different sets of sites whose tables do not compare. The
//! recorded tables in this branch's PR were taken at `bins=180`.
//!
//! ```sh
//! # the owner's own view line, with a stored per-camera calibration
//! cargo run --release -p kjerag-spike --bin crossing -- <file.insv> \
//!   time=50.117 yaw=-80.28 pitch=0.06 fov=55.69 lock=1 bins=180 \
//!   seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91
//! # the null control: lens 0 against its own picture, which must read zero
//! cargo run --release -p kjerag-spike --bin crossing -- <file.insv> ... bins=180 null=1
//! # the plant control: a known calibration delta, read back at every site
//! cargo run --release -p kjerag-spike --bin crossing -- <file.insv> ... bins=180 plant=yaw:0.10
//! ```
//!
//! **That `yaw` is re-derived and a stale one runs without a word.** The lock
//! became world-fixed on 2026-08-06, so the frame a `lock=1` yaw is measured
//! in no longer follows the aircraft's slow heading and its zero is the file's
//! opening heading instead. The owner's line above said `yaw=-74.43` until
//! that date and is the same picture at `-80.28`; the tables this branch's PR
//! recorded were read at the old one, on the old build.
//! `new_yaw = old_yaw + carried(t)`, which `--bin carried` computes for a
//! line, and docs/research/reference-views.md has the rule and the re-derived
//! registry.
//!
//! **What is measured, and what is not.** Only the decoded raw lens frames,
//! projected through the app's own map. The projection is the unbent one,
//! which is the calibration's own geometry; the band's per-frame bend is a
//! second layer on top of it and is not in these numbers. That is deliberate:
//! this instrument screens calibration candidates, and a reading of it does
//! not depend on how many frames of film ran into the one being measured.
//!
//! **Every run states its own floor.** `sensitivity:` re-runs the whole
//! measurement with the calibration moved by a thousandth of a degree each
//! way and prints how far the medians travel. Nothing this instrument says
//! is worth more digits than that line. It exists because the first version
//! of this file did not have it and its tables were quoted to a precision
//! they did not have.
//!
//! **Per site, and no further.** Sites a fraction of a degree apart share
//! their content and their calibration error, so they are not independent
//! observations of anything. Each is reported on its own, the only summary is
//! a median and a spread, and a view that shows the seam twice gets two
//! summaries and no combined one.
//!
//! A table goes to the terminal and a CSV of the same rows to gitignored
//! `scratch/`, because a row of it locates content in somebody's real flight.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kjerag_media::{Fallible, Plane};
use kjerag_meta::CalibrationSet;
use kjerag_render::{Camera, Cue, Horizon, Reframe, Scene, SeamFit, Size};
use kjerag_spike::crossing::{self, Axes, Floor, Reading, Refused, Site, Source, Support};
use kjerag_spike::{Walk, seam_fit};

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let calibration = CalibrationSet::from_insv(&options.input)?;
    let baseline = calibration
        .lenses
        .get(1)
        .map_or([0.0; 3], |lens| lens.pose.translation_m);
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);

    let base = map(&options, options.seam)?;
    let mut walk = Walk::open(&options.input, base.at.as_secs_f64(), frame)?;
    let pair = walk
        .next_pair()?
        .ok_or("no synchronized raw lens pair at that instant")?;
    if pair.at != base.at {
        return Err(format!(
            "refused: the mapped frame is at {:.9} s and the raw pair at {:.9} s",
            base.at.as_secs_f64(),
            pair.at.as_secs_f64()
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

    let sites = crossing::trace(&base.map, options.raster(), baseline, options.bins);
    println!(
        "view:   yaw {:.2}, pitch {:.2}, fov {:.2}, lock {}, raster {} px",
        options.yaw, options.pitch, options.fov, options.lock as u8, options.size
    );
    println!(
        "sites:  {} traced on the visible crossover, {} arc bins ({:.2} deg each)",
        sites.len(),
        options.bins,
        360.0 / options.bins as f64
    );
    if sites.is_empty() {
        println!("refused: this view shows no two-lens crossover contour at all");
        return Ok(());
    }
    let support = options.support();
    println!(
        "support: patch {:.2} deg, search {:.2} deg, step {:.3} deg; floors contrast {:.1} codes, agreement {:.2}",
        support.span_deg,
        support.search_deg,
        support.step_deg,
        options.floor().contrast,
        options.floor().agreement,
    );

    let planted = options
        .plant
        .map(|(knob, amount)| plant(&options, knob, amount))
        .transpose()?;
    let mut rows = read_all(&base, &options, front, back, &sites, planted.as_ref());
    report(&options, &sites, &mut rows)?;
    sensitivity(&options, baseline, front, back)?;
    Ok(())
}

/// How far this run's own answer moves when the calibration is dithered by
/// less than any calibration is known to.
///
/// Printed with every reading, because an instrument that does not state its
/// own reproducibility gets quoted to a precision it does not have, and this
/// one was: its first tables were selected among tied candidates by the last
/// bit of an `f32` and did not survive a rerun. This is that number now, and
/// it is a floor under the instrument, not an uncertainty on the seam.
///
/// The dither turns the three **angle** knobs together, `+d` and `-d`, and
/// the band is the distance between what the two runs answer. It never moves
/// `cx` or `cy`, so this is an **angle** floor: a principal-point wobble of
/// the same physical size is not covered by the number it prints.
///
/// Each dithered run's accepted count goes on the line beside the band,
/// because a band can be set two ways. Equal counts mean the same sites, and
/// then the band is how far their readings moved. Different counts mean the
/// dither moved a site in or out of the accepted set, and then the band is a
/// median stepping over a different population, which is what a thin run in
/// glare does and is worth seeing rather than folded into one digit.
fn sensitivity(options: &Options, baseline: [f64; 3], front: &Plane, back: &Plane) -> Fallible<()> {
    let Some(dither) = options.dither else {
        return Ok(());
    };
    let Seam::Stored(fit) = options.seam else {
        println!("sensitivity: withheld; a dither needs seam=<stored fit> to move");
        return Ok(());
    };
    let mut swing = Vec::new();
    for sign in [-1.0, 1.0] {
        let moved = SeamFit {
            roll_deg: fit.roll_deg + sign * dither,
            yaw_deg: fit.yaw_deg + sign * dither,
            pitch_deg: fit.pitch_deg + sign * dither,
            ..fit
        };
        swing.push(answer(options, Seam::Stored(moved), baseline, front, back)?);
    }
    let (low, high) = (swing[0], swing[1]);
    match (low, high) {
        (Some((low, under)), Some((high, over))) => println!(
            "sensitivity: at a +/-{dither} deg dither of the angle knobs the medians move \
             {:.2} view px on epi and {:.2} on perp, over {under} and {over} accepted sites",
            (low.epi - high.epi).abs(),
            (low.perp - high.perp).abs(),
        ),
        _ => println!("sensitivity: withheld; a dithered run had no accepted site to compare"),
    }
    Ok(())
}

/// One run's whole answer in view px, with nothing printed: the medians over
/// its accepted sites after the along-seam gate, and how many there were.
fn answer(
    options: &Options,
    seam: Seam,
    baseline: [f64; 3],
    front: &Plane,
    back: &Plane,
) -> Fallible<Option<(Axes, usize)>> {
    let base = map(options, seam)?;
    let sites = crossing::trace(&base.map, options.raster(), baseline, options.bins);
    let mut rows = read_all(&base, options, front, back, &sites, None);
    for run in crossings(&sites, options.bins) {
        gate_run(options, &mut rows, &run);
    }
    let read: Vec<Pixels> = rows.iter().filter_map(pixels).collect();
    Ok((!read.is_empty()).then(|| {
        let axes = Axes {
            epi: median(&read.iter().map(|p| p.view.epi).collect::<Vec<_>>()),
            perp: median(&read.iter().map(|p| p.view.perp).collect::<Vec<_>>()),
        };
        (axes, read.len())
    }))
}

/// One site's whole answer: what was measured, what it is worth in pixels,
/// and, under a plant, what the perturbed map should have moved it by.
struct Row {
    site: Site,
    reading: Result<Reading, Refused>,
    source: Result<Axes, crossing::NoScale>,
    view: Result<Axes, crossing::NoScale>,
    /// `(measured change, predicted change)` in radians, under `plant=`.
    plant: Option<(Axes, Axes)>,
}

/// Measure every site, in parallel over the machine's cores. Sites share only
/// immutable state: one map, two decoded pictures and one declared support.
fn read_all(
    base: &Mapped,
    options: &Options,
    front: &Plane,
    back: &Plane,
    sites: &[Site],
    planted: Option<&Mapped>,
) -> Vec<Row> {
    let lanes = std::thread::available_parallelism().map_or(1, |count| count.get());
    let chunk = sites.len().div_ceil(lanes).max(1);
    std::thread::scope(|scope| {
        let workers: Vec<_> = sites
            .chunks(chunk)
            .map(|chunk| scope.spawn(|| read_chunk(base, options, front, back, chunk, planted)))
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("a measuring lane panicked"))
            .collect()
    })
}

fn read_chunk(
    base: &Mapped,
    options: &Options,
    front: &Plane,
    back: &Plane,
    sites: &[Site],
    planted: Option<&Mapped>,
) -> Vec<Row> {
    // The null control reads lens 0's own picture twice. It is the same code
    // path as a measurement, which is the point of it: a null that ran
    // different code would clear nothing.
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
    sites
        .iter()
        .map(|site| {
            let reading = crossing::measure(
                &base.map,
                reference,
                target,
                *site,
                options.support(),
                options.floor(),
            );
            Row {
                site: *site,
                source: crossing::source_scale(&base.map, target.lens, *site),
                view: crossing::view_scale(&base.map, *site, options.raster()),
                plant: planted.and_then(|planted| {
                    plant_row(base, planted, options, reference, target, *site, &reading)
                }),
                reading,
            }
        })
        .collect()
}

/// What the plant moved this site by, and what the map says it should have.
fn plant_row(
    base: &Mapped,
    planted: &Mapped,
    options: &Options,
    reference: Source<'_>,
    target: Source<'_>,
    site: Site,
    held: &Result<Reading, Refused>,
) -> Option<(Axes, Axes)> {
    let (_, amount) = options.plant?;
    let held = held.as_ref().ok()?;
    let moved = crossing::measure(
        &planted.map,
        reference,
        target,
        site,
        options.support(),
        options.floor(),
    )
    .ok()?;
    // The prediction is the secant between the two maps this run actually
    // used, so it is the displacement of the planted step itself rather than
    // of an infinitesimal one.
    let predicted = crossing::response(
        &base.map,
        &base.map,
        &planted.map,
        target.lens,
        site,
        amount / 2.0,
    )
    .ok()?;
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
}

/// The map one view is drawn through, and the frame it belongs to.
struct Mapped {
    map: Reframe,
    at: Duration,
}

/// Build the map for one calibration.
///
/// No render, and no GPU: the projection this reads is geometry, and the band
/// state a render would warm is a layer this instrument deliberately excludes.
/// That also makes every reading reproducible from the file and the view line
/// alone, with no warm history in it.
fn map(options: &Options, seam: Seam) -> Fallible<Mapped> {
    let scene = Scene::still(&options.input, Cue::Time(options.at()))?;
    seam.hold(&scene);
    // The along-seam table this camera would be drawn with (issue #103, stage
    // 9), so what this instrument reads is what the picture still disagrees
    // by rather than what it would have without one. `Table::REST` unless a
    // run names a table, and then nothing here moves at all.
    scene.use_table(options.table);
    scene.set_horizon(match options.lock {
        true => Horizon::Locked,
        false => Horizon::Free,
    });
    let (_, at) = scene.frame().ok_or("no frame decoded at that instant")?;
    let map = scene
        .mapped(options.camera(), 1.0)
        .ok_or("no frame to map")?;
    Ok(Mapped { map, at })
}

/// The same view under one knob moved by a known amount.
fn plant(options: &Options, knob: usize, amount: f64) -> Fallible<Mapped> {
    let Seam::Stored(fit) = options.seam else {
        return Err("plant=<knob>:<amount> requires seam=<stored fit> to perturb".into());
    };
    println!(
        "plant:  {} by {:+.3} on top of the stored fit",
        KNOBS[knob], amount
    );
    map(options, Seam::Stored(perturb(fit, knob, amount)))
}

const KNOBS: [&str; 5] = ["roll", "yaw", "pitch", "cx", "cy"];

/// One camera's along-seam table, off the file `kjerag-spike --bin table`
/// writes.
fn read_table(path: &str) -> Fallible<kjerag_render::Table> {
    let text = std::fs::read_to_string(path)?;
    kjerag_render::Table::read(&text).ok_or_else(|| {
        format!(
            "{path} is not {} numbers, one per direction",
            kjerag_render::AZIMUTHS
        )
        .into()
    })
}

fn perturb(mut fit: SeamFit, knob: usize, amount: f64) -> SeamFit {
    match knob {
        0 => fit.roll_deg += amount,
        1 => fit.yaw_deg += amount,
        2 => fit.pitch_deg += amount,
        3 => fit.cx_px += amount,
        _ => fit.cy_px += amount,
    }
    fit
}

/// Which sites belong to the same crossing of the view, in traced order.
///
/// A view can show the seam circle twice, entering on one side and leaving on
/// the other, and the owner's 2026-05-01 frame is exactly that: one crossing
/// he called good and one he called bad. Reporting them together would average
/// the two answers he is asking about into one that describes neither.
///
/// Azimuth is a circle, so the runs are found on a circle: the sites arrive
/// sorted, the list is turned until its widest gap is at the ends, and every
/// remaining gap of more than a few bins starts another crossing. Splitting a
/// sorted list at `-180` instead reports one crossing that straddles the
/// wrap as two, which is what a view looking backwards along the body always
/// shows.
fn crossings(sites: &[Site], bins: usize) -> Vec<Vec<usize>> {
    if sites.is_empty() {
        return Vec::new();
    }
    let gap = |at: usize| {
        (sites[at].node.phi - sites[(at + sites.len() - 1) % sites.len()].node.phi)
            .rem_euclid(std::f64::consts::TAU)
    };
    let widest = (0..sites.len()).max_by(|one, other| gap(*one).total_cmp(&gap(*other)));
    let start = widest.unwrap_or(0);
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

/// What the along-seam gate did to one crossing, or why it did nothing.
enum Gated {
    Off,
    /// Too few readings, or a scatter too wide to have a middle. Carries the
    /// count and the scatter, so the reason is on the page.
    Withheld(usize, Option<f64>),
    Judged(crossing::Plausible, usize),
}

/// Judge one crossing's readings against its own along-seam value.
///
/// Per crossing and not per run, because a principal-point error is one cycle
/// round the azimuth: the far side of the seam circle is a different number.
fn gate_run(options: &Options, rows: &mut [Row], run: &[usize]) -> Gated {
    let Some(tolerance) = options.perp_gate else {
        return Gated::Off;
    };
    let readings: Vec<crossing::Reading> = run
        .iter()
        .filter_map(|at| rows[*at].reading.as_ref().ok().copied())
        .collect();
    let plausible = match options.perp_reference {
        Some(reference) => Some(crossing::Plausible::declared(reference, tolerance)),
        None => crossing::Plausible::measured(&readings, tolerance),
    };
    let Some(plausible) = plausible else {
        return Gated::Withheld(readings.len(), crossing::Plausible::scatter(&readings));
    };
    let mut results: Vec<Result<crossing::Reading, crossing::Refused>> =
        run.iter().map(|at| rows[*at].reading).collect();
    let taken = crossing::gate(&mut results, plausible);
    for (at, result) in run.iter().zip(results) {
        rows[*at].reading = result;
    }
    Gated::Judged(plausible, taken)
}

fn say_gate(gated: &Gated, source: f64) {
    match gated {
        Gated::Off => println!("gate:   along-seam plausibility off"),
        Gated::Withheld(have, scatter) => println!(
            "gate:   along-seam plausibility WITHHELD, nothing refused: {have} readings{}",
            match scatter {
                None => format!(
                    ", under the {} a reference needs",
                    crossing::Plausible::ENOUGH
                ),
                Some(scatter) => format!(
                    " scatter {:.3} deg ({:.1} src px), over the {:.0}% of tolerance a middle has to sit inside",
                    scatter.to_degrees(),
                    scatter * source,
                    crossing::Plausible::STEADY * 100.0,
                ),
            }
        ),
        Gated::Judged(plausible, taken) => println!(
            "gate:   along-seam {}{:+.3} deg ({:+.1} src px); tolerance {:.2} deg ({:.1} src px); refused {taken}",
            match plausible.from {
                0 => "declared ".to_owned(),
                n => format!(
                    "from this crossing, {n} readings, scatter {:.3} deg, ",
                    plausible.spread_rad.to_degrees()
                ),
            },
            plausible.reference_rad.to_degrees(),
            plausible.reference_rad * source,
            plausible.tolerance_rad.to_degrees(),
            plausible.tolerance_rad * source,
        ),
    }
}

fn report(options: &Options, sites: &[Site], rows: &mut [Row]) -> Fallible<()> {
    let mut csv = String::from(
        "crossing,arc_deg,view_x,view_y,epi_src_px,perp_src_px,epi_view_px,perp_view_px,\
         sigma_epi_src_px,sigma_perp_src_px,ncc,status\n",
    );
    let runs = crossings(sites, options.bins);
    for (index, indices) in runs.iter().enumerate() {
        println!(
            "\ncrossing {}: arc {:.1} to {:.1} deg, {} sites",
            index + 1,
            rows[indices[0]].site.node.phi.to_degrees(),
            rows[indices[indices.len() - 1]].site.node.phi.to_degrees(),
            indices.len(),
        );
        let gated = gate_run(options, rows, indices);
        let source = indices
            .iter()
            .find_map(|at| rows[*at].source.ok())
            .map_or(1.0, |scale| scale.perp);
        say_gate(&gated, source);
        let run: Vec<&Row> = indices.iter().map(|at| &rows[*at]).collect();
        println!(
            "   arc     view px       epi src  perp src   epi view perp view    sig epi sig perp    ncc  status"
        );
        for row in &run {
            println!("{}", line(row));
            writeln!(csv, "{},{}", index + 1, comma(row))?;
        }
        summarize(&run, !matches!(gated, Gated::Judged(..)));
    }
    // No pooled line over the crossings. A view can show the seam twice and
    // the two are different azimuths of it; averaging them gives an answer
    // that describes neither, which is the thing this module's own header
    // says not to do. One printed it anyway until 2026-08-05.
    if runs.len() > 1 {
        println!(
            "\n{} crossings, reported apart. There is no combined answer: they are different azimuths of the seam.",
            runs.len()
        );
    }
    let out = options.out();
    std::fs::create_dir_all(out.parent().unwrap_or(Path::new(".")))?;
    std::fs::write(&out, csv)?;
    println!("\nwrote {}", out.display());
    Ok(())
}

/// A site's whole reading in the pixels of both rasters.
struct Pixels {
    source: Axes,
    view: Axes,
    sigma: Axes,
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
        sigma: Axes {
            perp: reading.sigma_rad.perp * source.perp,
            epi: reading.sigma_rad.epi * source.epi,
        },
    })
}

fn line(row: &Row) -> String {
    let head = format!(
        "{:6.1} {:6.1},{:6.1}",
        row.site.node.phi.to_degrees(),
        row.site.view_pixel[0],
        row.site.view_pixel[1],
    );
    let Some(pixels) = pixels(row) else {
        let departure = match row.reading {
            Err(crossing::Refused::PerpImplausible(off)) => {
                let scale = row.source.map_or(1.0, |source| source.perp);
                format!(" by {:.1} src px along the seam", off * scale)
            }
            _ => String::new(),
        };
        return format!(
            "{head}          -         -          -         -          -       - {:>6}  {}{departure}",
            correlation(row).map_or("-".to_owned(), |peak| format!("{peak:.3}")),
            status(row),
        );
    };
    let plant = match row.plant {
        None => String::new(),
        Some((measured, predicted)) => format!(
            "  plant read [{:+.4}, {:+.4}] deg against [{:+.4}, {:+.4}] predicted",
            measured.epi.to_degrees(),
            measured.perp.to_degrees(),
            predicted.epi.to_degrees(),
            predicted.perp.to_degrees(),
        ),
    };
    format!(
        "{head} {:+9.2} {:+9.2}  {:+9.2} {:+9.2}  {:9.2} {:7.2} {:6.3}  {}{plant}",
        pixels.source.epi,
        pixels.source.perp,
        pixels.view.epi,
        pixels.view.perp,
        pixels.sigma.epi,
        pixels.sigma.perp,
        correlation(row).unwrap_or_default(),
        status(row),
    )
}

fn comma(row: &Row) -> String {
    let head = format!(
        "{:.4},{:.2},{:.2}",
        row.site.node.phi.to_degrees(),
        row.site.view_pixel[0],
        row.site.view_pixel[1],
    );
    let peak = correlation(row).map_or(String::new(), |peak| format!("{peak:.5}"));
    let Some(pixels) = pixels(row) else {
        return format!("{head},,,,,,,{peak},{}", status(row));
    };
    format!(
        "{head},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{peak},{}",
        pixels.source.epi,
        pixels.source.perp,
        pixels.view.epi,
        pixels.view.perp,
        pixels.sigma.epi,
        pixels.sigma.perp,
        status(row),
    )
}

/// The peak correlation a site reached, whether or not it cleared the floor:
/// where the floor was drawn stays visible in the table.
fn correlation(row: &Row) -> Option<f64> {
    match &row.reading {
        Ok(reading) => Some(reading.correlation),
        Err(why) => why.correlation(),
    }
}

fn status(row: &Row) -> &'static str {
    match &row.reading {
        Ok(_) if row.source.is_err() || row.view.is_err() => "no-scale",
        Ok(_) => "accepted",
        Err(why) => why.label(),
    }
}

/// One line per axis over the accepted sites, and the refusals counted.
///
/// A median and a spread, and nothing that assumes these sites are
/// independent: they are a fraction of a degree apart and they are not.
/// `ungated` goes on every median line, not only on the gate's own notice.
/// A reader quoting one line has to see that nothing judged these readings:
/// the notice is four lines up and gets scrolled past.
fn summarize(rows: &[&Row], ungated: bool) {
    let read: Vec<Pixels> = rows.iter().filter_map(|row| pixels(row)).collect();
    let mut refusals: Vec<(&str, usize)> = Vec::new();
    for row in rows {
        let label = status(row);
        if label == "accepted" {
            continue;
        }
        match refusals.iter_mut().find(|(name, _)| *name == label) {
            Some((_, count)) => *count += 1,
            None => refusals.push((label, 1)),
        }
    }
    println!(
        "  accepted {} of {} sites{}",
        read.len(),
        rows.len(),
        refusals
            .iter()
            .map(|(name, count)| format!("; {name} {count}"))
            .collect::<String>(),
    );
    if read.is_empty() {
        return;
    }
    for (name, source, view) in [
        (
            "epi ",
            read.iter().map(|p| p.source.epi).collect::<Vec<_>>(),
            read.iter().map(|p| p.view.epi).collect::<Vec<_>>(),
        ),
        (
            "perp",
            read.iter().map(|p| p.source.perp).collect::<Vec<_>>(),
            read.iter().map(|p| p.view.perp).collect::<Vec<_>>(),
        ),
    ] {
        println!(
            "  {name}:{} median {:+.2} src px (spread {:.2}), {:+.2} view px (spread {:.2}); \
             median magnitude {:.2} src px, {:.2} view px",
            match ungated {
                true => " UNGATED,",
                false => "",
            },
            median(&source),
            deviation(&source),
            median(&view),
            deviation(&view),
            median(&source.iter().map(|v| v.abs()).collect::<Vec<_>>()),
            median(&view.iter().map(|v| v.abs()).collect::<Vec<_>>()),
        );
    }
    // A site the along-seam gate took is not a site the plant may keep: the
    // deltas were computed before the gate ran. The reading on the PERTURBED
    // map is not gated at all, and cannot be: it has a different along-seam
    // value by construction, which is the thing the plant is measuring. So a
    // plant row survives on its base reading's plausibility alone.
    let planted: Vec<(Axes, Axes)> = rows
        .iter()
        .filter(|row| row.reading.is_ok())
        .filter_map(|row| row.plant)
        .collect();
    if !planted.is_empty() {
        report_plant(&planted);
    }
}

fn report_plant(planted: &[(Axes, Axes)]) {
    for (name, measured, predicted) in [
        (
            "epi ",
            planted.iter().map(|(m, _)| m.epi).collect::<Vec<_>>(),
            planted.iter().map(|(_, p)| p.epi).collect::<Vec<_>>(),
        ),
        (
            "perp",
            planted.iter().map(|(m, _)| m.perp).collect::<Vec<_>>(),
            planted.iter().map(|(_, p)| p.perp).collect::<Vec<_>>(),
        ),
    ] {
        let error: Vec<f64> = measured
            .iter()
            .zip(&predicted)
            .map(|(m, p)| (m - p).to_degrees())
            .collect();
        println!(
            "  plant {name}: read median {:+.4} deg against {:+.4} predicted; \
             error median {:+.4} deg (spread {:.4}), over {} sites",
            median(&measured.iter().map(|v| v.to_degrees()).collect::<Vec<_>>()),
            median(&predicted.iter().map(|v| v.to_degrees()).collect::<Vec<_>>()),
            median(&error),
            deviation(&error),
            error.len(),
        );
    }
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    match sorted.len() % 2 {
        0 => (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0,
        _ => sorted[sorted.len() / 2],
    }
}

/// The median absolute deviation, which is the median's own spread: one
/// outlying site widens it by a site's worth and not by its own size.
fn deviation(values: &[f64]) -> f64 {
    let middle = median(values);
    median(
        &values
            .iter()
            .map(|v| (v - middle).abs())
            .collect::<Vec<_>>(),
    )
}

/// Which calibration this view is drawn through. The same three paths `step`
/// and `reframe` expose; the raw pixels stay raw either way.
#[derive(Clone, Copy)]
enum Seam {
    Factory,
    File,
    Stored(SeamFit),
}

impl Seam {
    fn hold(self, scene: &Scene) {
        match self {
            Self::Factory => println!("seam:   factory calibration, no correction"),
            Self::File => scene.fit_seam(true),
            Self::Stored(fit) => scene.use_seam(fit),
        }
    }
}

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
    /// How far a reading's along-seam term may sit from its crossing's own,
    /// in radians. `None` switches the gate off.
    perp_gate: Option<f64>,
    /// An along-seam reference the caller brings, in radians, instead of the
    /// crossing's own middle.
    perp_reference: Option<f64>,
    /// How far to move the calibration, in degrees, to state how far the
    /// answer moves with it. `None` skips the two extra runs.
    dither: Option<f64>,
    null: bool,
    plant: Option<(usize, f64)>,
    seam: Seam,
    /// The along-seam table the picture would be drawn with (issue #103,
    /// stage 9). `Table::REST` unless a run names one, and then this
    /// instrument reads exactly what it read before the stage existed.
    table: kjerag_render::Table,
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
            // A degree of azimuth. The structure this has to resolve is a few
            // pixels wide, and a 55 degree view holds about 55 of these.
            bins: 360,
            // Measured on the owner's 2026-05-01 frame, sweeping each while
            // holding the others: from 1.10 to 3.00 degrees of patch, 0.035
            // to 0.070 of step and 0.3 to 0.5 of agreement, the medians move
            // under a pixel and only how many sites are accepted changes.
            // 2.20 degrees is 69 source px along the seam and 36 across it on
            // this camera family, and takes 7 of 13 sites where 1.10 takes 4.
            //
            // `search` is not that kind of knob. At 2.60 degrees the same
            // frame reads a median magnitude of 18.7 source px with a spread
            // of 19.8, because content two degrees away is allowed to win.
            // A railed site is the honest answer; a wider search is not.
            span: 2.20,
            search: 1.40,
            step: 0.07,
            contrast: 2.0,
            agreement: 0.5,
            // 0.40 degrees, which is 12.6 source px along the seam on this
            // camera family. It is a chosen operating point and not a line
            // between two populations: over 750 accepted readings from three
            // flights, |perp - its crossing's own value| runs p50 1.85 src
            // px, p75 5.13, p90 22.13, and **11.9% of readings sit in the 8
            // to 25 px stretch this cut is in**. An earlier version of this
            // comment called that stretch empty and derived the number from
            // it, which was wrong.
            //
            // What the recorded data does say is that the choice inside that
            // stretch barely matters: put the cut anywhere from 8 to 20 src
            // px and 4.0 to 5.7% of readings change side. So a reading near
            // the cut is a reading whose acceptance is arbitrary, and about
            // one in twenty is. No conclusion should rest on them, and the
            // reproducibility work in the PR is what checks that none does.
            perp_gate: Some(0.40_f64.to_radians()),
            perp_reference: None,
            // A thousandth of a degree: far under anything a calibration is
            // known to, so what comes back is the instrument's own floor
            // rather than a real response. `dither=0` skips the two runs.
            dither: Some(0.001),
            null: false,
            plant: None,
            seam: Seam::File,
            table: kjerag_render::Table::REST,
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
                Some(("span", value)) => out.span = number("span", value)?,
                Some(("search", value)) => out.search = number("search", value)?,
                Some(("step", value)) => out.step = number("step", value)?,
                Some(("contrast", value)) => out.contrast = number("contrast", value)?,
                Some(("ncc", value)) => out.agreement = number("ncc", value)?,
                Some(("perpgate", value)) => {
                    let degrees = number("perpgate", value)?;
                    if degrees < 0.0 {
                        return Err(
                            "perpgate= is a tolerance in degrees, or 0 to switch it off; \
                                    a negative one would refuse every site"
                                .into(),
                        );
                    }
                    out.perp_gate = (degrees > 0.0).then_some(degrees.to_radians());
                }
                Some(("perpref", value)) => {
                    out.perp_reference = Some(number("perpref", value)?.to_radians())
                }
                Some(("dither", value)) => {
                    let degrees = number("dither", value)?;
                    if degrees < 0.0 {
                        return Err("dither= is how far the calibration is moved, in degrees, \
                                    or 0 to skip the two extra runs"
                            .into());
                    }
                    out.dither = (degrees > 0.0).then_some(degrees);
                }
                Some(("null", value)) => out.null = value.parse::<u32>()? != 0,
                Some(("plant", value)) => out.plant = Some(knob(value)?),
                Some(("out", value)) => out.out = Some(value.to_owned()),
                Some(("seam", value)) => {
                    out.seam = match value {
                        "factory" => Seam::Factory,
                        "file" => Seam::File,
                        _ => Seam::Stored(seam_fit(value)?),
                    }
                }
                Some(("table", value)) => out.table = read_table(value)?,
                Some((key, _)) => return Err(format!("no argument called {key}. {USAGE}").into()),
            }
        }
        if out.input.as_os_str().is_empty() {
            return Err(USAGE.into());
        }
        if out.bins == 0 {
            return Err("bins= is how many arc bins the crossing is cut into".into());
        }
        if !out.support().valid() {
            return Err(
                "span=, search= and step= are positive degrees, and the patch and the search \
                 are each at least one step wide"
                    .into(),
            );
        }
        if out.null && out.plant.is_some() {
            return Err("null=1 and plant= are two controls, and each is a run of its own".into());
        }
        if out.perp_reference.is_some() && out.perp_gate.is_none() {
            return Err("perpref= is the value perpgate= judges against, and that is off".into());
        }
        Ok(out)
    }

    fn at(&self) -> Duration {
        Duration::from_secs_f64(self.time)
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

    fn out(&self) -> PathBuf {
        let name = self.out.clone().unwrap_or_else(|| {
            format!(
                "crossing-t{:.3}-yaw{:.0}-pitch{:.0}-fov{:.0}{}.csv",
                self.time,
                self.yaw,
                self.pitch,
                self.fov,
                match (self.null, self.plant) {
                    (true, _) => "-null".to_owned(),
                    (_, Some((knob, amount))) => format!("-plant-{}{amount:+.3}", KNOBS[knob]),
                    _ => String::new(),
                }
            )
        });
        PathBuf::from("scratch").join(name)
    }
}

/// A number a threshold can be made of.
///
/// `"nan"` parses as an `f64` and every comparison against it is false, so a
/// floor set to one lets everything through and a gate set to one refuses
/// nothing, silently. Refuse it here, where it is still a command line.
fn number(name: &str, value: &str) -> Fallible<f64> {
    let parsed: f64 = value.parse()?;
    if !parsed.is_finite() {
        return Err(format!("{name}= must be a finite number, not {value}").into());
    }
    Ok(parsed)
}

fn knob(value: &str) -> Fallible<(usize, f64)> {
    let (name, amount) = value
        .split_once(':')
        .ok_or("a plant is knob:amount, e.g. yaw:0.10")?;
    let index = KNOBS
        .iter()
        .position(|knob| *knob == name)
        .ok_or_else(|| format!("no calibration knob called {name}"))?;
    Ok((index, amount.parse()?))
}

const USAGE: &str = "usage: crossing <file.insv> [time=seconds] [yaw=deg] [pitch=deg] [fov=deg] \
     [lock=1] [size=px] [bins=n] [span=deg] [search=deg] [step=deg] [contrast=codes] [ncc=score] \
     [perpgate=deg | perpgate=0] [perpref=deg] [dither=deg] [null=1] [plant=knob:amount] \
     [seam=factory|file|roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9] [table=table.txt] [out=name.csv]";

/// The view line the app copies is this instrument's command line too, which
/// is the only reason a reported reading can be pointed at a picture the owner
/// looked at. Nothing here restates the format: a field renamed on either side
/// stops parsing or stops matching, and both are this test failing.
#[cfg(test)]
mod tests {
    use kjerag_render::Framing;

    use super::*;

    fn parse(line: &str) -> Options {
        Options::parse(line.split_whitespace().map(str::to_owned))
            .unwrap_or_else(|why| panic!("crossing would not take {line:?}: {why}"))
    }

    #[test]
    fn a_copied_view_line_is_a_crossing_command() {
        let framing = Framing {
            at: Duration::from_millis(50_117),
            camera: Camera {
                yaw: (-74.43_f32).to_radians(),
                pitch: 0.06_f32.to_radians(),
                fov: 55.69_f32.to_radians(),
            },
            horizon: Horizon::Locked,
        };
        let options = parse(&framing.copied(Path::new("/home/pilot/Videos/VID_0001.insv")));
        assert_eq!(options.input, PathBuf::from("VID_0001.insv"));
        assert!(options.lock);
        assert!((options.time - 50.117).abs() < 0.001, "{}", options.time);
        for (parsed, wanted, axis) in [
            (options.camera().yaw, framing.camera.yaw, "yaw"),
            (options.camera().pitch, framing.camera.pitch, "pitch"),
            (options.camera().fov, framing.camera.fov, "fov"),
        ] {
            let off = (parsed - wanted).to_degrees().abs();
            assert!(off < 0.005, "{axis} is {off} degrees out");
        }
    }

    /// A support that cannot hold a patch is rejected before any media is
    /// opened, because the alternative is a run that refuses every site for a
    /// reason that is the command line's.
    #[test]
    fn an_impossible_support_is_refused_at_the_command_line() {
        for line in [
            "f.insv step=0",
            "f.insv span=0.01 step=0.07",
            "f.insv search=0.01 step=0.07",
            "f.insv bins=0",
            "f.insv null=1 plant=yaw:0.1",
            "f.insv plant=twist:0.1",
            // A reference with nothing to judge against it
            "f.insv perpgate=0 perpref=-0.2",
            // Every comparison against a NaN threshold is false, so one that
            // parsed would switch a floor or a gate off without saying so.
            "f.insv ncc=nan",
            "f.insv contrast=nan",
            "f.insv perpgate=nan",
            "f.insv perpref=nan",
            "f.insv dither=nan",
            "f.insv span=inf",
            "f.insv perpgate=-1",
            "f.insv dither=-1",
        ] {
            assert!(
                Options::parse(line.split_whitespace().map(str::to_owned)).is_err(),
                "{line} should not have parsed"
            );
        }
    }

    /// The along-seam gate is on by default, switched off by a zero, and
    /// carries whatever reference the caller declares.
    #[test]
    fn the_along_seam_gate_is_on_unless_it_is_turned_off() {
        let on = parse("f.insv");
        assert!(
            (on.perp_gate.expect("on by default").to_degrees() - 0.40).abs() < 1e-9,
            "{:?}",
            on.perp_gate
        );
        assert_eq!(on.perp_reference, None);
        assert_eq!(parse("f.insv perpgate=0").perp_gate, None);
        let declared = parse("f.insv perpref=-0.205");
        assert!((declared.perp_reference.expect("declared").to_degrees() + 0.205).abs() < 1e-9);
    }

    /// The two crossings of one view are two answers, and the split between
    /// them is a gap in the traced arc rather than a chosen azimuth.
    #[test]
    fn a_view_showing_the_seam_twice_reports_two_crossings() {
        let sites = arc(&[-100.0, -99.0, -98.0, 80.0, 81.0]);
        assert_eq!(crossings(&sites, 360), vec![vec![0, 1, 2], vec![3, 4]]);
        assert_eq!(crossings(&sites[..3], 360), vec![vec![0, 1, 2]]);
        assert!(crossings(&[], 360).is_empty());
    }

    /// One crossing that straddles the wrap is one crossing. The owner's
    /// 2026-05-01 view looks backwards along the body and its arc runs from
    /// `177` through `-180` to `-117`, which a sorted split reports as two.
    #[test]
    fn a_crossing_that_straddles_the_wrap_is_still_one_crossing() {
        let sites = arc(&[-175.1, -170.1, -166.0, 173.5, 177.2]);
        assert_eq!(crossings(&sites, 72), vec![vec![3, 4, 0, 1, 2]]);
    }

    fn arc(azimuths: &[f64]) -> Vec<Site> {
        let mut sites: Vec<Site> = azimuths
            .iter()
            .map(|phi_deg| Site {
                node: crossing::Node {
                    centre: [phi_deg.to_radians().cos(), phi_deg.to_radians().sin(), 0.0],
                    perp: [0.0, 0.0, 1.0],
                    epi: [0.0, 0.0, 1.0],
                    phi: phi_deg.to_radians(),
                },
                view_ray: [0.0, 0.0, 1.0],
                view_pixel: [0.0, 0.0],
            })
            .collect();
        sites.sort_by(|one, other| one.node.phi.total_cmp(&other.node.phi));
        sites
    }

    #[test]
    fn a_median_and_its_spread_are_the_only_summary() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
        assert_eq!(deviation(&[1.0, 2.0, 3.0, 100.0]), 1.0);
    }
}
