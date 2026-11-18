// Master-bus math for the Android audio path. Pure arithmetic, NO dependencies —
// `audio_mix_selftest.rs` at the repo root `include!`s this very file, so the numbers
// below are the ones that ship (see CLAUDE.md: `cargo test -p wie_android` cannot run
// on the Windows host).
//
// ── WHY THESE NUMBERS EXIST ──────────────────────────────────────────────────
// Player report 2026-08-16, three symptoms that turned out to be one bug:
//   ① 배경음이 휴대폰 볼륨을 최대로 올려야 겨우 들린다
//   ② 배경음 3단계와 효과음 3단계의 음량 차이가 너무 크다
//   ③ 효과음이 나는 스킬을 쓰면 배경음이 끊긴다
//
// Hero3 ships its two buses as physically different SMAF chunks — measured from the
// game's own jar, `res/snd/bgm*_mf` are `MTR` score tracks (MIDI, rendered here by
// oxisynth) and `res/snd/sd*_mf` are `ATR` audio tracks (Yamaha ADPCM, played as PCM).
// They never met in the same code path, so nothing ever compared their levels.
//
// Rendered with the then-shipping `GeneralUser.sf3` (20 s per song) at the game's DEFAULT
// setting — the probe shows `MC_mdaClipSetVolume(clip, 60)` for BOTH buses, i.e. both
// sliders on 3:
//
//     bus / file           peak dBFS     RMS dBFS
//     BGM  bgm0_mf           -25.2         -42.5
//     BGM  bgm3_mf           -20.6         -38.7
//     BGM  bgm13_mf          -28.2         -47.8
//     BGM  bgm16_mf          -28.6         -43.0
//     SFX  sd008_mf           -4.4          -8.3
//     SFX  sd004_mf (크리티컬) -4.4         -11.2
//     SFX  sd009_mf           -4.4         -15.0
//     SFX  sd002_mf          -10.4         -24.2
//
// A **27-39 dB** gap from one and the same setting. That is ①, ② and ③ at once: the
// music is 30 dB down so the phone has to be at maximum (①); the effect is 30 dB above
// it at the same slider position (②); and an SFX that already peaks at 0 dBFS *before*
// the slider both masks the music and pushes the summed mix past full scale, where the
// device clamps and the music is not merely buried but arithmetically destroyed (③).
//
// Three causes, three constants below.

/// Synth output gain — `0.2` -> `1.0` -> `0.38`.
///
/// `0.2` is oxisynth's inherited FluidSynth default, chosen for a *desktop synth*
/// playing arbitrary MIDI with 100+ voices, where the sum has to stay clear of full
/// scale. Hero3's scores are 16-channel game MIDI at ~12 note-ons a second: measured
/// peaks across the four songs above at unity gain were 0.45 / 0.49 / 0.66 / 1.18 with
/// the old GeneralUser bank, so `0.2` was throwing away ~14 dB the mix never needed to
/// give up, and unity was the right baseline for THAT content.
///
/// ★★★ 2026-09-04 — `1.0` -> `0.41`, AND THIS IS NOT A TASTE CHANGE. The bundled
/// SoundFont is no longer a General MIDI bank: `assets/hero3_ma3.sf3` now carries the
/// game's own Yamaha MA-3 tones (see `audio_sink::SF3_BYTES`). Chip tones sustain where
/// sampled acoustic instruments decay, so the SAME scores come out far hotter. Measured
/// over ALL 19 `bgm*_mf` at unity, 20 s renders, on the host with this exact synth
/// configuration (44100, polyphony 64, reverb+chorus on):
///
/// ```text
///     bank / scores                      worst peak       aggregate RMS
///     GeneralUser + original scores      1.1823 (bgm3)      -20.17 dB   <- the baseline
///     MA-3        + original scores      2.0840 (bgm0)      -12.42 dB   (+7.75)
///     MA-3        + RETUNED scores       2.6251 (bgm3)      -11.84 dB   (+8.33)  SHIPS
///
/// ```
/// The third row is what ships: `tools_re/patch_bgm_ma3.py` also retunes WHICH MA-3 tone
/// each score asks for, and that is worth another +0.6 dB on its own.
///
/// `0.38` is the RMS match against the FIRST row - the loudness every constant below was
/// tuned against - so [`SFX_TRIM`], [`SFX_LEVELS`] and the duck keep meaning what their
/// measurement tables say they mean. Measured residual: **-0.07 dB**. The worst peak
/// lands at 0.9975, just under full scale, and [`soft_limit`] catches it exactly as it
/// caught the old 1.18.
///
/// DO re-measure whenever the bank OR the scores change: they are ONE calibration, not
/// two. Going from the middle row to the bottom one moved the answer 0.41 -> 0.38.
///
/// ⛔ Do NOT "fix" the resulting loudness by touching `wie_backend::volume_gain`. That
/// function is the one definition of what a slider step sounds like and BOTH buses read
/// it, so moving it moves the effects too and hands back the 27-39 dB problem this file
/// exists to solve. Tune here; the table above is the reference.
pub const SYNTH_GAIN: f32 = 0.38;

/// PCM (SFX) bus trim.
///
/// The game's effect samples are *brickwalled*: four of the fourteen decode to a
/// literal 0 dBFS peak and `sd008_mf` sits at -3.9 dBFS RMS. Nothing downstream can
/// mix on top of a full-scale source, so the effect bus gives up fixed headroom and the
/// music bus keeps unity, instead of the 27-39 dB spread it was.
///
/// ★ `0.5` (-6.0 dB) -> `0.4` (-8.0 dB) after the player listened to both buses on 3:
/// "효과음이 상대적으로 좀 크게 들려서 … 아주 조금만 낮추는게 좋을것같아". That is a
/// -2 dB nudge, which moves the measured RMS gap at matched sliders from +4.6 dB to
/// **+2.6 dB** — effects still cut through sustained music, but no longer sit on top
/// of it. Tune here and nowhere else; the measurement table above is the reference.
pub const SFX_TRIM: f32 = 0.4;

