//! The across-seam field: the part of the seam's disagreement that does not
//! change, carried as one displacement of lens 1's whole picture instead of as
//! a ramp across the corridor (issue #103, the epi fork;
//! docs/research/seam-ghost.md).
//!
//! **What this is for.** The band measures a per-direction across-seam
//! disagreement and spends it as a ramp: zero at one edge of the handover, the
//! whole of it at the other. Where that disagreement is parallax the ramp is
//! right and stays. Where it is the **camera** - on the owner's May-01
//! downward arc, 0.89 degrees, steady over the whole file - the ramp bends
//! straight lines and sweeps the bend along as the seam moves, which is his
//! standing complaint: we *"migrate/warp the bad stitches together"* where
//! Studio *"just ghosts"*.
//!
//! So the steady part is split off and applied the **along-seam** channel's
//! way, which has displaced lens 1's whole picture since stage 5 and which
//! nobody has ever complained about. What is left over stays in the corridor,
//! ramped, because for real parallax a ramp is what a ramp is for. What
//! neither can carry shows up as soft doubling in the crossfade, which is the
//! principle (docs/research/seam-temporal.md 1: *corrections displace,
//! residuals ghost*).
//!
//! **It is a servo and not an estimator, and that is the design.** The field
//! integrates what the band is **still** reading; it never tries to know what
//! the truth is. Its fixed point is the band having nothing steady left to
//! report. Four things fall out of that and each replaces a piece of machinery
//! #171 had to build and the owner refused:
//!
//! - `|T - truth|` is directly observable, because the residual **is** the
//!   input. stage9 12.3 is emphatic that the safety quantity is the residual
//!   and that nothing knows the truth before the band has measured through the
//!   term; here a wrong field produces a large reading and is corrected by the
//!   next tick, in the direction that reduces it.
//! - There is no harvest. The band already measures all [`AZIMUTHS`]
//!   directions every frame while it draws, so the 27.3 s #171 spent reading
//!   the file before it drew anything is deleted rather than trimmed.
//! - There is no staged walk with abort criteria. A servo never has a whole
//!   field to validate: it only ever moves by a step derived from a live
//!   in-window measurement, so it cannot walk itself outside the window it was
//!   measured in.
//! - The seed and the settling are one line. The first reading at a direction
//!   **is** the estimate; the hundredth is a hundredth of a correction.
//!
//! **The closed loop is real and it is not modelled here.** The band's compute
//! pass reads lens 1 *through* [`Self::table`] (`band.rs`'s `measure`, the
//! `still` term), exactly as `Reframe::tabled` does for the CPU-side sampler,
//! so what a cell reports is already the residual and this file never
//! subtracts anything. The research probe on `research/seam-ghost` had to
//! model that line because it applied nothing; this does not.

use super::band::{AZIMUTHS, Cell, KEEP, SMOOTH_DEG, TAU_TRUST_S, Table, ease};

/// The largest single reading the servo will step by, in degrees.
///
/// A rail on the **step** and not on the value: one correlation that found the
/// wrong feature may not move the field further than the band's own filter
/// would have moved the picture.
const STEP_MAX_DEG: f32 = 0.25;

/// The class bound on the applied term, in degrees (stage9 12.3): the largest
/// term the gain sweep measured to leave the band's evidence intact.
///
/// **It is a rail and not the guard**, and this file does not pretend
/// otherwise. The guard is the residual, which is the servo's own input. This
/// is what stops a field that is not a calibration at all from running away;
/// a direction that reaches it stops integrating. Measured headroom on the
/// owner's own arc: the largest value the probe ever applied was 1.10 degrees.
const RAIL_DEG: f32 = 2.8;

