use std::{collections::VecDeque, sync::Arc};

use crate::audio_mix::{
    DUCK_ATTACK_MS, DUCK_RELEASE_MS, MUSIC_ENV_RELEASE_MS, declick, duck_hold_next, duck_next_to, duck_step, duck_target, music_env_decay,
    music_env_next, peak_abs, soft_limit,
};

pub fn resample(samples: &[f32], channels: usize, from: u32, to: u32) -> Vec<f32> {
    if channels == 0 || from == 0 || to == 0 || samples.len() % channels != 0 {
        return Vec::new();
    }
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let frames = samples.len() / channels;
    let output_frames = ((frames as u64 * to as u64 + from as u64 / 2) / from as u64) as usize;
    let cutoff = (to as f64 / from as f64).min(1.0) * 0.94;
    let radius = (24.0 / cutoff).ceil() as i64;
    let mut a = from;
    let mut b = to;
    while b != 0 {
        (a, b) = (b, a % b);
    }
    let phases = to / a;
    let taps = (radius * 2) as usize;
    let mut kernel = Vec::with_capacity(phases as usize * taps);
    for phase in 0..phases {
        let fraction = phase as f64 / phases as f64;
        let start = kernel.len();
        let mut sum = 0.0;
        for tap in -radius + 1..=radius {
            let x = tap as f64 - fraction;
            let p = std::f64::consts::PI * x * cutoff;
            let sinc = if p.abs() < 1e-12 { 1.0 } else { p.sin() / p };
            let window = if x.abs() <= radius as f64 {
                0.42 + 0.5 * (std::f64::consts::PI * x / radius as f64).cos() + 0.08 * (2.0 * std::f64::consts::PI * x / radius as f64).cos()
            } else {
                0.0
            };
            let weight = (cutoff * sinc * window) as f32;
            kernel.push(weight);
            sum += weight;
        }
        for weight in &mut kernel[start..] {
            *weight /= sum;
        }
    }
    let mut out = vec![0.0; output_frames * channels];
    for frame in 0..output_frames {
        let position = frame as u64 * from as u64;
        let center = (position / to as u64) as i64;
        let phase = ((position % to as u64) / a as u64) as usize;
        for tap in 0..taps {
            let source = center + tap as i64 - radius + 1;
            if source < 0 || source >= frames as i64 {
                continue;
            }
            let weight = kernel[phase * taps + tap];
            for channel in 0..channels {
                out[frame * channels + channel] += samples[source as usize * channels + channel] * weight;
            }
        }
    }
    // Zero-padding is used outside the clip; taper only the reconstructed boundaries.
    let edge = ((to as usize / 2000).max(1)).min(output_frames / 2);
    for frame in 0..edge {
        let gain = frame as f32 / edge as f32;
        for channel in 0..channels {
            out[frame * channels + channel] *= gain;
            out[(output_frames - 1 - frame) * channels + channel] *= gain;
        }
    }
    out
}

struct CachedClip {
    channels: usize,
    from: u32,
    to: u32,
    input: Vec<i16>,
    output: Arc<[f32]>,
}

impl CachedClip {
    fn bytes(&self) -> usize {
        self.input.len() * 2 + self.output.len() * 4
    }
}

#[derive(Default)]
pub struct ClipCache {
    entries: VecDeque<CachedClip>,
    bytes: usize,
}

impl ClipCache {
    const MAX_BYTES: usize = 16 * 1024 * 1024;
    const MAX_ENTRIES: usize = 96;

