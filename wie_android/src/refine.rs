// 제련 확률 (REFINE ODDS) — the pure half.
//
// ── WHY THIS EXISTS (player request, 2026-08-26) ─────────────────────────────
// "제련시 제련확률을 표시해주고싶어. … 제련석을 올릴때마다 확률도 마찬가지로 계산되어서
//  최신화 되어야해."
//
// The game never tells the player their odds. 71차-H decoded the smith's whole
// judgement statically; this module is that judgement written back out as arithmetic,
// so the host can print the numbers the game is about to roll against.
//
// ── DEPENDENCY-FREE ──────────────────────────────────────────────────────────
// `refine_selftest.rs` at the repository root `include!`s **this file**, so it may not
// name anything outside `core`/`alloc`/`std`. ⛔ No `use crate::…`, no `//!` docs.
// The guest reads live in `items.rs`, the drawing in `overlay.rs`.
//
// ── THE GUEST'S OWN JUDGEMENT ────────────────────────────────────────────────
// Handler `0x25234`-`0x25460` (LGT 01.00.07). It is called once per frame while the
// 제련중입니다 box is up and does nothing until `*(base+4)` passes 3, then rolls:
//
// ```text
//   n = (s8)*(rec + REC_REFINE)                    the item's current 제련도, cap 10
//   B = (u8)*(base + 0x415)                        stones the spinner is showing
//   C = (u32)*(base + 0x41c)                       consecutive-failure bonus, 0/1/3
//
//   1) roll1 = rand(1,100);  T1 = 200*B / ((n+4)*(n+1)) + 6*C      (0x25330-0x2535e)
//      roll1 <= T1                    ->  성공, 제련도 +1
//   2) roll2 = rand(1,100);  T2 = 12*(n+1) - 2*B - 3*C             (0x253b6-0x253dc)
//      roll2 >  T2                    ->  제련이 실패하였습니다.   (item untouched)
//   3) roll3 = rand(1,100)                                         (0x253de-0x253e8)
//      roll3 <= 50                    ->  파괴  (item gone)
//      roll3 >  50                    ->  초기화 (back to +0)
// ```
//
// `rand` is `0x1bfa0(min,max)` = `min + rand() % (max-min+1)`, i.e. **[1,100] uniform**
// — the same generator the socket roll uses (literal `0x1bfa1` in both pools). So a
// threshold IS the percentage: `roll <= T` happens `clamp(T, 0, 100)` times in 100.
//
// The integer division at `0x2533e` (helper `0x5a4b0`) truncates, and both operands are
// non-negative here, so `floor` is the whole story.
//
// ★ The stones are consumed whatever happens — all four endings join at `0x25454`,
// which calls `takeItem(0x0e, 3, B)`.
// ★ Failure bumps `*(base+0x420)` and re-derives `C` from it: `<= 4 -> 1`, else `3`
// (`0x2543a`-`0x25452`). Success zeroes both (`0x2536a`, `0x25378`).

/// The smith refuses to go past this (`0x21594`: `cmp #9; ble`, else
/// `더 이상 제련할 수 없습니다.`).
pub const MAX_LEVEL: i8 = 10;
/// The spinner's own clamps, from the four arrow arms at `0x215f4`/`0x21608`/
/// `0x2161a`/`0x21630`: UP `+1`, DOWN `-1`, RIGHT `+10`, LEFT `-10`, held to `1..=99`.
pub const STONE_MIN: u8 = 1;
pub const STONE_MAX: u8 = 99;
/// The one equipment category the smith turns away (`0x2159e`: `cmp #0xc`, then
/// `장신구는 제련할 수 없습니다.`).
pub const CAT_ACCESSORY: u16 = 0x0c;
/// Equipment runs `0x00..=0x0c` — see `orbcombine`'s note about 투구 being `0x00`.
/// Anything outside it never reaches the smith at all.
pub const CAT_EQUIP_MAX: u16 = 0x0c;
/// The consecutive-failure bonus only ever holds one of these three (`0x2544c`/
/// `0x25450` write 3 and 1, success writes 0). A wider value means a bad read.
pub const STREAK_MAX: u8 = 3;

