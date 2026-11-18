// 공격속도 / 스킬사용속도 — the host half of the new stat (2026-09-23, TODO `DT`).
//
// The guest half is `wie_core_arm::binary_patches::atkspeed`: one hook on the tail of
// `0x4aee0`, the accessor every pose-length comparison in the player's field state
// machine goes through. It reads ONE number out of `wie_util::stats`, the percent this
// file computes. `0` is off, and off is byte-identical to the original.
//
// ── WHERE THE NUMBER COMES FROM ──────────────────────────────────────────────
// Equipment options. `0x4f014`, the game's own stat recompute, walks the nine worn
// slots and for each one does
//
// ```text
// row = *(*(grid + 0) + cat * 4) + idx * 0x20          // the category's DB table
// addStat((i8)row[0x1a], (i8)row[0x1b])                // option pair 1
// addStat((i8)row[0x1c], (i8)row[0x1d])                // option pair 2
// ```
//
// (runtime `row+0x1a` is byte `b16` of a 20-byte equipment row — the runtime row is the
// asset row with 10 bytes of pointers in front, checked against `2b 00 11 00` in 스피어's
// rows — and `b14` of an 18-byte accessory row: different asset offsets, the SAME runtime
// one, which is why the guest can read it without knowing the category, and why this code
// reads `+0x1a` rather than computing from an asset byte number.)
// `addStat` is bounds-checked: `id - 2 > 0x15` returns without doing
// anything, so an id of [`ATTACK_SPEED_STAT`] is **silently ignored by the guest** and
// is ours to define. Nothing else in the image reads those two pairs.
//
// ⛔ THE STOCK ASSET'S ROWS DO NOT USE THEM. All 13 equipment categories of the original
// asset were scanned (131차): **zero** rows have a non-zero option pair. The legacy rows
// (133차) are the first to carry any, and of those only `"섬광의군화` carries
// [`ATTACK_SPEED_STAT`] — so for a character not wearing it the sum below is 0, the knob
// stays disarmed and the hook keeps the guest's own timing.
//
// The item-detail screen names an option `label[80 + statId]` (`tools_re/ingame_ids.txt`
// numbering), so the id and its label move together: `add_attack_speed_label.py` appended
// 「공격속도 +」 as row 198 of `InGame_txt` (131차-D), which is where `0x76` = 198 − 80 comes
// from, and `patch_asset.py --check` holds the boots' id to that row (135차-B).
//
// This file is pure and has no dependencies: `atkspeed_selftest.rs` `include!`s it.
// ⛔ Do not add a `use crate::…` here.

/// The option id 공격속도 rides on.
///
/// Two constraints fix it. ABOVE the guest's own range: `addStat`'s gate is
/// `id - 2 <= 0x15`, i.e. ids `0x02..=0x17` do something and everything else is a no-op, so
/// an item can carry this id without the game adding it to some unrelated derived stat.
/// And ON THE LABEL ROW: the detail screen prints `label[80 + id]`, and 「공격속도 +」 is row
/// 198 of `InGame_txt` (131차-D) — 198 − 80 = `0x76`. The first cut used `0x18` (the first
/// ignored id) and the screen called the option 「힘 +」, row 104.
///
/// ⛔ Do not move it into `0x02..=0x17` — every one of those is a real stat (`0x04` looks
/// free but it is an empty arm INSIDE the gate, and the label table still names it).
/// ⛔ Do not move it without moving the label row (`add_attack_speed_label.py`) and
/// `add_legacy_boots.ATTACK_SPEED_STAT` with it.
pub const ATTACK_SPEED_STAT: u8 = 0x76;

/// The most the host will ever publish, matching the hook's own cap. The measured ceiling
/// is lower still — 3.00x at +300 % (132차-B: every pose keeps two ticks, the last one the
/// game's own), so past +300 % more percent buys nothing.
pub const MAX_PCT: u32 = 400;

/// Worn slots the game's stat recompute walks (`0x4f014`).
pub const WORN_SLOTS: usize = 9;

/// Sum the 공격속도 carried by a set of `(statId, value)` option pairs.
///
/// Negative values subtract (an item may legitimately be heavy), the total is floored at
/// 0 — a negative total would mean "longer than the original", and the hook has no way to
/// express that — and capped at [`MAX_PCT`].
pub fn sum_options(pairs: &[(u8, i8)]) -> u32 {
    let total: i32 = pairs
        .iter()
        .filter(|(id, _)| *id == ATTACK_SPEED_STAT)
        .map(|(_, v)| *v as i32)
        .sum();
    total.clamp(0, MAX_PCT as i32) as u32
}