/// The effect bus trims the settings row offers, quietest first.
///
/// ★★ 2026-08-27 — WHY THIS IS A SETTING AND NOT ANOTHER GUESS. [`SFX_TRIM`] was tuned
/// by ear on ONE phone (an S25 Ultra) and then a Fold 6 player reported the opposite
/// symptom: effects too quiet. Both reports are right. The trim is a fixed −8.0 dB at
/// EVERY slider step — `volume_gain` is the same square law on both buses, so the effect
/// bus sits exactly 8 dB under the music bus whatever the player does — and 8 dB of a
/// loud phone's headroom is not 8 dB of a quiet one's. A third global number would just
/// move the complaint to a third handset.
///
/// So the number the ear picks is the number the ear can change. Five steps of 3 dB,
/// centred on the shipped value: **−6 / −3 / 0 / +3 / +6 dB** relative to it. The
/// default index is [`SFX_LEVEL_DEFAULT`] and its entry IS `SFX_TRIM`, so a player who
/// never opens the row hears exactly what shipped — that is the whole point of the
/// choice, and the reason the tuned phone cannot regress.
///
/// ⛔ The top step stops at 0.8 (−1.9 dB), not 1.0. Four of the game's fourteen effect
/// samples decode to a literal 0 dBFS peak; at unity they leave the mix no headroom at
/// all and [`soft_limit`] starts eating the music underneath them, which is symptom ③
/// of the 2026-08-16 report coming straight back.
pub const SFX_LEVELS: [f32; 5] = [0.2, 0.283, 0.4, 0.566, 0.8];

/// Index into [`SFX_LEVELS`] that reproduces [`SFX_TRIM`] exactly.
pub const SFX_LEVEL_DEFAULT: u8 = 2;

/// The trim for a stored level, clamped. Out-of-range (a hand-edited file, a future
/// build that shrank the table) falls back to the shipped default rather than to
/// silence or to full scale — both of which are worse than "unchanged".
pub fn sfx_trim(level: u8) -> f32 {
    match SFX_LEVELS.get(level as usize) {
        Some(&t) => t,
        None => SFX_LEVELS[SFX_LEVEL_DEFAULT as usize],
    }
}

/// The row's own label for a level: how far it is from what shipped, in dB.
pub fn sfx_level_label(level: u8) -> &'static str {
    match level {
        0 => "-6dB",
        1 => "-3dB",
        2 => "기본",
        3 => "+3dB",
        4 => "+6dB",
        _ => "기본",
    }
}

/// Next level in the row's cycle, wrapping — the same grammar the PAD HUD and NET rows use.
pub fn sfx_level_next(level: u8) -> u8 {
    if usize::from(level) + 1 >= SFX_LEVELS.len() { 0 } else { level + 1 }
}

// ── 효과음이 울릴 때 음악을 비켜준다 (2026-08-27) ────────────────────────────
//
// ★★★ WHY DUCKING, AND WHY IT IS NOT A TASTE CHANGE.
//
// [`soft_limit`] is applied to the SUM, and a static waveshaper on a sum has a property
// nobody chose: **whatever arrives last gives way**. Once the music alone is past the
// knee, an effect added on top is not mixed in, it is compressed away. Measured on this
// game's own numbers (with the GeneralUser bank this file was first tuned on: `bgm3_mf`
// peaks at 1.18 with `SYNTH_GAIN = 1.0`, effect at the default trim peaks at 0.4), with
// both of the game's sliders at maximum. The MA-3 bank that ships now lands its own worst
// case at 0.998 (`SYNTH_GAIN` is chosen to put it there), so the table below still says
// what the limiter does to an effect laid on top of it:
//
//     music peak   effect as heard      vs. no music
//        0.42 (기본 설정)   0.1440          ±0.0 dB   <- the limiter never engages
//        0.66              0.2036          -5.9 dB
//        1.18              0.0391         -20.2 dB   <- effectively inaudible
//
// A player on a quiet handset raises both sliders, which is exactly how they arrive at
// the bottom row. That is the 폴드6 report ("효과음이 너무 작다"), and no amount of
// [`SFX_LEVELS`] fixes it: the effect bus is not quiet, it is being erased.
//
// The fix is to decide WHICH signal gives way instead of letting the arithmetic decide.
// The effect is the event the player needs to hear; the music is the bed. Ducking the
// music is also closer to the game's own design than what we do now — the original
// client owns ONE sound slot and literally STOPS the music to play an effect (52차-B);
// the host is the thing that keeps the music running underneath.
//
// With the music ducked, the sum stays near the knee and the effect passes through
// essentially uncompressed:
//
//     music peak 1.18, duck 0.5 -> effect -3.8 dB (was -20.2)
//     music peak 0.66, duck 0.5 -> effect -0.1 dB (was  -5.9)
//     default settings          -> effect  ±0.0 dB (unchanged; the limiter was idle)
//
// ⛔ Deeper is not better. 0.4 buys 2.3 dB more effect and 0.3 buys 3.6 dB, but the
// music dip becomes the thing you notice. -6 dB is the conventional duck and it already
// recovers 16.4 dB of the 20.2 that were being lost.

/// How far the music bus drops while an effect is sounding. `1.0` = no ducking.
pub const DUCK_GAIN: f32 = 0.5;

/// Time to reach [`DUCK_GAIN`] once an effect starts.
///
/// ★★★ 12 -> 120 ms (2026-08-30). ⛔ DO NOT SHORTEN THIS BACK. 12 ms was chosen so the duck
/// was in place before the effect's own transient, and it is the reported 「툭」: the full
/// span in 12 ms is 502 dB/s, which on a sustained MIDI pad is an event and not a fade.
/// The cost of being late is a few ms of [`soft_limit`] under the loudest moment of the
/// loudest thing in the mix; the cost of being fast is a click the player hears on every
/// single effect. 120 ms is 50 dB/s — a fade — and still fully in place for the body of
/// even the shortest of this game's effects (`sd003`, 199 ms). See the block above.
pub const DUCK_ATTACK_MS: f32 = 120.0;

/// Time to return to unity after the last effect ends. Long enough that a run of
/// effects (a critical hit, a spell) does not pump the music between them.
///
/// ★ 140 -> 400 ms (2026-08-30), the other end of the same gesture. Nothing is waiting on
/// this one — once the effect is over there is no deadline to be back at unity by — so it
/// is simply as slow as the ear stops noticing.
pub const DUCK_RELEASE_MS: f32 = 400.0;

