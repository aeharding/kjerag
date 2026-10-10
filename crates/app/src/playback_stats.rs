//! Optional diagnostics sample counters, never advance playback or wait for work.

use std::time::{Duration, Instant};

use kjerag_render::{PlaybackSnapshot, Stats};

pub(crate) const INTERVAL: Duration = Duration::from_millis(500);

#[derive(Default)]
pub(crate) struct Sampler {
    previous: Option<(Instant, Stats, bool)>,
    previous_input: Option<(Instant, u64)>,
    pub snapshot: Option<PlaybackSnapshot>,
    /// Source promotions and playback checks per second, not physical scanout.
    pub rates: Option<(f64, f64)>,
    /// File-input megabits/sec, independent of queue drain or playback state.
    pub input_mbps: Option<f64>,
}

impl Sampler {
    pub fn sample(&mut self, now: Instant, snapshot: PlaybackSnapshot) {
        let media = snapshot.media;
        let read_bytes = media.input.and_then(|input| input.read_bytes);
        self.input_mbps = self.previous_input.and_then(|(at, before)| {
            let over = now.checked_duration_since(at)?.as_secs_f64();
            let bytes = read_bytes?.checked_sub(before)?;
            (over > 0.0).then(|| bytes as f64 * 8.0 / over / 1_000_000.0)
        });
        self.previous_input = read_bytes.map(|bytes| (now, bytes));
        let running = media.playing
            && !media.buffering
            && !media.seeking
            && !media.preparing
            && !snapshot.stopped;
        self.rates = self.previous.and_then(|(at, before, was_running)| {
            let over = now.checked_duration_since(at)?.as_secs_f64();
            if !running
                || !was_running
                || over == 0.0
                || media.stats.presented < before.presented
                || media.stats.redraws < before.redraws
            {
                return None;
            }
            let delta = media.stats.since(before);
            Some((delta.presented as f64 / over, delta.redraws as f64 / over))
        });
        self.previous = Some((now, media.stats, running));
        self.snapshot = Some(snapshot);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use kjerag_render::PlaybackDiagnostics;

    use super::*;

    pub(crate) fn snapshot(presented: u64, redraws: u64) -> PlaybackSnapshot {
        PlaybackSnapshot {
            media: PlaybackDiagnostics {
                stats: Stats {
                    presented,
                    redraws,
                    ..Stats::default()
                },
                input: None,
                decoded: 0,
                audio_queued: None,
                has_audio: false,
                source_fps: 29.97,
                playing: true,
                buffering: false,
                seeking: false,
                preparing: false,
                ended: false,
            },
            stitched_ready: Some(0),
            stopped: false,
        }
    }

    #[test]
    fn rates_use_elapsed_time_and_dont_call_checks_video_frames() {
        let now = Instant::now();
        let mut sampler = Sampler::default();
        sampler.sample(now, snapshot(100, 200));
        assert_eq!(sampler.rates, None);
        sampler.sample(now + Duration::from_secs(2), snapshot(160, 440));
        assert_eq!(sampler.rates, Some((30.0, 120.0)));
    }

    #[test]
    fn pause_buffering_and_resume_do_not_average_in_stopped_time() {
        let now = Instant::now();
        let mut sampler = Sampler::default();
        sampler.sample(now, snapshot(10, 20));
        let mut held = snapshot(10, 30);
        held.media.buffering = true;
        sampler.sample(now + INTERVAL, held);
        assert_eq!(sampler.rates, None);
        held.media.buffering = false;
        held.media.playing = false;
        sampler.sample(now + INTERVAL * 2, held);
        assert_eq!(sampler.rates, None);
        sampler.sample(now + INTERVAL * 3, snapshot(11, 40));
        assert_eq!(sampler.rates, None);
        sampler.sample(now + INTERVAL * 4, snapshot(26, 70));
        assert_eq!(sampler.rates, Some((30.0, 60.0)));
    }

    #[test]
    fn zero_time_and_replaced_counters_do_not_make_false_rates() {
        let now = Instant::now();
        let mut sampler = Sampler::default();
        sampler.sample(now, snapshot(100, 200));
        sampler.sample(now, snapshot(101, 201));
        assert_eq!(sampler.rates, None);
        sampler.sample(now + INTERVAL, snapshot(1, 2));
        assert_eq!(sampler.rates, None);
    }

    fn input_snapshot(bytes: u64) -> PlaybackSnapshot {
        let mut snapshot = snapshot(0, 0);
        snapshot.media.input = Some(kjerag_render::InputBuffer {
            read_bytes: Some(bytes),
            ..Default::default()
        });
        snapshot
    }

    #[test]
    fn input_rate_uses_decimal_megabits_and_elapsed_time_not_queue_fill() {
        let now = Instant::now();
        let mut sampler = Sampler::default();
        sampler.sample(now, input_snapshot(1_000_000));
        assert_eq!(sampler.input_mbps, None);
        let mut next = input_snapshot(19_750_000);
        next.media.input.as_mut().unwrap().bytes = 0;
        sampler.sample(now + INTERVAL, next);
        assert_eq!(sampler.input_mbps, Some(300.0));
        next.media.input.as_mut().unwrap().bytes = 100_000_000;
        sampler.sample(now + INTERVAL * 2, next);
        assert_eq!(
            sampler.input_mbps,
            Some(0.0),
            "full/cache changes are not IO"
        );
    }

    #[test]
    fn input_rate_keeps_measuring_during_pause_and_buffering() {
        let now = Instant::now();
        let mut sampler = Sampler::default();
        sampler.sample(now, input_snapshot(0));
        let mut paused = input_snapshot(10_000_000);
        paused.media.playing = false;
        sampler.sample(now + INTERVAL, paused);
        assert_eq!(sampler.rates, None);
        assert_eq!(sampler.input_mbps, Some(160.0));
        let mut buffering = input_snapshot(20_000_000);
        buffering.media.buffering = true;
        sampler.sample(now + INTERVAL * 2, buffering);
        assert_eq!(sampler.input_mbps, Some(160.0));
    }

    #[test]
    fn input_rate_restarts_after_missing_seek_or_replaced_observations() {
        let now = Instant::now();
        let mut sampler = Sampler::default();
        sampler.sample(now, input_snapshot(20_000_000));
        let mut seeking = snapshot(0, 0);
        seeking.media.seeking = true;
        sampler.sample(now + INTERVAL, seeking);
        assert_eq!(sampler.input_mbps, None);
        sampler.sample(now + INTERVAL * 2, input_snapshot(30_000_000));
        assert_eq!(sampler.input_mbps, None);
        sampler.sample(now + INTERVAL * 3, input_snapshot(0));
        assert_eq!(sampler.input_mbps, None);
        sampler.sample(now + INTERVAL * 4, input_snapshot(10_000_000));
        assert_eq!(sampler.input_mbps, Some(160.0));
        sampler.sample(now + INTERVAL * 4, input_snapshot(20_000_000));
        assert_eq!(sampler.input_mbps, None);
        sampler.sample(now, input_snapshot(30_000_000));
        assert_eq!(sampler.input_mbps, None);
    }
}
