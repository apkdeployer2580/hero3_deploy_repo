// Standalone self-test of the audio master-bus math.
//
//   rustc --edition 2021 --test audio_mix_selftest.rs -o tt_audio.exe && ./tt_audio.exe
//
// ★ This `include!`s the REAL source, not a copy — so the gain constants and the
// limiter curve under test are exactly the ones the app ships.
// `wie_android/src/audio_mix.rs` must therefore stay dependency-free (same contract as
// endpoint.rs / buffs.rs / tls_buf.rs; see CLAUDE.md).
//
// Why standalone at all: `cargo test -p wie_android` dies at startup on this Windows
// host because of the linked winit/android deps.

include!("wie_android/src/audio_mix.rs");

// ── the volume law ───────────────────────────────────────────────────────────
// A mirror of `wie_backend::volume_gain`, which cannot be `include!`d here (it lives in
// the vendored no_std crate alongside the trait it serves). `law_matches_wie_backend`
// below pins it to that source text, so the two cannot drift apart silently — this is
// the mistake `items_selftest.rs` made and 44차 paid for.
fn volume_gain(volume: u8) -> f32 {
    let v = volume.min(100) as f32 / 100.0;
    v * v
}

#[cfg(test)]
mod law {
    use super::*;

    #[test]
    fn law_matches_wie_backend() {
        // Read the real definition and check the body still says what the mirror says.
        let src = std::fs::read_to_string("vendor/wie/wie_backend/src/audio_sink.rs")
            .expect("run this from the repo root: rustc --test audio_mix_selftest.rs");
        let body = src
            .split("pub fn volume_gain(volume: u8) -> f32 {")
            .nth(1)
            .expect("wie_backend::volume_gain has been renamed or removed — update this mirror");
        let body: String = body.split("\n}").next().unwrap().split_whitespace().collect();
        assert_eq!(
            body, "letv=volume.min(100)asf32/100.0;v*v",
            "wie_backend::volume_gain changed; update the mirror in this file"
        );
    }

    #[test]
    fn the_five_in_game_steps() {
        // Hero3 sends level*20 (probe-confirmed: `MC_mdaClipSetVolume(clip, 60)` for
        // both buses at the default settings).
        let want = [0.0f32, 0.04, 0.16, 0.36, 0.64, 1.0];
        for (level, want) in want.iter().enumerate() {
            let got = volume_gain((level * 20) as u8);
            assert!((got - want).abs() < 1e-6, "level {level}: got {got}, want {want}");
        }
    }

    #[test]
    fn both_buses_answer_to_the_same_law() {
        // The whole point of the fix: at any given level the two buses are scaled by
        // the SAME number, so "배경음 3 / 효과음 3" means one thing and not two.
        //
        // What it used to be: PCM got a LINEAR vol/100, MIDI got velocity*vol/100 which
        // the SoundFont's concave velocity->attenuation curve turns into roughly the
        // square of that. Level 3 therefore meant -4.4 dB on one bus and about -8.9 dB
        // *plus* a 14 dB synth-gain deficit on the other.
        let level3 = volume_gain(60);
        let old_pcm_linear = 0.6f32;
        assert!(level3 < old_pcm_linear, "the effect bus must get quieter at mid levels");
        let db = |v: f32| 20.0 * v.log10();
        assert!((db(level3) - db(old_pcm_linear) + 4.44).abs() < 0.05);
    }

    #[test]
    fn full_scale_effect_plus_music_cannot_clip_the_device() {
        // The end-to-end guarantee for "효과음이 나면 배경음이 끊긴다": four of the
        // game's fourteen effects decode to a literal 0 dBFS peak, and the loudest
        // measured BGM peak at unity synth gain is 2.625 (bgm3_mf, MA-3 bank with the
        // retuned scores; the same song was 1.182 when the font was GeneralUser).
        for level in [0u8, 20, 40, 60, 80, 100] {
            let g = volume_gain(level);
            let sfx = 1.0 * g * SFX_TRIM;
            let bgm = 2.625 * SYNTH_GAIN * g;
            let out = soft_limit(sfx + bgm);
            assert!(out.abs() < 1.0, "level {level}: {} -> {out} must stay inside full scale", sfx + bgm);
        }
    }