/// How much evidence a direction is believed at full strength, in readings.
///
/// The support the smoothing weighs a direction by is `seen / SUPPORT_FULL`,
/// capped at one, so a direction with nothing contributes nothing to either
/// sum and an unread arc is **exact identity by arithmetic and not by a
/// branch**. Eight because the band's own cell is a two-second average
/// already: eight accepted readings of it is a quarter of a second of agreeing
/// with itself, which is the point past which the scatter is the kernel's job
/// and not the ridge's.
const SUPPORT_FULL: f32 = 8.0;

/// How much a direction with thin support is shrunk by, in directions' worth
/// of support.
///
/// **Chosen for this axis and not inherited, and the difference is
/// measurable.** `band::TABLE_RIDGE` is 1.0 and its argument is that an entry
/// resting on one *reading* should be halved - where a reading there is one
/// correlation of one patch, with 0.05 degrees of scatter on it. A direction
/// here carries [`SUPPORT_FULL`] accepted readings of a cell that is itself a
/// two-second average, so one direction's worth is not one reading's worth and
/// copying the number copies the wrong units. At 0.5 an entry resting on half
/// a direction is halved, and a fully supported arc gives up 10.5 percent of
/// its **arrival rate** against the 19 percent the inherited 1.0 costs.
///
/// It costs arrival rate rather than value, and that is worth stating because
/// the probe's own table reads like a haircut: the loop is closed, so the
/// servo keeps integrating until what is **applied** matches what the band
/// reads, and the ridge slows that rather than biasing where it lands.
const RIDGE: f32 = 0.5;

/// How long the field remembers a direction it has stopped confirming, in
/// seconds of media.
///
/// **This is the forgetting bound, and it is the one thing the design memo did
/// not have.** A correctly-learned field must not outlive its content. The
/// pure `1/n` schedule the memo proposed has no bound at all: a field learned
/// over a minute of near content arrives at gain `1/1800` and survives more
/// than thirty seconds of far-content playback, spending the first eight of
/// them making the corridor's load **worse than no field at all** (measured on
/// `research/seam-ghost`, the `seen=` plant).
///
/// So the evidence leaks. Every direction forgets at this constant on every
/// tick, read or not, and a reading puts one unit back. Two things fall out of
/// one line:
///
/// - **The memory is bounded**, because the leak is also the floor under the
///   gain. A direction can never become so sure of itself that it stops
///   answering the film: whatever evidence a wrong value arrived with, it is
///   walked out at this constant. That is the difference between a bound and
///   a hope, and the `seen=655` plant is what tells them apart.
/// - **A direction that goes dark keeps what it holds and gets its agility
///   back.** The leak stops at [`SUPPORT_FULL`], so an arc that stops
///   correlating is still drawn with what it learned - #172's finding, that a
///   direction failing towards nothing is worse than one failing towards the
///   reading it held - and its gain is back at `1/8`, so the first reading
///   after it comes back is worth an eighth of the answer instead of a
///   hundredth.
///
/// And it makes the readback rate stop being load-bearing, which the memo
/// flagged as a surprise: under `1/n` convergence is counted in ticks, so 2 Hz
/// settled at -0.505 degrees where every frame settled at -0.894. Here the
/// floor is a media-time constant, so a tick at 10 Hz moves the field three
/// times as far as a tick at 30 Hz and the wall clock is the same.
///
/// **Twice [`TAU_TRUST_S`], and that is the relation rather than the number.**
/// What is learned has to be slower than nothing and faster than the picture
/// is allowed to forget, and the picture's own clock is the staging filter's.
/// At twice it, what the field holds is always an average over more film than
/// the walk it is drawn through, so the whole-picture displacement stays the
/// slow thing PR #173's refusal requires it to be, and a field the content has
/// left is a hundredth of itself in five of these.
const TAU_MEMORY_S: f32 = 2.0 * TAU_TRUST_S;

/// The longest tick this will take at face value, in seconds of media.
///
/// `band::Watch::GAP_S`'s own number and its own argument: past it the film is
/// somewhere else, and a leak that took a minute of gap literally would forget
/// the whole ring for a scrub. A seek is otherwise a **no-op** here - the
/// field is a function of direction and not of position in the file, so there
/// is nothing to desynchronize.
const GAP_S: f32 = 0.5;

