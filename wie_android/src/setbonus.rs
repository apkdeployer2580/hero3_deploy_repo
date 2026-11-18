// Set effects — the host half (142차 85제 · 142차-C 90제, 2026-09-30 · 150차 규칙, 2026-10-01).
// `LEGACY_ITEMS_REWORK_85_90.md §3 · §11`.
//
// PURE and dependency-free: `setbonus_selftest.rs` `include!`s this file and `set_items.rs`.
//
// Each set has its five equipment pieces — 투구 · 갑옷 · 장갑 · 신발 · 방패 — and at most one
// set accessory, and ANY FOUR distinct ones make its effect (150차 user decision, 2026-10-01:
// 「6개중 4개」; it was five). The accessory is there because the shield slot only opens for a
// class whose main or sub weapon is a spear (`0x4d858`'s `0x4d936`): 7 of the 10 classes can
// never wear a shield, and the accessory stands in for it. A second copy of the same
// accessory in the other accessory slot is still ONE piece. The set accessories carry their
// own 150차 options (`acc_options.rs`) — the old set-themed option overwrite is gone.
// ★ 154차 (2026-10-02, user decision): a set is FOUR, FIVE or SIX pieces — never three, never
// seven (「4or5or6 이야」; `gen_set_items.py` refuses anything else). 레아의목걸이 moved from
// 크루세이더 to 붉은마녀 (the set of 「붉은 마녀」 레아), 굴베이그의탈리스만 is no set piece any more,
// and 크루세이더 is five pieces with no accessory. ★ 155차 (2026-10-02, user decision): 템플러 and
// 코르버스 lost their accessory too (이안의맹세 · 닉스의펜던트 — five pieces each), 빙해's accessory is
// 이안의맹세 (was 클레르의가보), 클레르의가보 · 닉스의펜던트 are plain accessories. Everything below
// reads each set's pieces off `set_items.rs`, so a set's size is just its row count there (the
// 상세보기 prints 「착용 N/5」 and 「5부위 중 4부위 착용 시」 for the three five-piece sets).
//
// ⛔ Not the game's own set rule. The native one (`0x4f014`'s `0x4f270..0x4f354`) checks the
// FOUR armour slots' row byte b11 (투구 · 갑옷 · 장갑 · 신발 all the same id > 0) and adds
// +20 to max HP · attack · defence and -20 SP cost inside the wear recompute; 150차 gives
// each legacy set its own b11 (템플러 8 … 빙해 13). The two rules are independent: the
// host's needs any four of six, the game's needs exactly the four armour pieces.
//
// The worker reads the nine worn `(cat, idx)` (`items::worn_items`), turns them into a mask
// here and publishes it (`wie_util::stats::set_set_bonus`); the guest hooks read that mask
// on every hit (`playerhp` for damage taken, `dmgwide` for 기습 · 처형 · 빙해의 일격 and the
// kill that starts 마녀의 생명력); the max-HP half of 마녀의 생명력 is `playerhp`'s
// `ComputeMaxHP` hook, which the worker re-runs when the buff turns on or off. The bits are
// the `wie_util::stats::SET_*` bits — `lib.rs` asserts the two agree at compile time.
//
// The HUD lines also carry the two TIMED accessory buffs of 150차 (레아의목걸이 「적 처치 시
// 20초간 최대 HP +20%」, 굴베이그의탈리스만 「피격 시 20초간 최대 HP +20%」) — plain numbers
// in, so this file stays dependency-free: the percents are `stats::AccEffects`'
// `kill_max_hp_pct` / `hit_max_hp_pct`, the clocks [`Clocks`]. The kill clock
// (`stats::WITCH_UNTIL_MS`) is SHARED by 마녀의 생명력 and 레아의목걸이 — one trigger, one 20 s.
// (154차: 레아의목걸이 is now a 붉은마녀 piece. Worn with three more, a kill starts both +20 %
// on that one clock — `stats::max_hp_bonus_pct` adds them, as it did when 레아 was 크루세이더's.)

/// How many of a set's pieces (four to six, 154차) make its effect — any of them, distinct (150차).
pub const PIECES_FOR_BONUS: u8 = 4;

