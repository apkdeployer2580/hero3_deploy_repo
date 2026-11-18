//! Reaver drop — one extra, species-gated roll at the END of the game's loot routine.
//!
//! ── WHY A HOOK AND NOT A TABLE WRITE ─────────────────────────────────────────
//! The `{리버…` rows (cat 4..0xb, appended to the item DB) were meant to drop from the
//! eight `{` hard bosses. The first attempt (vc204) wrote a "slot 1" into the drop table
//! at `*(CHAR+0x9c88)` from the host. That table has NO slot 1, and the write could not
//! have worked anyway:
//!
//! * the table is rebuilt on EVERY map load (`0x1770` → `0x37694`) and indexed by the
//!   map's object number (`enemy+0x3e`), not by species — `ptr + species*0x14` is off the
//!   end of a table a few records long;
//! * spawn (`0x37150`) copies the 20-byte record into the enemy (`enemy+0x13c`) in the
//!   same call chain as the load, so anything the host writes afterwards is too late;
//! * the record has exactly ONE specific-item slot (`+0xe` cat / `+0xf` idx, band
//!   `+8 < roll < +0xc`), and that slot is the boss's stock 종결.
//!
//! ── THE LOOT ROUTINE (LGT `binary.mod`, md5 6a352944…) ───────────────────────
//! `0x38e70(enemy)`: for each of `(s8)enemy[0xbe]` rolls, `r = rand(0, 0xffff)` is
//! bucketed against the record's cumulative thresholds (weapon / armour / 혼 / specific /
//! consumable; each arm writes one `(cat, idx)` pair into per-roll arrays). Then
//! `0x38fb2..0x38ffa` compacts the hits into two u16 arrays and `0x38ffc cmp r7,#0` /
//! `0x38ffe ble 0x39036` decides whether to spawn loot at all; `0x39000..0x39034` spawns
//! `[sp+0xc]` items from `sp+0x28` (cats) and `sp+0x14` (idxs).
//!
//! The hook sits on that `cmp r7,#0` — after every stock roll has been decided and
//! compacted, before the spawn — and APPENDS one item when the enemy's species is one of
//! the eight bosses and a fresh guest `rand(0,99)` lands below [`RATE_PCT`]; WHICH of the
//! eight reaver rows is a second guest draw, `rand(0,7)` (player decision 2026-09-03:
//! any boss can drop any reaver, not a fixed one per boss). A second, independent
//! `rand(0,99)` gives the two boss-only accessories ([`BOSS_ACCESSORIES`]). It then
//! performs the displaced compare itself by choosing the resume point: `exit_pc`
//! (`0x39000`, spawn) when anything is to be dropped, `skip_pc` (`0x39036`, the tail)
//! otherwise. So the stock drops — 종결 included — are untouched, and the reaver is an
//! independent per-kill chance layered on top.
//!
//! ★★★ 152차-B: that boss gate is HARD's rule. In HELL the reaver rolls on every kill whose
//! live level is 68..=[`REAVER_HELL_LV_MAX`] (81) instead — any species — and not on the eight
//! bosses, who fight at 98..100 there ([`rolls_reaver`]). The rows themselves are 80제 now
//! (`tools_re/add_reaver_weapons.py`): «하드코어와 헬 모두에서 파밍이 가능한 80제 무기».
//!
//! ⛔ THE COMPACTION LOOP OWNS `r5`. The enemy sits there only for the ROLL loop; at the
//! hook site `r5` is that later loop's index. Recover the enemy from `r0` / `[sp+4]`
//! instead - [`enemy_from_frame`], and the report that fix came from.
//!
//! ── THE DICE ARE THE GAME'S OWN ──────────────────────────────────────────────
//! `rand(lo, hi)` (`0x1bfa0`) is `lo + random() % (hi - lo + 1)` over the BSD additive
//! generator at `0x1beec`: `*rptr += *fptr; result = (s32)*rptr >> 1`, both pointers
//! advancing through a state array and wrapping from `*0x14001a8` back to `*0x14001a4`.
//! [`random_step`] reproduces that step exactly and the hook writes the pointers back, so
//! a probe run stays deterministic and the reaver roll is one more draw from the same
//! stream — no host RNG, no wall clock.
//!
//! ── SPECIES ──────────────────────────────────────────────────────────────────
//! `enemy+0` → map object, `+0x1c` → the map record whose byte `+0xd` is the object
//! index the spawn stored at `enemy+0x3e` and whose byte `+0xe` is the species
//! (`0x3863c` compares that byte with 0xa0, the same guard the map loader applies to
//! its species list). The hook cross-checks `+0xd` against `enemy+0x3e` and declines on
//! a mismatch rather than trusting a pointer chain it cannot otherwise validate.

use alloc::format;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use wie_util::{ByteRead, Result, WieError, read_generic, write_generic};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// `cmp r7, #0` at `0x38ffc` — the instruction the hook replaces. Checked at install time
/// (see `hook::apply_hooks`): the hook performs that compare itself by picking the resume
/// point, so a moved build must refuse to install rather than skip a compare that is not
/// there.
pub const EXPECTED_HEAD: [u8; 2] = [0x00, 0x2f];

/// Chance, per boss kill, of one reaver row on top of the stock loot. Percent of a
/// guest `rand(0, 99)`. Player decision 2026-09-03: 5.
pub const RATE_PCT: u32 = 5;