/// How far the smoothing kernel reaches, in directions.
///
/// Derived from [`SMOOTH_DEG`] and the ring's own spacing rather than chosen,
/// and one further out so the last tap is the zero the raised cosine has at
/// its own edge. `the_kernel_is_zero_at_its_own_reach` checks the arithmetic.
const REACH: usize = (SMOOTH_DEG as usize * AZIMUTHS) / 360 + 1;

/// The live across-seam field: what it has learned, how much it is believed,
/// and what the picture is drawn with.
///
/// Three arrays of [`AZIMUTHS`] and nothing else, which is 1.5 kB per session
/// on the CPU. It rides into the picture inside the uniform block
/// `queue.write_buffer` writes every redraw anyway, so it costs no upload at
/// all (`Reframe::epi`, 512 bytes).
///
/// Per session and per file, never pooled: stage9 10.12 refused a pooled
/// per-azimuth table with a number, and six flights disagree at a given
/// azimuth by 0.597 degrees at the median against a pooled amplitude of 0.229
/// rms.
#[derive(Clone, Debug, PartialEq)]
pub struct Ghost {
    /// The per-direction estimate, in radians. Integrated from what the band
    /// still reads, so its fixed point is the band reading nothing steady.
    field: [f32; AZIMUTHS],
    /// Evidence at each direction, leaking at [`TAU_MEMORY_S`]. Both the gain
    /// schedule and the support the taper is read off, because they are the
    /// same question asked twice.
    seen: [f32; AZIMUTHS],
    /// What the picture is drawn with: [`Self::field`] smoothed along azimuth,
    /// tapered to nothing where there is no support, and walked in through the
    /// staging filter.
    applied: [f32; AZIMUTHS],
}

impl Default for Ghost {
    fn default() -> Self {
        Self::rest()
    }
}

impl Ghost {
    /// A session that has read nothing, which draws `main`'s picture byte for
    /// byte: [`Self::table`] is [`Table::REST`] and `Reframe` short-circuits on
    /// it.
    pub const fn rest() -> Self {
        Self {
            field: [0.0; AZIMUTHS],
            seen: [0.0; AZIMUTHS],
            applied: [0.0; AZIMUTHS],
        }
    }

    /// A field that is wrong by a known constant before the servo reads
    /// anything, which is the control: a servo that cannot walk one out is not
    /// a servo.
    ///
    /// `seen` is how much evidence the wrong field arrives **with**, and it is
    /// the difference between the easy control and the honest one. A field
    /// planted at rest is walked out at gain 1 on its first reading; a field
    /// the servo spent a minute of near content learning correctly arrives at
    /// gain `1/n` with `n` large. Both are the same wrong number in the
    /// picture and they are two very different clocks, which is why
    /// [`TAU_MEMORY_S`] exists.
    pub fn planted(radians: f32, seen: f32) -> Self {
        Self {
            field: [radians; AZIMUTHS],
            seen: [seen; AZIMUTHS],
            applied: [radians; AZIMUTHS],
        }
    }

