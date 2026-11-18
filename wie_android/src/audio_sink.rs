use std::{
    io::Cursor,
    num::NonZero,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    thread,
    time::Duration,
};

use oxisynth::{InterpolationMethod, MidiEvent, SoundFont, Synth, SynthDescriptor};
use rodio::{
    ChannelCount, DeviceSinkBuilder, SampleRate, Source,
    mixer::{MixerSource, mixer},
};

use crate::audio_mix::{SYNTH_GAIN, clip_trim, sfx_trim};
use crate::audio_quality::{ClipCache, MixDynamics};

static AUDIO_STATUS: OnceLock<Mutex<String>> = OnceLock::new();
static MUSIC_VOLUME: AtomicU8 = AtomicU8::new(100);
static SF3_BYTES: &[u8] = include_bytes!("../assets/hero3_ma3.sf3");
const CHUNK_FRAMES: usize = 64;
const POLYPHONY: u16 = 64;

pub type WaveMsg = (u8, u32, Vec<i16>);
pub type MidiMsg = MidiEvent;

fn status_cell() -> &'static Mutex<String> {
    AUDIO_STATUS.get_or_init(|| Mutex::new("audio thread not started".into()))
}

fn set_status(s: String) {
    if let Ok(mut g) = status_cell().lock() {
        *g = s;
    }
}

pub fn audio_status() -> String {
    status_cell().lock().map(|g| g.clone()).unwrap_or_else(|_| "?".into())
}

pub fn set_music_level(level: u8) {
    let v = level.min(5).saturating_mul(20);
    if MUSIC_VOLUME.swap(v, Ordering::Relaxed) != v {
        log::info!("audio: music bus level {v} from game options");
    }
}

pub fn spawn_audio_thread(paused: Arc<AtomicBool>, lite: bool) -> (Sender<WaveMsg>, Sender<MidiMsg>) {
    let (wave_tx, wave_rx) = channel::<WaveMsg>();
    let (midi_tx, midi_rx) = channel::<MidiMsg>();
    thread::Builder::new()
        .name("wie-audio".into())
        .spawn(move || audio_thread(wave_rx, midi_rx, paused, lite))
        .expect("failed to spawn audio thread");
    (wave_tx, midi_tx)
}

fn audio_thread(wave_rx: Receiver<WaveMsg>, midi_rx: Receiver<MidiMsg>, paused: Arc<AtomicBool>, lite: bool) {
    set_status("opening output device...".into());
    let output_sink = {
        let mut attempt = 0u32;
        loop {
            match DeviceSinkBuilder::open_default_sink() {
                Ok(s) => break s,
                Err(e) => {
                    attempt += 1;
                    log::error!("audio: open_default_sink failed (attempt {attempt}): {e:?}");
                    set_status(format!("output open FAILED x{attempt}: {e:?}"));
                    while wave_rx.try_recv().is_ok() {}
                    while midi_rx.try_recv().is_ok() {}
                    thread::sleep(Duration::from_secs(if attempt < 5 { 1 } else { 10 }));
                }
            }
        }
    };
    let device_rate = output_sink.config().sample_rate();
    let rate = NonZero::new(device_rate.get().clamp(8_000, 96_000)).unwrap();
    let (music, music_out) = mixer(NonZero::new(2).unwrap(), rate);
    let (effects, effects_out) = mixer(NonZero::new(2).unwrap(), rate);
    match build_synth(lite, rate.get()) {
        Ok(synth) => {
            music.add(SynthSource::new(synth, midi_rx, rate, Arc::clone(&paused)));
            let quality = if lite { "linear/lite" } else { "7th-order" };
            let message = format!(
                "OK (MA-3 {quality}, sinc SFX, {} Hz / device {} Hz, gain {SYNTH_GAIN})",
                rate.get(),
                device_rate.get()
            );
            log::info!("audio: {message}");
            set_status(message);
        }
        Err(e) => {
            log::error!("audio: synth load failed ({e}); MIDI BGM disabled");
            set_status(format!("output open, but synth load FAILED: {e} (BGM off, SFX on)"));
        }
    }
    output_sink.mixer().add(MasterLimiter {
        music: music_out,
        effects: effects_out,
        paused: Arc::clone(&paused),
        dynamics: MixDynamics::new(rate.get(), CHUNK_FRAMES),
        buf: [0.0; CHUNK_FRAMES * 2],
        pos: CHUNK_FRAMES * 2,
    });
    let mut cache = ClipCache::default();
    while let Ok((channels, source_rate, wave_data)) = wave_rx.recv() {
        if paused.load(Ordering::Relaxed)
            || !(1..=2).contains(&channels)
            || source_rate == 0
            || wave_data.is_empty()
            || wave_data.len() % channels as usize != 0
        {
            continue;
        }
        let gain = sfx_trim(crate::sfx_level()) * clip_trim(source_rate, wave_data.len());
        let data = cache.prepare(&wave_data, channels as usize, source_rate, rate.get());
        if paused.load(Ordering::Relaxed) {
            continue;
        }
        effects.add(ClipSource {
            data,
            pos: 0,
            channels: NonZero::new(channels as u16).unwrap(),
            rate,
            gain,
            paused: Arc::clone(&paused),
        });
    }
}