/// Boss species that can drop a reaver: 29 카이저골렘, 38 블레이즈로커, 39 카오스로커, 52 시바,
/// 135 와일드스쿠툼, 138 다크니스이라, 141 크레이지타이탄, 144 와일드아르보르 — the eight `{`
/// bosses at Lv 65..67 (the other three `{` bosses, 35/36/142, are Lv 43..56 and excluded).
///
/// ⛔ NOT "hard mode only" (corrected 2026-09-03). The species number comes from the MAP
/// record (`MAPREC_SPECIES`), never from a stat table, and `res/dat/enemy_dat` and
/// `res/dat/enemyh_dat` both hold 161 records with the same names at the same indices — the
/// difficulty flag picks which STATS file to read, not which ids exist. For these eight the
/// level byte is identical in both files (29:65/65 · 38:65/65 · 39:66/66 · 52:67/67 ·
/// 135:66/66 · 138:67/67 · 141:66/66 · 144:65/65), so a NORMAL-mode player who reaches
/// 크레이지타이탄 gets the same 5 % roll. Ordinary rows do differ per difficulty — 커브이라
/// (136/137) is Lv 30/43 normal and 57/66 hard — which is why the id, not the level, is the gate.
/// ⛔ 136/137 are 커브이라, the ordinary counterpart sitting immediately before 138. They are
/// NOT in this list and must never be added: `only_the_eight_bosses_drop` pins that.
pub const BOSS_SPECIES: [u8; 8] = [29, 38, 39, 52, 135, 138, 141, 144];

/// The eight `{리버…` rows as `(cat, idx)`: spear, sword, knife, gun, cannon, dark stone,
/// holy stone at the first index after the stock 30 (`0x1e`), shield after the stock 26
/// (`0x1a`). Appended by `tools_re/add_reaver_weapons.py`; the count-table patches in
/// `binary_patches.toml` are what make the guest see them.
pub const REAVER_ITEMS: [(u16, u16); 8] = [
    (0x04, 0x1e),
    (0x05, 0x1e),
    (0x06, 0x1e),
    (0x07, 0x1e),
    (0x08, 0x1e),
    (0x09, 0x1e),
    (0x0a, 0x1e),
    (0x0b, 0x1a),
];

/// ★★★ 152차-B (user decision 2026-10-02): «하드코어모드에서는 보스8종에게서만 나오고 헬에서는 ~81
/// 몬스터 구간에서 나오게 … 하드코어와 헬 모두에서 파밍이 가능한 80제 무기». In HELL (difficulty
/// byte 2) the reaver is the low band's farming drop: every kill whose LIVE level (the host's hell
/// fork, `helldrop::HELL_LV_MIN`..) is at or below this rolls it at [`RATE_PCT`], any species,
/// named or not — and the eight bosses, who fight at 98..100 in hell, do not. In HARD nothing
/// changed: the eight bosses only. (The rows themselves became 80제 in the same change —
/// `tools_re/add_reaver_weapons.py`.)
pub const REAVER_HELL_LV_MAX: i8 = 81;

/// Does this kill roll the reaver? `hell` = the difficulty byte is 2, `lv` = the kill's live
/// level (read only in hell; `None` = not read). Hard: [`is_boss`]. Hell: on the ladder and
/// <= [`REAVER_HELL_LV_MAX`] — an unforked record still carries its hard-table level (38..67),
/// below the ladder, so it rolls nothing (fail closed, like hell's own rolls).
pub fn rolls_reaver(hell: bool, species: u8, lv: Option<i8>) -> bool {
    if hell {
        lv.is_some_and(|lv| super::helldrop::on_ladder(lv) && lv <= REAVER_HELL_LV_MAX)
    } else {
        is_boss(species)
    }
}

/// Boss-only accessories, species → `(cat, idx)` in cat 0x0c (혼/악세): 크레이지타이탄 →
/// `{타이탄의 파편` (idx 0x28), 다크니스이라 → `{검은 눈물` (idx 0x29). Rows appended by
/// `tools_re/add_boss_accessories.py`; the cat-0x0c count patch in `binary_patches.toml`
/// makes the guest see them. Rolled independently of the reaver at the same [`RATE_PCT`].
pub const BOSS_ACCESSORIES: [(u8, u16, u16); 2] = [(141, 0x0c, 0x28), (138, 0x0c, 0x29)];

/// Frame slots, SP-relative at the hook site (`sub sp,#0x64` at `0x38e72`).
const SP_COUNT: u32 = 0xc;
const SP_IDXS: u32 = 0x14;
const SP_CATS: u32 = 0x28;
/// Both arrays are `0x14` bytes = ten u16 — one per roll, and the roll count is a byte.
const MAX_ITEMS: u32 = 10;

const ENEMY_OBJECT_INDEX: u32 = 0x3e;
const MAPOBJ_RECORD: u32 = 0x1c;
const MAPREC_OBJECT_INDEX: u32 = 0xd;
const MAPREC_SPECIES: u32 = 0xe;

/// `(s8)enemy[0xbe]` — the roll count. The loot routine keeps a POINTER to this byte
/// rather than to the enemy itself, in `r0` and in `[sp+4]`, which is how the enemy is
/// recovered at the hook site. See [`enemy_from_frame`].
/// Kills this process has run the hook for - the counter behind the debut log.
static KILLS: AtomicU32 = AtomicU32::new(0);
/// How many kills report themselves before the log goes quiet. See `try_append`.
const DEBUT_KILLS: u32 = 20;

const ENEMY_ROLL_COUNT: u32 = 0xbe;
/// Frame slot holding that same `enemy + 0xbe` pointer (`0x38ec0 str r2,[sp,#4]`).
const SP_ROLL_COUNT_PTR: u32 = 4;