// ── the smith's session context ──────────────────────────────────────────────
//
// `base` is constant-folded out of two literals and never goes through a runtime
// pointer, which is why the host can name it at all:
//
// ```text
//   0x1068:  ldr r0,[pc,#0]        literal @0x106c = 0x01503ab0   (= CHAR + 0x28)
//            bx  lr
//   0x1fd08: r3 = @0x1fd18 = 0x1069 ; blx r3 ; r0 += @0x1fd1c = 0xa838 ; ret
// ```
//
// ⇒ `base = CHAR + 0x28 + 0xa838` = [`CTX_OFF`] from the character record.
//
// ⚠ `PLAY_FEEDBACK_FIXES.md §71차-H` says "비용 … `base+0xa4`" for gold. That is a
// DIFFERENT base: gold is `0x1068()+0xa4` = `CHAR+0xcc` (`0x215c2`-`0x215ca` and the
// deduction at `0x25468`), while the four fields below hang off this one.

/// The session context, as an offset from the character record (LGT 01.00.07).
pub const CTX_OFF: u32 = 0xa860;

/// `s8` — which page of the 제련 flow is live.
///
/// ★★★ THE GATE THAT MATTERS. The window id says `13` for the whole flow, but the
/// roll's epilogue **zeroes the stone count** on its way out (`0x25490 strb r4,[r2]`
/// with `r2 = base+0x415`) and drops this byte back to [`STAGE_PICKER`]
/// (`0x25498 strb r3,[r5,#1]`, `r3 = 4`). A readout gated on the window id alone would
/// therefore print `성공 0 %` over the item picker — a confident, wrong number on an
/// irreversible action.
pub const CTX_STAGE: u32 = 0x001;
/// `u8` (`ldrb` at `0x25334`/`0x253c0`) — stones the spinner is showing, `B`.
pub const CTX_STONES: u32 = 0x415;
/// `u32` — `300*(n+1)^2`, written by the OK arm (`0x2158a`) and zeroed by the epilogue.
/// Read for reference only; [`cost`] recomputes it, which cannot race.
#[allow(dead_code)] // named for the layout record and the selftest; `cost()` recomputes it
pub const CTX_COST: u32 = 0x418;
/// `u32` (`ldr`, **not** a byte — `0x2534c`/`0x253c2`, writes `0x25378`/`0x25452`) — `C`.
pub const CTX_STREAK: u32 = 0x41c;
/// `u32` — the raw consecutive-failure count [`CTX_STREAK`] is derived from (`0x25442`).
#[allow(dead_code)] // the raw counter behind CTX_STREAK; read by nothing, documented here
pub const CTX_FAILS: u32 = 0x420;

/// [`CTX_STAGE`] while the stone spinner owns the screen — written by the picker's OK
/// arm in the same breath as `B = 1` (`0x237b4 strb r3,[r2]`, `0x237b6 strb r3,[r1,#1]`).
pub const STAGE_SPINNER: u8 = 1;
/// …and what the roll's epilogue leaves behind (`0x25498`).
#[allow(dead_code)] // the value the gate must NOT accept — asserted in the selftest
pub const STAGE_PICKER: u8 = 4;

// ── the filtered equipment view ──────────────────────────────────────────────
//
// ⛔⛔ THE INVENTORY CURSOR IS NOT A BAG COORDINATE ON THIS SCREEN. `items.rs`'s
// `(tab, slot) = (CHAR+0x9c71, CHAR+0x9c72+tab)` pair addresses a bag cell everywhere
// else, but the smith lists EQUIPMENT ONLY, and the cursor walks that filtered list.
//
// Builder `0x4db50`-`0x4dba6`: for each bag cell it loads `rec+0x3b6` (the category),
// `cmp r3,#0xc / bhi` — skipping anything above `0x0c` — and appends `{s16 tab, s16 slot}`
// at `grid + 0x166c + count*4`, then `count += 1`. The count is an `s8` at
// `grid + 0x18ec` (literals `@0x4dbb4 = 0x166c` and `@0x4dbb8 = 0x18ec`, both loaded by
// that one function). Unused entries are `-1`.
//
// Three consumers do byte-identical arithmetic on it — the picker's OK arm `0x23780`,
// the spinner's OK arm `0x21520`, and the roll handler `0x252b8` — so this is the only
// way to name the item the smith is about to work on.

/// `{s16 tab, s16 slot}` per entry, at this offset from the grid base.
pub const VIEW_TABLE: u32 = 0x166c;
/// `s8` live entry count, immediately past the table.
pub const VIEW_COUNT: u32 = 0x18ec;
/// Entries the table has room for — `(0x18ec - 0x166c) / 4`.
pub const VIEW_CAPACITY: usize = ((VIEW_COUNT - VIEW_TABLE) / 4) as usize;
/// Cells one page of the cursor covers (`page << 4` at `0x2529e` / `0x2152e`).
pub const VIEW_PAGE_CELLS: usize = 16;