    #[test]
    fn music_is_far_louder_than_it_was_at_every_level() {
        // Old chain: synth gain 0.2, and the level applied to velocity (~squared).
        // New chain: the level applied to the output (squared), so the velocity path and
        // the output path land on the same exponent and what is left is the synth gain
        // that was being thrown away.
        //
        // ★ 2026-09-04 — the comparison is now made in LOUDNESS, not in the raw gain
        // number. `SYNTH_GAIN` dropped 1.0 -> 0.38 when the bundled bank became the
        // game's own MA-3 tones, which are +8.3 dB hotter once the scores are retuned
        // onto them (`tools_re/patch_bgm_ma3.py`); the two
        // changes cancel, and the point of this test — "the music is no longer 14 dB
        // down from where it should be" — is unchanged.
        let bank_offset_db = 8.33_f32; // MA-3 + retuned scores minus GeneralUser, at unity
        for level in [20u8, 40, 60, 80, 100] {
            let old = 0.2 * volume_gain(level);
            let new = SYNTH_GAIN * volume_gain(level);
            let gain_db = 20.0 * (new / old).log10() + bank_offset_db;
            assert!((gain_db - 13.98).abs() < 0.2, "level {level}: {gain_db:+.2} dB");
        }
    }
}

// ── 효과음 레벨 행 (2026-08-27) ──────────────────────────────────────────────
//
// The row exists because one global trim cannot be right on two handsets: an S25 Ultra
// player asked for effects DOWN (that is how `SFX_TRIM` became 0.4) and a Fold 6 player
// then reported them inaudible. These tests pin the two properties that make the row
// safe to ship — the default is bit-identical to what shipped, and the top step still
// leaves the master limiter some headroom.

#[test]
fn the_default_level_is_exactly_what_shipped() {
    // ⛔ The whole promise of the row: a player who never touches it hears no change.
    assert_eq!(sfx_trim(SFX_LEVEL_DEFAULT), SFX_TRIM);
    assert_eq!(SFX_LEVELS[SFX_LEVEL_DEFAULT as usize], SFX_TRIM);
    assert_eq!(sfx_level_label(SFX_LEVEL_DEFAULT), "기본");
}

#[test]
fn the_levels_are_three_db_apart_and_ordered() {
    for w in SFX_LEVELS.windows(2) {
        assert!(w[1] > w[0], "levels must increase: {w:?}");
        let db = 20.0 * (w[1] / w[0]).log10();
        assert!((db - 3.0).abs() < 0.05, "each step is 3 dB, got {db}");
    }
}

#[test]
fn the_loudest_level_still_leaves_the_limiter_room() {
    // ⛔ Not 1.0. Four of the game's effect samples decode to a literal 0 dBFS peak, so a
    // unity effect bus hands `soft_limit` a full-scale source and the music underneath it
    // starts disappearing — symptom ③ of the 2026-08-16 report, straight back.
    let top = SFX_LEVELS[SFX_LEVELS.len() - 1];
    assert!(top < 1.0, "the top step must not be unity");
    assert!(top <= LIMIT_KNEE + (1.0 - LIMIT_KNEE) / 2.0, "a 0 dBFS sample at the top step must stay near the knee");
}

#[test]
fn an_out_of_range_level_falls_back_to_the_shipped_default() {
    // A hand-edited file, or a build that shrank the table. Silence and full scale are
    // both worse than "unchanged".
    for bad in [SFX_LEVELS.len() as u8, 99, u8::MAX] {
        assert_eq!(sfx_trim(bad), SFX_TRIM, "level {bad} must fall back");
        assert_eq!(sfx_level_label(bad), "기본");
    }
}

#[test]
fn the_cycle_visits_every_level_and_wraps() {
    let mut seen = vec![];
    let mut lv = 0u8;
    for _ in 0..SFX_LEVELS.len() {
        seen.push(lv);
        lv = sfx_level_next(lv);
    }
    assert_eq!(lv, 0, "the cycle must wrap back to the first step");
    let want: Vec<u8> = (0..SFX_LEVELS.len() as u8).collect();
    assert_eq!(seen, want, "every step must be reachable from the row");
}

// ── 효과음이 울릴 때의 음악 덕킹 (2026-08-27) ────────────────────────────────
//
// `soft_limit` is on the SUM, so before this the loudest music simply erased the effect
// on top of it (-20.2 dB measured at both sliders max on `bgm3_mf`). These tests pin the
// envelope's two properties that matter in the real-time path: it always reaches its
// target, and it can never leave the music parked below unity.