/// `wie_attack_speed.txt` — the device/probe test knob, read the way
/// `wie_reaver_test.txt` is: one file, one line, applied to THIS run only.
///
/// ```text
/// pct 50
/// ```
///
/// `None` = no override (use the equipment sum). `Some(0)` explicitly disarms, which is
/// how a tester turns the feature off again without deleting the file.
///
/// ⛔ Anything unparseable is `None`, never 0: a typo must not silently look like a
/// deliberate "off" in a log that is trying to explain why nothing got faster.
pub fn parse_override(text: &str) -> Option<u32> {
    let mut found = None;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some(rest) = line.strip_prefix("pct") else { continue };
        if let Ok(v) = rest.trim().parse::<u32>() {
            found = Some(v.min(MAX_PCT));
        }
    }
    found
}

/// What the host publishes: the override when a tester armed one, else the equipment sum.
pub fn effective_pct(equipment: u32, override_pct: Option<u32>) -> u32 {
    override_pct.unwrap_or(equipment).min(MAX_PCT)
}

/// One basic attack, MEASURED (132차-B, 2026-09-24): probe frames from the key to idle,
/// ×100, at each percent — the pinned-monster harness, 40+ attacks per point, one facing.
///
/// A probe frame is half a game tick: 1 frame of entry, then 2 per tick of swing (the pose
/// script, state 2) and of follow-through (state 4's countdown). At 0 % that is
/// 1 + 2×3 + 2×4 = 15. The floor is 1 + 2×1 + 2×1 = 5 and it is reached by +300 %:
///
/// * the swing's counter is dragged to `frames - 1` in the tick its pose is set and the
///   guest's own `+= 1` finishes it on the next — and that next tick is ALSO the
///   follow-through's first, which is why a "two-tick" swing costs one visible tick;
/// * the follow-through ends on the first tick the countdown reaches `<= 0`.
///
/// ⛔ A table, not a formula: the hook's two carries cycle with a phase that depends on how
/// many times per attack the game asks `0x4aee0`, which is not a number this crate knows —
/// a replay of the arithmetic came out non-monotonic (+127 % beating +150 %). Between the
/// points the length is interpolated. +25 % is measured as nearly nothing: its extra tick
/// almost always lands in the follow-through, where it cannot end the countdown sooner.
pub const ATTACK_FRAMES_MEASURED: [(u32, u32); 8] =
    [(0, 1500), (25, 1490), (50, 1100), (100, 900), (127, 847), (200, 700), (300, 500), (400, 500)];

/// A basic attack's length at `pct`, in probe frames ×100 — [`ATTACK_FRAMES_MEASURED`],
/// interpolated. Purely informational.
pub fn attack_frames_x100(pct: u32) -> u32 {
    let pct = pct.min(MAX_PCT);
    let t = &ATTACK_FRAMES_MEASURED;
    for w in t.windows(2) {
        let ((p0, f0), (p1, f1)) = (w[0], w[1]);
        if pct <= p1 {
            // f is non-increasing along the table, so this never underflows.
            return f0 - (f0 - f1) * (pct - p0) / (p1 - p0);
        }
    }
    t[t.len() - 1].1
}

/// Roughly how fast an attack runs at `pct`, x100 — for the log line only.
///
/// The guest hook does not shorten any pose: it drags the pose clock towards the length
/// `0x4aee0` is answering for the pose running right now, and stops one tick short so the
/// game's own `+= 1` finishes it (a pose that completes on the tick it starts loses its
/// hit). ⛔ There is no cap on the bonus — the clamp against the LIVE length is what makes
/// it safe, not a step limit (see `binary_patches::atkspeed`). Measured ceiling: 3.00x,
/// reached at +300 % — the old estimate here said 2.00x and was simply wrong.
pub fn speed_x100_at(pct: u32) -> u32 {
    attack_frames_x100(0) * 100 / attack_frames_x100(pct).max(1)
}