    /// One readback tick: what the band is still reading, integrated.
    ///
    /// `seconds` is the media time since the last tick, so the field settles in
    /// the same wall clock whatever the readback rate is and a paused window
    /// does not age it.
    ///
    /// **The reading is already the residual.** The band's compute pass
    /// searches lens 1 through what [`Self::table`] applies, so `cell.disparity`
    /// is what the term still leaves and nothing is modelled here. What the
    /// band would have read with **nothing** applied is that plus the applied
    /// value, and it is that sum the two questions below are asked about: the
    /// far gate, because a distance is read off the whole disagreement and not
    /// off a remainder, and the step, for the reason in the next paragraph.
    ///
    /// **The staging filter is deliberately OUTSIDE this loop, and that is the
    /// one thing the design memo had wrong.** The memo's servo integrated
    /// `disparity - applied`, which puts a two second lag inside an integrator:
    /// the field goes on winding up while the picture is still walking towards
    /// where it was sent, so it sails past the answer and rings back. The loop
    /// is second order with a damping ratio of `0.5 * sqrt(TAU_MEMORY_S /
    /// TAU_TRUST_S)`, so at the first reading's gain of 1 it is a ratio of
    /// 0.065 and it rings for half a minute. `the_servo_does_not_ring` is that
    /// failure, kept as a test.
    ///
    /// So the servo is told where the picture is **heading** rather than where
    /// it has got to. `want` is the value the staging filter is walking each
    /// direction towards, and the outstanding error is measured against that.
    /// The filter then walks `applied` to `want` exactly as before - the field
    /// is still drawn as a fade and no single reading can still reach the
    /// picture - but it is no longer in the feedback path, and the gain
    /// schedule is free to be as hot as an estimator's again.
    pub fn tick(&mut self, cells: &[Cell], seconds: f32) {
        let step = STEP_MAX_DEG.to_radians();
        let rail = RAIL_DEG.to_radians();
        let seconds = seconds.clamp(0.0, GAP_S);
        let forget = ease(seconds, TAU_MEMORY_S);
        // Where the picture is heading, from what is known before this tick.
        let want = self.smoothed();
        for (index, cell) in cells.iter().enumerate().take(AZIMUTHS) {
            // Every direction forgets, read or not, but only ever back DOWN to
            // a direction's worth of support and never past it. See
            // TAU_MEMORY_S: this is the whole of the bounded memory, and the
            // floor is #172's own rule read one level out - a direction that
            // stops confirming its reading fails towards the reading it held
            // and not towards nothing, because what stopped is the evidence
            // and not the camera. A direction that has never been read is at
            // zero and this cannot lift it, which is what keeps an unread arc
            // at exact identity.
            self.seen[index] -= (self.seen[index] - SUPPORT_FULL).max(0.0) * forget;
            // The band's own gate on whether a reading may enter a state, read
            // one level out. A direction below it contributes nothing at all,
            // which is what keeps an unread arc at identity.
            if cell.confidence < KEEP || !cell.disparity.is_finite() {
                continue;
            }
            // THE FAR GATE, and it is the reason this ships on by default.
            //
            // Parallax on this axis is one-signed and positive: the baseline
            // runs along the lens axes, so a near subject is displaced towards
            // the front lens at every azimuth and `Cell::metres` is
            // `reach_m / disparity` for a positive disparity only. A reading
            // short of zero is therefore a distance no content can stand at
            // and is the camera; a reading past zero is the camera plus
            // whatever the scene put on top of it, and no distance
            // discriminates those two.
            //
            // So the field refuses to be moved by one at all. Measured on the
            // landing chapter, where the ground is three metres away and every
            // reading is past zero: the hazard the ungated servo learns there,
            // +0.240 degrees of field off ground that is about to leave, comes
            // down to EXACT IDENTITY. At cruise it costs 0.001 degrees,
            // because there is nothing there for it to remove.
            //
            // What the band reads through an applied term is the residual, so
            // the quantity the sign is asked about is the sum.
            let whole = cell.disparity + self.applied[index];
            if whole > 0.0 {
                continue;
            }
            self.seen[index] += 1.0;
            // `1/n` is the seed and the settling in one line - the first
            // reading at a direction IS the estimate, the hundredth is a
            // hundredth of a correction - and the leak is its FLOOR, which is
            // what bounds the memory. A direction can never be so sure of
            // itself that it stops answering the film: the slowest this can
            // walk a wrong value out is [`TAU_MEMORY_S`], whatever evidence
            // the value arrived with.
            let gain = (1.0 / self.seen[index]).clamp(forget, 1.0);
            // Against where the picture is heading, not where it has got to.
            let moved = self.field[index] + gain * (whole - want[index]).clamp(-step, step);
            self.field[index] = moved.clamp(-rail, rail);
        }
        // Arrival, through the staging filter's own class on a different
        // quantity: the field is drawn as a fade and never as a step, which is
        // the second half of why no single reading can reach the picture.
        let learn = ease(seconds, TAU_TRUST_S);
        for (applied, want) in self.applied.iter_mut().zip(want) {
            *applied += (want - *applied) * learn;
        }
    }