#[test]
fn the_duck_reaches_its_target_in_about_the_stated_time() {
    let rate = 44_100;
    let atk = duck_step(DUCK_ATTACK_MS, rate);
    let rel = duck_step(DUCK_RELEASE_MS, rate);
    // ⚠ A RELATIVE tolerance, not "+/-1 frame". The envelope is an `f32` accumulator, so
    // its error grows with the number of additions it makes — and 2026-08-30 lengthened
    // both times by an order of magnitude (12 -> 120 ms, 140 -> 400 ms) to stop the duck
    // being audible as a step. At 400 ms the 17,640 additions land 16 frames early, which
    // is 0.09 % and inaudible; the absolute bound simply encoded the old, shorter times.
    let close = |n: u32, want: u32, what: &str| {
        let slack = (want as f32 * 0.005).max(1.0) as u32;
        assert!(
            n + slack >= want && n <= want + slack,
            "{what} took {n} frames, wanted ~{want} (+/-{slack})"
        );
    };
    // Attack: from open to fully ducked within the attack window.
    let want_atk = (DUCK_ATTACK_MS / 1000.0 * rate as f32).ceil() as u32;
    let mut g = 1.0;
    let mut n = 0;
    while g > DUCK_GAIN && n < want_atk * 2 {
        g = duck_next_to(g, DUCK_GAIN, atk, rel);
        n += 1;
    }
    assert_eq!(g, DUCK_GAIN, "attack must reach the target");
    close(n, want_atk, "attack");
    // Release: back to unity within the release window.
    let want_rel = (DUCK_RELEASE_MS / 1000.0 * rate as f32).ceil() as u32;
    let mut n = 0;
    while g < 1.0 && n < want_rel * 2 {
        g = duck_next_to(g, 1.0, atk, rel);
        n += 1;
    }
    assert_eq!(g, 1.0, "release must return to unity");
    close(n, want_rel, "release");
}

#[test]
fn the_duck_envelope_is_bounded_at_both_ends() {
    let (atk, rel) = (duck_step(DUCK_ATTACK_MS, 44_100), duck_step(DUCK_RELEASE_MS, 44_100));
    // ⛔ However long it is held, it may not go below the target — an unbounded ramp in
    // the render loop is a music bus that fades to silence and never comes back.
    let mut g = 1.0;
    for _ in 0..1_000_000 {
        g = duck_next_to(g, DUCK_GAIN, atk, rel);
    }
    assert_eq!(g, DUCK_GAIN);
    // …and it may not drift above unity, which would AMPLIFY the music bus.
    for _ in 0..1_000_000 {
        g = duck_next_to(g, 1.0, atk, rel);
    }
    assert_eq!(g, 1.0);
}

#[test]
fn a_degenerate_sample_rate_jumps_instead_of_dividing_by_zero() {
    // A caller with no rate has no envelope to run; the step is the whole span, so one
    // frame lands exactly on the target. ⛔ Never a NaN or an infinity in the render loop.
    for rate in [0, 1] {
        let step = duck_step(DUCK_ATTACK_MS, rate);
        assert!(step.is_finite(), "rate {rate} produced {step}");
        assert_eq!(duck_next_to(1.0, DUCK_GAIN, step, step), DUCK_GAIN);
    }
}

#[test]
fn ducking_recovers_the_effect_the_sum_limiter_was_eating() {
    // The measured worst case: the loudest BGM peak with both of the game's sliders at
    // maximum, with an effect at the default trim (0.4).
    //
    // ★ 2026-09-04 — 2.625 * SYNTH_GAIN (bgm3_mf on the MA-3 bank, scores retuned by
    // `tools_re/patch_bgm_ma3.py`) = 0.998, where this used to be 1.18 (the same song on
    // GeneralUser at unity). Landing within 0.2 dB of the old worst case is the POINT of
    // that gain, so the limiter still eats most of an un-ducked effect — 0.060 of 0.4
    // survives, against 0.039 before — and the duck is as load-bearing as it ever was.
    let (music, effect) = (2.625_f32 * SYNTH_GAIN, 0.4_f32);
    let heard = |bed: f32| soft_limit(bed + effect) - soft_limit(bed);
    let before = heard(music);
    let after = heard(music * DUCK_GAIN);
    assert!(before < effect * 0.3, "the bug: most of the effect was eaten, got {before}");
    assert!(after > effect * 0.6, "ducked, most of the effect must survive, got {after}");
    assert!(after > before * 3.0, "the whole point is a large recovery: {before} -> {after}");
    // ⛔ And it must not become an AMPLIFIER: ducking may not make the effect louder
    // than the effect.
    assert!(after <= effect, "the effect cannot exceed its own level");
}

