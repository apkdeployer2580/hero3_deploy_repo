//! 헬모드 drops — the guest half of 133차; the two item pools and the Lv 68 floor are 134차;
//! the level ladder of WHAT drops (조합서 · 실물 · 닉스코어파편) is 152차.
//!
//! ── WHAT HELL MODE IS ────────────────────────────────────────────────────────
//! The game reads its difficulty byte (`reaverdrop::HARD_FLAG_ADDR`, 0x150ddc8) with
//! `cmp #0` everywhere — 0 = normal, anything else = hard — and both save directions
//! copy the byte VERBATIM (`serialiseSave` 0x1aed4 `ldrb/strb`, `deserialiseSave`
//! 0x1a9d0/0x1a9d4). So the value **2** is invisible to the game (it keeps serving the
//! hard tables) and is the HOST'S mark for hell: written by the host when the ENDING
//! window closes on a character whose difficulty was already non-zero (a hardcore
//! clear — `emulator.rs`, the same falling edge that runs the bag transition), carried
//! by the save like any other byte, and read back here on every kill.
//!
//! ── THE DROP RULE (152차, user decision 2026-10-02) ─────────────────────────
//! «헬 몬스터 ~78레벨까지는 이전 70레벨 아이템 + 리버 무기가 일반적인 확률(기존과 같은 확률)로
//! 나오면 되고 79~85 까지는 신규아이템의 모든 조합서가 낮을 확률로 나오면되고 헬 85~ 는 실제
//! 신규 아이템이 낮은 확률로 나오면돼. 그리고 헬 79~ 에서 닉스코어 파편 1개가 낮은 확률로
//! 나오면돼. 낮은 확률이라함은 전부 1%를 말하는거야.»
//!
//! Judged on the kill's LIVE level (`+0x70` of the creature record — the level the host's hell
//! fork raised it to, [`HELL_LV_MIN`]..=[`HELL_LV_MAX`]). Every roll below is its own die at
//! [`RATE_PCT`] (1 %) against the guest's own RNG (the reaver hook's dice):
//!
//! | live level | what the kill rolls on top of the stock loot |
//! |---|---|
//! | 68..=78  | nothing of these — the game's own hard loot (and, 152차-B, the reaver below) |
//! | 79..=85  | one 조합서, uniform over ALL 78 ([`HELL_RECIPES`] — «모든 조합서») |
//! | 85..=100 | one new item from the kill's pool (below) |
//! | 79..=100 | one 닉스코어파편 ([`HELL_SHARD`]) |
//!
//! ★ 152차-B (same day): the 80제 reaver is hell's farming drop at 68..=81 — every kill there,
//! any species, at the reaver's own 5 % (`reaverdrop::rolls_reaver`, `REAVER_HELL_LV_MAX`).
//!
//! ★ Lv 85 rolls BOTH a recipe and an item: the message gives «79~85» to the recipes and «85~»
//! to the items, and that is what it says. If the boundary is meant to be exclusive it is one
//! constant ([`RECIPE_LV_MAX`] or [`ITEM_LV_MIN`]).
//!
//! The item pools are unchanged from 134차 / 142차 (the 2026-09-24 decision «93~100 네임드보스 급
//! 들에서만 드랍» still stands — 152차 did not revisit it):
//!
//! * **Boss tier = 90제** — the rows whose 등급 (tier) is >= [`BOSS_TIER_REQ`] (43 rows: 전설 18 ·
//!   붉은마녀·빙해·코르버스 세트 15 · hero3 무기 7 · 악세 3). ONLY a *named* kill at hell level
//!   >= [`NAMED_LV_MIN`] rolls it. Named = the species the game itself names in red (`{` colour
//!   escape in `enemyh_dat`, the generated [`HELL_NAMED_SPECIES`]) or any `boss_dat` species
//!   ([`BOSS_DAT_BASE`]+).
//! * **Regular = 85제** — everything else (35 rows), for every other kill at 85+.
//!
//! ⛔ Below 79 this module adds NOTHING — that is the «기존과 같은 확률» half of the decision: the
//! game's hard tables (`droph_dat` — Lv70 종결 in the boss slots, the generic picker, the 혼 slot)
//! run exactly as they do in hard. ★ 152차-B moved the REAVER for hell: it no longer follows the
//! eight bosses there (they fight at 98..100 — hard 65..67 on the ladder) but the band 68..=81,
//! every kill, at its own 5 % (`reaverdrop::rolls_reaver`); the two boss accessories still follow
//! 141 / 138.
//!
//! The pools are GENERATED ([`helldrop_table`](super::helldrop_table)) by `tools_re/
//! gen_helldrop_table.py --check` from the shipped asset: [`HELL_ITEMS`] = every `"`-coloured row
//! as `(cat, idx, 등급)`, [`HELL_RECIPES`] = the 조합서 whose tail makes one of them (a bijection,
//! asserted — the same list the tower's exit reward pays from), [`HELL_SHARD`] = the tower material
//! looked up by name. ★ 150차: the third value is the 85제 / 90제 TIER (`gen_legacy_items.
//! legacy_tiers`), not the asset's 요구레벨 byte — the reworked accessories carry 요구 Lv 1. The
//! same tool emits the named-species list from `enemyh_dat` and cross-checks the ladder constants
//! against the host copy in `guardian.rs`.
//!
//! ⛔ The rolls live INSIDE `reaverdrop::handle_lgt` (the one place that decides whether loot
//! spawns at all), not in a second hook — and therefore inherit its gates: vanilla never
//! installs the hook, and `tower_no_drops` suppresses everything inside the tower before any die
//! is cast.
//! ⛔ A record the host's hell fork has NOT raised still carries its hard-table spawn level
//! (38..67) — below 79, so it rolls nothing of hell's: the ladder itself fails closed. A level
//! above [`HELL_LV_MAX`] (not a hell field record) rolls nothing either ([`on_ladder`]).