pub struct SetInfo {
    /// The set's name (the pieces' shared prefix).
    pub name: &'static str,
    /// The effect's name.
    pub bonus: &'static str,
    /// The effect in a few words, for the activation box — which is the game's message box:
    /// 4 lines of ~12 syllables (`overlay::NOTICE_LINES` / `NOTICE_LINE_W`), so keep it short.
    /// The 상세보기 set section shows it too (`itemnote::SetView::text`, wrapped there).
    pub text: &'static str,
    /// Its `wie_util::stats::SET_*` bit.
    pub bit: u32,
}

/// How many sets there are (the three 85제 then the three 90제).
pub const SET_COUNT: usize = 6;

pub const SETS: [SetInfo; SET_COUNT] = [
    SetInfo {
        name: "템플러",
        bonus: "수호의 맹세",
        text: "치명상 시 HP 1로 5초 무적 (재사용 100초)",
        bit: 1 << 0,
    },
    SetInfo {
        name: "크루세이더",
        bonus: "기습",
        // ★ 158차 P4: ×3 → ×2 (`wie_util::stats::AMBUSH_MULT`; `lib.rs` pins the two agree).
        text: "HP가 가득 찬 적에게 첫 공격 피해 2배",
        bit: 1 << 1,
    },
    SetInfo {
        name: "레갈리스근위",
        bonus: "교단의 가호",
        text: "받는 피해 20% 감소",
        bit: 1 << 2,
    },
    SetInfo {
        name: "코르버스",
        bonus: "처형",
        text: "HP 50% 이하의 적에게 주는 피해 2배",
        bit: 1 << 3,
    },
    SetInfo {
        name: "붉은마녀",
        bonus: "마녀의 생명력",
        text: "적 처치 시 20초간 최대 HP 20% 증가",
        bit: 1 << 4,
    },
    SetInfo {
        name: "빙해",
        bonus: "빙해의 일격",
        text: "모든 공격의 피해 1.5배",
        bit: 1 << 5,
    },
];

/// The bit of 템플러 — the one set whose state the HUD shows (its invulnerability and cooldown).
pub const GUARD_BIT: u32 = 1 << 0;
/// The bit of 붉은마녀 — its buff (max HP +20 % for 20 s after a kill) shows a countdown too.
pub const WITCH_BIT: u32 = 1 << 4;

/// How many DISTINCT pieces of each set are worn (a second copy of the same accessory in
/// the other accessory slot is still one piece).
pub fn piece_counts(worn: &[(u16, u16)], table: &[(u16, u16, u8)]) -> [u8; SET_COUNT] {
    let mut seen: [(u16, u16); 9] = [(u16::MAX, u16::MAX); 9];
    let mut n = 0;
    let mut counts = [0u8; SET_COUNT];
    for &w in worn {
        if seen[..n].contains(&w) {
            continue;
        }
        if n < seen.len() {
            seen[n] = w;
            n += 1;
        }
        if let Some(&(_, _, sid)) = table.iter().find(|&&(c, i, _)| (c, i) == w)
            && let Some(c) = counts.get_mut(sid as usize)
        {
            *c += 1;
        }
    }
    counts
}

/// The published mask: every set with at least [`PIECES_FOR_BONUS`] pieces worn.
pub fn active_mask(worn: &[(u16, u16)], table: &[(u16, u16, u8)]) -> u32 {
    let counts = piece_counts(worn, table);
    SETS.iter()
        .zip(counts)
        .filter(|(_, n)| *n >= PIECES_FOR_BONUS)
        .fold(0, |m, (s, _)| m | s.bit)
}

/// The set `(cat, idx)` is a piece of, if any.
pub fn set_of(cat: u16, idx: u16, table: &[(u16, u16, u8)]) -> Option<u8> {
    table.iter().find(|&&(c, i, _)| (c, i) == (cat, idx)).map(|&(_, _, sid)| sid)
}

/// The pieces of set `sid` in the table's order — 투구 · 갑옷 · 장갑 · 신발 · 방패 · 장신구
/// (`set_items.rs` is sorted by category) — each with whether it is worn: what the 상세보기
/// set section lists (`itemnote::Piece`, names from the item table).
pub fn piece_states(sid: u8, worn: &[(u16, u16)], table: &[(u16, u16, u8)]) -> Vec<((u16, u16), bool)> {
    table
        .iter()
        .filter(|p| p.2 == sid)
        .map(|&(c, i, _)| ((c, i), worn.contains(&(c, i))))
        .collect()
}