#[test]
fn quiet_mixes_are_untouched_by_ducking() {
    // Default settings: music 2.625*SYNTH_GAIN*0.36 = 0.359, effect 0.144. The sum never
    // reaches the knee, so the limiter was already idle and the effect level must not
    // change at all. (It was 1.18*0.36 = 0.425 on the GeneralUser bank at unity gain;
    // the sum moved 0.569 -> 0.503, both comfortably under LIMIT_KNEE.)
    let (music, effect) = (2.625_f32 * SYNTH_GAIN * 0.36, 0.144_f32);
    let heard = |bed: f32| soft_limit(bed + effect) - soft_limit(bed);
    assert!((heard(music) - effect).abs() < 1e-6, "the limiter was idle here");
    assert!((heard(music * DUCK_GAIN) - effect).abs() < 1e-6, "ducking must not change it either");
}

// ── 덕은 이제 깊이도 계산한다 (2026-08-28) ───────────────────────────────────
//
// ★★★ THESE ARE THE TESTS THAT SHOULD HAVE FAILED IN 71차-P. Player report 2026-08-27
// on vc185, the build that introduced ducking: "효과음발생할때마다 작게 뚝, 뚝, 뚝, 뚝
// 끊기는듯한 음량이 들려".
//
// `quiet_mixes_are_untouched_by_ducking` above uses the DEFAULT-settings numbers as its
// input and asserts the effect does not change — which was true, and was never the
// question. It never looked at the music, and at that very input the music was losing
// 6.02 dB to buy the effect +0.00 dB. A test that measures only the side that wins
// cannot see a feature that is a pure loss.
//
// The invariant below is the one that matters: **the duck may not cost the music
// anything it does not win the effect.**

/// The effect as heard, for a given music level, effect peak and duck gain.
fn heard_with(music: f32, effect: f32, duck: f32) -> f32 {
    let bed = music * duck;
    soft_limit(bed + effect) - soft_limit(bed)
}

/// Every (music level, effect peak) worth arguing about. Song peaks are the measured
/// unity-gain values from the table at the top of `audio_mix.rs`; both buses take the
/// same slider, which is the fact that made a single global trim unfixable.
fn mix_sweep() -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    for &song in &[0.45f32, 0.49, 0.656, 1.182] {
        for &slider in &[100u8, 80, 70, 60, 40, 20] {
            for &lvl in &[0u8, 2, 4] {
                out.push((song * volume_gain(slider), sfx_trim(lvl) * volume_gain(slider)));
            }
        }
    }
    out
}

/// Skip the handful of sweep points that land within rounding distance of the knee,
/// where "is the limiter engaged" is a coin toss between two equivalent expressions.
fn near_knee(music: f32, effect: f32) -> bool {
    (music + effect - LIMIT_KNEE).abs() < 1e-5
}

#[test]
fn a_duck_that_wins_the_effect_nothing_costs_the_music_nothing() {
    // ⛔ THE REPORTED BUG. Wherever the sum fits under the knee the limiter is idle,
    // there is nothing to win back, and `duck_target` must answer exactly 1.0 — no dip
    // at all. The old code answered DUCK_GAIN everywhere, unconditionally.
    let sweep = mix_sweep();
    let (mut idle, mut engaged) = (0, 0);
    for &(music, effect) in &sweep {
        if near_knee(music, effect) {
            continue;
        }
        let target = duck_target(music, effect);
        if music + effect <= LIMIT_KNEE {
            idle += 1;
            assert_eq!(target, 1.0, "limiter idle at music {music} effect {effect}, yet target {target}");
            // …and prove it really wins nothing there: the open mix already passes the
            // whole effect through.
            let open = heard_with(music, effect, 1.0);
            assert!((open - effect).abs() < 1e-6, "expected an idle limiter, got {open} vs {effect}");
        } else {
            engaged += 1;
            assert!(target < 1.0, "limiter engaged at music {music} effect {effect}, yet no duck");
        }
    }
    // Sanity: the sweep must contain both kinds, or this test proves nothing.
    assert!(idle > 0 && engaged > 0, "{idle} idle / {engaged} engaged of {}", sweep.len());
}