/// Per-FRAME envelope delta that traverses the whole `1.0 -> DUCK_GAIN` span in `ms`.
///
/// Returned as a magnitude; [`duck_next_to`] applies the sign. A zero or negative rate is
/// answered with the full span, i.e. "jump" — a caller with no sample rate has no
/// envelope to run and must not divide by it.
pub fn duck_step(ms: f32, rate: u32) -> f32 {
    let span = 1.0 - DUCK_GAIN;
    let frames = ms / 1000.0 * rate as f32;
    if frames < 1.0 { span } else { span / frames }
}

/// One frame of the duck envelope, toward a `target` in `[DUCK_GAIN, 1.0]`. Clamped at
/// the target from BOTH sides, so it can neither overshoot it on the way down nor drift
/// above unity on the way up however the steps are sized.
///
/// ★★★ 2026-08-28 — this replaced `duck_next(cur, ducking: bool, ..)`, because the duck
/// DEPTH is no longer a constant: see [`duck_target`]. A boolean could only ever mean
/// "all the way to [`DUCK_GAIN`]", which is the -6 dB that cost the music everything and
/// won the effect nothing at the default settings.
pub fn duck_next_to(cur: f32, target: f32, attack: f32, release: f32) -> f32 {
    if cur > target {
        (cur - attack).max(target)
    } else if cur < target {
        (cur + release).min(target)
    } else {
        cur
    }
}

// ── 덕은 리미터가 실제로 먹는 만큼만 걸어야 한다 (2026-08-28) ─────────────────
//
// ★★★ THE DUCK USED TO BE UNCONDITIONAL, AND THAT MADE IT A PURE LOSS AT THE
// DEFAULT SETTINGS. Player report 2026-08-27, on vc185 (71차-P), the build that
// introduced ducking: *"효과음발생할때마다 작게 「뚝」,「뚝」,「뚝」,「뚝」 끊기는듯한
// 음량이 들려"*.
//
// The table in the block above already says why: at the game's default sliders (both
// buses at `MC_mdaClipSetVolume(60)`, i.e. `volume_gain(60) = 0.36`) the summed peak is
// `1.18 * 0.36 + 0.4 = 0.569`, which is BELOW [`LIMIT_KNEE`] — *"default settings ->
// effect ±0.0 dB (unchanged; the limiter was idle)"*. The limiter never engages, so
// there is nothing for the duck to win back. The implementation ducked anyway:
//
//     measured, default sliders:  effect +0.00 dB  /  music -6.02 dB
//     7 of the 12 (song, slider) combinations measured came to EXACTLY +0.00 dB,
//     and every music slider at or below 70 was a flat -6.02 dB for zero gain.
//
// -6.02 dB reached in 12 ms is 502 dB/s. On a sustained MIDI pad that is not a fade,
// it is an event — and it repeats once per effect. That is the 「뚝, 뚝, 뚝, 뚝」.
//
// ⛔ The answer is NOT to remove the duck: on a quiet handset with both sliders raised
// the effect really is erased (-20.2 dB), which is the 폴드6 report 71차-P existed to
// answer. The answer is to duck by the amount the limiter is actually about to eat, and
// by nothing at all when it is idle. That is what the comment above always claimed the
// feature did.
//
// Re-measured with this function in place:
//
//     music 1.18 x slider 1.00, effect 0.40  ->  target 0.500  (unchanged; the -20.2 dB case)
//     music 0.45 x slider 1.00, effect 0.40  ->  target 0.667  (half the dip, same effect level)
//     music 1.18 x slider 0.36, effect 0.144 ->  target 1.000  (NO DUCK — the default)
//     music any,                effect 0.016 ->  target 1.000  (effect slider low: D3)

/// How far the music bus should drop RIGHT NOW: the deepest cut that is still needed to
/// keep `music + effect` off [`LIMIT_KNEE`], never deeper than [`DUCK_GAIN`], and
/// exactly `1.0` (no duck at all) whenever the sum already fits under the knee.
///
/// * `music_level` — the music bus peak **after** its own volume gain, before ducking.
/// * `sfx_peak` — the loudest sample of the effect that is sounding, after its trim.
///
/// Fails toward NO DUCK: an unknown (zero) music level, or an effect that leaves room,
/// both answer `1.0`. The one case that answers [`DUCK_GAIN`] is an effect that already
/// fills the knee on its own, where the music has to give way or be compressed away.
pub fn duck_target(music_level: f32, sfx_peak: f32) -> f32 {
    if !(music_level > 0.0) {
        return 1.0;
    }
    let room = LIMIT_KNEE - sfx_peak;
    if !(room > 0.0) {
        return DUCK_GAIN;
    }
    if music_level <= room {
        return 1.0;
    }
    (room / music_level).clamp(DUCK_GAIN, 1.0)
}

// ── 덕은 들려서는 안 된다 — 깊이가 아니라 속도의 문제였다 (2026-08-30) ────────
//
// ★★★ 71차-Q MADE THE DUCK CONDITIONAL AND THE 「툭」 SURVIVED, BECAUSE THE 「툭」 WAS NEVER
// ABOUT WHETHER IT DUCKS. Player report 2026-08-30, on vc197: *"여전히 툭툭 하고 끊기는
// 소리가 들려. 가만히 배경음만 들릴땐 괜찮은데 효과음이 발생한 직후에 계속 「툭」「툭」"*.
//
// 71차-Q wrote the answer down and then fixed the other half of it:
//
//     "-6.02 dB reached in 12 ms is 502 dB/s. On a sustained MIDI pad that is not a
//      fade, it is an event — and it repeats once per effect."
//
// It concluded that the cure was to stop ducking when the duck buys nothing, which is
// true and worth having — at the game's DEFAULT sliders there is now no duck and no
// artefact, which is why this report had to come from a player who had raised 배경음. But
// wherever [`duck_target`] does return a depth, the gesture is still 502 dB/s down and
// 43 dB/s back, and a gain step that fast on a sustained pad is a click at BOTH ends.
// Two ends is why the player writes it as 「툭」「툭」 rather than one sound, and "계속"
// is one pair per effect through a fight.
//
// So the envelope times are the fix, and they are cheap:
//
//   * ATTACK — 71차-Q kept it at 12 ms so the duck was in place before the effect's own
//     transient. Arriving late costs a few ms of [`soft_limit`] on the very onset of a
//     brickwalled effect, i.e. compression hidden underneath the loudest moment of the
//     loudest thing in the mix. Arriving fast costs an audible step in the music. The
//     effects run 199 ms to 5,968 ms, so a fade that takes 120 ms is still fully in place
//     for the body of every one of them.
//   * RELEASE — the same argument with nothing at all on the other side of the ledger:
//     once the effect is over there is no reason to be anywhere in particular by any
//     particular time.
//
// ⛔ Do NOT answer this by removing the duck. 71차-P exists because on a quiet handset with
// both sliders raised the effect is erased (-20.2 dB), and that is the same player
// population that raises 배경음 and hears this.
//
// Two supporting changes come with it, because a slow envelope only sounds like a fade if
// the thing it is chasing holds still:
//
//   * [`music_env_next`] — the renderer used to hand [`duck_target`] the peak of the last
//     64 frames. 1.45 ms is less than one cycle of the bass, so that number samples the
//     WAVEFORM, not the level: it collapses toward zero at every zero crossing and
//     recovers at every peak, dragging the target between 1.0 and [`DUCK_GAIN`] tens of
//     times a second. The old fast attack hid this by latching the gain at the deepest
//     target; a slow attack would have made it audible as a warble instead.
//   * [`duck_hold_next`] — the depth is now monotone for as long as an effect sounds, so
//     the shape is one dip and one recovery even if the music's level moves underneath it.