struct ClipSource {
    data: Arc<[f32]>,
    pos: usize,
    channels: ChannelCount,
    rate: SampleRate,
    gain: f32,
    paused: Arc<AtomicBool>,
}

impl Iterator for ClipSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.paused.load(Ordering::Relaxed) {
            self.pos = self.data.len();
        }
        let sample = self.data.get(self.pos).copied()?;
        self.pos += 1;
        Some(sample * self.gain)
    }
}

impl Source for ClipSource {
    fn current_span_len(&self) -> Option<usize> {
        Some(self.data.len() - self.pos)
    }
    fn channels(&self) -> ChannelCount {
        self.channels
    }
    fn sample_rate(&self) -> SampleRate {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(
            self.data.len() as f64 / self.channels.get() as f64 / self.rate.get() as f64,
        ))
    }
}

struct MasterLimiter {
    music: MixerSource,
    effects: MixerSource,
    paused: Arc<AtomicBool>,
    dynamics: MixDynamics,
    buf: [f32; CHUNK_FRAMES * 2],
    pos: usize,
}

impl Iterator for MasterLimiter {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.pos == self.buf.len() {
            let mut effects = [0.0; CHUNK_FRAMES * 2];
            for (m, e) in self.buf.iter_mut().zip(&mut effects) {
                *m = self.music.next().unwrap_or(0.0);
                *e = self.effects.next().unwrap_or(0.0);
            }
            self.dynamics.process(&mut self.buf, &effects, self.paused.load(Ordering::Relaxed));
            self.pos = 0;
        }
        let sample = self.buf[self.pos];
        self.pos += 1;
        Some(if self.paused.load(Ordering::Relaxed) { 0.0 } else { sample })
    }
}

impl Source for MasterLimiter {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        self.music.channels()
    }
    fn sample_rate(&self) -> SampleRate {
        self.music.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

fn build_synth(lite: bool, sample_rate: u32) -> Result<Synth, String> {
    let mut synth = Synth::new(SynthDescriptor {
        sample_rate: sample_rate as f32,
        polyphony: POLYPHONY,
        gain: SYNTH_GAIN,
        chorus_active: !lite,
        reverb_active: !lite,
        interpolation: if lite {
            InterpolationMethod::Linear
        } else {
            InterpolationMethod::SeventhOrder
        },
        ..Default::default()
    })
    .map_err(|e| format!("Synth::new: {e:?}"))?;
    let font = SoundFont::load(&mut Cursor::new(SF3_BYTES)).map_err(|e| format!("SoundFont::load: {e:?}"))?;
    synth.add_font(font, true);
    Ok(synth)
}

struct SynthSource {
    synth: Synth,
    midi_rx: Receiver<MidiMsg>,
    sample_rate: SampleRate,
    paused: Arc<AtomicBool>,
    buf: [f32; CHUNK_FRAMES * 2],
    pos: usize,
}

impl SynthSource {
    fn new(synth: Synth, midi_rx: Receiver<MidiMsg>, sample_rate: SampleRate, paused: Arc<AtomicBool>) -> Self {
        Self {
            synth,
            midi_rx,
            sample_rate,
            paused,
            buf: [0.0; CHUNK_FRAMES * 2],
            pos: CHUNK_FRAMES * 2,
        }
    }

    fn render_chunk(&mut self) {
        if self.paused.load(Ordering::Relaxed) {
            self.buf.fill(0.0);
        } else {
            while let Ok(ev) = self.midi_rx.try_recv() {
                let _ = self.synth.send_event(ev);
            }
            let gain = wie_backend::volume_gain(MUSIC_VOLUME.load(Ordering::Relaxed));
            for frame in self.buf.chunks_exact_mut(2) {
                let (l, r) = self.synth.read_next();
                frame[0] = l * gain;
                frame[1] = r * gain;
            }
        }
        self.pos = 0;
    }
}

impl Iterator for SynthSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.pos == self.buf.len() {
            self.render_chunk();
        }
        let sample = self.buf[self.pos];
        self.pos += 1;
        Some(sample)
    }
}

impl Source for SynthSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        NonZero::new(2).unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

pub struct AndroidAudioSink {
    wave_tx: Sender<WaveMsg>,
    midi_tx: Sender<MidiMsg>,
}

impl AndroidAudioSink {
    pub fn new(wave_tx: Sender<WaveMsg>, midi_tx: Sender<MidiMsg>) -> Self {
        Self { wave_tx, midi_tx }
    }
}

impl wie_backend::AudioSink for AndroidAudioSink {
    fn play_wave(&self, channel: u8, sampling_rate: u32, wave_data: &[i16]) {
        let _ = self.wave_tx.send((channel, sampling_rate, wave_data.to_vec()));
    }