    /// The field smoothed along azimuth and tapered by its own support.
    ///
    /// The raised cosine and the ridge are [`Table`]'s own, because this is the
    /// same job: a kernel that is zero at and past its own edge, so a direction
    /// walking into a window does not put a corner in the picture, and a ridge
    /// that takes an entry with less than its share of support to nothing.
    ///
    /// **This is what PR #173's refusal requires.** No cell walks on its own:
    /// every direction's value is a weighted mean over roughly six degrees of
    /// azimuth either side, so the field has **no teeth to comb**. The notch
    /// shape #172 deepened and #173 failed to fix is a shape that only exists
    /// where neighbouring cells move independently, and it cannot form in a
    /// field of this construction.
    fn smoothed(&self) -> [f32; AZIMUTHS] {
        let step = std::f32::consts::TAU / AZIMUTHS as f32;
        let width = SMOOTH_DEG.to_radians();
        // The kernel's own weights, worked out once for the whole ring rather
        // than per direction, and only over the directions it reaches. The
        // research probe's first cut swept all 128 against all 128 and cost
        // 98.7 us a tick; the arithmetic is the same and the cost is not.
        let weights: [f32; 2 * REACH + 1] =
            std::array::from_fn(|slot| kernel(slot as f32 - REACH as f32, width / step));
        std::array::from_fn(|index| {
            let mut total = 0.0;
            let mut weight = 0.0;
            for (slot, tap) in weights.iter().enumerate() {
                if *tap <= 0.0 {
                    continue;
                }
                let other = (index as i32 + slot as i32 - REACH as i32).rem_euclid(AZIMUTHS as i32)
                    as usize;
                let support = (self.seen[other] / SUPPORT_FULL).min(1.0);
                total += tap * support * self.field[other];
                weight += tap * support;
            }
            total / (weight + RIDGE)
        })
    }

    /// What the picture is drawn with, as the uniform block carries it.
    ///
    /// [`Table::REST`] on a session that has read nothing, which every caller
    /// short-circuits on, so a file's first frames are `main`'s picture byte
    /// for byte.
    pub fn table(&self) -> Table {
        Table::of_entries(self.applied)
    }

    /// The estimate, per direction, in radians. For an instrument: the picture
    /// is drawn with [`Self::table`].
    pub fn field(&self) -> [f32; AZIMUTHS] {
        self.field
    }

    /// How much evidence each direction is holding, in readings. For an
    /// instrument.
    pub fn seen(&self) -> [f32; AZIMUTHS] {
        self.seen
    }
}