/// Turn the live cursor pair into an index into the filtered view.
///
/// Both halves are read SIGNED by the guest (`ldrsb`), so both are signed here.
/// `None` for anything the table cannot address.
pub fn view_index(page: i8, cell: i8) -> Option<usize> {
    if page < 0 || cell < 0 || cell as usize >= VIEW_PAGE_CELLS {
        return None;
    }
    let i = page as usize * VIEW_PAGE_CELLS + cell as usize;
    (i < VIEW_CAPACITY).then_some(i)
}

/// Address of view entry `index`, given the grid base. `None` past the table.
pub fn view_entry_addr(grid: u32, index: usize) -> Option<u32> {
    (index < VIEW_CAPACITY).then(|| grid + VIEW_TABLE + index as u32 * 4)
}

/// Validate one decoded view entry against the live count.
///
/// ⛔ The count check is not optional: the table keeps whatever the LAST visit left in
/// it past `count`, so an index beyond it names a stale — and possibly since-compacted —
/// bag cell. The roll handler does NOT re-check (only the picker does, at
/// `0x2378e`/`0x23792`), so this is the host's own guard.
pub fn view_slot(tab: i16, slot: i16, index: usize, count: i8) -> Option<(u8, u8)> {
    if count <= 0 || index >= count as usize {
        return None;
    }
    if tab < 0 || slot < 0 || tab as usize >= 5 || slot as usize >= VIEW_PAGE_CELLS {
        return None;
    }
    Some((tab as u8, slot as u8))
}

/// A probability in **tenths of a percent** — `1000` is `100.0 %`.
///
/// Tenths and not percent: `파괴` is a product of two percentages and a half, so it
/// lands on `2.5` / `8.8` / `49.5` far more often than on a whole number, and rounding
/// those to `2` / `9` / `50` throws away the only digit the player is deciding on.
pub type Pct10 = u16;

/// One whole percent, in [`Pct10`].
#[allow(dead_code)] // the unit the tables are written in
pub const PCT: Pct10 = 10;
/// Certainty, in [`Pct10`].
pub const FULL: Pct10 = 1000;

/// Everything the readout prints, for one (item, stone count, streak) triple.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Odds {
    /// The item's 제련도 right now, `0..=9` (a `Ready` odds is never at the cap).
    pub level: i8,
    /// The stone count the spinner is showing — the number the player is moving.
    pub stones: u8,
    /// The consecutive-failure bonus in force, `0` / `1` / `3`.
    pub streak: u8,
    /// 성공.
    pub success: Pct10,
    /// 파괴 — the item is gone.
    pub destroy: Pct10,
    /// 초기화 — the item survives at `+0`. Always exactly [`Self::destroy`].
    pub reset: Pct10,
    /// 파괴 + 초기화, i.e. "this attempt costs me the item's refine level or the item".
    pub doom: Pct10,
    /// The chance of doom GIVEN that the attempt failed — the game's second roll on
    /// its own. Not printed; it is what makes the destroy figure explainable.
    pub doom_if_fail: Pct10,
    /// Gold this attempt costs, win or lose.
    pub cost: u32,
    /// The smallest stone count that makes [`Self::success`] certain — and with it,
    /// [`Self::doom`] zero, because a success never rolls the second die.
    pub guaranteed: u8,
    /// ★★ The attempt **cannot** cost the item: either success is certain, or the
    /// stones drove `T2` under the die's lowest face.
    ///
    /// ⛔ NOT the same as `destroy == 0`. At `+8` with 53 stones the real chance is
    /// `0.02 %`, which rounds to a printed `0.0` — and a readout that says `0.0%` where
    /// the truth is "unlikely, not impossible" is lying in the direction that costs the
    /// player the item. [`pct_cell`] is what keeps the two apart on screen.
    pub safe: bool,
}

/// What the readout should say about the item under the smith's cursor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refine {
    /// Refinable, with these odds.
    Ready(Odds),
    /// `제련도 10/10` — the smith refuses (`더 이상 제련할 수 없습니다.`).
    Maxed,
    /// 장신구, or something that is not equipment at all.
    Unrefinable,
}