/// ★★★ THE ENEMY POINTER AT THE HOOK SITE IS NOT `r5` (fixed 2026-09-03).
///
/// The loot routine puts the enemy in `r5` at `0x38e76` and the FIRST loop (the rolls)
/// reads its fields through it — but the COMPACTION loop that follows reuses `r5` as its
/// own index (`0x38fc4 ldr r5,[sp,#0x10]`, `0x38fee`..`0x38ff2`). By `0x38ffc cmp r7,#0`,
/// the hook site, `r5` is that index: a small integer like 0, 1, 2. The first version of
/// this hook read the enemy from it, so `species_of` dereferenced address ~0, declined,
/// and **no boss ever dropped a reaver or an accessory** — the exact report that came back
/// after vc213/vc219 («종결보스를 잡아도 리버가 안 뜨고 다크니스/크레이지타이탄에서 장신구도
/// 안 뜬다»).
///
/// What IS live at the hook site is `enemy + 0xbe`, in two places that must agree:
/// * `r0`, set by `0x38fb4 adds r0,r5,#0` + `0x38fb8 adds r0,#0xbe` and only ever read
///   (`0x38fbc` / `0x38ff6 ldrsb r3,[r0,r3]`) until the hook;
/// * `[sp+4]`, stored once at `0x38ec0` and read at `0x38fa0`.
///
/// Requiring the two to agree is a free frame check: if a future build moves the site or
/// the register allocation changes, they disagree and the hook declines instead of
/// appending loot derived from a garbage pointer. `None` = decline.
pub fn enemy_from_frame(r0: u32, sp_slot: u32) -> Option<u32> {
    if r0 != sp_slot || r0 < ENEMY_ROLL_COUNT {
        return None;
    }
    Some(r0 - ENEMY_ROLL_COUNT)
}

// ── the DIFFICULTY GATE (78차-C) ─────────────────────────────────────────────
//
// Player decision 2026-09-03: «리버와 장신구는 하드에서만 떨어지게 하고싶은거야 노말에선
// 떨어지면 안돼». Boss-only was already true; this adds hard-only on top.
//
// ★★★ THE FLAG, derived from the map loader (not taken from the repo notes, which are WRONG
// about it - see below). `0x37694` builds the per-map tables and twice branches on ONE byte:
//
//   0x376ca  ldr  r5,=0xa318 ; 0x376d0 adds r5,r2,r5   ; r2 = getBase()
//   0x376da  str  r5,[sp,#0x10]                        ; stash the pointer
//   0x376dc  ldrb r3,[r5] ; 0x376de cmp r3,#0 ; 0x376e0 beq 0x376e6
//   0x376e2  ldr  r0,="res/dat/enemyh_dat"   (flag != 0 -> HARD stats)
//   0x376e6  ldr  r0,="res/dat/enemy_dat"    (flag == 0 -> normal stats)
//
//   0x379d6  ldr  r2,[sp,#0x10]                        ; the SAME pointer
//   0x379e0  ldrb r3,[r2] ; 0x379e2 cmp r3,#0 ; 0x379e4 beq 0x379ea
//   0x379e6  ldr  r0,="res/dat/droph_dat"    (flag != 0 -> HARD drops)
//   0x379ea  ldr  r0,="res/dat/drop_dat"     (flag == 0 -> normal drops)
//
// So one unsigned byte selects BOTH the stat table and the drop table: 0 = normal, non-zero =
// hard. Gating on it is exactly "is this the hard-mode drop table the game itself loaded".
//
// ★ It is the game's own hard-mode predicate, not just a drop-table selector: the same byte
// picks `shoph_dat`/`shop_dat` (`0x1dc6a`), `smithh_dat`/`smith_dat` (`0x1fe02`),
// `bossh_dat`/`boss_dat` (`0x25b8a`) and adds a `+10000` bonus at `0x3d604`.
//
// ★ WHY READING IT AT KILL TIME IS SOUND. It has exactly three writers: `0x3ef80` stores 0
// (character reset `0x3ef70`, whose callers include the `setAppState(3)` arm `0x14fa` and world
// init `0x3abb0`), `0x3b0bc` stores 1 (start-hard-game `0x3af2c`, which resets the character
// first - starting hard IS starting a new character), and `0x1a9d4` in `deserialiseSave`.
// `serialiseSave` carries it back out (`0x1aed4`/`0x1aed8`), so difficulty is a PER-CHARACTER
// SAVED property, not a menu re-picked each run. The only path into the field is
// `setAppState(4)` -> world init `0x3ab68`, which clears the byte at `0x3abb0` and only then
// restores it at `0x3ac60`; the map load runs after that. So there is no window in which an
// enemy exists and the byte is stale.
// ⛔ `base+0xa31a+slot` (`0x3b0a4`, `0x3f5f2`) is a DIFFERENT, slot-indexed array - no reader
// ever indexes the flag by slot. Do not follow those two stores to "the flag".
//
// ⛔⛔ THE REPO NOTES SAY `CHAR+0xa318` AND THAT IS OFF BY 0x28. The literal is added to
// `getBase()`, and `getBase() == CHAR + 0x28` on this build - so the address is
// `CHAR + 0xa340`, not `CHAR + 0xa318`. Same confusion as the shell screen word
// (`base+0xa3d4` = `CHAR+0xa3fc`). Reading 0xa318 off CHAR lands 0x28 short, on an unrelated
// byte, and would have gated the drop on garbage.

/// LGT: CHAR is a static at `.bss 0x1503a88`, and `getBase()` returns `CHAR + 0x28`.
const GUEST_BASE: u32 = 0x0150_3a88 + 0x28;
/// Difficulty byte, base-relative. `0` = normal, anything else = hard.
const HARD_FLAG_OFF: u32 = 0xa318;
/// Absolute address of that byte on this build: `0x150ddc8`.
pub const HARD_FLAG_ADDR: u32 = GUEST_BASE + HARD_FLAG_OFF;

/// Is the game in HARD mode right now?
///
/// ⛔ FAILS CLOSED. An unreadable byte answers "not hard", so the drop does not happen. The
/// player asked for hard-only, which makes a false HARD the expensive error: it would put
/// reavers into a normal-mode save, and picking one up writes it into that save permanently.
/// A false NORMAL only costs one missed 5 % roll on a boss that can be killed again.
///
/// ⛔ But it does NOT decline silently. 78차 survived eight releases precisely because the
/// give-up path logged nothing, and a fail-closed gate that cannot read its byte looks exactly
/// like a player who is simply in normal mode. Once per process is enough: the read either
/// works or it never will.
fn is_hard(core: &mut ArmCore) -> bool {
    wie_util::stats::reaver_force_hard() || difficulty(core).is_some_and(|b| b != 0)
}