/// The raised cosine [`Table`] smooths with, in **taps** rather than radians:
/// zero at and past its own edge.
fn kernel(apart: f32, reach: f32) -> f32 {
    match apart.abs() < reach {
        true => 0.5 * (1.0 + (std::f32::consts::PI * apart / reach).cos()),
        false => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cell the band would be happy with, reading `degrees` across the seam.
    fn reading(degrees: f32) -> Cell {
        Cell {
            disparity: degrees.to_radians(),
            confidence: 1.0,
            reach_m: 0.033,
            ..Cell::default()
        }
    }

    fn ring(degrees: f32) -> Vec<Cell> {
        vec![reading(degrees); AZIMUTHS]
    }

    /// One thirtieth of a second, which is the tick the app takes.
    const FRAME_S: f32 = 1.0 / 30.0;

    /// Run the loop closed against a truth, at a readback rate, and keep what
    /// the picture was drawn with at every tick.
    ///
    /// The one line the research probe had to model is the one this does not:
    /// what the band reads is what the term still leaves, which is the truth
    /// minus what is applied, because the compute pass searches through it.
    fn run(ghost: &mut Ghost, degrees: f32, seconds: f32, tick: f32) -> Vec<f32> {
        (0..(seconds / tick) as usize)
            .map(|_| {
                let cells: Vec<Cell> = ghost
                    .applied
                    .iter()
                    .map(|applied| reading(degrees - applied.to_degrees()))
                    .collect();
                ghost.tick(&cells, tick);
                ghost.applied[0].to_degrees()
            })
            .collect()
    }

    fn settle(ghost: &mut Ghost, degrees: f32, seconds: f32) {
        run(ghost, degrees, seconds, FRAME_S);
    }

    #[test]
    fn a_session_that_has_read_nothing_draws_exactly_nothing() {
        assert!(Ghost::rest().table().is_rest());
        // And a ring of readings the band itself refuses leaves it there.
        let mut ghost = Ghost::rest();
        let refused = vec![
            Cell {
                confidence: KEEP * 0.5,
                disparity: (-1.0f32).to_radians(),
                ..Cell::default()
            };
            AZIMUTHS
        ];
        for _ in 0..300 {
            ghost.tick(&refused, FRAME_S);
        }
        assert!(ghost.table().is_rest(), "a refused ring moved the picture");
    }

    /// The kernel's last tap is the zero the raised cosine has at its own edge,
    /// so [`REACH`] is derived and not chosen.
    #[test]
    fn the_kernel_is_zero_at_its_own_reach() {
        let step = std::f32::consts::TAU / AZIMUTHS as f32;
        let reach = SMOOTH_DEG.to_radians() / step;
        assert_eq!(kernel(REACH as f32, reach), 0.0);
        assert!(kernel(REACH as f32 - 1.0, reach) > 0.0);
    }

    /// The fixed point is the band reading nothing, and the servo reaches it
    /// from cold on the loop closed.
    #[test]
    fn the_servo_settles_where_the_band_has_nothing_left_to_report() {
        let mut ghost = Ghost::rest();
        settle(&mut ghost, -0.9, 20.0);
        let applied = ghost.applied[0].to_degrees();
        assert!(
            (applied - -0.9).abs() < 0.05,
            "settled at {applied:.4} deg against a truth of -0.9"
        );
    }

    /// THE FAR GATE. A ring of positive readings is near content by
    /// construction, and the field refuses to learn one at all.
    #[test]
    fn a_reading_past_zero_cannot_move_the_field() {
        let mut ghost = Ghost::rest();
        // The landing chapter's own size: ground at about three metres.
        for _ in 0..900 {
            ghost.tick(&ring(0.63), FRAME_S);
        }
        assert!(
            ghost.table().is_rest(),
            "near content moved the field to {:.4} deg",
            ghost.applied[0].to_degrees()
        );
    }

    /// And the gate is asked about what the band would have read with nothing
    /// applied, not about the residual: a settled field reading its own noise
    /// must not ratchet.
    #[test]
    fn the_gate_reads_the_sum_and_not_the_residual() {
        let mut ghost = Ghost::planted((-0.9f32).to_radians(), 64.0);
        // A residual of +0.1 deg against an applied -0.9 is a total of -0.8,
        // which no distance can produce, so it is accepted and pulls the field
        // back up rather than being refused as near content.
        let before = ghost.field[0];
        for _ in 0..300 {
            ghost.tick(&ring(0.1), FRAME_S);
        }
        assert!(
            ghost.field[0] > before,
            "a negative-total reading was refused: {before:.6} to {:.6}",
            ghost.field[0]
        );
    }

    /// THE CONTROL. A field that is wrong before the servo has read anything is
    /// walked out in seconds, which is the whole warrant for there being no
    /// abort criteria: a servo's input IS the residual stage9 12.3 named as
    /// the safety quantity, so a wrong field is corrected rather than refused.
    #[test]
    fn a_wrong_field_planted_at_rest_is_gone_in_seconds() {
        // Planted WITH support, because a field with no evidence behind it is
        // tapered to identity by the ridge and would walk out for the wrong
        // reason - which would be the control passing for the wrong reason.
        // This is the control the memo states: half a degree, wrong.
        let plant = 0.5f32;
        let mut ghost = Ghost::planted(plant.to_radians(), SUPPORT_FULL);
        let track = run(&mut ghost, 0.0, 12.0, FRAME_S);
        // Measured: 0.2764 at 2 s, 0.0632 at 6, 0.0107 at 12.
        let at = |seconds: f32| track[(seconds / FRAME_S) as usize].abs();
        assert!(at(2.0) < plant * 0.6, "still {:.4} after 2 s", at(2.0));
        assert!(at(6.0) < plant * 0.2, "still {:.4} after 6 s", at(6.0));
        assert!(at(11.9) < plant * 0.03, "still {:.4} after 12 s", at(11.9));
    }

    /// THE LOOP IS STABLE, which nothing in the record had ever run.
    ///
    /// The design memo's servo integrated `disparity - applied`, which puts the
    /// two second staging filter inside the integrator and rings: measured
    /// here, it overshot a -0.9 degree truth to -1.29 and was still 0.29
    /// degrees the wrong side of it four seconds later. Against a truth
    /// approached from one side, a stable loop never crosses it by more than
    /// the ridge's own share.
    #[test]
    fn the_servo_does_not_ring() {
        let mut ghost = Ghost::rest();
        let track = run(&mut ghost, -0.9, 40.0, FRAME_S);
        let worst = track.iter().fold(0.0f32, |worst, at| worst.max(at.abs()));
        assert!(
            worst < 0.9 * 1.05,
            "the picture overshot -0.9 deg to {worst:.4}"
        );
        let settled = *track.last().expect("a track");
        assert!(
            (settled - -0.9).abs() < 0.05,
            "it settled at {settled:.4} rather than on the truth"
        );
    }

    /// THE POISON PLANT, which is the hazard the control above does not price:
    /// a field the servo learned CORRECTLY off content that has since left,
    /// arriving at a gain the memo's own schedule would never let it out of.
    ///
    /// [`TAU_MEMORY_S`] is what bounds it, so the bound is what is asserted.
    #[test]
    fn a_field_learned_off_content_that_has_left_is_forgotten_within_the_bound() {
        // The discriminator's own plant: what a minute of near ground teaches
        // the ungated servo, arriving with the evidence that minute gave it.
        // Under the memo's unbounded `1/n` this was still 0.119 degrees after
        // THIRTY seconds of far-content playback.
        let planted = 0.24f32;
        let mut ghost = Ghost::planted(planted.to_radians(), 655.0);
        let track = run(&mut ghost, 0.0, TAU_MEMORY_S * 5.0, FRAME_S);
        let at = |seconds: f32| track[((seconds / FRAME_S) as usize).min(track.len() - 1)].abs();
        // The bound is the constant, and it does not depend on what the value
        // arrived with: measured 0.0268 at three of them and 0.0046 at five.
        assert!(
            at(TAU_MEMORY_S * 3.0) < planted * 0.15,
            "still {:.4} of {planted} after three memory constants",
            at(TAU_MEMORY_S * 3.0),
        );
        assert!(
            at(TAU_MEMORY_S * 5.0) < planted * 0.03,
            "still {:.4} of {planted} after five",
            at(TAU_MEMORY_S * 5.0),
        );
        // And it never gets worse on the way, which is the other half of the
        // finding: the ungated servo spent its first eight seconds making the
        // corridor's load worse than no field at all.
        assert!(
            track
                .windows(2)
                .all(|pair| pair[1].abs() <= pair[0].abs() + 1e-6),
            "the plant grew before it shrank",
        );
    }

    /// The rail stops a runaway and the direction stops integrating at it.
    #[test]
    fn the_field_cannot_walk_past_the_rail() {
        let mut ghost = Ghost::rest();
        for _ in 0..6000 {
            ghost.tick(&ring(-9.0), FRAME_S);
        }
        assert!(
            ghost.field[0].to_degrees() >= -RAIL_DEG - 1e-4,
            "the field reached {:.4} deg past a rail of {RAIL_DEG}",
            ghost.field[0].to_degrees()
        );
    }

    /// An arc with evidence does not put a corner in the picture at its edge:
    /// the field is smooth by construction and this is the arithmetic of it.
    #[test]
    fn an_unread_arc_is_identity_and_its_neighbours_taper_into_it() {
        let mut ghost = Ghost::rest();
        let arc = 40..60;
        let mut cells = vec![Cell::default(); AZIMUTHS];
        for index in arc.clone() {
            cells[index] = reading(-0.9);
        }
        for _ in 0..600 {
            ghost.tick(&cells, FRAME_S);
        }
        let applied = ghost.applied;
        // Far from the arc, exactly nothing.
        assert_eq!(applied[0], 0.0);
        assert_eq!(applied[100], 0.0);
        // Inside it, the field.
        assert!(applied[50].to_degrees() < -0.5);
        // And no step anywhere: the largest jump between neighbours is a
        // fraction of what the arc itself carries.
        let worst = (0..AZIMUTHS)
            .map(|index| (applied[(index + 1) % AZIMUTHS] - applied[index]).abs())
            .fold(0.0f32, f32::max);
        assert!(
            worst < applied[50].abs() * 0.35,
            "the edge of the arc steps by {:.4} deg of {:.4}",
            worst.to_degrees(),
            applied[50].to_degrees(),
        );
    }

    /// The staging filter is what walks the field in, so a direction's first
    /// reading cannot reach the picture whole.
    #[test]
    fn no_single_reading_reaches_the_picture() {
        let mut ghost = Ghost::rest();
        ghost.tick(&ring(-2.0), FRAME_S);
        let applied = ghost.applied[0].to_degrees();
        assert!(
            applied.abs() < 0.05,
            "one reading of -2 deg put {applied:.4} on the picture"
        );
    }

    /// AN ARC THAT GOES DARK KEEPS WHAT IT LEARNED, which is #172's own
    /// finding and the one thing the forgetting must not undo.
    ///
    /// The first cut of the leak took `seen` to zero, so a direction that
    /// stopped correlating had its support tapered away and the picture gave
    /// the correction back while the camera error was still there. Measured on
    /// the owner's own down1 arc, which goes dark seventeen seconds in: the
    /// field fell from -0.776 to -0.412 degrees for no reason but the dark.
    #[test]
    fn an_arc_that_stops_correlating_keeps_what_it_learned() {
        let mut ghost = Ghost::rest();
        settle(&mut ghost, -0.9, 20.0);
        let learned = ghost.applied[0].to_degrees();
        // Thirty seconds of a seam nothing correlates on.
        let dark = vec![Cell::default(); AZIMUTHS];
        for _ in 0..900 {
            ghost.tick(&dark, FRAME_S);
        }
        let kept = ghost.applied[0].to_degrees();
        assert!(
            (kept - learned).abs() < 0.01,
            "thirty seconds of dark took {learned:.4} deg to {kept:.4}"
        );
    }

    /// The wall clock is the same at any readback rate, which the `1/n`
    /// schedule alone could not manage.
    #[test]
    fn the_readback_rate_is_not_load_bearing() {
        let landed = |rate: f32| {
            let mut ghost = Ghost::rest();
            run(&mut ghost, -0.9, 20.0, 1.0 / rate);
            ghost.applied[0].to_degrees()
        };
        let (fast, slow) = (landed(30.0), landed(10.0));
        assert!(
            (fast - slow).abs() < 0.05,
            "30 Hz landed at {fast:.4} and 10 Hz at {slow:.4}"
        );
    }
}