#[test]
fn the_adaptive_duck_buys_the_same_effect_for_less_music() {
    // The whole justification for turning 71차-P's constant into a function: for every
    // mix, the shallowest SUFFICIENT duck delivers at least as much effect as the flat
    // -6 dB did, while never cutting the music deeper.
    for (music, effect) in mix_sweep() {
        let target = duck_target(music, effect);
        let adaptive = heard_with(music, effect, target);
        let fixed = heard_with(music, effect, DUCK_GAIN);
        assert!(
            adaptive >= fixed - 1e-6,
            "music {music} effect {effect}: adaptive {adaptive} lost to fixed {fixed}"
        );
        assert!(target >= DUCK_GAIN, "music {music} effect {effect}: target {target} is deeper than DUCK_GAIN");
        assert!(target <= 1.0, "music {music} effect {effect}: target {target} would AMPLIFY the music");
    }
}

#[test]
fn the_loud_handset_case_71p_existed_for_still_ducks_all_the_way() {
    // `bgm3_mf` at 1.182 with both sliders at maximum and the default trim: the -20.2 dB
    // erasure. ⛔ If this stops reaching DUCK_GAIN, the 폴드6 report is back.
    let target = duck_target(1.182, SFX_TRIM);
    assert_eq!(target, DUCK_GAIN, "the erasure case must still duck fully");
    let recovered = heard_with(1.182, SFX_TRIM, target);
    let erased = heard_with(1.182, SFX_TRIM, 1.0);
    assert!(recovered > SFX_TRIM * 0.6, "most of the effect must survive, got {recovered}");
    assert!(recovered > erased * 4.0, "the recovery is the point: {erased} -> {recovered}");
}

#[test]
fn the_game_default_settings_do_not_duck_at_all() {
    // Both sliders at the game's own default (`MC_mdaClipSetVolume(clip, 60)`), loudest
    // song. This is the exact coordinate the player was listening to when they reported.
    let g = volume_gain(60);
    assert_eq!(duck_target(1.182 * g, SFX_TRIM * g), 1.0);
    // …and a quiet effect level never ducks anything, whatever the music is doing under
    // it — the "효과음은 안 들리는데 음악만 뚝뚝" case.
    assert_eq!(duck_target(0.45 * g, sfx_trim(0) * volume_gain(20)), 1.0);
}

#[test]
fn duck_target_is_monotone_and_never_returns_a_nonsense_number() {
    // Louder music -> deeper duck; louder effect -> deeper duck. Never NaN, never 0.
    let mut prev = 1.0f32;
    for i in 0..200 {
        let t = duck_target(i as f32 * 0.02, SFX_TRIM);
        assert!(t.is_finite() && t >= DUCK_GAIN && t <= 1.0, "music step {i} -> {t}");
        assert!(t <= prev + 1e-6, "not monotone in music at step {i}: {prev} -> {t}");
        prev = t;
    }
    let mut prev = 1.0f32;
    for i in 0..100 {
        let t = duck_target(1.0, i as f32 * 0.01);
        assert!(t.is_finite() && t >= DUCK_GAIN && t <= 1.0, "effect step {i} -> {t}");
        assert!(t <= prev + 1e-6, "not monotone in effect at step {i}: {prev} -> {t}");
        prev = t;
    }
    // Degenerate inputs fail toward NO DUCK, never toward silence.
    assert_eq!(duck_target(0.0, SFX_TRIM), 1.0, "silent music needs no duck");
    assert_eq!(duck_target(-1.0, SFX_TRIM), 1.0, "an impossible music level");
    assert_eq!(duck_target(1.0, LIMIT_KNEE), DUCK_GAIN, "an effect that fills the knee alone");
    assert_eq!(duck_target(1.0, 2.0), DUCK_GAIN, "an effect past full scale");
    assert_eq!(duck_target(f32::NAN, SFX_TRIM), 1.0, "NaN music must not duck");
}