/// The difficulty byte itself — 0 normal, 1 hard, 2 hell (133차, host-written; see
/// `super::helldrop::HELL_DIFFICULTY`). `None` = unreadable, reported once.
fn difficulty(core: &mut ArmCore) -> Option<u8> {
    match read_u8(core, HARD_FLAG_ADDR) {
        Ok(b) => Some(b),
        Err(e) => {
            if !HARD_UNREADABLE.swap(true, Ordering::Relaxed) {
                tracing::warn!("reaver drop: cannot read the difficulty byte at {HARD_FLAG_ADDR:#x} ({e:?}); every kill counts as NORMAL");
            }
            None
        }
    }
}

/// Whether the failure above has already been reported. See [`is_hard`].
static HARD_UNREADABLE: AtomicBool = AtomicBool::new(false);

/// BSD `random()` state words in LGT `.data` (loader `0x1beec`).
const RNG_FPTR: u32 = 0x140011c;
const RNG_RPTR: u32 = 0x1400120;
const RNG_START: u32 = 0x14001a4;
const RNG_END: u32 = 0x14001a8;

pub fn check_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == EXPECTED_HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "reaver_drop hook at {pc:#x}: expected {EXPECTED_HEAD:02x?} (cmp r7,#0), found {head:02x?}"
    )))
}

/// Whether a species is one of the eight reaver-dropping bosses.
pub fn is_boss(species: u8) -> bool {
    BOSS_SPECIES.contains(&species)
}

/// Which reaver a winning kill yields: `rand(0, 7)` over [`REAVER_ITEMS`], with `rand`
/// being the guest's `lo + random() % (hi-lo+1)` — every boss can drop every row.
pub fn pick(random: u32) -> (u16, u16) {
    REAVER_ITEMS[(random % REAVER_ITEMS.len() as u32) as usize]
}

/// The accessory a species can drop, or `None` for everything but the two.
pub fn accessory_for(species: u8) -> Option<(u16, u16)> {
    BOSS_ACCESSORIES.iter().find(|&&(s, _, _)| s == species).map(|&(_, cat, idx)| (cat, idx))
}

/// `rand(0, 99) < rate`, with `rand` being the guest's `lo + random() % (hi-lo+1)`.
/// Pure, so the shipped 5 % stays under test whatever the runtime knob says.
pub fn wins_at(random: u32, rate: u32) -> bool {
    random % 100 < rate
}

/// The rate actually in force: [`RATE_PCT`] unless a tester armed the knob
/// (`wie_util::stats::REAVER_RATE_PCT`, driven by `wie_reaver_test.txt`).
pub fn live_rate() -> u32 {
    wie_util::stats::reaver_rate_pct(RATE_PCT)
}

/// `rand(0, 99) <` the live rate.
pub fn wins(random: u32) -> bool {
    wins_at(random, live_rate())
}

/// The species the roll should be judged against: the one the map record actually holds,
/// unless a tester armed `wie_util::stats::REAVER_AS_SPECIES`.
///
/// The override is applied AFTER [`species_of`] has succeeded, never instead of it. A
/// test that skipped the pointer chain would mask exactly the bug 78차 fixed (the enemy
/// read from the wrong register, `species_of` declining in silence), so the knob widens
/// WHICH species drop - it never fakes that a species was read.
pub fn effective_species(real: u8) -> u8 {
    wie_util::stats::reaver_as_species().unwrap_or(real)
}