/// The difficulty byte value that means hell. 0 = normal, 1 = hard (the game's own
/// writer at `0x3b0bc`), 2 = hell (host-written; the game only ever tests `!= 0`).
pub const HELL_DIFFICULTY: u8 = 2;

/// Chance, per kill in hell, of EACH of the three rolls — the 조합서, the new item and the
/// 닉스코어파편 (152차: «낮은 확률이라함은 전부 1%»; 133차–151차 rolled the item alone at 3 %).
/// Percent of a guest `rand(0, 99)`. Tunable by the same `wie_reaver_test.txt` rate knob as the
/// reaver (they share the test file; arming it arms every roll).
pub const RATE_PCT: u32 = 1;

/// The hell ladder's ends — the host's `guardian::{HELL_LV_MIN, HELL_LV_MAX}` mirrored
/// here (a hook cannot see `wie_android`). 68 = user decision 2026-09-24 («헬 몬스터는
/// 레벨 68부터»), 100 = the 133차 spec. `gen_helldrop_table.py --check` asserts the two
/// copies agree. A live level outside this range is not a hell field record → no hell roll.
pub const HELL_LV_MIN: i8 = 68;
pub const HELL_LV_MAX: i8 = 100;

/// 152차: a kill at live level `RECIPE_LV_MIN..=RECIPE_LV_MAX` rolls one 조합서 («79~85»).
pub const RECIPE_LV_MIN: i8 = 79;
pub const RECIPE_LV_MAX: i8 = 85;
/// 152차: a kill at this live level or above rolls one new item («헬 85~»).
pub const ITEM_LV_MIN: i8 = 85;
/// 152차: a kill at this live level or above rolls one 닉스코어파편 («헬 79~»).
pub const SHARD_LV_MIN: i8 = 79;

