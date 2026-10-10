//! Optional diagnostics sample counters, never advance playback or wait for work.

use std::time::{Duration, Instant};

use kjerag_render::{PlaybackSnapshot, Stats};

pub(crate) const INTERVAL: Duration = Duration::from_millis(500);

#[derive(Default)]
pub(crate) struct Sampler {
    previous: Option<(Instant, Stats, bool)>,
    pub snapshot: Option<PlaybackSnapshot>,
    /// Source promotions and playback checks per second, not physical scanout.
    pub rates: Option<(f64, f64)>,
}

impl Sampler {
    pub fn sample(&mut self, now: Instant, snapshot: PlaybackSnapshot) {
        let media = snapshot.media;
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
}