#[test]
fn the_envelope_reaches_a_shallow_target_and_releases_from_it() {
    let (atk, rel) = (duck_step(DUCK_ATTACK_MS, 44_100), duck_step(DUCK_RELEASE_MS, 44_100));
    let target = 0.8;
    let mut g = 1.0;
    for _ in 0..1_000_000 {
        g = duck_next_to(g, target, atk, rel);
    }
    assert_eq!(g, target, "a shallow target must be held, not overshot");
    for _ in 0..1_000_000 {
        g = duck_next_to(g, 1.0, atk, rel);
    }
    assert_eq!(g, 1.0, "and released back to unity");
    // ⛔ Already AT the target is a no-op, not a jitter.
    assert_eq!(duck_next_to(target, target, atk, rel), target);
}

// ── 덕 상태 워드 ─────────────────────────────────────────────────────────────
//
// This arithmetic used to live in `audio_sink.rs`, which cannot be `include!`d (rodio,
// oxisynth), so the tail chunk and the extension case had ZERO tests while deciding
// whether the envelope comes out a ramp or a step.

#[test]
fn the_state_word_round_trips_both_halves() {
    for &(f, p) in &[(0u32, 0u32), (1, 1), (263_188, 0x3ecc_cccd), (u32::MAX, 0x7fff_ffff)] {
        assert_eq!(duck_unpack(duck_pack(f, p)), (f, p), "frames {f} peak {p:#x}");
    }
    // The low half really is an f32 bit pattern, and it survives the trip.
    let peak = 0.4321f32;
    let (_, bits) = duck_unpack(duck_pack(999, peak.to_bits()));
    assert_eq!(f32::from_bits(bits), peak);
}

#[test]
fn arming_extends_and_never_shortens_or_quietens() {
    let loud = 0.8f32.to_bits();
    let quiet = 0.1f32.to_bits();
    assert_eq!(duck_unpack(duck_arm(0, 100, quiet)), (100, quiet));
    // A longer but quieter effect extends the length and keeps the loud depth budget.
    let w = duck_arm(duck_arm(0, 100, loud), 500, quiet);
    assert_eq!(duck_unpack(w), (500, loud), "max on BOTH halves, independently");
    // ⛔ A shorter, quieter effect may change neither.
    assert_eq!(duck_unpack(duck_arm(w, 10, quiet)), (500, loud));
}

#[test]
fn consuming_walks_a_duck_down_and_retires_the_peak_with_it() {
    let peak = 0.4f32.to_bits();
    let chunk = 64;
    let mut w = duck_arm(0, 150, peak);
    let (d, next) = duck_consume(w, chunk);
    assert_eq!((d, duck_unpack(next)), (64, (86, peak)));
    w = next;
    let (d, next) = duck_consume(w, chunk);
    assert_eq!((d, duck_unpack(next)), (64, (22, peak)));
    w = next;
    // The TAIL chunk: 22 of the 64 frames owe a duck, and the word retires whole.
    let (d, next) = duck_consume(w, chunk);
    assert_eq!(d, 22, "partial tail chunk");
    assert_eq!(next, 0, "the peak must retire ATOMICALLY with the frame count");
    // An exhausted word stays exhausted and ducks nothing.
    assert_eq!(duck_consume(0, chunk), (0, 0));
    // An exact multiple retires on its last full chunk, not one chunk late.
    assert_eq!(duck_consume(duck_arm(0, 64, peak), chunk), (64, 0));
    // A zero-length chunk cannot spend anything (and must not divide or panic).
    let w = duck_arm(0, 10, peak);
    assert_eq!(duck_consume(w, 0), (0, w));
}

// ── 효과음 클립의 끝 계단과 DC 받침대 (2026-08-28) ───────────────────────────
//
// All fourteen of the game's effects end on a one-sample step off a DC pedestal as far
// out as -0.603 FS, and there was no fade anywhere in the audio path. See `declick`.

/// A clip shaped like the real ones: an audible burst, then a DC pedestal, cut dead.
fn pedestal_clip(frames: usize, ch: usize, dc: f32) -> Vec<f32> {
    let mut v = vec![0.0f32; frames * ch];
    for f in 0..frames {
        let t = f as f32 / frames as f32;
        let ac = if t < 0.4 { (f as f32 * 0.7).sin() * 0.3 } else { 0.0 };
        for c in 0..ch {
            v[f * ch + c] = dc + ac;
        }
    }
    v
}