    pub fn prepare(&mut self, input: &[i16], channels: usize, from: u32, to: u32) -> Arc<[f32]> {
        if let Some(index) = self
            .entries
            .iter()
            .position(|e| e.channels == channels && e.from == from && e.to == to && e.input == input)
        {
            let entry = self.entries.remove(index).unwrap();
            let output = Arc::clone(&entry.output);
            self.entries.push_back(entry);
            return output;
        }
        let mut samples: Vec<_> = input.iter().map(|&v| v as f32 / 32768.0).collect();
        declick(&mut samples, channels, from);
        let output: Arc<[f32]> = resample(&samples, channels, from, to).into();
        let size = input.len() * 2 + output.len() * 4;
        if size <= Self::MAX_BYTES {
            while self.bytes + size > Self::MAX_BYTES || self.entries.len() >= Self::MAX_ENTRIES {
                self.bytes -= self.entries.pop_front().unwrap().bytes();
            }
            self.bytes += size;
            self.entries.push_back(CachedClip {
                channels,
                from,
                to,
                input: input.to_vec(),
                output: Arc::clone(&output),
            });
        }
        output
    }
}

pub struct MixDynamics {
    gain: f32,
    hold: f32,
    music_env: f32,
    attack: f32,
    release: f32,
    decay: f32,
}

impl MixDynamics {
    pub fn new(rate: u32, frames: usize) -> Self {
        Self {
            gain: 1.0,
            hold: 1.0,
            music_env: 0.0,
            attack: duck_step(DUCK_ATTACK_MS, rate),
            release: duck_step(DUCK_RELEASE_MS, rate),
            decay: music_env_decay(MUSIC_ENV_RELEASE_MS, rate, frames),
        }
    }