/// How long the music level the duck budgets against takes to fall by 1/e with nothing
/// louder arriving.
///
/// Long relative to a musical cycle (200 ms is 40 cycles of a 200 Hz note, so the trough
/// of a waveform can never pull it down) and short relative to a song section, so the duck
/// still tracks a piece that genuinely gets quieter.
pub const MUSIC_ENV_RELEASE_MS: f32 = 200.0;

/// Per-chunk decay multiplier for the music level detector.
///
/// `0.0` for an unusable rate or chunk, which makes the detector a plain per-chunk peak —
/// i.e. exactly the old behaviour, which is the right fallback for a caller that cannot
/// tell us its timebase.
pub fn music_env_decay(ms: f32, rate: u32, chunk: usize) -> f32 {
    if rate == 0 || chunk == 0 || ms <= 0.0 {
        return 0.0;
    }
    let dt = chunk as f32 / rate as f32;
    (-dt / (ms / 1000.0)).exp().clamp(0.0, 1.0)
}

/// One chunk of the music level detector: instant attack, exponential decay.
///
/// Instant attack matters as much as the decay does — the duck has to be budgeted against
/// the loudest thing the music is about to do, and a detector that ramped UP would let a
/// swell through to the limiter before it noticed.
pub fn music_env_next(env: f32, chunk_peak: f32, decay: f32) -> f32 {
    let peak = if chunk_peak.is_finite() && chunk_peak > 0.0 { chunk_peak } else { 0.0 };
    let held = if env.is_finite() && env > 0.0 { env * decay.clamp(0.0, 1.0) } else { 0.0 };
    peak.max(held)
}

/// The depth actually used for the current duck: the deepest [`duck_target`] seen since it
/// was armed, released to `1.0` the moment nothing is owed.
///
/// ★ The second half of the 2026-08-30 fix, and it is what makes the shape a DIP rather
/// than a wobble. [`music_env_next`] removes the waveform from the detector, but the level
/// still moves — a cymbal, a chord change — and the ear hears a music bus that rises and
/// falls repeatedly as a fault whether the modulation is fast or slow. Monotone while
/// ducking is what every hardware ducker does, and it costs nothing: the depth is bounded
/// by [`DUCK_GAIN`] either way, and the release is one smooth ramp when the effect ends.
pub fn duck_hold_next(hold: f32, target: f32, ducking: bool) -> f32 {
    if !ducking {
        return 1.0;
    }
    let t = if target.is_finite() { target } else { 1.0 };
    let h = if hold.is_finite() { hold } else { 1.0 };
    h.min(t).clamp(DUCK_GAIN, 1.0)
}

/// Split a pending duck across one render chunk: `(frames in THIS chunk that owe a
/// duck, frames still owed after it)`.
///
/// ★ 2026-08-28 — pulled out of `audio_sink.rs` so it can be tested at all. That file
/// is not `include!`-able (it pulls in rodio and oxisynth), so the chunk arithmetic —
/// the tail chunk where `0 < owed < CHUNK_FRAMES`, and an overlapping effect extending
/// the count — had **zero** tests while being the thing that decides whether the
/// envelope is a ramp or a step.
#[cfg(test)]
pub fn ducked_in_chunk(owed: u64, chunk: usize) -> (usize, u64) {
    let c = chunk as u64;
    (owed.min(c) as usize, owed.saturating_sub(c))
}

// ── 덕 상태는 한 워드다 (2026-08-28) ─────────────────────────────────────────
//
// The duck needs TWO numbers — how many frames are still owed, and how loud the effect
// is ([`duck_target`]) — written by the clip thread and read by the render thread. As
// two separate atomics they can be torn apart in both directions: the renderer can
// clear a peak in the gap before its frame count arrives (that effect then does not
// duck), and it can read a fresh peak against a stale count. Packing them into one
// `u64` makes every transition a single CAS, so the pair is always consistent.
//
// `frames` is bounded by the longest effect in the game (`sd001`, 5,968 ms = 263,188
// frames at 44.1 kHz), so 32 bits is four orders of magnitude of headroom. A
// non-negative `f32` has its top bit clear, so a peak always fits in the low half.

/// Pack the duck state: frames owed in the high half, effect-peak bits in the low half.
#[cfg(test)]
pub fn duck_pack(frames: u32, peak_bits: u32) -> u64 {
    (u64::from(frames) << 32) | u64::from(peak_bits)
}

/// Inverse of [`duck_pack`] — `(frames owed, effect-peak bits)`.
#[cfg(test)]
pub fn duck_unpack(word: u64) -> (u32, u32) {
    ((word >> 32) as u32, (word & 0xffff_ffff) as u32)
}