/// A named kill at this hell level or above rolls the boss-tier pool («93~100 네임드보스»).
pub const NAMED_LV_MIN: i8 = 93;

/// Rows whose 등급 (the table's third value — the tier, 150차) is at or above this are boss
/// tier: the 90제 (142차 — a 요구레벨 of 95 when the ladder ran 73..99). Everything below (the
/// 85제) is the regular pool. The name keeps its 142차 `_REQ`.
pub const BOSS_TIER_REQ: u8 = 90;

/// Species ids from `boss_dat` start here (`enemy_dat` is 0..=160); every one of them
/// is a named boss by number. Checked against the asset by the generator.
pub const BOSS_DAT_BASE: u8 = 161;

/// `CreatureRecord::level` — the live monster level, `i8` at `+0x70` of the record the
/// loot routine was called for (`guest_anchors::CreatureRecord`, same struct).
pub const CREATURE_LEVEL_OFF: u32 = 0x70;

pub use super::helldrop_table::{HELL_ITEMS, HELL_NAMED_SPECIES, HELL_RECIPES, HELL_SHARD};

/// Which of the two item pools a kill rolls.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pool {
    /// 85제 — every kill that is not a named boss at 93+.
    Regular,
    /// 90제 — named bosses at hell level 93..100 only.
    BossTier,
}

/// Is this difficulty byte hell?
pub fn is_hell(byte: u8) -> bool {
    byte == HELL_DIFFICULTY
}

/// Is this species a named boss: red-named in `enemyh_dat` or from `boss_dat`?
pub fn is_named(species: u8) -> bool {
    species >= BOSS_DAT_BASE || HELL_NAMED_SPECIES.contains(&species)
}

/// Is this row's 등급 (the `HELL_ITEMS` third value, 85 / 90) in the boss tier?
pub fn is_boss_tier(req: u8) -> bool {
    req >= BOSS_TIER_REQ
}

/// The pool this kill rolls: boss tier for a named species at 93+, regular otherwise.
pub fn pool_for(lv: i8, named: bool) -> Pool {
    if named && lv >= NAMED_LV_MIN {
        Pool::BossTier
    } else {
        Pool::Regular
    }
}

/// Does this row belong to the pool?
pub fn row_in_pool(req: u8, pool: Pool) -> bool {
    is_boss_tier(req) == (pool == Pool::BossTier)
}

/// Is this live level on the hell ladder at all? Outside it the record is not a forked hell
/// field monster.
pub fn on_ladder(lv: i8) -> bool {
    (HELL_LV_MIN..=HELL_LV_MAX).contains(&lv)
}

/// Does a kill at this live level roll a 조합서? (79..=85)
pub fn rolls_recipe(lv: i8) -> bool {
    on_ladder(lv) && (RECIPE_LV_MIN..=RECIPE_LV_MAX).contains(&lv)
}

/// Does a kill at this live level roll a new item? (85..=100)
pub fn rolls_item(lv: i8) -> bool {
    on_ladder(lv) && lv >= ITEM_LV_MIN
}

/// Does a kill at this live level roll a 닉스코어파편? (79..=100)
pub fn rolls_shard(lv: i8) -> bool {
    on_ladder(lv) && lv >= SHARD_LV_MIN
}

/// The 조합서 a winning recipe roll yields: uniform over ALL of [`HELL_RECIPES`] — 85제 and
/// 90제 recipes alike («신규아이템의 모든 조합서»).
pub fn pick_recipe(draw: u32) -> (u16, u16) {
    let (cat, idx, _) = HELL_RECIPES[(draw % HELL_RECIPES.len() as u32) as usize];
    (cat, idx)
}

fn eligible(lv: i8, named: bool) -> impl Iterator<Item = &'static (u16, u16, u8)> {
    let pool = pool_for(lv, named);
    HELL_ITEMS.iter().filter(move |&&(_, _, r)| row_in_pool(r, pool))
}