    pub fn process(&mut self, music: &mut [f32], effects: &[f32], paused: bool) {
        assert_eq!(music.len(), effects.len());
        if paused {
            music.fill(0.0);
            self.gain = 1.0;
            self.hold = 1.0;
            self.music_env = 0.0;
            return;
        }
        self.music_env = music_env_next(self.music_env, peak_abs(music), self.decay);
        let peak = peak_abs(effects);
        self.hold = duck_hold_next(self.hold, duck_target(self.music_env, peak), peak > 0.00001);
        for (music_frame, effect_frame) in music.chunks_mut(2).zip(effects.chunks(2)) {
            self.gain = duck_next_to(self.gain, self.hold, self.attack, self.release);
            for (m, e) in music_frame.iter_mut().zip(effect_frame) {
                *m = soft_limit(*m * self.gain + *e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, hz: f32) -> Vec<f32> {
        (0..rate)
            .map(|n| (2.0 * std::f32::consts::PI * hz * n as f32 / rate as f32).sin() * 0.5)
            .collect()
    }

    fn amplitude(samples: &[f32], rate: u32, hz: f64) -> f64 {
        let margin = rate as usize / 20;
        let range = margin..samples.len() - margin;
        let count = range.len() as f64;
        let (mut re, mut im) = (0.0, 0.0);
        for n in range {
            let angle = 2.0 * std::f64::consts::PI * hz * n as f64 / rate as f64;
            re += samples[n] as f64 * angle.cos();
            im += samples[n] as f64 * angle.sin();
        }
        2.0 * re.hypot(im) / count
    }

    #[test]
    fn sinc_preserves_passband_and_rejects_upsampling_images() {
        for from in [4_000, 8_000] {
            for to in [44_100, 48_000] {
                let hz = from as f32 * 0.3;
                let output = resample(&tone(from, hz), 1, from, to);
                let fundamental = amplitude(&output, to, hz as f64);
                let image = amplitude(&output, to, (from as f32 - hz) as f64);
                assert!((fundamental - 0.5).abs() < 0.005, "{from}->{to}: {fundamental}");
                assert!(image < 0.0005, "{from}->{to}: image {image}");
                assert_eq!(output.len(), to as usize);
                assert_eq!(output[0], 0.0);
                assert_eq!(*output.last().unwrap(), 0.0);
            }
        }
    }

    #[test]
    fn downsampling_rejects_aliases() {
        let output = resample(&tone(48_000, 9_000.0), 1, 48_000, 8_000);
        assert!(amplitude(&output, 8_000, 1_000.0) < 0.0005);
    }

    #[test]
    fn stereo_channels_and_impulse_timing_survive_conversion() {
        let mut input = vec![0.0; 800 * 2];
        input[400 * 2] = 1.0;
        let output = resample(&input, 2, 8_000, 48_000);
        assert!(output.iter().skip(1).step_by(2).all(|&v| v == 0.0));
        let peak = output
            .iter()
            .step_by(2)
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap()
            .0;
        assert_eq!(peak, 2_400);
    }

    #[test]
    fn resampling_handles_empty_short_and_invalid_input() {
        assert!(resample(&[], 1, 8_000, 48_000).is_empty());
        assert!(resample(&[0.5], 0, 8_000, 48_000).is_empty());
        assert!(resample(&[0.5], 2, 8_000, 48_000).is_empty());
        assert!(resample(&[0.5], 1, 0, 48_000).is_empty());
        assert_eq!(resample(&[0.2, -0.3], 1, 48_000, 48_000), [0.2, -0.3]);
        assert!(resample(&[0.5], 1, 8_000, 48_000).iter().all(|v| v.is_finite()));
    }

    #[test]
    fn cache_reuses_exact_pcm_and_distinguishes_volume_and_rate() {
        let mut cache = ClipCache::default();
        let a = cache.prepare(&[0, 1_000, -1_000, 0], 1, 8_000, 48_000);
        let again = cache.prepare(&[0, 1_000, -1_000, 0], 1, 8_000, 48_000);
        assert!(Arc::ptr_eq(&a, &again));
        let quiet = cache.prepare(&[0, 500, -500, 0], 1, 8_000, 48_000);
        let other_rate = cache.prepare(&[0, 1_000, -1_000, 0], 1, 8_000, 44_100);
        assert!(!Arc::ptr_eq(&a, &quiet));
        assert!(!Arc::ptr_eq(&a, &other_rate));
    }

    #[test]
    fn overlapping_effects_budget_the_sum_and_recover() {
        let mut dynamics = MixDynamics::new(48_000, 64);
        for _ in 0..200 {
            let mut music = [0.4; 128];
            dynamics.process(&mut music, &[0.2; 128], false);
        }
        assert_eq!(dynamics.gain, 1.0);
        for _ in 0..200 {
            let mut music = [0.4; 128];
            dynamics.process(&mut music, &[0.4; 128], false);
            assert!(music.iter().all(|&v| v < 1.0));
        }
        assert!((dynamics.gain - 0.75).abs() < 0.001);
        for _ in 0..1_000 {
            dynamics.process(&mut [0.4; 128], &[0.0; 128], false);
        }
        assert_eq!(dynamics.gain, 1.0);
    }

    #[test]
    fn cache_eviction_keeps_playing_buffers_alive() {
        let mut cache = ClipCache::default();
        let input = vec![100; 800_000];
        let playing = cache.prepare(&input, 1, 48_000, 48_000);
        for value in [200, 300, 400] {
            cache.prepare(&vec![value; 800_000], 1, 48_000, 48_000);
        }
        assert!(cache.bytes <= ClipCache::MAX_BYTES);
        assert_eq!(playing.len(), input.len());
        let rebuilt = cache.prepare(&input, 1, 48_000, 48_000);
        assert!(!Arc::ptr_eq(&playing, &rebuilt));
        assert_eq!(&*playing, &*rebuilt);
    }

    #[test]
    fn tiny_clips_cannot_grow_cache_metadata_without_bound() {
        let mut cache = ClipCache::default();
        for value in 0..200 {
            cache.prepare(&[value], 1, 8_000, 48_000);
        }
        assert_eq!(cache.entries.len(), ClipCache::MAX_ENTRIES);
        assert!(cache.bytes <= ClipCache::MAX_BYTES);
    }

    #[test]
    fn pause_silences_both_buses_and_resets_duck() {
        let mut dynamics = MixDynamics::new(44_100, 64);
        dynamics.gain = 0.5;
        let mut music = [0.4; 128];
        dynamics.process(&mut music, &[0.8; 128], true);
        assert_eq!(music, [0.0; 128]);
        assert_eq!(dynamics.gain, 1.0);
        dynamics.process(&mut music, &[0.2; 128], false);
        assert_eq!(music, [0.2; 128]);
    }
}