#[test]
fn declick_removes_the_end_step_and_the_dc_pedestal() {
    for &ch in &[1usize, 2] {
        for &rate in &[4_000u32, 8_000] {
            let frames = rate as usize / 2; // 500 ms
            let mut v = pedestal_clip(frames, ch, -0.603);
            let before_last = v[v.len() - 1].abs();
            assert!(before_last > 0.5, "the fixture must reproduce the real step, got {before_last}");
            declick(&mut v, ch, rate);
            // ⛔ The click itself: the clip must end AT zero.
            assert_eq!(v[v.len() - 1], 0.0, "ch {ch} rate {rate}: the last sample must be silence");
            // …and approach it, not jump to it.
            let tail = declick_frames(DECLICK_TAIL_MS, rate, frames);
            assert!(tail >= 16, "tail is only {tail} frames at {rate} Hz");
            for k in 1..tail {
                let a = v[(frames - 1 - k) * ch].abs();
                let b = v[(frames - k) * ch].abs();
                assert!(a >= b - 1e-6, "ch {ch} rate {rate}: tail not monotone at k={k} ({a} < {b})");
            }
            // The pedestal is gone: the mean over the silent half is ~0.
            let n = frames - frames / 2;
            let mean: f32 = (frames / 2..frames).map(|f| v[f * ch]).sum::<f32>() / n as f32;
            assert!(mean.abs() < 0.02, "ch {ch} rate {rate}: DC left over, mean {mean}");
        }
    }
}

#[test]
fn declick_leaves_the_audible_body_alone() {
    // The burst is in the first 40 % of the clip; whatever the filter does to DC, the
    // body's peak must survive. ⛔ This is artifact removal, not a level change.
    let (ch, rate, frames) = (1usize, 8_000u32, 4_000usize);
    let mut v = pedestal_clip(frames, ch, 0.0);
    let before = peak_abs(&v);
    declick(&mut v, ch, rate);
    let after = peak_abs(&v[..frames * 4 / 10]);
    assert!(after > before * 0.9, "body peak {before} -> {after}");
}

#[test]
fn declick_survives_every_degenerate_input() {
    // ⛔ Runs on real clips off the wire; a panic here is silence for the rest of the app.
    let mut empty: Vec<f32> = Vec::new();
    declick(&mut empty, 2, 8_000);
    assert!(empty.is_empty());
    // A rate of 0 must not divide; the filter degrades to a plain difference.
    let mut v = vec![0.5f32; 100];
    declick(&mut v, 1, 0);
    assert!(v.iter().all(|s| s.is_finite()), "rate 0 produced a non-finite sample");
    // A length that is not a whole number of stereo frames is refused, not half-processed.
    let mut odd = vec![0.5f32; 7];
    let copy = odd.clone();
    declick(&mut odd, 2, 8_000);
    assert_eq!(odd, copy, "a torn stereo buffer must be left alone");
    // `channels = 0` is read as mono rather than dividing by zero.
    let mut mono = vec![0.5f32; 8];
    declick(&mut mono, 0, 8_000);
    assert_eq!(mono[7], 0.0);
    // A clip shorter than the fade still ends at zero.
    let mut tiny = vec![0.9f32; 3];
    declick(&mut tiny, 1, 8_000);
    assert_eq!(tiny[2], 0.0);
}

#[test]
fn peak_abs_is_the_peak() {
    assert_eq!(peak_abs(&[]), 0.0);
    assert_eq!(peak_abs(&[0.1, -0.9, 0.4]), 0.9);
    assert_eq!(peak_abs(&[-0.2]), 0.2);
}

// ── 클립별 트림 (2026-08-28) ─────────────────────────────────────────────────
//
// Player request: *"크리티컬 발생시 나오는 효과음을 조금 줄일수도있어?"*. The bus trim
// (`SFX_LEVELS`) cannot do that — it moves every effect together. `CLIP_TRIMS` keys one
// clip by `(rate, i16 count)`, which is the only identity a decoded effect has left by
// the time it reaches `AudioSink::play_wave`.

/// The fourteen shipping effects, measured from the jar: `(name, rate, i16 count)`.
/// ⛔ Keep this in step with the table in `audio_mix.rs` — it is what proves the
/// fingerprints cannot collide.
const SHIPPING_CLIPS: &[(&str, u32, usize)] = &[
    ("sd000", 8_000, 5_168),
    ("sd001", 4_000, 23_872),
    ("sd002", 4_000, 14_700),
    ("sd003", 8_000, 1_590),
    ("sd004", 8_000, 7_384), // 크리티컬
    ("sd005", 8_000, 1_704),
    ("sd006", 8_000, 2_988),
    ("sd007", 8_000, 4_906),
    ("sd008", 8_000, 7_502),
    ("sd009", 8_000, 13_608),
    ("sd010", 8_000, 13_654),
    ("sd011", 8_000, 13_330),
    ("sd012", 8_000, 25_376),
    ("sd013", 8_000, 15_930),
];