/// How many new items this kill can drop (0 below [`ITEM_LV_MIN`] or off the ladder).
pub fn candidates(lv: i8, named: bool) -> u32 {
    if !rolls_item(lv) {
        return 0;
    }
    eligible(lv, named).count() as u32
}

/// The new item a winning item roll yields: uniform over the kill's pool, `None` when nothing
/// is eligible (below 85, or off the ladder — decline rather than guess).
pub fn pick(lv: i8, named: bool, draw: u32) -> Option<(u16, u16)> {
    let n = candidates(lv, named);
    if n == 0 {
        return None;
    }
    eligible(lv, named)
        .nth((draw % n) as usize)
        .map(|&(cat, idx, _)| (cat, idx))
}

/// `rand(0, 99) < rate` — pure, so the shipped 1 % stays under test whatever the
/// runtime knob says.
pub fn wins_at(draw: u32, rate: u32) -> bool {
    draw % 100 < rate
}

/// The rate actually in force: [`RATE_PCT`] unless the reaver test knob is armed.
pub fn live_rate() -> u32 {
    wie_util::stats::reaver_rate_pct(RATE_PCT)
}

/// `rand(0, 99) <` the live rate.
pub fn wins(draw: u32) -> bool {
    wins_at(draw, live_rate())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req_of(p: (u16, u16)) -> u8 {
        HELL_ITEMS.iter().find(|&&(c, i, _)| (c, i) == p).expect("a table row").2
    }

    fn tier_len(boss: bool) -> u32 {
        HELL_ITEMS.iter().filter(|&&(_, _, r)| is_boss_tier(r) == boss).count() as u32
    }

    /// ★★★ 152차 — the user's ladder, level by level: nothing of hell's at 68..=78, a recipe at
    /// 79..=85, a new item at 85..=100, a shard at 79..=100 (85 rolls both a recipe and an item).
    #[test]
    fn the_152_ladder_is_the_users_words() {
        for lv in HELL_LV_MIN..=HELL_LV_MAX {
            let (recipe, item, shard) = (rolls_recipe(lv), rolls_item(lv), rolls_shard(lv));
            match lv {
                68..=78 => assert!(!recipe && !item && !shard, "Lv{lv}: the stock loot only"),
                79..=84 => assert!(recipe && !item && shard, "Lv{lv}: recipe + shard"),
                85 => assert!(recipe && item && shard, "Lv85: recipe + item + shard"),
                _ => assert!(!recipe && item && shard, "Lv{lv}: item + shard"),
            }
        }
        assert_eq!((RECIPE_LV_MIN, RECIPE_LV_MAX, ITEM_LV_MIN, SHARD_LV_MIN), (79, 85, 85, 79));
    }

    /// An unforked record (its hard-table spawn level, 38..67) and anything off the ladder rolls
    /// nothing of hell's — no recipe, no item, no shard. 67 is exactly one below the floor.
    #[test]
    fn levels_off_the_ladder_decline() {
        for lv in [1i8, 30, 38, 66, 67, 101, 120, 127, -1] {
            assert!(!rolls_recipe(lv) && !rolls_item(lv) && !rolls_shard(lv), "level {lv}");
            assert_eq!(pick(lv, false, 0), None, "level {lv}");
            assert_eq!(pick(lv, true, 0), None, "named level {lv}");
            assert_eq!(candidates(lv, false), 0);
        }
        assert!(!on_ladder(67) && on_ladder(68) && on_ladder(100) && !on_ladder(101));
    }

    /// Every kill at 85+ sees a WHOLE item pool (no band since 142차): the 85제 for an ordinary
    /// kill, the 90제 for a named kill at 93+. Below 85 there is no item at all.
    #[test]
    fn every_item_level_reaches_a_whole_pool() {
        for lv in HELL_LV_MIN..=HELL_LV_MAX {
            if lv < ITEM_LV_MIN {
                assert_eq!(candidates(lv, false), 0, "Lv{lv} drops no item");
                assert_eq!(candidates(lv, true), 0, "named Lv{lv} drops no item");
                continue;
            }
            assert_eq!(candidates(lv, false), tier_len(false), "ordinary hell level {lv}");
            let want = if lv >= NAMED_LV_MIN { tier_len(true) } else { tier_len(false) };
            assert_eq!(candidates(lv, true), want, "named hell level {lv}");
        }
    }

    /// 90제 rows never come out of an ordinary kill or a named kill below 93, and a named
    /// kill at 93+ yields nothing BUT 90제.
    #[test]
    fn boss_tier_rows_drop_only_from_named_bosses_at_93_plus() {
        for lv in ITEM_LV_MIN..=HELL_LV_MAX {
            for named in [false, true] {
                let n = candidates(lv, named);
                let boss = pool_for(lv, named) == Pool::BossTier;
                assert_eq!(boss, named && lv >= NAMED_LV_MIN, "pool at lv {lv} named {named}");
                for d in 0..n {
                    let req = req_of(pick(lv, named, d).expect("a draw"));
                    assert_eq!(is_boss_tier(req), boss, "lv {lv} named {named} drew a Lv{req} row");
                }
            }
        }
        // 142차: 90제 43 (전설 18 incl. 섬광의군화 · 세트 셋 15 · hero3 무기 7 · 악세 3), 85제 35.
        assert_eq!(tier_len(true), 43);
        assert_eq!(tier_len(false), 35);
        assert!(is_boss_tier(req_of((0x03, 0x31))), "섬광의군화 is 90제");
    }

    /// Uniform item pick: every candidate is reachable exactly once per residue and big draws
    /// fold like their residue.
    #[test]
    fn pick_is_uniform_over_the_pool() {
        for (lv, named) in [(85i8, false), (90, false), (100, false), (93, true), (100, true), (90, true)] {
            let n = candidates(lv, named);
            let pool = pool_for(lv, named);
            let mut seen = alloc::vec::Vec::new();
            for d in 0..n {
                let p = pick(lv, named, d).expect("a draw");
                assert!(row_in_pool(req_of(p), pool));
                assert!(!seen.contains(&p), "duplicate pick at lv {lv}");
                seen.push(p);
            }
            assert_eq!(seen.len() as u32, n);
            assert_eq!(pick(lv, named, n * 7 + 2), pick(lv, named, 2), "a big draw folds like its residue");
        }
    }

    /// ★ 152차 — the recipe pool: ALL 78 조합서 of the new items (cat 0x0f, the 138차 rows
    /// 0x27..=0x74), one per new item and both tiers in the same counts as the items, each
    /// reachable exactly once per residue.
    #[test]
    fn every_recipe_is_reachable_and_uniform() {
        assert_eq!(HELL_RECIPES.len(), HELL_ITEMS.len(), "one 조합서 per new item");
        let mut idxs: alloc::vec::Vec<u16> = alloc::vec::Vec::new();
        for &(cat, idx, tier) in &HELL_RECIPES {
            assert_eq!(cat, 0x0f, "a 조합서 is cat 0x0f");
            assert!((0x27..=0x74).contains(&idx), "idx {idx:#x} is not a 138차 tower recipe row");
            assert!(tier == 85 || tier == 90, "tier {tier}");
            idxs.push(idx);
        }
        let n = idxs.len();
        idxs.sort_unstable();
        idxs.dedup();
        assert_eq!(idxs.len(), n, "duplicate 조합서");
        let by_tier = |t: u8| HELL_RECIPES.iter().filter(|r| r.2 == t).count() as u32;
        assert_eq!((by_tier(85), by_tier(90)), (tier_len(false), tier_len(true)), "the recipes mirror the items");
        let picks: alloc::vec::Vec<(u16, u16)> = (0..HELL_RECIPES.len() as u32).map(pick_recipe).collect();
        for &(cat, idx, _) in &HELL_RECIPES {
            assert_eq!(picks.iter().filter(|&&p| p == (cat, idx)).count(), 1, "recipe {idx:#x}");
        }
        assert_eq!(pick_recipe(78 * 9 + 5), pick_recipe(5), "a big draw folds like its residue");
    }

    /// ★ 152차 — the shard is the tower material, 닉스코어파편 (`i14_dat` idx 0x2e).
    #[test]
    fn the_shard_is_the_tower_material() {
        assert_eq!(HELL_SHARD, (0x0e, 0x2e));
    }

    /// The rate: 1 of every 100 draws («전부 1%»), and the pure form is what the shipped
    /// constant means (the live knob is exercised by reaverdrop's own knob test).
    #[test]
    fn rate_is_one_of_a_hundred() {
        assert_eq!(RATE_PCT, 1);
        assert_eq!((0u32..100).filter(|&d| wins_at(d, RATE_PCT)).count() as u32, RATE_PCT);
        assert!(wins_at(0, RATE_PCT) && !wins_at(1, RATE_PCT) && !wins_at(99, RATE_PCT));
        assert_eq!(wins_at(1_000_000, RATE_PCT), wins_at(0, RATE_PCT));
        assert_eq!(wins_at(1_000_001, RATE_PCT), wins_at(1, RATE_PCT));
    }

    /// The named set: the game's own red names (`{`) from `enemyh_dat`, sorted, unique,
    /// all below `boss_dat`, and a superset of the reaver's eight drop bosses; every
    /// `boss_dat` id is named by number.
    #[test]
    fn named_bosses_are_the_red_names_and_boss_dat() {
        assert!(!HELL_NAMED_SPECIES.is_empty());
        for w in HELL_NAMED_SPECIES.windows(2) {
            assert!(w[0] < w[1], "sorted and unique");
        }
        for &s in &HELL_NAMED_SPECIES {
            assert!(s < BOSS_DAT_BASE, "species {s} is not an enemy_dat row");
            assert!(is_named(s));
        }
        for s in super::super::reaverdrop::BOSS_SPECIES {
            assert!(is_named(s), "reaver boss {s} must be a hell named boss too");
        }
        assert!(is_named(BOSS_DAT_BASE) && is_named(175) && is_named(u8::MAX));
        assert!(!is_named(0) && !is_named(23) && !is_named(160), "ordinary species are not named");
    }

    /// The table itself: 78 rows at exactly the two 142차 tiers, equipment cats only, no
    /// duplicates — and 섬광의군화 (the 131차-C row, normalised by 133차-B) is one of them.
    #[test]
    fn the_table_is_the_legacy_rework() {
        assert_eq!(HELL_ITEMS.len(), 78);
        assert!(HELL_ITEMS.contains(&(0x03, 0x31, 90)), "섬광의군화 must be in the pool at 90제");
        let mut reqs: alloc::vec::Vec<u8> = HELL_ITEMS.iter().map(|&(_, _, r)| r).collect();
        reqs.sort_unstable();
        reqs.dedup();
        assert_eq!(reqs, alloc::vec![85, 90], "142차: two tiers and nothing else");
        for &(cat, idx, _) in &HELL_ITEMS {
            assert!(cat <= 0x0c, "cat {cat:#x} is not equipment");
            assert!(idx >= 0x1a, "idx {idx:#x} would be a stock row");
        }
        let mut pairs: alloc::vec::Vec<(u16, u16)> = HELL_ITEMS.iter().map(|&(c, i, _)| (c, i)).collect();
        pairs.sort_unstable();
        pairs.dedup();
        assert_eq!(pairs.len(), HELL_ITEMS.len(), "duplicate (cat, idx)");
        // Hell is 2 and nothing else: the game's own writers use 0 and 1.
        assert!(is_hell(2) && !is_hell(0) && !is_hell(1) && !is_hell(3));
    }
}