/// Arm (or extend) the duck with one newly queued clip.
///
/// Both halves take the MAXIMUM: overlapping effects extend the duck rather than cutting
/// it short, and the louder of them sets the depth budget. ⛔ Not `store` — a second
/// effect arriving mid-first must never shorten or quieten what is already owed.
#[cfg(test)]
pub fn duck_arm(word: u64, frames: u32, peak_bits: u32) -> u64 {
    let (f, p) = duck_unpack(word);
    duck_pack(f.max(frames), p.max(peak_bits))
}

/// Spend one render chunk of the duck: `(frames of THIS chunk that owe a duck, the new
/// state word)`.
///
/// The word is zeroed outright once the duck is exhausted, which is what retires the
/// peak **atomically with** the frame count — a released duck can then never be talked
/// back down by an effect that has already finished.
#[cfg(test)]
pub fn duck_consume(word: u64, chunk: usize) -> (usize, u64) {
    let (frames, peak) = duck_unpack(word);
    let (ducked, left) = ducked_in_chunk(u64::from(frames), chunk);
    (ducked, if left == 0 { 0 } else { duck_pack(left as u32, peak) })
}

// ── 효과음 클립은 큰 DC 값에서 한 샘플 만에 끊긴다 (2026-08-28) ───────────────
//
// ★★★ ALL FOURTEEN OF THIS GAME'S EFFECTS END ON A STEP, AND NOTHING FADED THEM.
// Decoded from the shipping jar (`res/snd/sd000..sd013_mf`, SMAF `ATR` + Yamaha
// ADPCM-B) and measured sample-exact. The last 10 ms of every clip is effectively pure
// DC — AC RMS between -101 and -41 dBFS while the MEAN sits as far out as -0.602 FS:
//
//     file    last sample        step at the default chain (x0.144)
//     sd001   -19748 (-0.603)      -21.2 dBFS
//     sd008   -17781 (-0.543)      -22.1 dBFS
//     sd007    14808 ( 0.452)      -23.7 dBFS
//     sd004    12543 ( 0.383)      -25.2 dBFS   (크리티컬)
//     …11 of 14 above -34 dBFS; the three quiet ones are sd000/sd010/sd012.
//
// A one-sample step is a broadband impulse, and because the tail carries no AC the
// effect cannot mask its own click — it is a bare low "뚝" the instant the sound stops.
// The start needs nothing: every clip's first sample is within |0.0073| (the decoder
// starts from a zero history).
//
// The DC pedestal is worth removing on its own account: it is inaudible as sound but it
// SUMS INTO THE LIMITER, so `sd008`'s -0.543 FS offset was spending the headroom
// [`duck_target`] is trying to budget.
//
// This is artifact removal, not a mix change — nothing in the audible band moves.

/// Corner frequency of the effect-bus DC blocker. Low enough to be inaudible on a
/// 4 kHz ADPCM clip, high enough that its time constant (~8 ms) actually removes a
/// pedestal that only appears in the last few tens of ms.
pub const DC_BLOCK_HZ: f32 = 20.0;

/// Linear fade applied to the END of every effect clip, in ms.
///
/// ⛔ Do not raise this past ~5 ms: `sd005` (213 ms) and `sd006` (374 ms) are short
/// percussive hits and a longer fade eats their tail. 4 ms is 1.5 % of the shortest
/// clip (`sd003`, 199 ms) and removes a step of up to -6.3 dBFS.
pub const DECLICK_TAIL_MS: f32 = 4.0;

/// Fade applied to the START of every effect clip. Only there to absorb the DC
/// blocker's own turn-on transient — the clips themselves already start near zero.
pub const DECLICK_HEAD_MS: f32 = 0.5;

/// One-pole coefficient for a `fc`-Hz DC blocker at `rate`. `0.0` when the rate is
/// unusable, which turns the filter into a plain difference (still DC-free).
pub fn dc_block_pole(rate: u32, fc: f32) -> f32 {
    if rate == 0 {
        return 0.0;
    }
    let p = 1.0 - 2.0 * core::f32::consts::PI * fc / rate as f32;
    p.clamp(0.0, 0.9999)
}

/// `ms` of `rate` expressed in frames, at least 1 and never more than `frames`.
pub fn declick_frames(ms: f32, rate: u32, frames: usize) -> usize {
    if frames == 0 || rate == 0 || ms <= 0.0 {
        return 0;
    }
    let n = (ms / 1000.0 * rate as f32).round() as usize;
    n.max(1).min(frames)
}

/// Remove the DC pedestal and fade both edges of one interleaved effect clip, in place.
///
/// `channels` may be 1 or 2; `rate` is the CLIP's own sample rate (4 or 8 kHz here),
/// not the bus rate. Runs once per clip on the audio thread's receive loop — never in
/// the render callback.
///
/// Idempotent in the way that matters: a clip that is already DC-free and already ends
/// at zero comes out unchanged to within the filter's own rounding.
pub fn declick(samples: &mut [f32], channels: usize, rate: u32) {
    let ch = channels.max(1);
    if samples.is_empty() || samples.len() % ch != 0 {
        return;
    }
    let frames = samples.len() / ch;
    let pole = dc_block_pole(rate, DC_BLOCK_HZ);
    for c in 0..ch {
        let (mut x1, mut y1) = (0.0f32, 0.0f32);
        for f in 0..frames {
            let x = samples[f * ch + c];
            let y = x - x1 + pole * y1;
            x1 = x;
            y1 = y;
            samples[f * ch + c] = y;
        }
    }
    let head = declick_frames(DECLICK_HEAD_MS, rate, frames);
    for f in 0..head {
        let g = (f as f32 + 1.0) / (head as f32 + 1.0);
        for c in 0..ch {
            samples[f * ch + c] *= g;
        }
    }
    let tail = declick_frames(DECLICK_TAIL_MS, rate, frames);
    for k in 0..tail {
        let g = k as f32 / tail as f32;
        let f = frames - 1 - k;
        for c in 0..ch {
            samples[f * ch + c] *= g;
        }
    }
}