/// `300 * (n+1)^2` gold (`0x25312`-`0x25326`, stored at `base+0x418`; the same
/// arithmetic is repeated in the OK arm at `0x21576`-`0x2158a`).
pub fn cost(level: i8) -> u32 {
    let n = u32::from(level.clamp(0, MAX_LEVEL) as u8);
    300 * (n + 1) * (n + 1)
}

/// `T1` — the success threshold, exactly as `0x25330`-`0x25354` computes it.
///
/// Returned RAW (it can exceed 100, and does whenever the player has bought certainty);
/// [`odds`] is what clamps it into a probability.
pub fn success_threshold(level: i8, stones: u8, streak: u8) -> i32 {
    let n = i32::from(level.clamp(0, MAX_LEVEL));
    let b = i32::from(stones);
    let c = i32::from(streak.min(STREAK_MAX));
    // (n+4)*(n+1) is 4..=154 over the whole legal range, so it never divides by zero.
    200 * b / ((n + 4) * (n + 1)) + 6 * c
}

/// `T2` — the doom threshold GIVEN a failure, exactly as `0x253b6`-`0x253d4` computes
/// it. Can go negative, which is the game saying "this attempt cannot break anything".
pub fn doom_threshold(level: i8, stones: u8, streak: u8) -> i32 {
    let n = i32::from(level.clamp(0, MAX_LEVEL));
    let b = i32::from(stones);
    let c = i32::from(streak.min(STREAK_MAX));
    12 * (n + 1) - 2 * b - 3 * c
}

/// The smallest stone count in `1..=99` that makes success certain.
///
/// Closed form `ceil((n+4)(n+1)*(100-6C) / 200)` — but this searches instead, because
/// the thing that must be true is "[`success_threshold`] reaches 100", and searching
/// asks exactly that question of exactly the function the game uses. The selftest
/// asserts the closed form against it.
///
/// ★★ At that count the attempt is also **safe**: `T1 >= 100` means roll 1 always wins,
/// and the game only rolls for doom after a failure.
pub fn guaranteed_stones(level: i8, streak: u8) -> u8 {
    (STONE_MIN..=STONE_MAX)
        .find(|b| success_threshold(level, *b, streak) >= 100)
        .unwrap_or(STONE_MAX)
}

/// The full readout for one item, or why there is none.
///
/// Gate order mirrors the guest's own OK arm — level first (`0x21594`), then category
/// (`0x2159e`) — so a maxed 장신구 reports [`Refine::Maxed`] here exactly as the game
/// would say `더 이상 제련할 수 없습니다.` there.
pub fn evaluate(cat: u16, level: i8, stones: u8, streak: u8) -> Refine {
    if level >= MAX_LEVEL {
        return Refine::Maxed;
    }
    if cat > CAT_EQUIP_MAX || cat == CAT_ACCESSORY {
        return Refine::Unrefinable;
    }
    let level = level.max(0);
    let stones = stones.clamp(STONE_MIN, STONE_MAX);
    let streak = streak.min(STREAK_MAX);

    // A threshold IS a count out of 100, so clamping it into 0..=100 turns it into the
    // probability without any further arithmetic.
    let s = success_threshold(level, stones, streak).clamp(0, 100) as u32;
    let d = doom_threshold(level, stones, streak).clamp(0, 100) as u32;

    // destroy = P(fail) * P(doom | fail) * 1/2, in tenths of a percent:
    //   (100-s)/100 * d/100 * 1/2 * 1000  ==  (100-s)*d / 20
    // rounded half-up, because the exact value carries a half-tenth whenever
    // (100-s)*d is odd-times-ten and the digit shown is the one being decided on.
    let destroy = (((100 - s) * d + 10) / 20) as Pct10;
    Refine::Ready(Odds {
        level,
        stones,
        streak,
        success: (s * 10) as Pct10,
        destroy,
        reset: destroy,
        doom: destroy.saturating_mul(2),
        doom_if_fail: (d * 10) as Pct10,
        cost: cost(level),
        guaranteed: guaranteed_stones(level, streak),
        // Certainty on roll 1 means roll 2 never happens; `d == 0` means roll 2 can
        // never land. Either way the item is coming back.
        safe: s >= 100 || d == 0,
    })
}

/// Render a [`Pct10`] as `"12.5"` — one decimal, always present.
///
/// The readout writes its own `%`; this is only the number, so the caller can put the
/// sign where its layout wants it.
pub fn pct_text(p: Pct10) -> String {
    format!("{}.{}", p / 10, p % 10)
}