// ── HOLDING THE ATTACK KEY (132차, rules tightened in 132차-B and 132차-C) ────
//
// `handleCletEvent` throws KEY REPEATED away, so a thumb resting on OK is ONE keydown and
// ONE swing. The worker therefore hands the guest fresh keydowns while the key stays down,
// and this is the rule for WHEN — pure, so it is tested here and the probe
// (`wie_probe --hold-key`) runs the very same code against the real game.
//
// ★★★ AN ATTACK IS A COMBO, AND THE COMBO IS BUILT BY PRESSES DURING THE SWING (132차-C).
// Traced headless (2026-09-24, mashing OK every 4 frames): the attack's pose script
// `CHAR+0x23f[8]` starts as `[2]`, and an OK arriving while that pose runs APPENDS the next
// step — `[2, 3]`, then `[2, 3, 4]` (공격 1·2·3) — until the weapon's combo is full; only
// when the script runs out does the player drop into state 4 (the 장전/finish) and then
// idle. The first rule re-pressed only at IDLE, so every attack was a fresh `[2]`:
// "공격1, 장전, 공격1, 장전" (player, 2026-09-24). So a live hold also presses ONCE PER
// STEP while the swing is running with no next step queued — exactly what a perfect
// masher does, and nothing the game would not accept from a thumb.
//
// ⛔⛔ OK is not only "attack". The same key talks to the NPC the player is facing,
// turns a dialogue's pages, closes its last one, confirms a choice and dismisses a box.
// Measured headless (2026-09-24, scene 111, the 소녀 at 17,12): an OK arriving three frames
// after her dialogue closed OPENED IT AGAIN. The first rule (vc401) re-pressed whenever
// the field was idle, so an ordinary tap on a last page closed it and — the thumb still
// down 33 ms later — talked to her again. Hence three rules, all load-bearing:
//
// 1. A hold that has EVER seen a window, a dialogue, a choice, a modal box or a menu is
//    spoiled until the key is released. One that starts on a dialogue page is spoiled
//    before it can press anything, which is exactly the last-page case.
// 2. Never in the worker iteration of the physical press itself. That keydown has not been
//    through a tick yet, so "idle" is still the state from BEFORE it; vc401 answered every
//    press with a second keydown in the same frame.
// 3. At most every [`ATTACK_REPEAT_MS`] from idle, so a REFUSED attack (a cooldown, an OK
//    the game ignored) does not re-press on every single frame. ⛔ The combo press is NOT
//    under that floor — at the top of the stat a pose is one tick (~20 ms) long, so a
//    33 ms gap would skip steps. It is bounded by "once per step" instead.

/// How often a held attack key may be handed to the guest again.
///
/// ⛔ Shorter than the worker's generic key-repeat interval on purpose: an attack at the
/// top of the stat is only two or three ticks long, so a 100 ms floor would itself become
/// the cap on attack speed. It is not the pacing — the "back to idle" gate is.
pub const ATTACK_REPEAT_MS: u64 = 33;

/// What one worker iteration saw, for the held-attack decision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HoldView {
    /// On the field, no box up, and the player's own state byte at idle — the previous
    /// action has finished (`items::walk_idle_on_field`).
    pub idle: bool,
    /// Something other than the field owns OK right now: a window (dialogue, menu, shop),
    /// a choice, a modal box, or no field at all.
    pub ui: bool,
    /// `Some(step)` while the attack script is running (state 2) on `step` with NO next
    /// step queued yet — an OK now would chain the combo. `None` otherwise, including when
    /// the next step is already there.
    pub combo_open: Option<i8>,
}

/// One held attack key (OK or the 보조무기 `7`), from the physical press to the release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttackHold {
    /// The worker iteration the physical press was delivered in (rule 2).
    pressed_frame: u64,
    /// Earliest time the next keydown may go out (rule 3).
    next_ms: u64,
    /// Rule 1: this hold has seen OK mean something other than "attack".
    spoiled: bool,
    /// The script step this hold last chained a combo press for — once per step.
    chained: Option<i8>,
}

impl AttackHold {
    /// A hold whose physical press went to the guest in iteration `frame`, at `now_ms`.
    pub fn new(now_ms: u64, frame: u64) -> Self {
        Self {
            pressed_frame: frame,
            next_ms: now_ms.saturating_add(ATTACK_REPEAT_MS),
            spoiled: false,
            chained: None,
        }
    }

    /// Whether a UI has spoiled this hold — for the log line that explains a silence.
    pub fn spoiled(&self) -> bool {
        self.spoiled
    }

