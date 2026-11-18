use std::{fs, io::Cursor, num::NonZero, path::Path, time::Instant};

use hero3_audio_verify::{
    audio_mix::{SYNTH_GAIN, clip_trim, declick, peak_abs, sfx_trim},
    audio_quality::ClipCache,
};
use oxisynth::{InterpolationMethod, MidiEvent, SoundFont, Synth, SynthDescriptor};
use smaf_player::{SmafEvent, parse_smaf};

fn midi(event: &SmafEvent) -> Option<MidiEvent> {
    Some(match *event {
        SmafEvent::MidiNoteOn { channel, note, velocity } => MidiEvent::NoteOn {
            channel,
            key: note,
            vel: velocity,
        },
        SmafEvent::MidiNoteOff { channel, note, .. } => MidiEvent::NoteOff { channel, key: note },
        SmafEvent::MidiProgramChange { channel, program } => MidiEvent::ProgramChange {
            channel,
            program_id: program,
        },
        SmafEvent::MidiControlChange { channel, control, value } => MidiEvent::ControlChange {
            channel,
            ctrl: control,
            value,
        },
        SmafEvent::MidiPitchBend { channel, value } => MidiEvent::PitchBend { channel, value },
        _ => return None,
    })
}

fn render(events: &[(usize, SmafEvent)], rate: u32, interpolation: InterpolationMethod, font: &[u8]) -> (f32, f64, f64) {
    let mut synth = Synth::new(SynthDescriptor {
        sample_rate: rate as f32,
        interpolation,
        gain: SYNTH_GAIN,
        polyphony: 64,
        ..Default::default()
    })
    .unwrap();
    synth.add_font(SoundFont::load(&mut Cursor::new(font)).unwrap(), true);
    let duration = events.last().unwrap().0 + 2_000;
    let count = duration as u64 * rate as u64 / 1_000;
    let mut index = 0;
    let mut peak = 0.0f32;
    let mut power = 0.0;
    let started = Instant::now();
    for frame in 0..count {
        while index < events.len() && events[index].0 as u64 * rate as u64 <= frame * 1_000 {
            if let Some(event) = midi(&events[index].1) {
                synth.send_event(event).unwrap();
            }
            index += 1;
        }
        let (l, r) = synth.read_next();
        assert!(l.is_finite() && r.is_finite());
        peak = peak.max(l.abs()).max(r.abs());
        power += l as f64 * l as f64 + r as f64 * r as f64;
    }
    (peak, 10.0 * (power / (count * 2) as f64).log10(), started.elapsed().as_secs_f64())
}

fn main() {
    let directory = std::env::args().nth(1).expect("usage: hero3_audio_verify <extracted res/snd directory>");
    let font = include_bytes!("../../../wie_android/assets/hero3_ma3.sf3");
    println!("kind,name,rate,peak,rms_dbfs,render_seconds,rms_change_db");
    let songs = if std::env::args().any(|arg| arg == "--sfx-only") { 0 } else { 19 };
    for song in 0..songs {
        let name = format!("bgm{song}_mf");
        let data = fs::read(Path::new(&directory).join(&name)).unwrap();
        let events = parse_smaf(&data);
        assert!(!events.is_empty(), "empty score: {name}");
        let baseline = render(&events, 44_100, InterpolationMethod::FourthOrder, font);
        println!("bgm_old,{name},44100,{:.6},{:.3},{:.3},0", baseline.0, baseline.1, baseline.2);
        for rate in [44_100, 48_000] {
            let output = render(&events, rate, InterpolationMethod::SeventhOrder, font);
            println!(
                "bgm_new,{name},{rate},{:.6},{:.3},{:.3},{:.3}",
                output.0,
                output.1,
                output.2,
                output.1 - baseline.1
            );
            assert!((output.1 - baseline.1).abs() < 1.0, "BGM loudness drift: {name}");
        }
    }
    let mut cache = ClipCache::default();
    for effect in 0..14 {
        let name = format!("sd{effect:03}_mf");
        let data = fs::read(Path::new(&directory).join(&name)).unwrap();
        let events = parse_smaf(&data);
        let mut waves = 0;
        for (_, event) in events {
            if let SmafEvent::Wave {
                channel,
                sampling_rate,
                data,
            } = event
            {
                waves += 1;
                let gain = sfx_trim(hero3_audio_verify::sfx_level()) * clip_trim(sampling_rate, data.len());
                let mut original: Vec<_> = data.iter().map(|&s| s as f32 / 32768.0).collect();
                declick(&mut original, channel as usize, sampling_rate);
                let rms = |s: &[f32]| 10.0 * (s.iter().map(|&x| (x as f64 * gain as f64).powi(2)).sum::<f64>() / s.len() as f64).log10();
                let old: Vec<_> = rodio::conversions::SampleRateConverter::new(
                    original.iter().copied(),
                    NonZero::new(sampling_rate).unwrap(),
                    NonZero::new(44_100).unwrap(),
                    NonZero::new(channel as u16).unwrap(),
                )
                .collect();
                let baseline = rms(&old);
                for rate in [44_100, 48_000] {
                    let started = Instant::now();
                    let output = cache.prepare(&data, channel as usize, sampling_rate, rate);
                    assert!(output.iter().all(|v| v.is_finite()));
                    let db = rms(&output);
                    println!(
                        "sfx_new,{name},{rate},{:.6},{db:.3},{:.6},{:.3}",
                        peak_abs(&output) * gain,
                        started.elapsed().as_secs_f64(),
                        db - baseline
                    );
                    assert!((db - baseline).abs() < 1.5, "SFX loudness drift: {name}");
                }
            }
        }
        assert!(waves > 0, "empty effect: {name}");
    }
}