/// Render a [`Pct10`] as `"50"` when it is a whole percent and `"2.5"` when it is not.
///
/// 성공 is always whole (it is a threshold out of 100), 파괴 usually is not — and
/// printing `50.0%` next to `2.5%` wastes the width that makes both readable.
pub fn pct_short(p: Pct10) -> String {
    if p % 10 == 0 { format!("{}", p / 10) } else { pct_text(p) }
}

/// One cell of the readout: [`pct_short`], except that a chance which is REAL but too
/// small to print becomes `"<0.1"` rather than a flat `"0"`.
///
/// `impossible` is [`Odds::safe`] for the doom figures. ⛔ Do not pass `destroy == 0`:
/// that is the very confusion this exists to prevent.
pub fn pct_cell(p: Pct10, impossible: bool) -> String {
    if p == 0 && !impossible { "<0.1".to_string() } else { pct_short(p) }
}

// ── what the bar says ────────────────────────────────────────────────────────
//
// Composed here rather than in `overlay.rs` so the selftest can pin the exact strings:
// the game's font is KS X 1001 완성형 only, so a word with a cp949-extension syllable in
// it would be silently dropped on screen (CLAUDE.md), and a test is the only place that
// catches it before the device does.
//
// The vocabulary is the game's own: 성공 / 파괴 / 초기화 come verbatim from the four
// result strings at `0x5d0f4`-`0x5d158`, and `제련도 %d/10` from `0x5bf90`.

/// 파괴 at or above this is worth colouring — one attempt in ten costs the item.
pub const DANGER_AT: Pct10 = 100;

/// How a field should be coloured. The bar owns the actual palette.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Normal,
    /// Nothing can go wrong on this attempt.
    Good,
    /// The item is meaningfully at risk.
    Danger,
}

/// One `label value` pair on the bar.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Field {
    pub label: String,
    pub value: String,
    pub tone: Tone,
}

impl Field {
    fn new(label: &str, value: String, tone: Tone) -> Self {
        Self { label: label.to_string(), value, tone }
    }
}