    fn midi_note_on(&self, channel_id: u8, note: u8, velocity: u8) {
        // MIDI convention: note-on with velocity 0 == note-off.
        let ev = if velocity == 0 {
            MidiEvent::NoteOff {
                channel: channel_id,
                key: note,
            }
        } else {
            MidiEvent::NoteOn {
                channel: channel_id,
                key: note,
                vel: velocity,
            }
        };
        let _ = self.midi_tx.send(ev);
    }

    fn midi_note_off(&self, channel_id: u8, note: u8, _velocity: u8) {
        let _ = self.midi_tx.send(MidiEvent::NoteOff {
            channel: channel_id,
            key: note,
        });
    }

    fn midi_program_change(&self, channel_id: u8, program: u8) {
        let _ = self.midi_tx.send(MidiEvent::ProgramChange {
            channel: channel_id,
            program_id: program,
        });
    }

    fn midi_control_change(&self, channel_id: u8, control: u8, value: u8) {
        let _ = self.midi_tx.send(MidiEvent::ControlChange {
            channel: channel_id,
            ctrl: control,
            value,
        });
    }

    fn midi_pitch_bend(&self, channel_id: u8, value: u16) {
        let _ = self.midi_tx.send(MidiEvent::PitchBend { channel: channel_id, value });
    }

    fn midi_sysex(&self, _data: &[u8]) {
        // oxisynth has no raw-sysex event; the game's sysex is a GM reset, which
        // the synth already defaults to. Safe to ignore.
    }

    fn set_music_volume(&self, volume: u8) -> bool {
        let v = volume.min(100);
        // Logged only on an actual change, which is once per 배경음 setting change —
        // so it is the one line that says what the music bus is doing without being
        // chatty. (Effects never reach here: see `Score::is_music`.)
        if MUSIC_VOLUME.swap(v, Ordering::Relaxed) != v {
            log::info!("audio: music bus level {v} (gain {:.3})", wie_backend::volume_gain(v));
        }
        // We own a real music bus, so the player must leave note velocity alone.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;

    #[test]
    fn master_stays_alive_without_music_and_mutes_active_effects() {
        let rate = NonZero::new(48_000).unwrap();
        let channels = NonZero::new(2).unwrap();
        let (_music, music) = mixer(channels, rate);
        let (effects, effects_out) = mixer(channels, rate);
        let paused = Arc::new(AtomicBool::new(false));
        let mut master = MasterLimiter {
            music,
            effects: effects_out,
            paused: Arc::clone(&paused),
            dynamics: MixDynamics::new(rate.get(), CHUNK_FRAMES),
            buf: [0.0; CHUNK_FRAMES * 2],
            pos: CHUNK_FRAMES * 2,
        };
        for _ in 0..256 {
            assert_eq!(master.next(), Some(0.0));
        }
        effects.add(ClipSource {
            data: vec![0.2; 48_000].into(),
            pos: 0,
            channels,
            rate,
            gain: 1.0,
            paused: Arc::clone(&paused),
        });
        assert_eq!(master.next(), Some(0.2));
        paused.store(true, Ordering::Relaxed);
        for _ in 0..256 {
            assert_eq!(master.next(), Some(0.0));
        }
        paused.store(false, Ordering::Relaxed);
        for _ in 0..256 {
            assert_eq!(master.next(), Some(0.0));
        }
        effects.add(SamplesBuffer::new(channels, rate, vec![0.1; 256]));
        assert!((0..256).any(|_| master.next() == Some(0.1)));
    }

    #[test]
    fn shared_clip_restarts_without_copying_and_ends_permanently_on_pause() {
        let paused = Arc::new(AtomicBool::new(false));
        let mut source = ClipSource {
            data: vec![0.2, 0.4, 0.6].into(),
            pos: 0,
            channels: NonZero::new(1).unwrap(),
            rate: NonZero::new(48_000).unwrap(),
            gain: 0.5,
            paused: Arc::clone(&paused),
        };
        assert_eq!(source.next(), Some(0.1));
        paused.store(true, Ordering::Relaxed);
        assert_eq!(source.next(), None);
        paused.store(false, Ordering::Relaxed);
        assert_eq!(source.next(), None);
    }

    #[test]
    fn shipping_font_renders_seventh_order_at_both_device_rates() {
        for rate in [44_100, 48_000] {
            let mut synth = build_synth(false, rate).unwrap();
            synth
                .send_event(MidiEvent::ControlChange {
                    channel: 0,
                    ctrl: 0,
                    value: 0x7c,
                })
                .unwrap();
            synth
                .send_event(MidiEvent::ControlChange {
                    channel: 0,
                    ctrl: 32,
                    value: 1,
                })
                .unwrap();
            synth
                .send_event(MidiEvent::ProgramChange {
                    channel: 0,
                    program_id: 0x6a,
                })
                .unwrap();
            synth
                .send_event(MidiEvent::NoteOn {
                    channel: 0,
                    key: 60,
                    vel: 100,
                })
                .unwrap();
            let mut peak = 0.0f32;
            for _ in 0..rate {
                let (l, r) = synth.read_next();
                assert!(l.is_finite() && r.is_finite());
                peak = peak.max(l.abs()).max(r.abs());
            }
            assert!(peak > 0.001);
        }
    }
}