/// Loudest absolute sample, or `0.0` for an empty slice. The effect peak
/// [`duck_target`] needs, measured once per clip where the samples already are.
pub fn peak_abs(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

// ── 클립 하나만 줄이기 (2026-08-28) ──────────────────────────────────────────
//
// ★ [`SFX_LEVELS`] is the whole BUS. It cannot fix a clip that is louder than its
// neighbours in the source material, because moving it moves every effect together.
// The 크리티컬 hit is exactly that case — player request 2026-08-28: *"크리티컬 발생시
// 나오는 효과음을 조금 줄일수도있어?"*
//
// Measured from the shipping jar (`res/snd/sd*_mf`, SMAF `ATR` + Yamaha ADPCM-B):
//
//     clip     rate    i16 count   dur       peak dBFS   RMS dBFS
//     sd004    8 kHz       7,384   923 ms      0.00       -6.80   <- 크리티컬
//     sd008    8 kHz       7,502   938 ms      0.00       -3.90
//     sd006    8 kHz       2,988   374 ms      0.00       -7.72
//     sd009    8 kHz      13,608  1701 ms      0.00      -10.59
//
// `sd004` is brickwalled at 0 dBFS *and* fires on every critical, which is what makes
// it stand out rather than its absolute level (`sd008` is 3 dB hotter in RMS but rare).
//
// ⚠ WHY THE KEY IS `(rate, count)`: by the time the PCM reaches the host it has no name
// — `AudioSink::play_wave(channel, rate, data)` is the whole interface, and the SMAF
// filename never crosses it. The i16 count and the rate DO survive: the game's effect
// slider scales sample VALUES (`SmafPlayer::play`'s `Wave` arm), never the count.
// The counts of all fourteen clips are distinct, so this identifies a clip exactly.
// ⚠ The count is matched with a small tolerance so a decoder that ends a clip one nibble
// differently than the measurement did still matches. `audio_sink` logs the fingerprint
// of every clip it plays at `info`, so a mismatch is visible on the device rather than
// silent — that log line is also how to find the key for the NEXT clip someone wants
// adjusted.

/// How far a clip's measured `i16` count may differ from [`CLIP_TRIMS`] and still match,
/// as a fraction. 2 % of `sd004` is ±147 samples, while its nearest neighbour
/// (`sd008`, 7,502) is 118 away — so this is deliberately tight enough that the two
/// cannot be confused… and that is checked by `clip_trims_cannot_collide`.
pub const CLIP_FINGERPRINT_TOLERANCE: f32 = 0.01;

/// Per-clip trims on top of [`sfx_trim`]: `(sample rate, i16 count, gain)`.
///
/// `1.0` would be a no-op entry; only put a clip here when it is out of balance with the
/// others *in the source material*, and say why. ⛔ Do not use this to change the level
/// of the effect bus as a whole — that is [`SFX_LEVELS`], which the player controls.
pub const CLIP_TRIMS: &[(u32, usize, f32)] = &[
    // 크리티컬 (`sd004_mf`). -3 dB, the same 3 dB grid the settings row uses: the player
    // asked for "조금", and this clip is brickwalled at 0 dBFS while firing on every crit.
    (8_000, 7_384, 0.708),
];

/// The extra gain for one clip, or `1.0` when it is not in [`CLIP_TRIMS`].
///
/// Fails toward NO CHANGE: an unknown clip, a zero rate and an empty buffer all answer
/// `1.0`. ⛔ Never returns 0 — a fingerprint bug must not silence an effect.
pub fn clip_trim(rate: u32, samples: usize) -> f32 {
    if rate == 0 || samples == 0 {
        return 1.0;
    }
    for &(r, n, g) in CLIP_TRIMS {
        if r != rate || n == 0 {
            continue;
        }
        let slack = (n as f32 * CLIP_FINGERPRINT_TOLERANCE).max(1.0);
        if (samples as f32 - n as f32).abs() <= slack {
            return g;
        }
    }
    1.0
}

/// Where [`soft_limit`] stops being a straight wire, in linear amplitude (-3.1 dBFS).
pub const LIMIT_KNEE: f32 = 0.7;

/// Master soft limiter — the last thing the mixed BGM+SFX bus passes through.
///
/// Below [`LIMIT_KNEE`] this is the identity, so ordinary play is bit-for-bit
/// untouched. Above it the curve bends asymptotically to 1.0 and **can never exceed
/// it**, which is the point: AAudio hard-clamps float PCM to [-1, 1], and hard
/// clipping a loud effect summed with quiet music does not just distort the effect —
/// it deletes the music inside every clipped sample. That is symptom ③.
///
/// Stateless (no attack/release, so no pumping and nothing to get wrong in the
/// real-time callback) and C¹-continuous at the knee: the derivative of the upper
/// branch at `over = 0` is exactly 1, so there is no audible corner where it engages.
pub fn soft_limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= LIMIT_KNEE {
        return x;
    }
    let head = 1.0 - LIMIT_KNEE;
    let over = a - LIMIT_KNEE;
    let y = LIMIT_KNEE + head * (over / (over + head));
    if x < 0.0 { -y } else { y }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(v: f32) -> f32 {
        20.0 * v.log10()
    }

    #[test]
    fn limiter_is_transparent_below_the_knee() {
        for &x in &[0.0f32, 0.01, 0.25, 0.5, 0.69, LIMIT_KNEE] {
            assert_eq!(soft_limit(x), x, "positive {x}");
            assert_eq!(soft_limit(-x), -x, "negative {x}");
        }
    }

    #[test]
    fn limiter_never_reaches_full_scale() {
        // The whole reason it exists: AAudio clamps, and a clamp destroys the quiet
        // signal riding on the loud one.
        for i in 0..2000 {
            let x = i as f32 * 0.01; // up to 20.0, far past anything reachable
            let y = soft_limit(x);
            assert!(y < 1.0, "soft_limit({x}) = {y} must stay below full scale");
            assert!(soft_limit(-x) > -1.0);
        }
    }

    #[test]
    fn limiter_is_monotonic_and_odd() {
        let mut prev = f32::NEG_INFINITY;
        for i in 0..500 {
            let x = i as f32 * 0.01;
            let y = soft_limit(x);
            assert!(y > prev, "must be monotonic at {x}");
            prev = y;
            assert!((soft_limit(-x) + y).abs() < 1e-6, "must be odd at {x}");
        }
    }

    #[test]
    fn limiter_is_smooth_at_the_knee() {
        // No corner: the slope approaching the knee from above is ~1, so the limiter
        // fades in rather than switching on.
        let e = 1e-4;
        let slope = (soft_limit(LIMIT_KNEE + e) - soft_limit(LIMIT_KNEE)) / e;
        assert!((slope - 1.0).abs() < 1e-2, "slope at knee = {slope}");
    }

    #[test]
    fn worst_case_mix_survives_the_limiter() {
        // Measured worst case with the shipping bank AND scores: `bgm3_mf` peaks at
        // 2.625 at unity synth gain, so 0.998 as it is actually mixed, and a full-scale
        // effect adds SFX_TRIM on top of it. It was the same song at 1.182 and unity
        // gain when the bundled font was GeneralUser.
        let sum = 2.625 * SYNTH_GAIN + 1.0 * SFX_TRIM;
        let out = soft_limit(sum);
        // Below 1.0 (the guarantee) and above 0.85 (the headroom is not being wasted).
        assert!(out < 1.0 && out > 0.85, "worst-case mix {sum} -> {out}");
    }

    #[test]
    fn the_two_buses_end_up_within_a_sane_distance() {
        // ★★★ 2026-09-04 — THIS TEST NOW MEASURES RMS, AND THAT IS THE POINT.
        //
        // It used to compare PEAKS (`bgm0_mf` 0.656 at unity vs a brickwalled effect)
        // and that worked only because GeneralUser's sampled instruments decay, so peak
        // and loudness tracked each other. The MA-3 bank is chip tones: they sustain,
        // and the same scores come out with a much lower crest factor. Peak-matching it
        // would have made the music ~2.8 dB louder than every constant in this file was
        // tuned against, which is exactly the balance [`SFX_TRIM`] exists to hold.
        //
        // So the invariant is stated where it was actually measured: the music bus must
        // land at the loudness the effect trim was tuned against. Aggregate RMS over all
        // 19 `bgm*_mf`, 20 s renders at unity synth gain:
        //
        //     GeneralUser + original scores  -20.17 dBFS  (shipped at SYNTH_GAIN 1.0)
        //     MA-3        + RETUNED scores   -11.84 dBFS  (ships at 0.38 -> -20.24)
        let bgm_rms_at_unity = -11.84_f32;
        let tuned_against = -20.17_f32;
        let shipped = bgm_rms_at_unity + db(SYNTH_GAIN);
        assert!(
            (shipped - tuned_against).abs() < 0.5,
            "the music bus must stay where SFX_TRIM was tuned: {shipped:.1} vs {tuned_against:.1} dBFS"
        );
        // And the gap to the effect bus has to stay where the player left it. The
        // SFX_TRIM note above records the tuning target as a **+2.6 dB RMS gap at
        // matched sliders**; measuring both buses the same way (aggregate RMS, this
        // tool) reproduces it as +2.4 dB on the old bank and +2.3 dB on this one:
        //
        //     SFX  14 `sd*_mf`, whole clip, unity   -9.8 dBFS  -> -17.8 at SFX_TRIM
        //     BGM  19 `bgm*_mf`, 20 s, unity      -11.84 dBFS  -> -20.24 at SYNTH_GAIN
        //
        // ⛔ Compare aggregate WITH aggregate. `sd008_mf` alone is -3.9 dBFS, and
        // putting that one clip against the 19-song aggregate reads +8.3 dB and means
        // nothing — it is the loudest effect against the average of all the music.
        let sfx_rms_at_unity = -9.8_f32;
        let gap = (sfx_rms_at_unity + db(SFX_TRIM)) - shipped;
        let tuned_gap = 2.6_f32;
        assert!(
            (gap - tuned_gap).abs() < 1.0,
            "the bus gap the player tuned must survive the bank swap: {gap:+.1} vs {tuned_gap:+.1} dB"
        );
    }

    #[test]
    fn the_duck_gesture_is_a_fade_and_not_an_event() {
        // ★★★ THE 2026-08-30 REGRESSION TEST. 71차-Q measured the old attack at 502 dB/s
        // and wrote down that a step that fast "is an event and not a fade" — and then
        // left it at 12 ms. That number IS the reported 「툭」, so it is a test now.
        let span_db = -db(DUCK_GAIN); // 6.02 dB, the deepest the duck can ever go
        let attack_rate = span_db / (DUCK_ATTACK_MS / 1000.0);
        let release_rate = span_db / (DUCK_RELEASE_MS / 1000.0);
        assert!(attack_rate <= 60.0, "duck attack {attack_rate:.0} dB/s is a click, not a fade");
        assert!(release_rate <= 30.0, "duck release {release_rate:.0} dB/s is a click, not a fade");
        // …and the other side of the trade: the duck still has to be in place for the body
        // of the SHORTEST effect the game ships (`sd003`, 199 ms).
        assert!(
            DUCK_ATTACK_MS < 199.0 * 0.75,
            "attack {DUCK_ATTACK_MS} ms leaves the shortest effect unducked"
        );
    }

    #[test]
    fn the_music_level_detector_ignores_the_waveform() {
        // The renderer's chunk is 1.45 ms, which is less than one cycle of a bass note, so
        // its raw peak collapses at every zero crossing. Driven with a real 60 Hz sine at
        // `bgm3_mf`'s peak, the raw number swings across `duck_target`'s threshold and the
        // detector does not.
        let (rate, chunk, amp, sfx) = (44_100u32, 64usize, 1.18f32, 0.144f32);
        let chunk_peak = |k: usize| {
            (0..chunk).fold(0.0f32, |p, f| {
                let t = (k * chunk + f) as f32 / rate as f32;
                p.max((amp * (2.0 * core::f32::consts::PI * 60.0 * t).sin()).abs())
            })
        };

        let decay = music_env_decay(MUSIC_ENV_RELEASE_MS, rate, chunk);
        let mut env = 0.0f32;
        let (mut raw_lo, mut raw_hi) = (f32::MAX, 0.0f32);
        let (mut env_lo, mut env_hi) = (f32::MAX, 0.0f32);
        for k in 0..600 {
            let p = chunk_peak(k);
            env = music_env_next(env, p, decay);
            if k < 100 {
                continue; // let the detector fill; it starts at 0 by design
            }
            raw_lo = raw_lo.min(duck_target(p, sfx));
            raw_hi = raw_hi.max(duck_target(p, sfx));
            env_lo = env_lo.min(duck_target(env, sfx));
            env_hi = env_hi.max(duck_target(env, sfx));
        }
        assert!(
            raw_hi - raw_lo > 0.4,
            "the raw chunk peak should swing the target across its whole range, got {raw_lo:.3}..{raw_hi:.3}"
        );
        assert!(
            env_hi - env_lo < 0.01,
            "the detector must hold the target still, got {env_lo:.3}..{env_hi:.3}"
        );
    }

    #[test]
    fn the_music_level_detector_attacks_at_once_and_decays_on_its_own_clock() {
        let (rate, chunk) = (44_100u32, 64usize);
        let decay = music_env_decay(MUSIC_ENV_RELEASE_MS, rate, chunk);
        // A transient is never missed: one chunk is enough to reach it.
        assert_eq!(music_env_next(0.0, 0.9, decay), 0.9);
        // …and one 1.45 ms chunk barely moves it back down.
        assert!(decay > 0.99, "decay {decay} is far too fast for a level detector");

        // One time constant of silence leaves 1/e of it.
        let chunks = (MUSIC_ENV_RELEASE_MS / 1000.0 * rate as f32 / chunk as f32).round() as usize;
        let mut env = 1.0f32;
        for _ in 0..chunks {
            env = music_env_next(env, 0.0, decay);
        }
        assert!((env - core::f32::consts::E.recip()).abs() < 0.01, "after one tau: {env}");

        // An unusable timebase degrades to the plain per-chunk peak rather than to a
        // detector that never decays (which would hold a duck open forever).
        assert_eq!(music_env_decay(MUSIC_ENV_RELEASE_MS, 0, chunk), 0.0);
        assert_eq!(music_env_decay(MUSIC_ENV_RELEASE_MS, rate, 0), 0.0);
        assert_eq!(music_env_next(1.0, 0.25, 0.0), 0.25);
        // Garbage in never latches a duck: NaN cannot survive into the level.
        assert_eq!(music_env_next(f32::NAN, 0.5, decay), 0.5);
        assert_eq!(music_env_next(0.5, f32::NAN, 0.0), 0.0);
    }

    #[test]
    fn the_duck_depth_is_monotone_while_an_effect_sounds() {
        // One dip and one recovery, whatever the music does underneath.
        let mut hold = 1.0f32;
        hold = duck_hold_next(hold, 0.8, true);
        assert_eq!(hold, 0.8);
        hold = duck_hold_next(hold, 0.6, true);
        assert_eq!(hold, 0.6, "a louder passage may deepen the duck");
        hold = duck_hold_next(hold, 1.0, true);
        assert_eq!(hold, 0.6, "⛔ but a quieter one must not lift it back mid-effect");
        // The release is the ONLY way out, and it is immediate as a target — the ramp to
        // it is `duck_next_to`'s job, at `DUCK_RELEASE_MS`.
        assert_eq!(duck_hold_next(hold, 0.6, false), 1.0);
        // Never deeper than the cap, and never poisoned by a bad target.
        assert_eq!(duck_hold_next(1.0, 0.0, true), DUCK_GAIN);
        assert_eq!(duck_hold_next(1.0, f32::NAN, true), 1.0);
        assert_eq!(duck_hold_next(f32::NAN, 0.7, true), 0.7);
    }

    #[test]
    fn a_whole_effect_is_one_dip_and_one_recovery() {
        // End to end at the settings that reproduce the report: 배경음 at maximum over
        // `bgm3_mf`, a default-slider effect, 500 ms long. The gain must be monotone down
        // then monotone up — no direction change while the effect sounds, which is what
        // 「툭」「툭」 through a fight sounds like.
        let (rate, chunk) = (44_100u32, 64usize);
        let (attack, release) = (duck_step(DUCK_ATTACK_MS, rate), duck_step(DUCK_RELEASE_MS, rate));
        let decay = music_env_decay(MUSIC_ENV_RELEASE_MS, rate, chunk);
        let sfx_bits = 0.144f32.to_bits();
        let effect_frames = (0.5 * rate as f32) as u32;

        let mut word = duck_arm(0, effect_frames, sfx_bits);
        let (mut gain, mut hold, mut env) = (1.0f32, 1.0f32, 0.0f32);
        let mut trace: Vec<f32> = Vec::new();
        for k in 0..1000 {
            let (ducked, next) = duck_consume(word, chunk);
            let want = duck_target(env * 1.0, f32::from_bits(duck_unpack(word).1));
            hold = duck_hold_next(hold, want, ducked > 0);
            word = next;
            for i in 0..chunk {
                gain = duck_next_to(gain, if i < ducked { hold } else { 1.0 }, attack, release);
            }
            trace.push(gain);
            // A 60 Hz sine's chunk peak, the same driver as the detector test.
            let peak = (0..chunk).fold(0.0f32, |p, f| {
                let t = (k * chunk + f) as f32 / rate as f32;
                p.max((1.18 * (2.0 * core::f32::consts::PI * 60.0 * t).sin()).abs())
            });
            env = music_env_next(env, peak, decay);
        }

        let bottom = trace.iter().cloned().fold(f32::MAX, f32::min);
        assert!(bottom < 0.99, "the duck must actually engage at these settings, got {bottom:.3}");
        assert!(bottom >= DUCK_GAIN - 1e-6, "and never past the cap, got {bottom:.3}");
        let low = trace.iter().position(|g| *g == bottom).unwrap();
        for w in trace[..=low].windows(2) {
            assert!(w[1] <= w[0] + 1e-6, "the dip must be monotone: {w:?}");
        }
        for w in trace[low..].windows(2) {
            assert!(w[1] >= w[0] - 1e-6, "the recovery must be monotone: {w:?}");
        }
        assert!(
            (trace.last().copied().unwrap() - 1.0).abs() < 1e-6,
            "…and it must finish back at unity, got {:?}",
            trace.last()
        );
    }

    #[test]
    fn the_effect_bus_sits_below_unity_but_stays_audible() {
        // The player asked for "아주 조금만" quieter, not for the effects to disappear.
        // Anything past about -12 dB stops reading as a hit at all.
        let trim_db = db(SFX_TRIM);
        assert!(
            (-12.0..=-4.0).contains(&trim_db),
            "SFX_TRIM {trim_db:+.1} dB is outside the useful band"
        );
    }
}