#[test]
fn the_critical_hit_is_trimmed_and_nothing_else_is() {
    // The one clip the player asked about, at the -3 dB the settings row's grid uses.
    let crit = clip_trim(8_000, 7_384);
    assert!(crit < 1.0, "the crit must be reduced, got {crit}");
    let db = 20.0 * crit.log10();
    assert!((db + 3.0).abs() < 0.1, "expected about -3 dB, got {db:.2} dB");
    // ⛔ Every other shipping clip must come through untouched. This is the assertion
    // that fails if a future entry is keyed carelessly.
    for &(name, rate, n) in SHIPPING_CLIPS {
        let g = clip_trim(rate, n);
        if (rate, n) == (8_000, 7_384) {
            continue;
        }
        assert_eq!(g, 1.0, "{name} ({rate} Hz, {n}) must not be trimmed, got {g}");
    }
}

#[test]
fn clip_trims_cannot_collide() {
    // ★ THE POINT OF THE TOLERANCE TEST. Every trimmed key must be closer to its own
    // clip than the tolerance window is wide, and no OTHER shipping clip may fall inside
    // that window. `sd004` (7,384) and `sd008` (7,502) are 118 apart at the same rate,
    // which is what sets the ceiling on `CLIP_FINGERPRINT_TOLERANCE`.
    for &(rate, n, _) in CLIP_TRIMS {
        let slack = (n as f32 * CLIP_FINGERPRINT_TOLERANCE).max(1.0);
        for &(name, r2, n2) in SHIPPING_CLIPS {
            if r2 != rate || n2 == n {
                continue;
            }
            let d = (n2 as f32 - n as f32).abs();
            assert!(
                d > slack,
                "{name} ({n2}) is {d} from the trimmed key {n} but the window is +/-{slack} — the tolerance is too wide"
            );
        }
    }
}

#[test]
fn every_trim_entry_is_a_real_shipping_clip() {
    // ⛔ A key that matches nothing is a silent no-op: the player would report the sound
    // unchanged and nothing would say why.
    for &(rate, n, g) in CLIP_TRIMS {
        assert!(
            SHIPPING_CLIPS.iter().any(|&(_, r, c)| r == rate && c == n),
            "({rate} Hz, {n}) is not one of the shipping clips"
        );
        assert!(g > 0.0 && g <= 1.0, "a trim must attenuate, never silence or amplify: {g}");
    }
}

#[test]
fn an_unknown_clip_is_never_touched() {
    // Fails toward NO CHANGE. ⛔ A fingerprint bug must not silence an effect.
    for &(rate, n) in &[(8_000u32, 1usize), (8_000, 999_999), (44_100, 7_384), (11_025, 7_384)] {
        assert_eq!(clip_trim(rate, n), 1.0, "unknown clip ({rate} Hz, {n})");
    }
    // Degenerate inputs, which really do reach here (a torn or empty SMAF chunk).
    assert_eq!(clip_trim(0, 7_384), 1.0, "rate 0");
    assert_eq!(clip_trim(8_000, 0), 1.0, "empty buffer");
    assert_eq!(clip_trim(0, 0), 1.0);
}

#[test]
fn the_slider_still_owns_the_bus_and_the_clip_trim_rides_on_top() {
    // The two are independent multipliers, and the crit must stay quieter than its
    // untrimmed neighbour at EVERY slider position — that is what "조금 줄인다" means.
    for lvl in 0..SFX_LEVELS.len() as u8 {
        let bus = sfx_trim(lvl);
        let crit = bus * clip_trim(8_000, 7_384);
        let other = bus * clip_trim(8_000, 7_502);
        assert!(crit < other, "level {lvl}: crit {crit} must sit under sd008 {other}");
        // …and it must still be audible: -3 dB, not a mute.
        assert!(crit > bus * 0.5, "level {lvl}: crit {crit} fell more than 6 dB under the bus {bus}");
    }
}