/// The slot word the set section prints before a piece's name.
pub fn slot_word(cat: u16) -> &'static str {
    match cat {
        0x00 => "투구",
        0x01 => "갑옷",
        0x02 => "장갑",
        0x03 => "신발",
        0x0b => "방패",
        0x0c => "장신구",
        _ => "장비",
    }
}

/// Seconds left of a clock, rounded up (`0` once it has passed).
pub fn secs_left(now_ms: u64, until_ms: u64) -> u64 {
    until_ms.saturating_sub(now_ms).div_ceil(1000)
}

/// The clocks the HUD lines read, all play-clock ms (`stats::play_clock_ms` and the
/// statics of the same names; 0 = never).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Clocks {
    pub now_ms: u64,
    /// 수호의 맹세: invulnerable until (`GUARD_UNTIL_MS`) — the field line.
    pub guard_until_ms: u64,
    /// 수호의 맹세: ready again at (`GUARD_READY_MS`) — the menu line.
    pub guard_ready_ms: u64,
    /// The kill clock (`WITCH_UNTIL_MS`), shared by 마녀의 생명력 and 레아의목걸이.
    pub kill_until_ms: u64,
    /// The hit clock (`HIT_UNTIL_MS`) — 굴베이그의탈리스만.
    pub hit_until_ms: u64,
}

/// The two timed accessory buffs, as the HUD words them: 「처치 최대HP+N%」 · 「피격 최대HP+N%」.
fn acc_buffs(kill_pct: u8, hit_pct: u8, c: &Clocks) -> [Option<(String, u64)>; 2] {
    let run = |pct: u8, until: u64, word: &str| {
        (pct > 0 && c.now_ms < until).then(|| (format!("{word} 최대HP+{pct}%"), secs_left(c.now_ms, until)))
    };
    [run(kill_pct, c.kill_until_ms, "처치"), run(hit_pct, c.hit_until_ms, "피격")]
}

/// The field line: 수호의 맹세's invulnerability, 마녀의 생명력's buff and the accessories'
/// timed max-HP buffs while they run (`None` otherwise, so an ordinary field is untouched).
/// `kill_pct` / `hit_pct` = `stats::AccEffects::kill_max_hp_pct` / `hit_max_hp_pct`.
pub fn field_line(mask: u32, kill_pct: u8, hit_pct: u8, c: Clocks) -> Option<String> {
    let mut parts = Vec::new();
    if mask & GUARD_BIT != 0 && c.now_ms < c.guard_until_ms {
        parts.push(format!("수호의 맹세 무적 {}초", secs_left(c.now_ms, c.guard_until_ms)));
    }
    if mask & WITCH_BIT != 0 && c.now_ms < c.kill_until_ms {
        parts.push(format!("마녀의 생명력 {}초", secs_left(c.now_ms, c.kill_until_ms)));
    }
    for (words, secs) in acc_buffs(kill_pct, hit_pct, &c).into_iter().flatten() {
        parts.push(format!("{words} {secs}초"));
    }
    (!parts.is_empty()).then(|| parts.join(" / "))
}

/// The menu lines: one per active set, 템플러's with its cooldown and 붉은마녀's with its
/// buff while they run, then each accessory buff while it runs.
pub fn menu_lines(mask: u32, kill_pct: u8, hit_pct: u8, c: Clocks) -> Vec<String> {
    let mut lines: Vec<String> = SETS
        .iter()
        .filter(|s| mask & s.bit != 0)
        .map(|s| {
            if s.bit == GUARD_BIT && c.now_ms < c.guard_ready_ms {
                format!("세트 {} (재사용 {}초)", s.bonus, secs_left(c.now_ms, c.guard_ready_ms))
            } else if s.bit == WITCH_BIT && c.now_ms < c.kill_until_ms {
                format!("세트 {} (남은 {}초)", s.bonus, secs_left(c.now_ms, c.kill_until_ms))
            } else {
                format!("세트 {}", s.bonus)
            }
        })
        .collect();
    for (words, secs) in acc_buffs(kill_pct, hit_pct, &c).into_iter().flatten() {
        lines.push(format!("{words} (남은 {secs}초)"));
    }
    lines
}

/// The box shown when a set becomes complete: the set on the first line, the effect after.
pub fn activation_notice(bit: u32) -> Option<String> {
    SETS.iter()
        .find(|s| s.bit == bit)
        .map(|s| format!("{} 세트 효과\n{}: {}", s.name, s.bonus, s.text))
}