/// The whole readout, left to right.
///
/// 확정 is the field the player is really being handed: the stone count at which the
/// attempt becomes certain — and therefore safe, since a success never rolls for doom.
/// It moves with the live streak bonus, so it is not the table's fixed 2·5·9·14… once a
/// failure run is going.
pub fn readout(r: &Refine) -> Vec<Field> {
    match r {
        Refine::Maxed => vec![Field::new("제련도", format!("{MAX_LEVEL}/{MAX_LEVEL}"), Tone::Good)],
        Refine::Unrefinable => vec![Field::new("", "제련할 수 없습니다".into(), Tone::Normal)],
        Refine::Ready(o) => vec![
            Field::new(
                "성공",
                format!("{}%", pct_short(o.success)),
                if o.success == FULL { Tone::Good } else { Tone::Normal },
            ),
            Field::new(
                "파괴",
                format!("{}%", pct_cell(o.destroy, o.safe)),
                if o.destroy >= DANGER_AT { Tone::Danger } else { Tone::Normal },
            ),
            Field::new("확정", format!("{}개", o.guaranteed), Tone::Normal),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(cat: u16, level: i8, stones: u8, streak: u8) -> Odds {
        match evaluate(cat, level, stones, streak) {
            Refine::Ready(o) => o,
            other => panic!("expected odds, got {other:?}"),
        }
    }

    /// 71차-H's published table, which was derived by hand from the disassembly. Every
    /// row is `B = 1`, `C = 0` — the throw a player makes without thinking.
    ///
    /// (level, cost, success%, guaranteed stones, doom-if-fail%, destroy in tenths)
    const TABLE: [(i8, u32, u16, u8, u16, u16); 10] = [
        (0, 300, 50, 2, 10, 25),
        (1, 1_200, 20, 5, 22, 88),
        (2, 2_700, 11, 9, 34, 151),
        (3, 4_800, 7, 14, 46, 214),
        (4, 7_500, 5, 20, 58, 276),
        // ⚠ 339 in the 71차-H write-up: that table truncated where this rounds. The
        // exact value is 33.95 %, and the dice simulation in `refine_selftest.rs`
        // agrees with 34.0 to within the half-tenth that rounding is allowed to move.
        (5, 10_800, 3, 27, 70, 340),
        (6, 14_700, 2, 35, 82, 402),
        (7, 19_200, 2, 44, 94, 461),
        (8, 24_300, 1, 54, 100, 495),
        (9, 30_000, 1, 65, 100, 495),
    ];

    #[test]
    fn matches_the_published_table() {
        for (level, cost_g, succ, need, doom_if_fail, destroy) in TABLE {
            let o = ready(0x04, level, 1, 0);
            assert_eq!(o.cost, cost_g, "cost at +{level}");
            assert_eq!(o.success, succ * PCT, "success at +{level}");
            assert_eq!(o.guaranteed, need, "guaranteed stones at +{level}");
            assert_eq!(o.doom_if_fail, doom_if_fail * PCT, "doom|fail at +{level}");
            assert_eq!(o.destroy, destroy, "destroy at +{level}");
            assert_eq!(o.reset, destroy, "reset must equal destroy at +{level}");
            assert_eq!(o.doom, destroy * 2, "doom is the two of them at +{level}");
        }
    }

    /// The headline of 71차-H: `ceil((n+4)(n+1)/2)` stones, `2·5·9·14·20·27·35·44·54·65`,
    /// **275 for a run from +0 to +10**.
    #[test]
    fn guaranteed_stones_match_the_closed_form() {
        let mut total = 0u32;
        for level in 0..MAX_LEVEL {
            let n = i32::from(level);
            let closed = ((n + 4) * (n + 1)).div_euclid(2) + i32::from((n + 4) * (n + 1) % 2 != 0);
            assert_eq!(i32::from(guaranteed_stones(level, 0)), closed, "+{level}");
            total += u32::from(guaranteed_stones(level, 0));
        }
        assert_eq!(total, 275);
    }

    /// ★★★ "100 % 석을 넣으면 절대 안 깨진다" — the claim the readout is really selling.
    /// Checked at every level and every streak, not just the published `C = 0` row.
    #[test]
    fn the_guaranteed_count_is_also_safe() {
        for level in 0..MAX_LEVEL {
            for streak in [0u8, 1, 3] {
                let need = guaranteed_stones(level, streak);
                let o = ready(0x04, level, need, streak);
                assert_eq!(o.success, FULL, "+{level} streak {streak} should be certain");
                assert_eq!(o.destroy, 0, "+{level} streak {streak} must not break");
                assert_eq!(o.reset, 0);
                assert_eq!(o.doom, 0);
                // …and one stone short is NOT certain, or the number is not the smallest.
                if need > STONE_MIN {
                    assert!(
                        ready(0x04, level, need - 1, streak).success < FULL,
                        "+{level} streak {streak}: {need} is not minimal"
                    );
                }
            }
        }
    }

    /// ⚠ The warning the player most needs: at +9 a single stone is a coin flip on the
    /// item's existence.
    #[test]
    fn one_stone_at_plus_nine_is_a_coin_flip() {
        let o = ready(0x04, 9, 1, 0);
        assert_eq!(o.success, 1 * PCT);
        assert_eq!(o.doom_if_fail, FULL);
        assert_eq!(o.destroy, 495); // 49.5 %
        assert_eq!(o.doom, 990); // 99 % of attempts cost the item or its level
    }

    /// The streak bonus pushes BOTH ways — `+6` on success, `-3` on doom (`0x25350`
    /// and `0x253c8`-`0x253cc`).
    #[test]
    fn streak_helps_twice() {
        let none = ready(0x04, 3, 4, 0);
        let one = ready(0x04, 3, 4, 1);
        let three = ready(0x04, 3, 4, 3);
        assert_eq!(one.success, none.success + 6 * PCT);
        assert_eq!(three.success, none.success + 18 * PCT);
        assert_eq!(one.doom_if_fail, none.doom_if_fail - 3 * PCT);
        assert_eq!(three.doom_if_fail, none.doom_if_fail - 9 * PCT);
    }

    /// A read that hands us nonsense must not produce a confident number.
    #[test]
    fn out_of_range_inputs_stay_inside_the_probability_axis() {
        for level in [-128i8, -1, 0, 5, 9] {
            for stones in [0u8, 1, 99, 255] {
                for streak in [0u8, 1, 3, 200] {
                    let Refine::Ready(o) = evaluate(0x04, level, stones, streak) else {
                        continue;
                    };
                    assert!(o.success <= FULL);
                    assert!(o.destroy <= FULL && o.reset <= FULL && o.doom <= FULL);
                    assert!(o.doom_if_fail <= FULL);
                    assert!((STONE_MIN..=STONE_MAX).contains(&o.stones));
                    assert!((0..MAX_LEVEL).contains(&o.level));
                    assert!(o.streak <= STREAK_MAX);
                }
            }
        }
    }

    /// The two refusals, in the guest's own order.
    #[test]
    fn refusals_mirror_the_guest_gate_order() {
        assert_eq!(evaluate(0x04, MAX_LEVEL, 1, 0), Refine::Maxed);
        // level is tested BEFORE category (0x21594 then 0x2159e), so a maxed 장신구
        // is Maxed, not Unrefinable.
        assert_eq!(evaluate(CAT_ACCESSORY, MAX_LEVEL, 1, 0), Refine::Maxed);
        assert_eq!(evaluate(CAT_ACCESSORY, 0, 1, 0), Refine::Unrefinable);
        assert_eq!(evaluate(0x10, 0, 1, 0), Refine::Unrefinable); // an orb is not equipment
        assert_eq!(evaluate(0x12, 0, 1, 0), Refine::Unrefinable); // nor a potion
        // ⛔ 투구 is cat 0x00 and IS equipment — the bug 71차-C had to fix in orbcombine.
        assert!(matches!(evaluate(0x00, 0, 1, 0), Refine::Ready(_)));
        for cat in 0..=CAT_EQUIP_MAX {
            let expect_ready = cat != CAT_ACCESSORY;
            assert_eq!(matches!(evaluate(cat, 0, 1, 0), Refine::Ready(_)), expect_ready, "cat {cat:#x}");
        }
    }

    /// More stones is never worse — monotone on both axes, which is what makes the
    /// live readout worth watching while the spinner moves.
    #[test]
    fn more_stones_is_monotone() {
        for level in 0..MAX_LEVEL {
            for streak in [0u8, 1, 3] {
                let mut last_success = 0;
                let mut last_doom = FULL;
                for stones in STONE_MIN..=STONE_MAX {
                    let o = ready(0x04, level, stones, streak);
                    assert!(o.success >= last_success, "+{level} B={stones}");
                    assert!(o.doom <= last_doom, "+{level} B={stones}");
                    last_success = o.success;
                    last_doom = o.doom;
                }
            }
        }
    }

    /// The addressing that decides WHICH item the numbers are about. A silent error
    /// here is the worst failure this feature has: a plausible number for the wrong
    /// sword.
    #[test]
    fn view_addressing_matches_the_guest() {
        assert_eq!(VIEW_CAPACITY, 160); // (0x18ec - 0x166c) / 4
        // `page << 4 | cell`, the shape of `lsls #24 / asrs #20` + sign-extended cell.
        assert_eq!(view_index(0, 0), Some(0));
        assert_eq!(view_index(0, 15), Some(15));
        assert_eq!(view_index(1, 0), Some(16));
        assert_eq!(view_index(4, 15), Some(79));
        // …and every value the guest's signed reads can produce that the table cannot
        // address.
        assert_eq!(view_index(-1, 0), None);
        assert_eq!(view_index(0, -1), None);
        assert_eq!(view_index(0, 16), None);
        assert_eq!(view_index(10, 0), None); // 160 is one past the end
        assert_eq!(view_index(9, 15), Some(159));

        // grid + 0x166c + i*4
        assert_eq!(view_entry_addr(0x1000, 0), Some(0x1000 + 0x166c));
        assert_eq!(view_entry_addr(0x1000, 79), Some(0x1000 + 0x166c + 79 * 4));
        assert_eq!(view_entry_addr(0x1000, VIEW_CAPACITY), None);
        // The last addressable entry ends exactly where the count byte begins.
        assert_eq!(view_entry_addr(0, VIEW_CAPACITY - 1).unwrap() + 4, VIEW_COUNT);
    }

    /// ⛔ Past the live count the table holds whatever the last visit left there.
    #[test]
    fn view_entries_past_the_count_are_refused() {
        assert_eq!(view_slot(0, 0, 0, 1), Some((0, 0)));
        assert_eq!(view_slot(4, 15, 0, 1), Some((4, 15)));
        // one past the count — stale
        assert_eq!(view_slot(0, 0, 1, 1), None);
        // an empty entry, which the builder memsets to -1
        assert_eq!(view_slot(-1, -1, 0, 1), None);
        assert_eq!(view_slot(0, -1, 0, 1), None);
        assert_eq!(view_slot(-1, 0, 0, 1), None);
        // a count the read could not make sense of
        assert_eq!(view_slot(0, 0, 0, 0), None);
        assert_eq!(view_slot(0, 0, 0, -1), None);
        // out of the bag's own bounds
        assert_eq!(view_slot(5, 0, 0, 80), None);
        assert_eq!(view_slot(0, 16, 0, 80), None);
    }

    /// The stage byte is the whole reason this feature does not print `성공 0 %` over
    /// the item picker.
    #[test]
    fn the_two_stages_are_distinct() {
        assert_ne!(STAGE_SPINNER, STAGE_PICKER);
        assert_eq!(STAGE_SPINNER, 1);
        assert_eq!(STAGE_PICKER, 4);
        // The context offsets are the ones the disassembly names.
        assert_eq!(CTX_OFF, 0xa860);
        assert_eq!((CTX_STAGE, CTX_STONES, CTX_COST, CTX_STREAK, CTX_FAILS), (1, 0x415, 0x418, 0x41c, 0x420));
    }

    /// The exact line a player sees at the two ends of the ladder.
    #[test]
    fn the_readout_reads_the_way_the_player_asked() {
        // "제련석 1개일때 재련확률이 50%라고 표기되고" — the request, verbatim.
        let f = readout(&evaluate(0x04, 0, 1, 0));
        assert_eq!(f[0].label, "성공");
        assert_eq!(f[0].value, "50%");
        assert_eq!(f[0].tone, Tone::Normal);
        assert_eq!((f[1].label.as_str(), f[1].value.as_str()), ("파괴", "2.5%"));
        assert_eq!((f[2].label.as_str(), f[2].value.as_str()), ("확정", "2개"));

        // "2개로 올리면 재련확률이 100%로 바뀌고" — the same item, one more stone.
        let f = readout(&evaluate(0x04, 0, 2, 0));
        assert_eq!(f[0].value, "100%");
        assert_eq!(f[0].tone, Tone::Good);
        assert_eq!(f[1].value, "0%");
        assert_eq!(f[1].tone, Tone::Normal);

        // ⚠ +9 with one stone: the coin flip, coloured.
        let f = readout(&evaluate(0x04, 9, 1, 0));
        assert_eq!((f[0].value.as_str(), f[1].value.as_str(), f[2].value.as_str()), ("1%", "49.5%", "65개"));
        assert_eq!(f[1].tone, Tone::Danger);

        // ⛔ The rounding trap: a real chance must never print as a flat 0 %.
        let f = readout(&evaluate(0x04, 8, 53, 0));
        assert_eq!(f[1].value, "<0.1%");
        assert_ne!(f[1].tone, Tone::Good);

        // The streak moves 확정, so the hint is never the table's fixed number when a
        // failure run is going.
        assert_eq!(readout(&evaluate(0x04, 9, 1, 3))[2].value, "54개");

        // The two refusals.
        assert_eq!(readout(&Refine::Maxed)[0].value, "10/10");
        assert_eq!(readout(&Refine::Unrefinable)[0].value, "제련할 수 없습니다");
    }

    /// Every syllable the bar can print has to exist in the game's font, which carries
    /// KS X 1001 완성형 only — a cp949 extension would be dropped silently on screen.
    #[test]
    fn every_printable_syllable_is_ks_x_1001() {
        let mut seen = Vec::new();
        for r in [Refine::Maxed, Refine::Unrefinable] {
            seen.extend(readout(&r));
        }
        for level in 0..MAX_LEVEL {
            for streak in [0u8, 1, 3] {
                for stones in STONE_MIN..=STONE_MAX {
                    seen.extend(readout(&evaluate(0x04, level, stones, streak)));
                }
            }
        }
        for f in &seen {
            for ch in f.label.chars().chain(f.value.chars()) {
                let ok = ch == ' ' || ch.is_ascii_graphic() || ('\u{ac00}'..='\u{d7a3}').contains(&ch);
                assert!(ok, "{:?}/{:?}: {ch:?} is neither ASCII nor a Hangul syllable", f.label, f.value);
            }
        }
    }

    #[test]
    fn percent_text_keeps_the_deciding_digit() {
        assert_eq!(pct_text(25), "2.5");
        assert_eq!(pct_text(495), "49.5");
        assert_eq!(pct_text(FULL), "100.0");
        assert_eq!(pct_text(0), "0.0");
        assert_eq!(pct_short(500), "50");
        assert_eq!(pct_short(25), "2.5");
        assert_eq!(pct_short(0), "0");
        assert_eq!(pct_short(FULL), "100");
    }
}