/// One step of the guest generator, pure. Inputs are the two cursor words, the wrap
/// bounds, and the state words the cursors point at; outputs are the draw, the value to
/// store back at the old `rptr`, and the advanced cursors.
///
/// Mirrors `0x1beec` instruction for instruction: the sum is stored, both cursors move
/// on by one word, and the wrap check is `rptr` first — when `rptr` wraps, `fptr` is
/// stored unchecked (it is necessarily still inside the array).
pub fn random_step(fptr: u32, rptr: u32, start: u32, end: u32, f_val: u32, r_val: u32) -> RandomStep {
    let sum = r_val.wrapping_add(f_val);
    let result = ((sum as i32) >> 1) as u32;
    let mut new_fptr = fptr.wrapping_add(4);
    let mut new_rptr = rptr.wrapping_add(4);
    if new_rptr >= end {
        new_rptr = start;
    } else if new_fptr >= end {
        new_fptr = start;
    }
    RandomStep {
        result,
        store_at_rptr: sum,
        fptr: new_fptr,
        rptr: new_rptr,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RandomStep {
    pub result: u32,
    pub store_at_rptr: u32,
    pub fptr: u32,
    pub rptr: u32,
}

/// Draw one `random()` from the guest's generator, advancing it exactly as the guest would.
fn guest_random(core: &mut ArmCore) -> Result<u32> {
    let fptr: u32 = read_generic(core, RNG_FPTR)?;
    let rptr: u32 = read_generic(core, RNG_RPTR)?;
    let start: u32 = read_generic(core, RNG_START)?;
    let end: u32 = read_generic(core, RNG_END)?;
    let f_val: u32 = read_generic(core, fptr)?;
    let r_val: u32 = read_generic(core, rptr)?;
    let step = random_step(fptr, rptr, start, end, f_val, r_val);
    write_generic(core, rptr, step.store_at_rptr)?;
    write_generic(core, RNG_FPTR, step.fptr)?;
    write_generic(core, RNG_RPTR, step.rptr)?;
    Ok(step.result)
}

fn read_u8(core: &mut ArmCore, addr: u32) -> Result<u8> {
    let mut b = [0u8; 1];
    core.read_bytes(addr, &mut b)?;
    Ok(b[0])
}

/// Species of a live enemy, or `None` when the pointer chain does not agree with itself.
fn species_of(core: &mut ArmCore, enemy: u32) -> Result<Option<u8>> {
    let object: u32 = read_generic(core, enemy)?;
    if object == 0 {
        return Ok(None);
    }
    let record: u32 = read_generic(core, object + MAPOBJ_RECORD)?;
    if record == 0 {
        return Ok(None);
    }
    let rec_index = read_u8(core, record + MAPREC_OBJECT_INDEX)?;
    let enemy_index: u16 = read_generic(core, enemy + ENEMY_OBJECT_INDEX)?;
    if u16::from(rec_index) != enemy_index {
        tracing::warn!("reaver drop: enemy {enemy:#x} object index {enemy_index} != map record {rec_index}; declining");
        return Ok(None);
    }
    Ok(Some(read_u8(core, record + MAPREC_SPECIES)?))
}

/// Append one `(cat, idx)` to the compacted loot. `false` when the arrays are full.
fn append(core: &mut ArmCore, sp: u32, cat: u16, idx: u16) -> Result<bool> {
    let count: u32 = read_generic(core, sp + SP_COUNT)?;
    if count >= MAX_ITEMS {
        tracing::warn!("reaver drop: won the roll but the loot arrays are full ({count}); cat {cat:#x} idx {idx:#x} skipped");
        return Ok(false);
    }
    write_generic(core, sp + SP_CATS + count * 2, cat)?;
    write_generic(core, sp + SP_IDXS + count * 2, idx)?;
    write_generic(core, sp + SP_COUNT, count + 1)?;
    Ok(true)
}

/// Roll the reaver and, for the two accessory bosses, the accessory; append the wins to
/// the compacted loot. Returns whether anything was added.
///
/// Draw order is fixed — reaver win, reaver pick, accessory win, then (in hell, by live level)
/// recipe, item, shard — so a probe replay reproduces every kill. Each roll is independent:
/// 크레이지타이탄 can drop both, and a hell Lv 85 kill can add three more.
fn try_append(core: &mut ArmCore, enemy: u32, sp: u32) -> Result<bool> {
    let Some(real_species) = species_of(core, enemy)? else {
        return Ok(false);
    };
    let species = effective_species(real_species);
    let rate = live_rate();
    let diff = difficulty(core);
    let hard = wie_util::stats::reaver_force_hard() || diff.is_some_and(|b| b != 0);
    // 헬 is the BYTE and only the byte: the force-hard test knob widens the reaver
    // gate, never the hell one (a probe pokes the byte to 2 instead).
    let hell = diff.is_some_and(super::helldrop::is_hell);
    // THE FIRST FEW KILLS ALWAYS SPEAK (78차-B).
    //
    // Every log line below is gated on the kill being a BOSS, so an ordinary kill used to
    // produce EXACTLY ZERO lines - and the two `species_of` bail-outs are silent too. That
    // is the shape of the bug 78차 fixed: the hook read the enemy from the wrong register,
    // `species_of` dereferenced ~0, declined without a word, and eight shipped builds
    // dropped nothing while looking perfectly healthy. A tester whose save cannot reach a
    // Lv 65 boss had no way at all to tell the two states apart.
    //
    // So the first [`DEBUT_KILLS`] kills of a process report themselves. That is proof of
    // life for the whole chain - hook fires, frame check passes, pointer chain agrees,
    // species reads - on any monster the player can actually kill, and then it goes quiet
    // so a grinding session is not narrated. The loot routine has exactly ONE caller
    // (`0x3a5ca`, whole-image BL scan), so this is one line per kill and no more.
    let debut = KILLS.fetch_add(1, Ordering::Relaxed);
    if debut < DEBUT_KILLS || wie_util::stats::reaver_as_species().is_some() {
        let masked = if species == real_species {
            alloc::string::String::new()
        } else {
            format!(" (testing AS {species})")
        };
        // ★ 152차: the LIVE level too — hell's rolls are judged on it, and a hell kill that reads
        // its hard-table spawn level (38..67) here is a record the host's hell fork never raised.
        // `.ok()`: this line is diagnosis only and must not turn a read into a tick error.
        let lv = read_u8(core, enemy + super::helldrop::CREATURE_LEVEL_OFF).ok().map(|b| b as i8);
        tracing::warn!(
            "reaver drop: kill #{} enemy {enemy:#x} species {real_species}{masked} Lv{lv:?} - boss {} - reaver {} - hard {hard} - hell {hell} - rate {rate}%",
            debut + 1,
            is_boss(species),
            hard && rolls_reaver(hell, species, lv),
        );
    }
    // ★★★ HARD MODE ONLY (78차-C). Checked before any die is cast, so a normal-mode kill draws
    // nothing at all and the guest RNG stream is left exactly as the stock game would leave it -
    // a normal-mode session is byte-identical to one with no hook.
    if !hard {
        return Ok(false);
    }
    let mut added = false;
    // ★ 152차-B: hell's live level, read once — the reaver band and hell's own rolls both use it.
    let hell_lv = if hell { Some(read_u8(core, enemy + super::helldrop::CREATURE_LEVEL_OFF)? as i8) } else { None };
    if rolls_reaver(hell, species, hell_lv) {
        let roll = guest_random(core)? % 100;
        if wins(roll) {
            // The second draw picks the row, after the first decided the win.
            let (cat, idx) = pick(guest_random(core)?);
            if append(core, sp, cat, idx)? {
                added = true;
                match hell_lv {
                    Some(lv) => tracing::warn!("reaver drop: hell species {species} Lv{lv} rolled {roll} -> reaver cat {cat:#x} idx {idx:#x}"),
                    None => tracing::warn!("reaver drop: boss species {species} rolled {roll} -> reaver cat {cat:#x} idx {idx:#x}"),
                }
            }
        } else if !hell {
            // warn, not trace: the Android build filters the wie crates to warn, and a
            // boss kill is rare enough that the line is the evidence the hook is alive.
            // (A losing HELL roll stays silent — every Lv 68..81 kill rolls, constantly.)
            tracing::warn!("reaver drop: boss species {species} rolled {roll} (needs < {rate}); no reaver");
        }
    }
    if let Some((cat, idx)) = accessory_for(species) {
        let roll = guest_random(core)? % 100;
        if wins(roll) {
            if append(core, sp, cat, idx)? {
                added = true;
                tracing::warn!("reaver drop: boss species {species} rolled {roll} -> accessory cat {cat:#x} idx {idx:#x}");
            }
        } else {
            tracing::warn!("reaver drop: boss species {species} rolled {roll} (needs < {rate}); no accessory");
        }
    }
    // ★★★ 헬모드 (133차 · pools 134차 · ladder 152차): in hell a kill at LIVE level 79+ also
    // rolls the new loot, each roll its own die at `helldrop::RATE_PCT` (1 %): a 조합서 at
    // 79..=85 (uniform over all 78), a new item at 85+ (named 93+ → 90제, everyone else → 85제),
    // a 닉스코어파편 at 79+. Below 79 hell adds nothing of these — the stock hard loot and the
    // reaver dice above (152차-B: every hell kill at 68..=81) are the whole drop («~78레벨까지는 …
    // 기존과 같은 확률»). The level is the one the host's hell fork wrote (68..100); an unforked record
    // still reads its hard-table level (38..67) and so rolls nothing — the ladder fails closed.
    // Rolled AFTER the reaver dice so a hard-mode session's RNG stream is untouched (hell adds
    // draws only in hell, and only at 79+), in a fixed order — recipe, item, shard — so a probe
    // replay reproduces every kill. Losing rolls stay silent: an ordinary kill happens constantly.
    // ★ `species` is the MASKED id on purpose: the `as` test knob can stand in for a
    //   named boss (`as 141` = 크레이지타이탄) on any kill, the same way it tests the reaver.
    if let Some(lv) = hell_lv {
        use super::helldrop as hd;
        if hd::rolls_recipe(lv) {
            let roll = guest_random(core)? % 100;
            if hd::wins(roll) {
                let (cat, idx) = hd::pick_recipe(guest_random(core)?);
                if append(core, sp, cat, idx)? {
                    added = true;
                    tracing::warn!("hell drop: species {species} Lv{lv} rolled {roll} -> 조합서 cat {cat:#x} idx {idx:#x}");
                }
            }
        }
        if hd::rolls_item(lv) {
            let roll = guest_random(core)? % 100;
            if hd::wins(roll) {
                let named = hd::is_named(species);
                let pool = hd::pool_for(lv, named);
                match hd::pick(lv, named, guest_random(core)?) {
                    Some((cat, idx)) => {
                        if append(core, sp, cat, idx)? {
                            added = true;
                            tracing::warn!(
                                "hell drop: species {species} Lv{lv} named {named} pool {pool:?} rolled {roll} -> legacy cat {cat:#x} idx {idx:#x}"
                            );
                        }
                    }
                    // Unreachable while both pools hold rows (`rolls_item` is the same gate
                    // `candidates` uses) — said rather than assumed.
                    None => tracing::warn!("hell drop: species {species} Lv{lv} named {named} won the item roll but pool {pool:?} is empty; nothing dropped"),
                }
            }
        }
        if hd::rolls_shard(lv) {
            let roll = guest_random(core)? % 100;
            if hd::wins(roll) {
                let (cat, idx) = hd::HELL_SHARD;
                if append(core, sp, cat, idx)? {
                    added = true;
                    tracing::warn!("hell drop: species {species} Lv{lv} rolled {roll} -> 닉스코어파편 cat {cat:#x} idx {idx:#x}");
                }
            }
        }
    }
    Ok(added)
}

/// Hook body. `exit_pc` resumes into the spawn (`0x39001`), `skip_pc` into the tail
/// (`0x39037`); together they are the displaced `cmp r7,#0; ble`.
pub async fn handle_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32, skip_pc: u32) -> Result<JumpTo> {
    // ★★★ 하늘성: a tower floor hands out NOTHING (user decision 2026-09-11). This is
    // the one place in the image that decides whether loot is spawned at all, so the
    // rule is one branch here rather than a second hook — and it comes FIRST, before
    // the frame check and the reaver dice, because those exist only to add a drop.
    //
    // ⛔ Taking `skip_pc` is the game's own "nothing to drop" path (`0x39036`), not a
    // shortcut of ours: the roll loop has already run and its results are simply not
    // spawned. Stock drops and the reaver are both suppressed.
    // ⚠ `r7` is left alone. It is only ever tested against zero after this point, and
    // we are choosing the branch that test would have chosen for zero.
    if wie_util::stats::tower_no_drops() {
        return Ok(JumpTo(skip_pc));
    }
    let (roll_ptr, hits, sp) = {
        let inner = core.inner.lock();
        (
            // ⛔ NOT `r5` — the compaction loop owns it by now. See `enemy_from_frame`.
            inner.engine.reg_read(ArmRegister::R0),
            inner.engine.reg_read(ArmRegister::R7),
            inner.engine.reg_read(ArmRegister::SP),
        )
    };
    let sp_slot: u32 = read_generic(core, sp + SP_ROLL_COUNT_PTR)?;
    let mut hits = hits;
    let enemy = match enemy_from_frame(roll_ptr, sp_slot) {
        Some(e) => e,
        None => {
            tracing::warn!("reaver drop: frame check failed (r0 {roll_ptr:#x} vs [sp+4] {sp_slot:#x}); stock loot only");
            return Ok(JumpTo(if hits == 0 { skip_pc } else { exit_pc }));
        }
    };
    if try_append(core, enemy, sp)? && hits == 0 {
        // `r7` is only ever tested against zero after this point, but keep it truthful.
        hits = 1;
        core.inner.lock().engine.reg_write(ArmRegister::R7, hits);
    }
    // `cmp r7,#0; ble skip` — r7 is a count, so signed `<= 0` is `== 0`.
    Ok(JumpTo(if hits == 0 { skip_pc } else { exit_pc }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_is_cmp_r7_zero() {
        assert!(check_head([0x00, 0x2f], 0x38ffd).is_ok());
        assert!(check_head([0x2f, 0x00], 0x38ffd).is_err());
    }

    #[test]
    fn only_the_eight_bosses_drop() {
        for s in BOSS_SPECIES {
            assert!(is_boss(s), "species {s}");
        }
        // Neighbours, the ordinary golems, the three low-level `{` bosses, and the vermin rows.
        for s in [0u8, 28, 30, 35, 36, 55, 57, 59, 142, 145, 160, 255] {
            assert!(!is_boss(s), "species {s}");
        }
        assert_eq!(BOSS_SPECIES.len(), 8);
        // ★★★ 78차-B, THE SHIPPED ANSWER TO «노말에서 아무 몬스터나 잡아도 나오면 안 된다».
        // These are the species the reporter's own normal-mode save actually kills, read off the
        // device log on 2026-09-03 (`reaver drop: kill #N ... species 137 - boss false`): 136 and 137
        // are 커브이라, the normal-mode counterpart that sits immediately before 138 `{다크니스이라`
        // in `enemyh_dat`, and 37 / 152 are the ordinary monsters around it. None of them may drop a
        // reaver, and none of them may drop an accessory.
        for s in [136u8, 137, 37, 152] {
            assert!(!is_boss(s), "normal-mode species {s} must never roll a reaver");
            assert_eq!(accessory_for(s), None, "normal-mode species {s} must never roll an accessory");
        }
        // ⛔ 136/137 are NEIGHBOURS of a real boss id. The list must contain 138 and not 136/137 —
        // an off-by-one in either direction is the failure this pins.
        assert!(is_boss(138) && is_boss(141));
        assert!(!is_boss(136) && !is_boss(137) && !is_boss(139) && !is_boss(140));
    }

    #[test]
    fn every_reaver_is_reachable_and_uniform() {
        // rand(0,7) over eight rows: each residue names exactly one distinct row.
        let picks: alloc::vec::Vec<_> = (0u32..8).map(pick).collect();
        for &(cat, idx) in &REAVER_ITEMS {
            assert_eq!(picks.iter().filter(|&&p| p == (cat, idx)).count(), 1, "cat {cat:#x}");
        }
        // Weapons are cat 4..=0xa at idx 0x1e, the shield is cat 0xb at idx 0x1a.
        for &(cat, idx) in &REAVER_ITEMS {
            assert_eq!(idx, if cat == 0x0b { 0x1a } else { 0x1e });
            assert!((0x04..=0x0b).contains(&cat));
        }
        // A large draw folds like its residue.
        assert_eq!(pick(8 * 12345 + 3), pick(3));
    }

    #[test]
    fn accessories_belong_to_one_boss_each() {
        assert_eq!(accessory_for(141), Some((0x0c, 0x28)));
        assert_eq!(accessory_for(138), Some((0x0c, 0x29)));
        for s in BOSS_SPECIES {
            if s != 141 && s != 138 {
                assert_eq!(accessory_for(s), None, "species {s}");
            }
            // Both accessory bosses are reaver bosses too: two independent rolls there.
        }
        assert!(is_boss(141) && is_boss(138));
        assert_eq!(accessory_for(29), None);
        // The two idx are the rows appended after the 40 stock ones, distinct.
        assert_ne!(BOSS_ACCESSORIES[0].2, BOSS_ACCESSORIES[1].2);
        for &(_, cat, idx) in &BOSS_ACCESSORIES {
            assert_eq!(cat, 0x0c);
            assert!(idx >= 0x28 && idx < 0x2a);
        }
    }

    #[test]
    fn rate_is_five_of_a_hundred() {
        let hits = (0u32..100).filter(|&r| wins_at(r, RATE_PCT)).count();
        assert_eq!(hits, RATE_PCT as usize);
        assert!(wins_at(0, RATE_PCT) && wins_at(4, RATE_PCT) && !wins_at(5, RATE_PCT) && !wins_at(99, RATE_PCT));
        // `random % 100` folds the whole draw: a big draw behaves like its residue.
        assert_eq!(wins_at(1_000_003, RATE_PCT), wins_at(3, RATE_PCT));
    }

    /// 78차-B - the test knobs are OFF unless something arms them, and armed they do
    /// exactly two things: change the rate, and change which species the dice judge.
    #[test]
    fn the_test_knobs_are_inert_by_default() {
        assert_eq!(live_rate(), RATE_PCT, "no knob armed -> the shipped rate");
        assert_eq!(effective_species(7), 7, "no knob armed -> the species the map record holds");
        // A rate of 100 wins on every draw; a rate of 0 never wins. Both are pure.
        assert!((0u32..100).all(|r| wins_at(r, 100)));
        assert!(!(0u32..100).any(|r| wins_at(r, 0)));
        // Armed, the species override makes an ordinary monster take a boss's dice -
        // including the accessory roll, which is what widening `BOSS_SPECIES` cannot do.
        wie_util::stats::set_reaver_test(Some(100), Some(141), false);
        assert_eq!(live_rate(), 100);
        assert_eq!(effective_species(7), 141);
        assert!(is_boss(effective_species(7)));
        assert_eq!(accessory_for(effective_species(7)), Some((0x0c, 0x28)));
        // ...and disarming restores the shipped behaviour exactly.
        wie_util::stats::set_reaver_test(None, None, false);
        assert_eq!(live_rate(), RATE_PCT);
        assert_eq!(effective_species(7), 7);
        assert_eq!(accessory_for(effective_species(7)), None);

        // ★★★ THE SHIPPED INVARIANT, stated on `wins` itself and not just on `wins_at`.
        // A test that only checks `wins_at(r, RATE_PCT)` proves nothing about what the hook calls:
        // `wins` reads the LIVE rate. Unarmed, the two must agree for every draw, and 5 of every
        // 100 draws must win — this is the assertion that would fail if a build ever shipped with
        // the 100 % test rate baked in.
        assert!(
            (0u32..100).all(|r| wins(r) == wins_at(r, RATE_PCT)),
            "unarmed `wins` must be the shipped rate"
        );
        assert_eq!((0u32..100).filter(|&r| wins(r)).count(), 5);
        // ...and the species the reporter's normal-mode save kills still roll nothing, through the
        // same `effective_species` the hook uses.
        for sp in [136u8, 137, 37, 152] {
            assert!(!is_boss(effective_species(sp)), "unarmed: species {sp} is not a boss");
            assert_eq!(accessory_for(effective_species(sp)), None, "unarmed: species {sp} has no accessory");
        }
        // The difficulty knob is its own switch and is off by default.
        assert!(!wie_util::stats::reaver_force_hard());
        wie_util::stats::set_reaver_test(None, None, true);
        assert!(wie_util::stats::reaver_force_hard());
        wie_util::stats::set_reaver_test(None, None, false);
        assert!(!wie_util::stats::reaver_force_hard());
    }

    /// ★★★ 78차-C — the difficulty flag address, derived from the map loader.
    ///
    /// The repo notes said CHAR+0xa318; the literal is added to getBase(), which is CHAR+0x28,
    /// so the byte is at CHAR+0xa340. Getting this wrong reads an unrelated byte and gates the
    /// whole feature on garbage, so the arithmetic is pinned here.
    #[test]
    fn the_hard_flag_is_base_relative_not_char_relative() {
        const CHAR: u32 = 0x0150_3a88;
        assert_eq!(GUEST_BASE, CHAR + 0x28, "getBase() is CHAR+0x28 on this build");
        // The literal lives at pool 0x37a48 and is LOADED at 0x376ca; 0x376dc and 0x379e0 are
        // the two `ldrb` of the byte it points at (stats and drops).
        assert_eq!(HARD_FLAG_OFF, 0xa318, "pool 0x37a48, loaded at 0x376ca");
        assert_eq!(HARD_FLAG_ADDR, 0x0150_ddc8);
        assert_eq!(HARD_FLAG_ADDR - CHAR, 0xa340, "CHAR-relative, for anyone cross-checking the notes");
        assert_ne!(HARD_FLAG_ADDR, CHAR + 0xa318, "the note's address is 0x28 short");
    }

    /// ★★★ 152차-B — who rolls the reaver: in HARD the eight bosses only, at any level; in
    /// HELL every kill on the ladder at or below 81, whatever the species, and nobody above —
    /// the eight bosses fight at 98..100 in hell. An unforked hell record (its hard-table level,
    /// 38..67) and an unread level roll nothing.
    #[test]
    fn the_reaver_is_boss_only_in_hard_and_the_low_band_in_hell() {
        for s in 0u8..=255 {
            assert_eq!(rolls_reaver(false, s, None), is_boss(s), "hard species {s}");
            assert_eq!(rolls_reaver(false, s, Some(100)), is_boss(s), "hard ignores the level ({s})");
        }
        for lv in -1i8..=127 {
            let want = (68..=81).contains(&lv);
            for s in [18u8, 29, 136, 138, 141, 164, 0] {
                assert_eq!(rolls_reaver(true, s, Some(lv)), want, "hell species {s} Lv{lv}");
            }
        }
        // The eight bosses' hell levels (hard 65..67 on the 38..67 -> 68..100 ladder) are 98..100.
        for lv in [98i8, 99, 100] {
            for s in BOSS_SPECIES {
                assert!(!rolls_reaver(true, s, Some(lv)), "boss {s} at hell Lv{lv} is past the band");
            }
        }
        assert!(!rolls_reaver(true, 141, None), "an unread level rolls nothing");
        assert_eq!(REAVER_HELL_LV_MAX, 81);
        assert!(super::super::helldrop::on_ladder(REAVER_HELL_LV_MAX), "the band ends on the ladder");
    }

    #[test]
    fn the_enemy_comes_from_the_frame_not_from_r5() {
        // At the hook both `r0` and `[sp+4]` hold `enemy + 0xbe`.
        assert_eq!(enemy_from_frame(0x1234_00be, 0x1234_00be), Some(0x1234_0000));
        // ⛔ The regression this replaced: `r5` there is the compaction index (0, 1, 2 ...),
        // which never equals the frame slot, so a disagreement must DECLINE.
        assert_eq!(enemy_from_frame(2, 0x1234_00be), None, "a stale index is not an enemy");
        assert_eq!(enemy_from_frame(0x1234_00be, 0), None);
        // Anything that cannot hold the roll-count offset is not a frame we understand.
        assert_eq!(enemy_from_frame(0, 0), None);
        assert_eq!(enemy_from_frame(0xbd, 0xbd), None);
        assert_eq!(enemy_from_frame(0xbe, 0xbe), Some(0), "the boundary is the offset itself");
    }

    #[test]
    fn random_step_matches_the_guest_loop() {
        let (start, end) = (0x1000, 0x1000 + 31 * 4);
        // Mid-array: both cursors advance, the sum is stored, result is the sum >> 1.
        let s = random_step(0x1010, 0x1020, start, end, 7, 9);
        assert_eq!(
            s,
            RandomStep {
                result: 8,
                store_at_rptr: 16,
                fptr: 0x1014,
                rptr: 0x1024
            }
        );
        // Arithmetic shift, then reinterpreted unsigned — exactly `asrs r0,r2,#1`.
        let s = random_step(0x1010, 0x1020, start, end, 0x8000_0000, 0);
        assert_eq!(s.result, 0xc000_0000);
        // rptr wraps: it goes back to start and fptr is stored unchecked.
        let s = random_step(0x1010, end - 4, start, end, 1, 1);
        assert_eq!((s.fptr, s.rptr), (0x1014, start));
        // fptr wraps when rptr does not.
        let s = random_step(end - 4, 0x1010, start, end, 1, 1);
        assert_eq!((s.fptr, s.rptr), (start, 0x1014));
    }
}