    /// Feed one iteration's view; `true` = hand the guest one more keydown now.
    ///
    /// ⛔ Call it on EVERY iteration the key is held, not only when a press is due: a
    /// dialogue that opens and closes between two due checks must still spoil the hold.
    pub fn step(&mut self, now_ms: u64, frame: u64, view: HoldView) -> bool {
        if view.ui {
            self.spoiled = true;
        }
        if self.spoiled || frame <= self.pressed_frame {
            return false;
        }
        if view.idle {
            // A new combo from here on: step 0 may be chained again.
            self.chained = None;
            if now_ms < self.next_ms {
                return false;
            }
            self.next_ms = now_ms.saturating_add(ATTACK_REPEAT_MS);
            return true;
        }
        // The swing is running with room for another step: chain it, once per step. A
        // press the game refuses (the combo is full) is not retried on the same step.
        match view.combo_open {
            Some(step) if self.chained != Some(step) => {
                self.chained = Some(step);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stat_id_is_one_the_guest_ignores() {
        // `addStat`'s gate, verbatim: `(u32)(id - 2) > 0x15` -> return.
        let guest_acts_on = |id: u8| (id.wrapping_sub(2)) <= 0x15;
        assert!(!guest_acts_on(ATTACK_SPEED_STAT), "the guest must not act on our id");
        for id in 0x02u8..=0x17 {
            assert!(guest_acts_on(id), "id {id:#04x} is one of the game's own");
        }
        assert!(!guest_acts_on(0x18));
        // ★ The id is not free-floating: it is WHERE the 공격속도 label landed when
        // `tools_re/add_attack_speed_label.py` appended it to `InGame_txt`
        // (index 198, and the detail screen reads `label[80 + id]`). Move the row and this
        // number moves with it, or the option line goes back to naming somebody else's stat.
        assert_eq!(ATTACK_SPEED_STAT, 198 - 80, "id must match the label's index");
    }

    #[test]
    fn nothing_equipped_is_nothing_published() {
        assert_eq!(sum_options(&[]), 0);
        // Every other option id is somebody else's stat.
        assert_eq!(sum_options(&[(0x02, 50), (0x11, 20), (0x0f, 90)]), 0);
    }

    #[test]
    fn options_add_up_and_stay_in_range() {
        assert_eq!(sum_options(&[(ATTACK_SPEED_STAT, 10)]), 10);
        assert_eq!(sum_options(&[(ATTACK_SPEED_STAT, 10), (ATTACK_SPEED_STAT, 15)]), 25);
        // Mixed with other stats, only ours counts.
        assert_eq!(sum_options(&[(0x02, 90), (ATTACK_SPEED_STAT, 30), (0x11, 5)]), 30);
        // A negative subtracts but can never ask for a SLOWER attack.
        assert_eq!(sum_options(&[(ATTACK_SPEED_STAT, 30), (ATTACK_SPEED_STAT, -10)]), 20);
        assert_eq!(sum_options(&[(ATTACK_SPEED_STAT, -30)]), 0);
        // Nine slots of the largest an i8 can hold still cannot exceed the cap.
        let all = [(ATTACK_SPEED_STAT, 127i8); WORN_SLOTS * 2];
        assert_eq!(sum_options(&all), MAX_PCT);
    }

    #[test]
    fn the_test_file_is_read_the_way_the_reaver_one_is() {
        assert_eq!(parse_override("pct 50\n"), Some(50));
        assert_eq!(parse_override("  pct   120  \n"), Some(120));
        assert_eq!(parse_override("pct 0\n"), Some(0), "an explicit off");
        assert_eq!(parse_override("pct 9000\n"), Some(MAX_PCT), "capped, not refused");
        // Comments and unrelated lines.
        assert_eq!(parse_override("# pct 50\nrate 3\npct 25"), Some(25));
        // ⛔ A typo is "no override", not "off".
        assert_eq!(parse_override("pct fifty"), None);
        assert_eq!(parse_override("pctt 50"), None);
        assert_eq!(parse_override(""), None);
        assert_eq!(parse_override("pct -5"), None);
    }

    #[test]
    fn the_override_wins_and_absence_does_not() {
        assert_eq!(effective_pct(0, None), 0);
        assert_eq!(effective_pct(30, None), 30);
        assert_eq!(effective_pct(30, Some(0)), 0, "a tester can turn it off");
        assert_eq!(effective_pct(0, Some(75)), 75);
        assert_eq!(effective_pct(10, Some(9999)), MAX_PCT);
    }

    #[test]
    fn the_quoted_length_matches_the_measured_attack() {
        // The measured points come back exactly — they are the table.
        for (pct, frames) in ATTACK_FRAMES_MEASURED {
            assert_eq!(attack_frames_x100(pct), frames, "pct {pct}");
        }
        // 1 entry frame + a 3-tick swing + a 4-tick follow-through, two frames a tick.
        assert_eq!(attack_frames_x100(0), (1 + 2 * 3 + 2 * 4) * 100);
        // The floor: one visible tick of each. Nothing may claim to beat it.
        for pct in 0..=MAX_PCT + 100 {
            assert!(attack_frames_x100(pct) >= (1 + 2 + 2) * 100, "pct {pct}");
        }
    }
}
