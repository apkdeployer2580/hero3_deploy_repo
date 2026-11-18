//! Field action-quickbar HUD support: locate the live CHAR, read the 5 quickbar
//! slots (skill / item / empty), resolve item stack counts, and gate on the
//! real-time field state. The HUD buttons ACTIVATE a slot by sending its fixed
//! hotkey (see `crate::quickslot_hotkey`) — a normal key event the game handles
//! natively (dispatcher 0x143048), so there is NO memory injection or guest call
//! on the trigger path.
//!
//! All guest-RAM reads go through a caller-supplied `read(addr, len) -> Vec<u8>`
//! (backed by `Emulator::read_memory`), so the parsing is pure + unit-testable.
//!
//! ── RE PROVENANCE (Hero3 client.bin, md5 3792a476) ───────────────────────────
//! CONFIRMED (HIGH) by workflows wf_b06784c0 / wf_9fcbeeaf + live device dumps:
//!   getCHAR (guest 0x14ad10) = *(sl+0x444), sl = 0x1b2c40  → CHAR ptr @ 0x1b3084.
//!   Quickbar (CHAR-relative, stride-1 parallel arrays over 5 slots):
//!     type[s]      u8 @ CHAR+0x252+s   (0=empty, 1=skill, 2=item)
//!     item-ref[s]  u8 @ CHAR+0x257+s   (tab = ref>>5, slot = ref&0x1f)
//!     skill-ref[s] u8 @ CHAR+0x25c+s   (skill id = ref&0x1f)
//!     current      u8 @ CHAR+0x261
//!   Slot->hotkey (static, from dispatcher 0x143048 jump table): 0=1,1=3,2=9,3=0,4=#.
//!   Item stack count lives in the inventory grid (double-indirect WIPI handle):
//!     base = *(*(CHAR+0x9e28)) + 8 ; record = base + tab*0x3c4 + slot*0x3c ;
//!     count u16 @ record+0x3ba.  (Live-resolve every read; the base moves.)
//!   State = *(u32)(CHAR+0x7c); field free-roam == 4; selector overlay == 5.
//!   Field gate = (state==4) && (*(u8)(CHAR+0x9c6c)==0).

use crate::{IconRef, QuickSlot, SlotKind, Stats};

// Guest address model: guest = file + 0x100000; static base sl = 0x1b2c40.

/// Pointer global read by getCHAR (guest 0x14ad10 = *(sl+0x444)); sl+0x444 = 0x1b3084.
/// Loose plausible-pointer window (reject null/tiny). TWO live regions: binary/
/// static ~0x10_0000.. and dynamic heap 0x4000_0000+ (grid handles / GameSave).
const HEAP_LO: u32 = 0x10_0000;
const HEAP_HI: u32 = 0x8000_0000;

/// Quickbar arrays, relative to CHAR (stride 1, 5 slots). [CONFIRMED]
const QB_SLOTS: u32 = 5;
const QB_TYPE_OFF: u32 = 0x252;
const QB_ITEMREF_OFF: u32 = 0x257;
const QB_SKILLREF_OFF: u32 = 0x25c;
const TY_SKILL: u8 = 1;
const TY_ITEM: u8 = 2;
/// Consumable item category (i18_dat); its icon frame comes from the item-DB.
const CONSUMABLE_CAT: u16 = 0x12;

/// Inventory grid — used only to resolve an item slot's count. Reached through
/// [`deref_container`], which is where the per-build hop count lives.
const GRID_HANDLE_OFF: u32 = 0x9e28;
const TAB_STRIDE: u32 = 0x3c4;
const SLOT_STRIDE: u32 = 0x3c;
const COUNT_OFF: u32 = 0x3ba;

// Game-state gate. A COARSE app state — sole writer `setGameState` (KTF guest
// 0x148bf8): 2 = boot, 3 = title/out-of-world shell, 4 = real-time field free-roam,
// 5 = quick-item selector overlay. ★ It does NOT change when one of the game's
// windows (menu / map / save / inventory) opens, so it is not enough on its own to
// tell "is a window covering the screen?" — see [`WINDOW_ID_OFF`].
//
// The OFFSET is the one field this crate reads that the two carrier builds disagree
// about (KTF `CHAR+0x7c`, LGT `CHAR+0x78`), so it lives in
// `guest_build::state_off()` rather than in a constant here. The VALUES below are
// the same on both — device-measured on KTF, probe-measured on LGT.
const STATE_FIELD: u32 = 4;
/// The menu / quick-item selector overlay. ★ MEASURED on a device 2026-08-06, and it
/// settles a question two earlier sessions could only guess at: opening the menu moves
/// `CHAR+0x7c` from 4 to **5** while the window id stays 3 and the selector flag goes
/// to 1 (134 probe samples: `window=3 state=5 selector=1 types=[1,1,1,1,2]`).
/// A character is just as live here as in the field, which is why [`in_game_state`]
/// accepts it — see the bug that cost.
const STATE_SELECTOR: u32 = 5;
const SELECTOR_FLAG_OFF: u32 = 0x9c6c;

/// ★ The window manager's "current window id" (int8 at `CHAR+0xa0c0`, i.e. the
/// window object `CHAR+0x9e30` + 0x290). SOLE writer = `openWindow` (guest
/// 0x12c6a4, store at 0x12ca7c); three parallel 16-entry KEY / PAINT / ENTER jump
/// tables are all indexed by `id - 3`:
///   3 = NO window (gameplay / field)   4 = script / cutscene playback
///   7, 17, 18 = confirm windows        8 = MAP window
///   14 = save window                   15, 16 = inventory / equipment grid
/// This is the discriminator the overlay needs: `id != 3` means one of the game's
/// own windows owns the screen, so the field HUD must stand down and the buttons
/// must become menu-navigation keys. [RE: workflow wf_6d270fb0 Q3]
const WINDOW_ID_OFF: u32 = 0xa0c0;
/// 117차-D — the SHOP window's own SCREEN STATE: `base+0xac68` with `base = getBase() =
/// CHAR+0x28` (`0x1068`), i.e. `CHAR+0xac90`, a signed byte the shop's KEY handler
/// (`0x1e060`, window 12's KEY table entry) switches on 13 ways (`cmp #0xc; bls` → table
/// `0x5cb68`; its OK arm has a second 13-way table at `0x5cb9c`) and its PAINT routine
/// (`0x1f78c`) compares against. The OK arms pin the values that matter here:
///
///   * state 0 = the shop's own menu (구매 / 판매, widget mode 7): OK with the cursor off
///     row 0 zeroes the inventory TAB byte (`base+0x9c49` = `INV_TAB_OFF`) and the cell
///     cursor (`base+0x9c48 + tab + 2` = `INV_CURSOR_BASE + tab`) and stores **8**;
///   * state 1 = the category list (방어구 / 무기 / 기타, widget mode 8): OK stores the
///     row into `base+0xac8c`, builds the stock (`0x1dd84(category)`), zeroes the same two
///     cursor bytes and stores **2**;
///   * state **2** = the BUY grid — its OK arm is where 105차's gold SUBTRACT `0x1e538`
///     lives (→ 3 = 구매 확인, widget 4; → 4 = a count for `cat 0x12`);
///   * state **8** = the SELL grid — its OK arm is where 105차's price-0 refusal `0x1e364`
///     and gold ADD `0x1e466` live (→ 0xa = 판매 확인; → 0xc = a message).
///
/// ⛔⛔ THIS is the fact 105차 / 109차 / 117차-B were missing. 구매 and 판매 are the same
/// window id, share the cell cursor, and — after a category is chosen — the buy screen is
/// a GRID too, with the same yellow cell cursor drawn, so neither "the window is 12", nor
/// "a cell cursor is on screen" (109차's key gate), nor "the tap landed on a grid cell"
/// (117차-B's tap gate) tells the two apart. Only this byte does. Player report that
/// exposed it (2026-09-20, vc366): *"아이템 구매 -> 방어구 -> 1,2,3번째 칸 선택하면 3종가방
/// 나오고 무기 … 기타 … 도"*.
pub const SHOP_STATE_OFF: u32 = 0xac90;
/// [`SHOP_STATE_OFF`] while the shop's BUY grid is on screen — documentary: the gate only
/// ever asks for [`SHOP_STATE_SELL_GRID`], everything else (this included) is the game's.
#[allow(dead_code)]
pub const SHOP_STATE_BUY_GRID: u8 = 2;
/// [`SHOP_STATE_OFF`] while the shop's SELL grid (the player's bag) is on screen — the one
/// shop screen where a pinned bag-item cell may open its box.
pub const SHOP_STATE_SELL_GRID: u8 = 8;

/// The shop window's screen state ([`SHOP_STATE_OFF`]), or `None` while CHAR is unreadable.
/// Meaningful only while `window_id() == bagbox::SHOP_WINDOW`.
pub fn shop_state(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u8> {
    let c = resolve_char(read)?;
    rd_u8(read, c + SHOP_STATE_OFF)
}

/// The SMITH window's input (125차): the same byte as [`shop_state`], read the frame window
/// 13 opens. Window 13's ENTER (`0x3f6f6`) hands `(s8)CHAR+0xac90` to the craft-screen
/// installer `0x1feb8(mode)`, and `0x1fdc0` copies `smith_dat` rows `(mode-1)*5 .. +5` into
/// the 일반조합 list — so it is WHICH SMITH the player is talking to (1-based; 0 reads the
/// same rows as 1). The NPC script leaves it there before `openWindow(13)`, so it is
/// stable by the time the host sees the window. ⛔ Only meaningful on window 13:
/// on window 12 the same byte is the shop's state machine (117차-D).
/// ★ 125차-C: the host no longer reads this for a forge plan; the smith counts the box.
#[allow(dead_code)]
pub fn smith_id(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u8> {
    shop_state(read)
}
/// No window open — the field has the screen.
pub const WIN_NONE: u8 = 3;
/// SCRIPT / cutscene playback: story scenes and NPC dialogue. Distinct from every
/// other window id, which is what makes "is this a cutscene or a menu?" answerable
/// at all (player question 2026-08-03) — the overlay strips the control set down to
/// OK + `#` here instead of showing menu-navigation keys that do nothing.
pub const WIN_SCRIPT: u8 = 4;
/// The game's own map window (what `*` toggles, and what paints the minimap
/// buffer we mirror). Its key handler closes on ANY key (guest 0x124360).
pub const WIN_MAP: u8 = 8;
// ⛔ 135차-B: `WIN_ENDING` (18) is gone. "The ending window was up and now it is not" was
// the new-game-plus trigger, polled every five worker iterations — and the window lives
// only until the next key, so a mashed ending slipped between two polls (135차 §2 ②). The
// transition itself (`0x3af2c`, the window's KEY handler's first call) is hooked now:
// `wie_core_arm::binary_patches::ending` → `wie_util::stats::ending_transition`.
/// Script-window CHOICE-LIST flag — see [`script_choice_open`], which is also where
/// the device trace that found it is written down.
const SCRIPT_CHOICE_OFF: u32 = 0xa0dc;

// ── Player gauges ────────────────────────────────────────────────────────────
//
// ★ CORRECTED 2026-07-30 (workflow wf_6d270fb0 Q1, CONFIRMED HIGH by an
// independent verifier + this session's own re-disassembly): the previous mapping
// had **HP and SP SWAPPED**, which is exactly the player-reported bug "the HP bar
// does not refill when I drink a potion" — the red bar was tracking SP, and the
// blue bar was tracking HP.
//
// Everything lives in a stats sub-struct `H = CHAR + 0xb4`, so the same fields
// appear in the game's code as `H + (off - 0xb4)`:
//   AddHP(H, delta)  @ guest 0x1467a8:  cur = (s16)H[0x3e] (= CHAR+0xf2); cur += delta;
//                                       clamp to (s32)H[0x38] (= CHAR+0xec) and to 0.
//   AddSP(H, delta)  @ guest 0x1467d0:  cur = (s32)H[0x44] (= CHAR+0xf8); clamp 0..1000.
//   ComputeMaxHP(H)  @ guest 0x145f4c:  H[0x38] = (36*level + 10*CHAR[0xd9])
//                                                 * (100 + (s16)CHAR[0xf0]) / 100
//                                       — the SOLE writer of max HP.
//   Level-up @ 0x146c28: level++ (cap 70), ComputeMaxHP, AddHP(+maxHP), AddSP(+1000).
//   Full restore (revive / new game) @ 0x124458 / 0x1540cc / 0x1570c4:
//                                       HP = *(CHAR+0xec); SP = 1000.
// The game's own field bars corroborate the split: HP bar (red) = len * CHAR[0xf2]
// / CHAR[0xec] (proportional), SP bar (blue) = len * (CHAR[0xf8]/10) / 100 (i.e.
// /1000, which is where the old "max HP = hardcoded 1000" reading came from — it
// is SP's max, not HP's).
const HP_OFF: u32 = 0xf2; // s16 current HP
const HP_MAX_OFF: u32 = 0xec; // s32 max HP (ComputeMaxHP writes it)
const SP_OFF: u32 = 0xf8; // s32 current SP, range 0..1000
/// Max SP really IS a hardcoded constant — the AddSP clamp at guest 0x1467d0.
pub const SP_MAX: u16 = 1000;
const EXP_OFF: u32 = 0xe4; // u32 current EXP (within level)
const EXP_NEXT_OFF: u32 = 0xe8; // u32 EXP-to-next
const LEVEL_OFF: u32 = 0xe2; // u16 level
/// 소지금, u32 — `GBASE + 0xa4`, and GBASE is `CHAR + 0x28` (`getBase` 0x1068), so
/// `CHAR + 0xcc`.
///
/// ★★★ 105차 — CONFIRMED BY THE SHOP ITSELF, in both directions, which is what makes it
/// safe to write. The sell arm credits it and the buy arm debits it, eight bytes apart in
/// the same function:
///
/// ```text
///   0x1e466  adds r4, #0xa4      ; r4 = base + 0xa4
///   0x1e46a  ldr  r3, [r4]       ; gold
///   0x1e46e  adds r3, r3, r2     ; gold + 판매가          <- 판매
///
///   0x1e538  adds r5, #0xa4
///   0x1e548  muls r2, r3, r2     ; 가격 * 수량
///   0x1e54c  subs r3, r3, r2     ; gold - 비용            <- 구매
/// ```
///
/// `ldr` / `str`, so it is 32 bits wide, not the 16 the HP fields are.
pub const GOLD_OFF: u32 = 0xcc;
/// Level cap. The game's own `doLevelUp` clamps at `#0x46` = 70 (KTF guest
/// 0x146c3a/0x146c44; LGT 0x4d690 `0x4d6a8`/`0x4d6ac`) — and **111차 (2026-09-19) raised
/// the LGT clamp to 99** with two imm8 patches in `binary_patches.toml` (player decision:
/// the original title reached 71..99 only through the carrier's network PvP, which no
/// longer exists, so single play carries it now). This is a CEILING on garbage for the
/// liveness tests below; it must be >= the guest's clamp on every shipped build (KTF is
/// not shipped and its 70 is inside 99).
/// ★ 114차 (2026-09-19): the VANILLA flavour does not install those two patches
/// (`flavor = "main"` in the toml), so its guest clamps at 70 while this ceiling stays
/// 99 — deliberately LOOSER, never tighter: a Lv 71+ character made on main must still
/// resolve when its save is opened on vanilla (it just cannot level further there).
/// See `flavor.rs`.
pub const LEVEL_CAP: u16 = 99;
/// Sanity ceiling for the GAME's max-HP field (`CHAR+0xec`): since 142차 `hp_max_lgt`
/// stores `min(true max, 32767)` there, so anything above the i16 ceiling means we are not
/// looking at a character. (It was 30000 — which the 142차 HP% stacking can pass, and a
/// real Lv99 would then have read as "no character" everywhere.)
pub const HP_CAP: u32 = 32767;
/// Sanity ceiling for the TRUE max HP the hooks keep (`wie_util::stats::PLAYER_HP_MAX`) —
/// a garbage bound only, the same number as the damage cap.
pub const HP_TRUE_CAP: u32 = 9_999_999;

// ── unspent stat / skill points (the red + / blue + badges) ──────────────────
//
// The game's OWN bottom-HUD renderer draws these two badges at the end of its
// block (guest 0x14a2d2..0x14a3ae) — which is inside the range our HUD-skip patch
// branches over (see `hud_bottom_patch`), so they vanished when we took that block
// away. The overlay redraws them; these are the two counters it reads.
//
//   addStatPoints  (guest 0x146834): *(u16*)(CHAR+0x18a) += n, clamped to 276, >=0
//   addSkillPoints (guest 0x146868): *(u8*) (CHAR+0x18c) += n, clamped to 44
//
// Both caps corroborate the offsets arithmetically, which is why they are pinned
// here rather than left as magic numbers:
//   * level-up calls addStatPoints(4) EVERY level -> 69 level-ups * 4 = 276 ✓
//   * level-up calls addSkillPoints(1) every level to 19, then only on EVEN levels
//     (guest 0x146c88 skips when `level > 19 && (level & 1)`) -> 18 + 26 = 44 ✓
// A reading above either cap therefore means the bytes are not (yet) a character,
// so `plausible_stats` rejects it and the HUD keeps its last good copy.
// ★ 111차 (2026-09-19): the LGT clamps moved WITH the level cap (`binary_patches.toml`,
// LGT `addStatPoints` 0x4d610 / `addSkillPoints` 0x4d648): 98 level-ups x 4 = 392, and
// 19 (levels 2..20) + 39 (even 22..98) = 58. The ceilings below must agree with the
// guest or a hoarder past 70 reads as "not a character".
// ★ 114차: on VANILLA those clamp patches are not installed (guest 276 / 44 again) while
// the ceilings stay 392 / 58 — looser is fine, it is only a tighter ceiling that rejects a
// character (`LEVEL_CAP` has the same note).
const STAT_POINTS_OFF: u32 = 0x18a; // u16, read as s16 by the game (`ldrsh`)
const SKILL_POINTS_OFF: u32 = 0x18c; // u8 (`ldrb`)
/// `addStatPoints` clamp — KTF guest 0x146834 `> 0x8a<<1` -> 276; LGT 0x4d628 patched to
/// `0xc4<<1` -> 392 (111차, level cap 99).
pub const STAT_POINT_CAP: u16 = 392;
/// `addSkillPoints` clamp — KTF guest 0x146868 `> 0x2c` -> 44; LGT 0x4d664/0x4d668
/// patched to `0x3a` -> 58 (111차, level cap 99).
pub const SKILL_POINT_CAP: u8 = 58;

/// Worn-gear skill-cooldown reduction, percent.
///
/// `addStat(0x11)` (LGT `0x4ee34` arm `0x4eeb4`) writes the u16 at
/// H+0x70 = CHAR+0x124. `0x4f014` zeroes the derived-stat block (this word
/// included) then re-adds every equipped piece — 폭풍의 orbs and any item
/// whose statId is 0x11. ⛔ Do not call `0x4f014` from the host.
/// ⛔ 156차-B: on this LGT build the cast path DOES read it — `0x4fdb8` → `0x4bb82` sets the
/// skill's cooldown to `desc × (100 − this) / 100` (signed; a total of 100 % or more is a
/// one-tick cooldown, `ITEM_BALANCE_AUDIT_156.md §4`). The old KTF note here said combat
/// never reads it; the host overlay reads it too.
const CD_REDUCE_OFF: u32 = 0x124;

/// Skill icon sheet id for the active member (byte, value 4..10 = s4..s10_bm);
/// atlas sheet index = value - 4. [CONFIRMED wf_9add52f0, helper 0x19c280.]
const SKILL_SHEET_OFF: u32 = 0xd1;

// ── skill icon = (per-slot party member's sheet, strip cell) ──────────────────
// Mirrors the game's own HUD draw, guest 0x14a51a (static disasm 2026-07-28) —
// see the CHAR+0x9bf8 note below. Per skill slot `s`:
//   onfield = ((skill-ref[s] as i8) >> 6) != 0    (0 = first on-field hero, else
//             the second; the game only ever loads TWO skill sheets on the field
//             and picks between them with this 2-bit test — it does NOT index by
//             the raw member 0..3)
//   sheetId = u8 @ CHAR+0xd1 + onfield (0 -> 7 = s7, 1 -> 8 = s8, …)
//   atlas   = sheetId - 4              (s4..s10 -> 0..6)
//   cell    = int8 @ CHAR+0x26c+s      (0..8, the base-skill strip column stored
//             directly by the bind at guest 0x145cf0; <0 = empty), blitted as
//             srcX = cell*15, srcY = 0, 15x15.
// Ground truth this save: slot0 ref 0x6a idx 5 -> hero1/s8/cell5 = 일심;
// slot1 ref 0x2d idx 5 -> hero0/s7/cell5 = 장탄.
// (The earlier sk+idx*0x38+0x173 pointer-chain was the skill-PICKER draw, not the
// field HUD; it read the wrong byte and collapsed both slots to cell 0 = 사격.)
//
// ── where the game's HUD icons come from (guest 0x140448, RE 2026-07-28) ──────
// The HUD skill branch samples the image array `*(*(CHAR+0x9bf8))+8`, a handle
// distinct from the class-sheet manager at CHAR+0x9bf0 — which is why an earlier
// pass concluded "the HUD draws some other image". It does not: the setter at
// guest 0x140448 allocates a second 2-slot manager and fills it from the name
// template `*(sl+0x328)` = "/skill/s4_bm" with the digit patched to the sheet id
// `CHAR+0xd1+i`, i = 0,1. Same s{N}_bm family the atlas is baked from, so the
// (sheet, cell, 15x15) triple above is exactly what the game blits.
const QB_SKILLIDX_OFF: u32 = 0x26c; // int8 strip cell 0..8 per slot
const SKILL_STRIP_CELLS: i8 = 9; // s{N}_bm = 9 cells (0..8)
/// Item-DB access for the consumable icon frame (CONFIRMED wf_9add52f0, icon-draw
/// 0x1533a8): itemDB = *(*(grid_base+0x38))+8; rec = itemDB + idx*0x18; frame u8
/// @rec+0x14 (raw 0-based item_bm frame). Consumables = category 0x12.
const ITEMDB_HANDLE_OFF: u32 = 0x38;
const ITEMDB_STRIDE: u32 = 0x18;
const ITEMDB_ICON_OFF: u32 = 0x14;
const ITEM_ICON_FALLBACK: u16 = 0x11; // generic item frame if DB lookup fails

// ── low-level guest reads ────────────────────────────────────────────────────

fn rd_u16(read: &impl Fn(u32, usize) -> Vec<u8>, addr: u32) -> Option<u16> {
    let b = read(addr, 2);
    (b.len() == 2).then(|| u16::from_le_bytes([b[0], b[1]]))
}

fn rd_u32(read: &impl Fn(u32, usize) -> Vec<u8>, addr: u32) -> Option<u32> {
    let b = read(addr, 4);
    (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn rd_u8(read: &impl Fn(u32, usize) -> Vec<u8>, addr: u32) -> Option<u8> {
    let b = read(addr, 1);
    (b.len() == 1).then(|| b[0])
}

fn rd_i8(read: &impl Fn(u32, usize) -> Vec<u8>, addr: u32) -> Option<i8> {
    let b = read(addr, 1);
    (b.len() == 1).then(|| b[0] as i8)
}

fn in_heap(p: u32) -> bool {
    (HEAP_LO..HEAP_HI).contains(&p)
}

// ── CHAR + grid-base resolution ──────────────────────────────────────────────

/// Follow a field that holds a WIPI allocation to its payload, validating every hop.
///
/// ★ The number of hops is a CARRIER property, not a game one (2026-08-10): KTF's
/// runtime hands the game HANDLES, so the payload is `*(handle) + 8`, while LGT's
/// `data_ptr` is the identity and the field is already the payload address. Before
/// this existed, every container here double-dereferenced unconditionally, so on an
/// LGT boot the inventory grid / area object / item DB resolved to the first word of
/// the real payload and read neighbouring bytes as records — a silently wrong
/// quickbar rather than an absent one. See `guest_build::container_data_off`.
fn deref_container(read: &impl Fn(u32, usize) -> Vec<u8>, field: u32) -> Option<u32> {
    let first = rd_u32(read, field)?;
    if !in_heap(first) {
        return None;
    }
    let Some(off) = crate::guest_build::container_data_off() else {
        return Some(first);
    };
    let block = rd_u32(read, first)?;
    if !in_heap(block) {
        return None;
    }
    let base = block.checked_add(off)?;
    in_heap(base).then_some(base)
}

/// "Are these three numbers a live character record?" — the whole liveness rule, as a
/// pure function of the bytes, so a test can reach it (the LGT static arm of
/// [`resolve_char`] otherwise needs a whole fake address space).
///
/// **`level` is the only witness.** A record that has not been created yet reads level 0
/// — that is what the title screen shows — and `LEVEL_CAP` is the game's own clamp
/// (`doLevelUp 0x4d690`, `cmp #0x46` — 99 since the 111차 patch). The other two are CEILINGS on garbage and nothing
/// more; see [`plausible_stats`] and 80차-E for the two separate reports that came from
/// asking them to witness liveness instead.
pub fn char_live(level: u16, hp_max: u32, sp: u32) -> bool {
    (1..=LEVEL_CAP).contains(&level) && hp_max <= HP_CAP && sp <= u32::from(SP_MAX)
}

/// Resolve the live CHAR pointer (pointer global first, validated in-heap; else
/// the observed static fallback). `None` if neither looks sane.
pub fn resolve_char(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u32> {
    match crate::guest_build::char_anchor()? {
        crate::guest_build::CharAnchor::Pointer { global, fallback } => {
            if let Some(p) = rd_u32(read, global)
                && in_heap(p)
            {
                return Some(p);
            }
            let probe = read(fallback, 4);
            (probe.len() == 4).then_some(fallback)
        }
        // A static always reads back, so unlike the pointer build there is no address
        // question to answer — only "has a character been created yet". Before the
        // class-select screen commits one the record is still zeroed, and a zero level
        // fails this. Cheap: four reads, and it is the same plausibility test
        // `read_stats` applies to every gauge it publishes.
        crate::guest_build::CharAnchor::Static(base) => {
            let level = rd_u16(read, base + LEVEL_OFF)?;
            let hp_max = rd_u32(read, base + HP_MAX_OFF)?;
            let sp = rd_u32(read, base + SP_OFF)?;
            // ⛔⛔ `hp <= hp_max` WAS A CONDITION HERE AND IS GONE (71차-L, TODO `AP`).
            //
            // The game produces `hp > hp_max` legitimately, and the most ordinary way to
            // do it is to UNEQUIP: equipment carries max-HP, so taking a piece off drops
            // `hp_max` below the current `hp` until something clamps it. Treating that as
            // "there is no character here" made the host go blind at exactly the wrong
            // moment, and it cost two separate reports:
            //   * 해제하면 HUD·퀵슬롯이 사라진다 (the plates and the minimap are gated on
            //     this), and
            //   * the 오브결합 refresh sweep found ZERO equipment cells and burned both of
            //     its passes on an empty bag — and the combine feature REQUIRES the item to
            //     be unequipped, so this fired on the one path that needs it most.
            // ⛔ Do not add it back "to be safe": a gauge that reads high is a gauge to
            // CLAMP when drawing, not evidence that the character does not exist.
            //
            // ⛔⛔ …AND FOR THE SAME REASON `hp_max` IS A CEILING, NOT A FLOOR (80차-E).
            // `CHAR+0xec` is a DERIVED field — `ComputeMaxHP` is its sole writer, and that
            // runs at world init, level-up and equip. The **network warehouse never runs
            // world init**: the online slot list deserialises the record (71차-L) and goes
            // straight to app state 3, so max HP is still 0 while everything else in the
            // record is live. Device measurement (vc250, S9, 싱글가방 with items on screen):
            //
            //     char stats at 0x1503a88: level=Some(3) hp_max=Some(0) sp=Some(1000) -> REFUSED
            //
            // Requiring it non-zero therefore called a record the GAME WAS DRAWING THE BAG
            // OUT OF "no character", and that took the whole memory-resolving half of
            // `menu_tap_decide` with it — the player's report was *"네트워크 창고 접속했을때
            // 가방칸 내부에서 터치로 템 포커스 이동이 안되고있어"* (2026-09-05).
            // `level in 1..=LEVEL_CAP` is what actually answers "has a character been
            // created" — a zeroed record reads level 0, which is exactly what the title
            // screen shows. The other two stay as ceilings, which is all they can honestly
            // be: they reject garbage, they cannot witness liveness.
            char_live(level, hp_max, sp).then_some(base)
        }
    }
}

/// The CHAR record's address WITHOUT the liveness gate — for fields that are valid
/// before a character exists (the app-state word the shell test reads). Pointer
/// builds still have to chase/validate the pointer; a static anchor simply IS the
/// address. Everything that reads character data must keep using [`resolve_char`].
fn char_base_raw(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u32> {
    match crate::guest_build::char_anchor()? {
        crate::guest_build::CharAnchor::Pointer { global, fallback } => {
            if let Some(p) = rd_u32(read, global)
                && in_heap(p)
            {
                return Some(p);
            }
            let probe = read(fallback, 4);
            (probe.len() == 4).then_some(fallback)
        }
        crate::guest_build::CharAnchor::Static(base) => Some(base),
    }
}

/// Resolve the inventory grid base — `*(*(CHAR+0x9e28)) + 8` on KTF, one hop fewer
/// on LGT (see [`deref_container`]).
fn resolve_grid_base(read: &impl Fn(u32, usize) -> Vec<u8>, char_ptr: u32) -> Option<u32> {
    deref_container(read, char_ptr + GRID_HANDLE_OFF)
}

/// Resolve the current AREA object at `CHAR+0x9c90`. Same container shape as the
/// inventory grid — see [`crate::save::AREA_HANDLE_OFF`] for why the guest's own
/// cached copy must not be used instead.
fn resolve_area_base(read: &impl Fn(u32, usize) -> Vec<u8>, char_ptr: u32) -> Option<u32> {
    deref_container(read, char_ptr + crate::save::AREA_HANDLE_OFF)
}

/// The game's own "may I save in this area?" byte, or `None` while it cannot be read
/// (boot, teardown, the moment of a map change).
///
/// `None` is deliberately distinct from `Some(false)`: an unreadable chain is "ask me
/// again next frame", while `Some(false)` is the game saying no. Callers that must
/// answer anyway fail OPEN — the alternative is what 27차 shipped, where one
/// unresolvable read turned into a dead autosave and two vanished controls.
pub fn area_can_save(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<bool> {
    let c = resolve_char(read)?;
    let base = resolve_area_base(read, c)?;
    Some(rd_i8(read, base + crate::save::AREA_CAN_SAVE_OFF)? != 0)
}

// ── 하늘성 / THE TOWER: reads and write PLANS ─────────────────────────────────
//
// The floor state machine lives in `crate::guardian` and is pure. This is the half
// that touches the guest: it reads what `guardian::Obs` needs, and it turns a
// `guardian::Fork` or a warp into (address, bytes) pairs for the worker to write.
//
// Every offset comes from `guest_anchors::SceneWorld` / `CreatureRecord`, so a build
// where that table is absent (KTF, SKT) gets `None` from all of it and the tower is
// off — not "off silently with plausible offsets", which is the failure this split
// exists to prevent.

/// CHAR-relative difficulty byte — the tower's ENDING GATE.
///
/// ⛔⛔ `0xa318` IS THE BASE-RELATIVE LITERAL, NOT THIS. `getBase()` returns
/// `CHAR + 0x28`, so the CHAR-relative offset is `0xa340` (78차-C lost a session to
/// exactly this arithmetic, and `reaverdrop.rs` carries the same warning).
pub const DIFFICULTY_OFF: u32 = 0xa340;

/// `0` = normal, anything else = hard, i.e. this character has finished the game.
/// `None` = unreadable, which `guardian::entry_allowed` turns into a refusal.
///
/// Read RAW (`char_base_raw`), like [`shell_state`]: the byte is a property of the
/// loaded save, and demanding a plausible character first is how 80차-E froze the
/// gauges — the network-warehouse screens legitimately fail `resolve_char`.
pub fn difficulty_byte(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u8> {
    let c = char_base_raw(read)?;
    rd_u8(read, c + DIFFICULTY_OFF)
}

/// 헬 승격 (133차) — the one write that makes the next playthrough hell: difficulty
/// `<- 2`, planned right after the game's own ENDING transition wrote 1. Read-verify
/// like every other guardian write: `None` = already hell, or unreadable (skip).
///
/// The byte round-trips the save VERBATIM (`serialiseSave` 0x1aed4 `ldrb`/`strb`,
/// `deserialiseSave` 0x1a9d0/0x1a9d4), and its three writers (0 char reset · 1 the
/// ending transition · the deserialise copy) never normalise it — so 2 persists once
/// the player saves, and reloading a pre-clear save rolls it back, same loan rule as
/// the bag transition (124차-B).
pub fn hell_difficulty_write(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u32, u8)> {
    let c = char_base_raw(read)?;
    let addr = c + DIFFICULTY_OFF;
    (rd_u8(read, addr)? != crate::guardian::HELL_DIFFICULTY).then_some((addr, crate::guardian::HELL_DIFFICULTY))
}

/// The scene the world is on (`SceneWorld::scene_id_off`).
pub fn scene_now(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u16> {
    let sw = crate::guest_build::scene_world()?;
    let c = char_base_raw(read)?;
    rd_u16(read, c + sw.scene_id_off)
}

/// How many creature records the map loader actually filled — the length of the live
/// array, from the MAP object rather than from a guess.
fn spawned_count(read: &impl Fn(u32, usize) -> Vec<u8>, char_ptr: u32, sw: &crate::guest_build::SceneWorld) -> Option<i32> {
    let map = resolve_area_base(read, char_ptr)?;
    Some(rd_u32(read, map + sw.map_spawn_count_off)? as i32)
}

/// A snapshot of the live monster records, in the shape the state machine wants.
///
/// ⛔⛔ THE ARRAY POINTER IS RE-READ EVERY TIME, and it has to be: a scene load
/// REALLOCATES it (measured across one round trip, `0x4014ceac` -> `0x40146884`), so a
/// cached base is a wild write into whatever took its place.
/// ⛔ The length comes from `MAP+0x34`. Reading past it walks off the array into the
/// heap and would invent monsters.
pub fn guardian_creatures(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<Vec<crate::guardian::Creature>> {
    let sw = crate::guest_build::scene_world()?;
    let c = char_base_raw(read)?;
    let arr = rd_u32(read, c + sw.creatures_off)?;
    let n = spawned_count(read, c, &sw)?;
    if arr == 0 || n <= 0 {
        return Some(vec![]);
    }
    let r = sw.rec;
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let rec = arr.wrapping_add(i as u32 * sw.creature_stride);
        // A record that will not read is skipped rather than defaulted: a zero would
        // read as a dead monster and could clear the floor.
        // ★ The EFFECTIVE fields, not the bases (88차): a `bossh_dat` record spawns with
        // zero in `+0x82`/`+0x68`/`+0x6a`/`+0x8c`, and `guardian::FloorMemory` takes
        // its base from the first reading of these instead.
        let (Some(hp_max), Some(hp_cur), Some(atk), Some(dp), Some(ds)) = (
            rd_u16(read, rec + r.hp_max),
            rd_u16(read, rec + r.hp_cur),
            rd_u16(read, rec + r.atk_eff),
            rd_u16(read, rec + r.def_phys_eff),
            rd_u16(read, rec + r.def_spec_eff),
        ) else {
            continue;
        };
        // ⛔ An unreadable respawn timer must read as DISARMED, not as 0: zero is
        // "a respawn is pending and has just started", and defaulting to it would make
        // the host write to a record it could not read.
        let respawn_timer = rd_u16(read, rec + r.respawn_timer).unwrap_or(crate::guardian::RESPAWN_DISARMED);
        // The species, through the same chain the reaver hook validates
        // (`*(rec+actor)` -> object, `+0x1c` -> map record, `+0x0e` -> species).
        let species = rd_u32(read, rec + r.actor)
            .filter(|o| *o != 0)
            .and_then(|o| rd_u32(read, o + 0x1c))
            .filter(|m| *m != 0)
            .and_then(|m| rd_u8(read, m + 0x0e));
        // The virtual HP word the `dmg_apply_lgt` hook drains. An unreadable word
        // reads as "already seeded" so the host never seeds blind.
        let vhp = rd_u32(read, rec + r.vhp).unwrap_or(u32::MAX);
        // The level byte (112차): its first sighting on a floor is the SPAWN level the
        // stat growth is measured from. Unreadable = `None` = that record grows nothing.
        let level = rd_u8(read, rec + r.level).map(|b| b as i8);
        out.push(crate::guardian::Creature {
            index: i,
            hp_max,
            respawn_timer,
            vhp,
            level,
            // Signed, like every other HP read here: the writer clamps at 0 but the
            // field is `i16` and a negative would read as a huge positive.
            hp_cur: (hp_cur as i16).max(0) as u16,
            atk_eff: atk,
            // 160차 T4: a boss's two skill attacks. Unreadable = 0 = "no such attack" (the
            // fork then leaves the field alone), and only a boss's are ever written.
            atk_skill: [rd_u16(read, rec + r.atk_skill1).unwrap_or(0), rd_u16(read, rec + r.atk_skill2).unwrap_or(0)],
            def_phys_eff: dp,
            def_spec_eff: ds,
            species,
        });
    }
    Some(out)
}

/// Cancel the pending respawns named by [`crate::guardian::respawn_disarms`].
///
/// One `u16` write per corpse, absolute value, so re-asserting is idempotent. Returns
/// an empty vec when nothing is pending, which is the normal state while fighting.
pub fn guardian_respawn_writes(read: &impl Fn(u32, usize) -> Vec<u8>, indices: &[i32]) -> Option<Vec<(u32, Vec<u8>)>> {
    if indices.is_empty() {
        return Some(vec![]);
    }
    let sw = crate::guest_build::scene_world()?;
    let c = char_base_raw(read)?;
    let arr = rd_u32(read, c + sw.creatures_off)?;
    if arr == 0 {
        return None;
    }
    let off = sw.rec.respawn_timer;
    Some(
        indices
            .iter()
            .map(|i| {
                let rec = arr.wrapping_add(*i as u32 * sw.creature_stride);
                (rec + off, crate::guardian::RESPAWN_DISARMED.to_le_bytes().to_vec())
            })
            .collect(),
    )
}

/// Park a dead boss's map object off the map — see [`crate::guardian::corpse_hides`].
///
/// Read-compare-write on the X half, so a corpse already parked costs nothing. The
/// object is reached through the record's actor pointer exactly as the species is;
/// a chain that will not read is skipped, never guessed.
pub fn guardian_corpse_hide_writes(read: &impl Fn(u32, usize) -> Vec<u8>, indices: &[i32]) -> Option<Vec<(u32, Vec<u8>)>> {
    if indices.is_empty() {
        return Some(vec![]);
    }
    let sw = crate::guest_build::scene_world()?;
    let c = char_base_raw(read)?;
    let arr = rd_u32(read, c + sw.creatures_off)?;
    if arr == 0 {
        return None;
    }
    let hidden = crate::guardian::CORPSE_HIDE_POS as u16;
    let mut out = Vec::new();
    for i in indices {
        let rec = arr.wrapping_add(*i as u32 * sw.creature_stride);
        let Some(obj) = rd_u32(read, rec + sw.rec.actor)
            .filter(|a| *a != 0)
            .and_then(|a| rd_u32(read, a + 0x1c))
            .filter(|o| *o != 0)
        else {
            continue;
        };
        if rd_u16(read, obj + 0x10) != Some(hidden) {
            out.push((obj + 0x10, hidden.to_le_bytes().to_vec()));
            out.push((obj + 0x12, hidden.to_le_bytes().to_vec()));
        }
    }
    Some(out)
}

/// ⛔ TEST LEVER ONLY — the `clear` line of `wie_guardian.txt` (143차): write 0 to the named
/// records' current HP, which the game then runs its own death for.
///
/// This used to "park" a floor's surplus monsters on every floor, and that WAS the bug
/// the player reported as monsters dying on arrival (145차); the surplus is now never
/// spawned (`tower_spawn_cap_lgt`). ⛔ Do not call this from any path a player can reach.
///
/// Read-compare-write on the HP-cur halfword, so a record already at 0 costs nothing.
pub fn guardian_zero_hp_writes(read: &impl Fn(u32, usize) -> Vec<u8>, indices: &[i32]) -> Option<Vec<(u32, Vec<u8>)>> {
    if indices.is_empty() {
        return Some(vec![]);
    }
    let sw = crate::guest_build::scene_world()?;
    let c = char_base_raw(read)?;
    let arr = rd_u32(read, c + sw.creatures_off)?;
    if arr == 0 {
        return None;
    }
    let off = sw.rec.hp_cur;
    let mut out = Vec::new();
    for i in indices {
        let rec = arr.wrapping_add(*i as u32 * sw.creature_stride);
        if rd_u16(read, rec + off) != Some(0) {
            out.push((rec + off, 0u16.to_le_bytes().to_vec()));
        }
    }
    Some(out)
}

/// The three writes that post a warp, IN ORDER.
///
/// ⛔ The order is the whole safety property: destination and entry point first, the
/// request LAST, because the request is what arms the handler — it polls every tick
/// and consumes the byte before dispatching, so a request written first can be
/// serviced with a stale destination.
/// The legacy entry byte is the arrival facing, always `guardian::WARP_ENTRY` (down).
pub fn guardian_warp_writes(read: &impl Fn(u32, usize) -> Vec<u8>, scene: u8) -> Option<Vec<(u32, Vec<u8>)>> {
    let sw = crate::guest_build::scene_world()?;
    let c = char_base_raw(read)?;
    Some(vec![
        (c + sw.warp_entry_off, vec![crate::guardian::WARP_ENTRY]),
        (c + sw.warp_dest_off, vec![scene]),
        (c + sw.warp_request_off, vec![1]),
    ])
}

/// ★ 143차: is a posted warp request still waiting for the game (`Some(true)`), or has the
/// handler consumed it (`Some(false)`)? `0x3f1d4` clears the byte BEFORE it dispatches, so
/// "consumed but the scene word never moved" is a request the game ate without loading —
/// the one case the way home re-posts (`emulator.rs`'s landing block). A request still at
/// 1 is only waiting (a window open), and posting it again would change nothing.
pub fn warp_request_pending(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<bool> {
    let sw = crate::guest_build::scene_world()?;
    let c = char_base_raw(read)?;
    Some(rd_u8(read, c + sw.warp_request_off)? != 0)
}

/// Turn one floor's forks into writes — READ, COMPARE, then write only what differs.
///
/// ★ The compare is not an optimisation, it is the buff pattern (`buffs.rs`,
/// `autoloot_to_force`): a fork that is already in place produces an EMPTY plan, so
/// the steady state of a floor is zero writes per poll and the log line "N write(s)"
/// says something real. Re-asserting exists for the cases that do move — a monster
/// respawning, the game recomputing an effective stat — and those are rare.
///
/// ⛔ The level goes LAST for each record, the way `MenuTapAction::FocusEquip` orders
/// its three bytes: a torn read then sees the old, self-consistent stats rather than
/// a monster whose level says one thing and whose HP says another.
pub fn guardian_fork_writes(read: &impl Fn(u32, usize) -> Vec<u8>, forks: &[crate::guardian::Fork]) -> Option<Vec<(u32, Vec<u8>)>> {
    let sw = crate::guest_build::scene_world()?;
    let c = char_base_raw(read)?;
    let arr = rd_u32(read, c + sw.creatures_off)?;
    if arr == 0 {
        return None;
    }
    let r = sw.rec;
    let mut out = Vec::new();
    for f in forks {
        let rec = arr.wrapping_add(f.index as u32 * sw.creature_stride);
        let want16 = |off: u32, v: u16, out: &mut Vec<(u32, Vec<u8>)>| {
            if rd_u16(read, rec + off) != Some(v) {
                out.push((rec + off, v.to_le_bytes().to_vec()));
            }
        };
        want16(r.hp_max, f.hp_max, &mut out);
        want16(r.hp_cur, f.hp_cur, &mut out);
        want16(r.atk_eff, f.atk_eff, &mut out);
        // 160차 T4: a boss's second and third attack values (`None` on everything else).
        if let Some([a, b]) = f.atk_skill {
            want16(r.atk_skill1, a, &mut out);
            want16(r.atk_skill2, b, &mut out);
        }
        want16(r.def_phys_eff, f.def_phys_eff, &mut out);
        want16(r.def_spec_eff, f.def_spec_eff, &mut out);
        // The virtual HP seed: once, and only onto a zero word (the hook owns it after).
        // The total goes first so the hook never sees a remainder without its denominator.
        if let Some(total) = f.vhp {
            if rd_u32(read, rec + r.vhp) == Some(0) {
                out.push((rec + r.vhp_max, total.to_le_bytes().to_vec()));
                out.push((rec + r.vhp, total.to_le_bytes().to_vec()));
            }
        }
        // ★ 156차 hell: the three roll fields, read-compare-write like the stats. The 명중
        // halfword is signed in the game's eyes; the record simply holds its two's complement.
        if let Some(h) = f.hit {
            want16(r.hit, h as u16, &mut out);
        }
        if let Some(d) = f.dodge {
            want16(r.dodge, d, &mut out);
        }
        if let Some(s) = f.res {
            want16(r.crit_res, s, &mut out);
        }
        if let Some(lv) = f.level {
            if rd_u8(read, rec + r.level) != Some(lv as u8) {
                out.push((rec + r.level, vec![lv as u8]));
            }
        }
    }
    Some(out)
}

/// The live character's level as the hell fork wants it (`i8`, 1..=`LEVEL_CAP`), RAW base
/// — the fork runs on the plain field, where the record is loaded; `None` when there is
/// no character or the halfword is not a level (then the fork uses the monster's own
/// level, i.e. the base knobs).
pub fn player_level(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<i8> {
    let c = char_base_raw(read)?;
    let lv = rd_u16(read, c + LEVEL_OFF)?;
    (1..=u16::from(LEVEL_CAP)).contains(&lv).then_some(lv as i8)
}

/// ★ 156차 방패무시 — the summed 방패무시 % the game's own `addStat` left in the stat block
/// (`CHAR+0xb4+0x72`, option id 0x12: every worn pair, the accessories' `Stat` thirds through
/// the wear-recompute hook, passives — exactly what the game sums and then never reads:
/// no code loads `stat+0x72`, which is why the option did nothing until the `dmgwide`
/// hook took it up). Clamped to 0..=100 (more than the whole defence means nothing);
/// `0` = nothing worn, or nothing readable.
pub const SHIELD_PIERCE_OFF: u32 = 0x72;

pub fn shield_pierce_pct(read: &impl Fn(u32, usize) -> Vec<u8>) -> u32 {
    let Some(block) = player_stat_block(read) else { return 0 };
    let v = rd_u16(read, block + SHIELD_PIERCE_OFF).map_or(0, |v| i32::from(v as i16));
    v.clamp(0, 100) as u32
}

/// Is the worn main weapon (`EQUIPPED[0]`) one the damage formula resolves against the
/// monster's SPECIAL defence? The resolver `0x4bca0` picks `+0x6e` for weapon categories
/// above 6 (단총 · 캐논 · 암흑성 · 광휘석 — `0x4bd00 cmp #6; ble`) and `+0x6c` for 4..6
/// (스피어 · 스워드 · 나이프). `false` when nothing readable is worn (a bare fist is
/// physical).
pub fn worn_weapon_special(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    let Some(c) = resolve_char(read) else { return false };
    let rec = c + WORN_BASE_OFF;
    match rd_u16(read, rec + WORN_CAT_OFF).map(|v| v as i16) {
        Some(cat) if cat > 6 && cat <= EQUIP_CAT_MAX => true,
        _ => false,
    }
}

/// Forbid saving in the tower, by writing the game's own area byte.
///
/// This is the one fork that buys UI for free: 27차 made the SAVE button and the
/// minimap follow [`area_can_save`], so a floor that reads "no save here" hides both
/// without another line of code — which is also what the boss-room request asked for.
///
/// ★ NO CLEANUP IS NEEDED. The byte belongs to the AREA record, which the map loader
/// rewrites on every scene load — and leaving the tower IS a scene load. So this
/// re-asserts while a floor is live and un-asserts itself the moment the player is
/// somewhere else.
/// `None` while the chain cannot be read, and `None` when it already says no.
///
/// ⛔ RAW base (`char_base_raw`), not [`resolve_char`]. This is a SAFETY fork and it
/// fails open — a refused read leaves saving allowed, and a save taken inside the
/// tower loads back into a floor with no run behind it. `resolve_char` is exactly the
/// gate that refused legitimate characters twice (71차-L, 80차-E), so it must not be
/// what decides this.
pub fn guardian_no_save_write(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u32, u8)> {
    let (addr, v) = guardian_can_save_byte(read)?;
    (v != 0).then_some((addr, 0))
}

/// Where the AREA canSave byte lives right now, and what it says.
///
/// The same chain [`guardian_no_save_write`] writes through, split out because the
/// tower has to GIVE THE BYTE BACK when a run ends and therefore needs the value it
/// found before it wrote — see [`crate::guardian::restore_can_save`] for the device
/// measurement that made the restore mandatory.
///
/// ⛔ RAW base, for the same reason [`guardian_no_save_write`] uses it: this serves a
/// safety fork, and `resolve_char` is the gate that refused legitimate characters
/// twice (71차-L, 80차-E).
///
/// ★ The address is returned so a diagnostic can print it. A device report that says
/// "the byte is stuck" needs to distinguish "the AREA object moved and we are reading
/// a new one" from "it is the same address we wrote", and only the address does that.
pub fn guardian_can_save_byte(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u32, u8)> {
    let c = char_base_raw(read)?;
    let base = resolve_area_base(read, c)?;
    let addr = base + crate::save::AREA_CAN_SAVE_OFF;
    Some((addr, rd_u8(read, addr)?))
}

/// (count, cat, idx) of the inventory item at (tab, slot); zeros if unresolved.
fn item_record(read: &impl Fn(u32, usize) -> Vec<u8>, char_ptr: u32, tab: u8, slot: u8) -> (u16, u16, u16) {
    let Some(base) = resolve_grid_base(read, char_ptr) else {
        return (0, 0, 0);
    };
    let rec = base + u32::from(tab) * TAB_STRIDE + u32::from(slot) * SLOT_STRIDE;
    let count = rd_u16(read, rec + COUNT_OFF).unwrap_or(0);
    let cat = rd_u16(read, rec + 0x3b6).unwrap_or(0);
    let idx = rd_u16(read, rec + 0x3b8).unwrap_or(0);
    (count, cat, idx)
}

/// item (cat, idx) -> IconRef::Item(frame). Consumables (cat 0x12) read the icon
/// frame live from the item-DB, exactly as the game (0x1533a8) does; else default.
fn item_icon(read: &impl Fn(u32, usize) -> Vec<u8>, char_ptr: u32, cat: u16, idx: u16) -> IconRef {
    // Same rule the game uses (see `icon_frame`); the generic frame is only a
    // last resort for a record we could not resolve at all.
    IconRef::Item(icon_frame(read, char_ptr, cat, idx).unwrap_or(ITEM_ICON_FALLBACK))
}

/// Live item-DB icon-frame byte for consumable index `idx`, or None.
fn item_db_icon(read: &impl Fn(u32, usize) -> Vec<u8>, char_ptr: u32, idx: u16) -> Option<u16> {
    let base = resolve_grid_base(read, char_ptr)?;
    let db = deref_container(read, base + ITEMDB_HANDLE_OFF)?;
    let rec = db + u32::from(idx) * ITEMDB_STRIDE;
    rd_u8(read, rec + ITEMDB_ICON_OFF).map(u16::from)
}

/// Current SP, without the HUD plausibility gate.
///
/// The skill-cast watch (27차) used [`read_stats`] as its SP signal. That function
/// still requires `hp <= hp_max`, which the game itself violates on unequip
/// (71차-L). A refused `read_stats` makes **both** watch samples `None`, so
/// "SP dropped" can never confirm — and on LGT the other signal
/// ([`skill_cast_active`], KTF `+0x9bb5/+0x2c9`, matrix 🔶 #18) is unverified.
/// Together that is a silent "no cooldown for any skill". This read only needs
/// CHAR + the SP word.
pub fn read_sp(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u16> {
    let c = resolve_char(read)?;
    Some(rd_u32(read, c + SP_OFF)?.min(u32::from(SP_MAX)) as u16)
}

/// Live `addStat(0x11)` total (percent), as the CAST PATH now sees it. `None` when CHAR is
/// not a character — the caller then applies 0 (base length unchanged). Same gate as
/// [`read_sp`]: no `hp <= hp_max`.
///
/// ★ 158차 P2: capped at `wie_util::stats::COOLDOWN_REDUCE_CAP_PCT` (70) — the guest hook
/// `cooldown_cap_lgt` caps the same word at the same value where the game reads it
/// (`0x4bbe0`), so the host's cooldown ring and the game's countdown agree. The word itself
/// is left as `addStat` summed it (the game rewrites it on every wear recompute anyway).
///
/// ★ 163차: the cap is 개정판 (`crate::flavor::REBALANCE`) — vanilla's guest has no cap hook,
/// so its ring reads the game's own sum (`config::apply_cd_reduce` stops at 99 %).
pub fn read_cd_reduce_pct(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u16> {
    let c = resolve_char(read)?;
    let raw = rd_u16(read, c + CD_REDUCE_OFF)? as i16;
    Some(i32::from(raw).clamp(0, cd_reduce_cap()) as u16)
}

/// The 쿨타임 감소 % the ring may apply: the guest hook's cap on main, the whole sum on vanilla.
pub fn cd_reduce_cap() -> i32 {
    if crate::flavor::REBALANCE { wie_util::stats::COOLDOWN_REDUCE_CAP_PCT } else { 100 }
}

/// Read the player gauges for the HUD. `None` when the reading does not look like
/// a live character (boot/teardown, a moved pointer, a partially written struct):
/// the caller then KEEPS its previous copy, because a gauge that flashes to empty
/// reads as "you are about to die" to the player.
pub fn read_stats(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<Stats> {
    let c = resolve_char(read)?;
    // Signed: the death gates (guest 0x12426c / 0x124568) test `HP <= 0`, so a dead
    // character carries a negative value — show it as an empty bar, not as 65535.
    let gauge = (rd_u16(read, c + HP_OFF)? as i16).max(0) as u32;
    // ★ 142차: both are the GAME's numbers — the max is `min(true max, 32767)` since the
    // `hp_max_lgt` hook, and the current is a proportional gauge above that. The HUD shows
    // the TRUE pair the HP hooks keep (identity whenever the max is <= 32767).
    let gmax = rd_u32(read, c + HP_MAX_OFF)?;
    let (hp, hp_max) = wie_util::stats::player_hp_view(gauge, gmax);
    let s = Stats {
        hp,
        hp_max,
        sp: rd_u32(read, c + SP_OFF)?.min(u32::from(SP_MAX)) as u16,
        sp_max: SP_MAX,
        exp: rd_u32(read, c + EXP_OFF)?,
        exp_next: rd_u32(read, c + EXP_NEXT_OFF)?,
        level: rd_u16(read, c + LEVEL_OFF)?,
        // The game reads this one SIGNED (`ldrsh` at 0x14a2de) and only draws the
        // badge when it is > 0, so a negative value means "none" — not 65000-odd.
        stat_points: (rd_u16(read, c + STAT_POINTS_OFF)? as i16).max(0) as u16,
        skill_points: rd_u8(read, c + SKILL_POINTS_OFF)?,
    };
    plausible_stats(&s).then_some(s)
}

/// Sanity gate for [`read_stats`]: a live character has a level in 1..=LEVEL_CAP, and
/// every other field within the bound the game's own clamps put on it. Anything else
/// means the bytes are not (yet) a character.
///
/// ⛔⛔ **THE SAME TWO TERMS THAT WERE WRONG IN [`resolve_char`] WERE WRONG HERE**
/// (80차-E). 71차-L took `hp <= hp_max` out of `resolve_char` and left this copy of it
/// standing, and this one also demanded `hp_max >= 1`; between them they refused every
/// unequipped character AND every character in the network warehouse, where `hp_max` is
/// legitimately 0 because `ComputeMaxHP` has not run yet. A refusal here is not harmless
/// either — the publisher is `read_stats(&read).unwrap_or(last_stats)`, so it freezes the
/// gauges at whatever they last were (level 0 for a whole session that entered the
/// warehouse straight off the title, which is what gated the touch arm).
/// ⛔ Do not restore either term: `bar()` already clamps with `cur.min(max)` and treats
/// `max == 0` as an empty bar, which is the right place for it.
fn plausible_stats(s: &Stats) -> bool {
    (1..=LEVEL_CAP).contains(&s.level)
        && s.hp_max <= HP_TRUE_CAP
        && s.sp <= s.sp_max
        // The game's own clamps bound these two, so exceeding them is proof the
        // bytes are not a character yet (boot / teardown / a moved pointer).
        && s.stat_points <= STAT_POINT_CAP
        && s.skill_points <= SKILL_POINT_CAP
}

/// The (atlas sheet, cell) a STORED skill entry draws, without that skill having to
/// be in the guest's bar at all.
///
/// The same rule as [`skill_icon`], with the two bytes it reads out of the quickbar
/// arrays supplied by the caller instead: `cat` IS the ref's top two bits, so
/// `cat != 0` is the game's own 2-way on-field hero pick (`(ref as i8) >> 6 != 0`),
/// and `cell` is what `CHAR+0x26c+slot` would have held.
fn skill_icon_of(read: &impl Fn(u32, usize) -> Vec<u8>, char_ptr: u32, cat: u8, cell: u8) -> Option<IconRef> {
    if i16::from(cell) >= i16::from(SKILL_STRIP_CELLS) {
        return None;
    }
    let sheet_id = rd_u8(read, char_ptr + SKILL_SHEET_OFF + u32::from(cat != 0))?;
    let atlas = sheet_id.checked_sub(4).filter(|&s| s <= 6)?;
    Some(IconRef::Skill(atlas, cell))
}

/// Drawing state for the host's PLATE row (`quickpage::PLATES` of them), resolved
/// from the stored identities rather than from the guest's five-slot bar.
///
/// ★ WHY NOT A DIRECT READ OF THE GUEST'S BAR (2026-09-04): there are more plates
/// than guest slots, so most of the row is not in the guest at any given moment.
/// `quickpage::Pages` carries the identities and is refreshed from the live bar every
/// poll (`Pages::capture`), which makes it the one source that can describe all of
/// them. For a RESIDENT plate the two agree by construction — the capture that
/// produced the entry read exactly the bytes a direct scan would have. (The direct
/// scan itself is gone: it answered for five plates, which is no longer the row.)
///
/// An item entry is re-validated against the bag on every call, because a plate that
/// is not resident gets no help from the game's own unbind sweep (guest `0x466b4`,
/// which clears matching item refs when a stack is used up): a stale coordinate draws
/// as EMPTY rather than as somebody else's potion. Skills need no such check — the
/// class stamp is checked when the plate is actually bound, and an icon is not a
/// binding.
pub fn plate_views(
    read: &impl Fn(u32, usize) -> Vec<u8>,
    plates: &[crate::quickpage::Entry],
    window: &crate::quickpage::Window,
    boxed_count: &impl Fn(u8, u8) -> u16,
) -> Vec<QuickSlot> {
    use crate::quickpage::Entry;
    let Some(char_ptr) = resolve_char(read) else {
        return Vec::new();
    };
    plates
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let plate = i as u8;
            let (kind, count, icon) = match *e {
                Entry::Empty => (SlotKind::Empty, 0, None),
                Entry::Skill { cat, cell, .. } => (SlotKind::Skill, 0, skill_icon_of(read, char_ptr, cat, cell)),
                Entry::Item { tab, slot, cat, idx } => {
                    let (count, live_cat, live_idx) = item_record(read, char_ptr, tab, slot);
                    if count == 0 || live_cat != u16::from(cat) || live_idx != u16::from(idx) {
                        (SlotKind::Empty, 0, None)
                    } else {
                        (SlotKind::Item, count, Some(item_icon(read, char_ptr, live_cat, live_idx)))
                    }
                }
                // ★ 소비가방 (2026-09-14): the count is the BOX'S, not a bag cell's, and a
                // count of zero still draws the icon — the plate names a kind the player
                // registered, and an empty box is "none left", not "nothing bound".
                Entry::Boxed { cat, idx } => (
                    SlotKind::Item,
                    boxed_count(cat, idx),
                    Some(item_icon(read, char_ptr, u16::from(cat), u16::from(idx))),
                ),
            };
            QuickSlot {
                slot: plate,
                guest: window.guest_of(plate),
                kind,
                count,
                icon,
            }
        })
        .collect()
}

// ── quickbar PAGES: guest identity, and the writes a page restore needs ──────
//
// See `quickpage.rs` for why a page stores identities rather than the guest's bytes.
// This section is the guest-facing half of that: reading the identity of what is
// currently bound, and the minimal writes to put a stored one back.

// `skill_bind(cat, cell, slot)` is a per-build address and lives in
// `guest_anchors::GuestBuild::skill_bind_fn` (2026-08-11). KTF's `0x145c48` is callable
// because it takes its arguments in registers and never touches the caller's frame
// (verified across 0x145c48..0x145cf8), which gives us the learned check, the dedup via
// `unbind_ref` (0x145e98), the per-class ref packing and consistent writes to all four
// parallel arrays.
//
// ★ `slot` must be < 5 either way. The register path at 0x145caa has **no upper bound of
// its own**, and 5 is the unregister sentinel (0x145ca6) — see
// [`crate::quickpage::SKILL_BIND_UNREGISTER`]. `quickpage` never emits either.

/// The class id sitting in party slot `cat` — the same byte `skill_bind` reads at
/// guest 0x145c6c to pick its per-class jump table, and therefore the thing that
/// decides what `(cat, cell)` actually names.
///
/// This is the page store's identity stamp for a skill. `cat` is a party SLOT, so
/// after a party change the same `(cat, cell)` is somebody else's skill; comparing
/// this byte is how a restore tells those apart.
pub fn class_of(read: &impl Fn(u32, usize) -> Vec<u8>, cat: u8) -> Option<u8> {
    if u32::from(cat) >= u32::from(crate::quickpage::PARTY_SLOTS) {
        return None;
    }
    let c = resolve_char(read)?;
    rd_u8(read, c + SKILL_SHEET_OFF + u32::from(cat))
}

/// The `(category, index)` of the item record in bag cell `(tab, slot)` — the page
/// store's identity stamp for an item, since the bound ref is only a coordinate and
/// the bag shifts when items are used or sold.
///
/// `None` when the cell is empty or the record does not fit a stamp, which makes it
/// unstampable and therefore not restorable — the safe direction.
pub fn item_at(read: &impl Fn(u32, usize) -> Vec<u8>, tab: u8, slot: u8) -> Option<(u8, u8)> {
    let c = resolve_char(read)?;
    let (count, cat, idx) = item_record(read, c, tab, slot);
    if count == 0 {
        return None;
    }
    Some((cat.try_into().ok()?, idx.try_into().ok()?))
}

/// The live quickbar as page IDENTITIES (not as drawing state — that is
/// [`plate_views`], which resolves the whole PLATE row, guest-resident or not).
///
/// Skill decode: `cat` is bits 6..7 of the skill-ref, read **unsigned**. The game's own
/// `unbind_ref` sign-extends there instead (`asrs #0x1e`), which is self-consistent for
/// its purposes but would decode party slot 2 as `-2`; we need the original 0..2 back to
/// hand to `skill_bind`. `cell` comes from `CHAR+0x26c`, NOT from the ref's low bits —
/// the per-class jump tables rewrite those into a skill id (`movs r4,#0xe`, `adds r4,#9`,
/// …), which is exactly why the game keeps the cell in a fourth array.
pub fn quickbar_identities(read: &impl Fn(u32, usize) -> Vec<u8>) -> Vec<crate::quickpage::Live> {
    use crate::quickpage::Live;
    let Some(c) = resolve_char(read) else {
        return Vec::new();
    };
    (0..QB_SLOTS)
        .map(|s| match rd_u8(read, c + QB_TYPE_OFF + s).unwrap_or(0) {
            TY_SKILL => {
                let Some(rf) = rd_u8(read, c + QB_SKILLREF_OFF + s) else {
                    return Live::Empty;
                };
                let Some(cell) = rd_i8(read, c + QB_SKILLIDX_OFF + s) else {
                    return Live::Empty;
                };
                if cell < 0 {
                    return Live::Empty;
                }
                Live::Skill {
                    cat: (rf >> 6) & 0x3,
                    cell: cell as u8,
                    // The ref byte VERBATIM, so a page restore can put back exactly what
                    // the game wrote (`quickpage::Action::BindSkill`). The decode above
                    // is for identity; this is for reproduction.
                    sref: rf,
                }
            }
            TY_ITEM => match rd_u8(read, c + QB_ITEMREF_OFF + s) {
                Some(rf) => Live::Item {
                    tab: rf >> 5,
                    slot: rf & 0x1f,
                },
                None => Live::Empty,
            },
            _ => Live::Empty,
        })
        .collect()
}

/// The byte writes that empty quickbar slot `slot`, or bind an item ref into it.
///
/// Both are host writes into the four arrays, staying strictly inside the 5-slot range
/// (the caller's `slot` comes from `quickpage`, which refuses anything >= 5).
///
/// Items get raw writes because their bind body (guest 0x13ce34) is a jump-table target
/// inside `popup_choose` whose first instructions read the caller's stack frame — it is
/// not callable. Writing the ref is the whole of what it does to these arrays, and
/// unlike skills there is no packing table involved. Skills go through
/// [`crate::guest_build::skill_bind_fn`] instead.
///
/// `cell = -1` matches what the game's own `unbind_ref` writes for an emptied slot
/// (0x145f00: `movs r3,#1 / rsbs r3,r3,#0`), so an emptied slot is byte-identical to one
/// the game emptied itself.
///
/// ★ 2026-08-11: gated on `guest_build::quickbar_writes_ok()` rather than on
/// `guest_code_addresses_valid()`. The blanket flag was answering the wrong question here
/// — these are four DATA offsets, and §2.16's popup A/B verified them on LGT from the
/// other side (the game's own binder writes `+0x252` / `+0x257`). The blanket gate was
/// what made the page button do nothing on LGT.
pub fn slot_writes(read: &impl Fn(u32, usize) -> Vec<u8>, slot: u8, item_ref: Option<u8>) -> Option<[(u32, u8); 4]> {
    if !crate::guest_build::quickbar_writes_ok() {
        return None;
    }
    if u32::from(slot) >= QB_SLOTS {
        return None;
    }
    let c = resolve_char(read)?;
    let s = u32::from(slot);
    let (ty, iref) = match item_ref {
        Some(r) => (TY_ITEM, r),
        None => (0, 0),
    };
    Some([
        (c + QB_TYPE_OFF + s, ty),
        (c + QB_ITEMREF_OFF + s, iref),
        (c + QB_SKILLREF_OFF + s, 0),
        (c + QB_SKILLIDX_OFF + s, 0xff),
    ])
}

/// The byte writes that bind a SKILL into quickbar slot `slot` without calling the game:
/// the ref byte and the strip cell exactly as they were captured, plus the kind byte and
/// a cleared item ref.
///
/// Used only where [`crate::guest_build::skill_bind_fn`] is `None`. See
/// `quickpage::Action::BindSkill` for why replaying these four bytes is equivalent to the
/// call there (short version: the identity stamp is checked first, and every plan clears
/// all five slots before binding, which is what `skill_bind`'s dedup would have done).
pub fn skill_slot_writes(read: &impl Fn(u32, usize) -> Vec<u8>, slot: u8, sref: u8, cell: u8) -> Option<[(u32, u8); 4]> {
    if !crate::guest_build::quickbar_writes_ok() {
        return None;
    }
    if u32::from(slot) >= QB_SLOTS || u32::from(cell) >= u32::from(crate::quickpage::SKILL_CELLS) {
        return None;
    }
    let c = resolve_char(read)?;
    let s = u32::from(slot);
    Some([
        (c + QB_TYPE_OFF + s, TY_SKILL),
        (c + QB_ITEMREF_OFF + s, 0),
        (c + QB_SKILLREF_OFF + s, sref),
        (c + QB_SKILLIDX_OFF + s, cell),
    ])
}

/// The window id currently owning the screen (see [`WIN_NONE`] / [`WIN_MAP`]).
/// `None` when CHAR cannot be resolved (boot / teardown).
pub fn window_id(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u8> {
    let char_ptr = resolve_char(read)?;
    rd_u8(read, char_ptr + WINDOW_ID_OFF)
}

/// Is the INVENTORY page on screen with nothing over it — menu overlay up, item page,
/// no action popup, no `openWindow` window? The bag-box window is put back here after a
/// game window (귀환서's 예/아니오, 116차) closed over it, and nowhere else: a box window
/// over the field or over another window would be wrong.
pub fn inventory_page_up(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    let Some(c) = resolve_char(read) else { return false };
    rd_u8(read, c + SELECTOR_FLAG_OFF).unwrap_or(0) != 0
        && rd_u8(read, c + MENU_PAGE_OFF) == Some(PAGE_ITEM)
        && rd_u8(read, c + POPUP_OPEN_OFF) == Some(0)
        && window_id(read) == Some(WIN_NONE)
}

/// Whether the game is on its out-of-world SHELL (app state 3: title screen, slot
/// select, …). Those screens are laid out for the original 240-px width; on a wide
/// framebuffer the game fills the extra width by mirroring its background art,
/// which reads as "the title is cut off at both sides" (player-reported
/// 2026-07-30). The overlay letterboxes it instead.
///
/// ★ Must NOT resolve CHAR through [`resolve_char`]: the shell screens are exactly
/// where a live character does not exist yet, and the Static-anchor build (LGT)
/// gates resolution on liveness (level >= 1) — so the letterbox never drew there
/// (MIGRATION_LGT.md §2.18-(3)). The state word is valid before the record is:
/// probe run 2026-08-11 shows state=3 across 타이틀→슬롯→클래스 with a zeroed
/// record (state=2 on the boot notices/logos, 4/5 in game) — same values as KTF,
/// so the `== 3` test itself ports unchanged.
pub fn shell_state(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    let Some(c) = char_base_raw(read) else { return false };
    rd_u32(read, c + crate::guest_build::state_off()) == Some(3)
}

/// The online module's SCREEN word (`*(base+0xa3d4)`, `base = CHAR+0x28`): 1 main menu ·
/// 9 게임시작 slot list · **10/11/12 the character-creation screens** · 2/3 online. `None`
/// when the build has no anchor for it (KTF/SKT) or CHAR cannot be found.
///
/// ★★★ 76차 — read RAW (`char_base_raw`), like [`shell_state`]: the value that matters is
/// the one left over from the shell AFTER the world was entered, and it keeps its last
/// shell value through app state 4/5 (nothing outside the shell module writes it; probe
/// 2026-09-03: 12 for 2,400 frames after a creation). Paired with the slot cursor it is the
/// pin source for a CREATED character — `savevault::creation_slot`.
/// The shell screen word's value on the ONLINE screens (창고 · 캐시샵) — the one value of
/// [`shell_screen`] that is app state 3 WITHOUT being the title or a slot list (66차-C).
/// Both the worker (quickbar re-warm skip) and the UI (the slot-list tap arm, 80차-P) key
/// on it. The others, for reference: 1 main menu · 2 online slot list · 9 game-start slot
/// list · 13 login notice · 10..12 character creation.
pub const SHELL_SCREEN_ONLINE: u32 = 3;

pub fn shell_screen(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u32> {
    let off = crate::guest_build::shell_screen_off()?;
    let c = char_base_raw(read)?;
    rd_u32(read, c + off)
}

/// Whether the game's own MAP window is the one on screen (window id 8). The
/// overlay then paints a big centred copy of the mirrored map over it — the game
/// draws its map 1:1, which is tiny on a wide framebuffer.
pub fn map_window_open(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    window_id(read) == Some(WIN_MAP)
}

/// Whether the real-time field owns the screen: no window open, app state ==
/// field, and the quick-item selector overlay closed — the context where the
/// quickbar hotkeys are valid and where our HUD (gauges, skill plates, minimap)
/// belongs. Everything else is "a game window is up": the overlay then hides the
/// HUD and offers menu-navigation keys instead.
pub fn in_field_state(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    let Some(char_ptr) = resolve_char(read) else {
        return false;
    };
    let window = rd_u8(read, char_ptr + WINDOW_ID_OFF);
    let state = rd_u32(read, char_ptr + crate::guest_build::state_off());
    let selector = rd_u8(read, char_ptr + SELECTOR_FLAG_OFF);
    matches!((window, state, selector), (Some(WIN_NONE), Some(STATE_FIELD), Some(0)))
}

/// Whether the SCRIPT window (id 4 — story scene / NPC dialogue) owns the screen.
/// See [`WIN_SCRIPT`].
pub fn cutscene_open(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    window_id(read) == Some(WIN_SCRIPT)
}

/// The script window's CHOICE-LIST flag — 1 while the 예/아니오 (or n-row) selection
/// box is up inside a dialogue, 0 for plain narration and plain dialogue text.
///
/// ★ THE byte that splits the two things window id 4 conflates (2026-08-17). The
/// player asked for exactly that split — "컷신일때는 안나오고 인게임 내에서 대화에
/// 선택지가 있을때는 나오고" — and the window id cannot give it: `cutscene_open`'s own
/// doc says id 4 is "story scene / NPC dialogue", and a device trace confirmed the
/// quest guide's dialogue reads `win=4` exactly like the prologue narration.
///
/// ── HOW IT WAS FOUND (device, 2026-08-17, LGT vc92) ─────────────────────────────
/// `items::flag_window_changes` (the 43 KB whole-record watcher) across three
/// transitions in the 안내원 quest dialogue:
///   * dialogue OPENS (text only): `0xa0c0 3->4` **and `0xa0dc 1->0`** — entering the
///     script window CLEARS it, which is what makes it safe to read while id == 4.
///   * choice box APPEARS:         **`0xa0dc 0->1`**, with the row table at
///     `0xa16c..0xa187` filled in the same sample (4 rows x 4 bytes, pointer-shaped).
///   * choice box CLOSES:          `0xa0c0 4->3` and `0xa148..0xa16f` zeroed.
/// The cursor index is a separate byte, `CHAR+0xa284` (3 = the 4th row highlighted;
/// UP moved it 3->2, DOWN 2->3) — that one is NOT the gate: it legitimately reaches 0
/// when the player picks the first row.
///
/// ★ Measured on LGT only. It is read for a DRAWING decision (do the arrows appear),
/// so a wrong value on another build costs a stray control, not corruption.
pub fn script_choice_open(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    if window_id(read) != Some(WIN_SCRIPT) {
        return false;
    }
    let Some(c) = resolve_char(read) else { return false };
    rd_u8(read, c + SCRIPT_CHOICE_OFF) == Some(1)
}

/// Whether a CHARACTER is in the world at all — app state 4, regardless of which
/// window is on top. The quickbar exists (and its icons are meaningful) for the
/// whole of that, not only during field free-roam, which is what lets the overlay
/// keep the skill/potion plates on screen with a game window open (player request
/// 2026-08-03). Boot (2) and the title/slot shell (3) are excluded: CHAR is not a
/// live character there and the slot bytes are whatever was left in the heap.
pub fn in_game_state(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    let Some(char_ptr) = resolve_char(read) else {
        return false;
    };
    matches!(
        rd_u32(read, char_ptr + crate::guest_build::state_off()),
        Some(STATE_FIELD | STATE_SELECTOR)
    )
}

/// App state **3** — and it is NOT only the title / slot / class shell.
///
/// ★★ 66차-C, and it cost a build to find (the UI's new `menu tap:` log said
/// `REFUSED … shell=true` on a live 창고 screen): the ONLINE module runs in this same
/// state. That is already written down elsewhere in this crate without the consequence
/// being drawn — `show_disconnect_modal` gates itself on [`shell_state`] *because* the
/// online screens are state 3 (48차). So "state 3" means "the shell OR an online
/// screen", and anything that used it as "out of world" was refusing 창고 and 캐시샵 too.
const STATE_ONLINE: u32 = 3;

/// Whether an ONLINE screen (네트워크/싱글 창고 · 캐시샵) owns the display: app state 3
/// **with a live character record**.
///
/// ★ The liveness test IS the discriminator, and it is free: on the Static-anchor build
/// [`resolve_char`] refuses to resolve until the record holds a real character
/// (level >= 1), which is exactly what the title / slot / class screens lack — that is
/// why [`shell_state`] is documented as having to read the state word RAW. So
/// `resolve_char` succeeding under state 3 says "there is someone playing, and they are
/// on an online screen", the one thing the state word alone cannot say.
pub fn online_screen(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    let Some(c) = resolve_char(read) else { return false };
    rd_u32(read, c + crate::guest_build::state_off()) == Some(STATE_ONLINE)
}

// ── the game's own context popup -> "퀵바등록" ────────────────────────────────
//
// The player asked for the game's five separate "1키등록 / 3키등록 / 9키등록 /
// 0키등록 / #키등록" rows to become ONE "퀵바등록" row, which then highlights our
// on-screen plates so they can pick the slot by tapping it.
//
// ── HOW, AND WHY THIS WAY ────────────────────────────────────────────────────
// The host REWRITES THE GUEST'S OWN LABEL BUFFERS in RAM and lets the guest draw
// its own list. Three designs were scoped and adjudicated (workflow wf_95a21c44,
// 5 RE questions x independent adjudicator + synthesis); the two obvious ones lose:
//
//   * DRAW OVER THE POPUP — impossible today. `draw_popup` (guest 0x12e450) takes
//     x/y as arguments and they never leave its stack frame, so the popup's screen
//     anchor is not readable. We cannot cover something we cannot locate.
//   * PATCH THE ROW TABLE — fatal on the skill side. `skill_bind`'s second caller
//     (guest 0x154462) hardcodes `movs r2,#5` in an unregister-all sweep, so
//     repointing the `slot==5` unbind sentinel at 0x145ca6 would make a routine
//     player action write one byte past EVERY 5-entry quickbar array. Silent save
//     corruption.
//
// What makes the RAM rewrite work: `draw_popup` recomputes only the label CACHE
// POINTER each frame — `text[i] = *handle[i] + 8` at guest 0x12e592..0x12e598 — and
// never rewrites the payload. So bytes the host writes into `*handle[i] + 8` persist
// and the guest renders them, at its own anchor, in its own style. `CHAR+0x9c85`
// (row count) is likewise read live by the draw loop, the box height and the up/down
// clamp, so shrinking it shrinks the whole widget consistently.
//
// Registration itself stays 100 % NATIVE: the host writes the highlighted-row byte
// and asks the guest to confirm. Nothing in this module builds a quickbar ref — the
// skill ref packing runs through nested class tables (0x1a86bc/0x1a86d8) and is
// exactly the kind of thing a host should never reimplement.
//
// ── THE ROW MAP (the part that silently mis-fires if you get it wrong) ────────
// `popup_choose` (guest 0x13c920) dispatches on (tag, RAW ROW INDEX) only — labels
// and the row count play no part. Item rows go through an 8-entry table at
// 0x1a77cc, bound-checked `cmp #7; bls` at 0x13cd04. Skill rows are a bare
// `row == 0 ? invest : skill_bind(cat, cursor, row - 1)` at 0x13d308 with NO upper
// bound at all. So the host is authoritative about turning a DISPLAY row back into
// a GUEST row, and about clamping.
//
//   item  (tag 0x66, 8 native rows):  0 사용하기 | 1..5 N키등록 | 6 상세보기 | 7 버리기
//   skill (tag 0x67, 7 native rows):  0 포인트투자 | 1..5 N키등록 | 6 등록취소
//
//   display 0 -> guest 0        (사용하기 / 포인트투자, passed straight through)
//   display 1 -> guest slot+1   (our 퀵바등록, resolved when the player taps a plate)
//   display 2 -> guest 6        (상세보기 / 등록취소)
//   display 3 -> guest 7        (버리기 — items only)

/// Menu page, and the `kind` argument `popup_key`/`popup_choose` expect: 2 = the
/// inventory page, 4 = the skill page. [CONFIRMED — page draw/key tables at guest
/// 0x1ae5e0 / 0x1ae608 are indexed by this byte.]
const MENU_PAGE_OFF: u32 = 0x9c6e;
pub const PAGE_ITEM: u8 = 2;
pub const PAGE_SKILL: u8 = 4;

/// Popup state block, all CHAR-relative. [CONFIRMED]
const POPUP_OPEN_OFF: u32 = 0x9c84;

/// **대화 선택지가 지금 화면에 있다** — the script module's own flag (79차-E).
///
/// ★★★ Static, both halves, and it is the GAME's own predicate:
/// * paint `0x3cb9e`: `getBase() + 0xa1fc`, `cmp #0` → nonzero calls the SHARED list
///   painter `0x575bc` (literal `0x575bd` at `0x3cc34`);
/// * keys `0x3ed7a`: same `base + 0xa1fc` (literal at `0x3eeb4`), nonzero → OK (−5) or
///   `'5'` reads the shared cursor (`0x574c0`), **writes 0 back into the flag**, and
///   applies the choice (`0x3cd5c`).
///
/// `base = getBase() = CHAR + 0x28`, so the byte is **`CHAR + 0xa224`**.
/// ⛔ It is the only usable gate: the script WINDOW (id 4) stays up for the whole
/// conversation and the shared widget keeps the last choice's `rows`/`axis` — the
/// device log shows `win=4 … rows=4 axis=1` minutes after the choice was answered. A
/// tap routed on those stale bytes would move the cursor of the NEXT choice.
/// ⛔ Not `CHAR+0xa0dc` (66차's note): that is `base+0xa0b4`, a different byte, and no
/// literal in the paint or key path reads it.
const CHOICE_FLAG_OFF: u32 = 0xa224;

/// The shared widget's ROW POINTER ARRAY, CHAR-relative.
///
/// The painter (`0x575bc`) reads `base+0xa228` as an array of `u32` string pointers
/// and copies `base+0xa210` of them out of `base+0xa230`; the script fills all three
/// in the choice setup (`0x3c5e4`, the one caller of `0x3c4dc`). `base = CHAR+0x28`,
/// so `base+0xa228` is `CHAR+0xa250`.
///
/// ⛔ Kept as a MEASUREMENT, not as a signature. 79차-E's rule stands and 2026-09-10
/// sharpened it: `[예, 아니오]` is the exact row pair of **43 other choice sites** in
/// this game, so "the rows say X" can never identify one of ours. The tower entrance
/// used to match strings here and does not any more — see [`guardian_entry_flag`].
#[allow(dead_code)]
const CHOICE_ROWS_OFF: u32 = 0xa250;
/// The shared widget's cursor and row count (66차, 79차-E). One byte each.
#[allow(dead_code)]
const CHOICE_CURSOR_OFF: u32 = 0xa284;
#[allow(dead_code)]
const CHOICE_ROWCOUNT_OFF: u32 = 0xa285;

// ── 하늘성: the entrance is the passer-by's OWN dialogue, and the signal is a FLAG ──
//
// `tools_re/patch_guardian_npc.py` rewrites the town scene's EXISTING passer-by row
// (scene 111, `A[10]` — tile (32,19), sprite 0x2a, kind 4, the "(무섭다...)" one; ⛔ NOT
// `A[1]`, which is the map exit at the top edge — vc285 shipped that mistake) so its
// payload is the game's own
// CONDITION CONTAINER: in hard mode it runs a new script, otherwise it runs the
// shipped one byte for byte. GUARDIAN_CASTLE_FEASIBILITY.md §J is the whole decode;
// the three facts this file stands on:
//
// ★★ The script VM has a PERSISTENT FLAG ARRAY at `base+0x9e98`, 256 bytes, one per
//    id. `op 0x23 <id> <val>` writes it (guest setter `0x3f590`) and the condition
//    `fa <id> <0|1>` reads it (guest getter `0x3f570`, whose only caller in the whole
//    image is the condition evaluator `0x3bffc`). The array round-trips through the
//    save — `deserialiseSave 0x1aa84` and `serialiseSave 0x1affc` each memcpy `0x100`
//    bytes of it. That is how the game remembers quests, and it is why "did they
//    already hear the explanation" needs no host state at all.
//
// ★★★ SO THE HOST DOES NOT MATCH STRINGS ANY MORE. The 「예」 branch of our script runs
//    `op 0x23 <ENTER> 1`, and the host polls that ONE BYTE. Nothing else in the image
//    writes it. The old detector compared the two row strings, which was only safe
//    while the rows were `좋아`/`지금은 아니야` (zero matches in the game's 4,841 rows);
//    the player asked for 예/아니오, which collides 43 ways, so the signature moved off
//    strings entirely rather than becoming 43 times more dangerous.
//
// ⛔ There is NO difficulty condition in the VM — §J.3 enumerates all five arms of the
//    evaluator and none reads the difficulty byte. `HARD` is therefore the one thing
//    the host supplies: it mirrors the difficulty into a flag id, and the asset's
//    condition then reads it like any other flag.
//
// ⛔ These ids must equal `patch_guardian_npc.py`'s `FLAG_*`. They are free: across all
//    244 scenes the scripts read (`fa`) or write (`op 0x23`) 105 distinct ids, and the
//    longest free run is `0x3d..0xc8` — which begins exactly where the guest's own
//    clearing loop (`0x46a00`, `setFlag(0x33..0x3c, 0)`) ends. Two independent
//    boundaries, not one off.
//
// ⛔ Writing here is a SAVE-BACKED state change (the 0x100 memcpy above). Every write
//    is read-verify-then-write and only happens when the byte actually differs.

/// The persistent flag array, CHAR-relative. Guest `base+0x9e98`, `base = CHAR+0x28`.
const GUARDIAN_FLAGS_OFF: u32 = 0x9ec0;
/// Host-maintained mirror of "this character is in hard mode".
pub const GUARDIAN_FLAG_HARD: u32 = 0xc0;
/// Set by the script once the explanation has been heard. Lives in the save.
pub const GUARDIAN_FLAG_HEARD: u32 = 0xc1;
/// Set by the 「예」 branch. The host consumes it and writes it back to 0.
pub const GUARDIAN_FLAG_ENTER: u32 = 0xc2;
/// ★★★ THE TOWER SEAL. Raised for the length of a run; every A row of every floor
/// scene is gated on it being ZERO (`tools_re/patch_tower_gate.py`), so while it is 1
/// the scene's own scripts select nothing at all — no dialogue, no quest flag, no item,
/// and no door out of the room.
///
/// ⛔ The floor table depends on this being written. Without it a floor is an ordinary
/// dungeon room: clearing a boss arena runs the game's post-boss cutscene and warps the
/// player into the story (reported 2026-09-11, "몬스터 잡았더니 갑자기 퀘스트가 진행됐어").
///
/// ⛔ And it must be LOWERED again when the run ends, or the player's whole game goes
/// script-dead. Who writes it, exactly (code review 2026-09-11 — the old text here put
/// the lowering in the wrong block): it is RAISED on `Action::Warp` and re-asserted on
/// every floor poll (`emulator.rs::hold_floor`), and LOWERED by the idle entrance block
/// in `emulator.rs` (`frame % 5 == 2`, gated on `current_floor() == 0 && !forking()`) —
/// read-verify-then-write in both directions, so a settled state costs one byte read.
/// ⚠ When a run ends with no origin to warp back to (a test entry made from the floor's
/// own scene), the player is left standing IN the floor scene and the idle block lowers
/// the seal there, so that room becomes an ordinary room again, doors and all. That is
/// the intended reading: the alternative, a seal that stays up, is the script-dead game.
pub const GUARDIAN_FLAG_TOWER: u32 = 0xc3;

/// The town scenes an entrance NPC stands in: 사막마을 엔자크 (scene 111, the passer-by,
/// 리츠) and — 113차 (2026-09-19) — 네오 솔티아 (scene 5, 로빈, 케이). The cursor-reset for
/// the entrance confirm is gated on these so it cannot touch a choice on any other map.
/// `tools_re/patch_guardian_npc.py::TARGETS` is the asset side of the same list.
pub const GUARDIAN_ENTRANCE_SCENES: [u16; 2] = [111, 5];

/// Address of one persistent flag, or `None` when CHAR is unreadable.
///
/// ⛔ RAW base (`char_base_raw`), like the other tower reads: this runs in the middle
/// of a dialogue and `resolve_char` is the gate that refused legitimate characters
/// twice (71차-L, 80차-E).
fn guardian_flag_addr(read: &impl Fn(u32, usize) -> Vec<u8>, id: u32) -> Option<u32> {
    Some(char_base_raw(read)? + GUARDIAN_FLAGS_OFF + id)
}

/// One persistent flag byte.
pub fn guardian_flag(read: &impl Fn(u32, usize) -> Vec<u8>, id: u32) -> Option<u8> {
    rd_u8(read, guardian_flag_addr(read, id)?)
}

// ── 아바타 옷장 (167차) ────────────────────────────────────────────────────────────
//
// The wardrobe's state is thirteen bytes of the SAME flag array — the six worn sets at
// `FLAG_WEAR` (0xb0..0xb5) and the owned bitmap at `FLAG_OWN` (0xb6..0xbc), `wie_util::avatar`
// — so the save carries it both ways, and an unsaved reward rolls back with the save. ⛔ The
// same read-verify-then-write rule as every flag write above.

/// The first avatar byte (`FLAG_WEAR`), RAW base like the guardian flags.
pub fn avatar_flags_addr(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u32> {
    guardian_flag_addr(read, u32::from(crate::avatar_rules::FLAG_WEAR))
}

/// The save's avatar bytes: six worn sets, then the owned bitmap.
pub fn avatar_bytes(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<[u8; crate::avatar::Wardrobe::LEN]> {
    let b = read(avatar_flags_addr(read)?, crate::avatar::Wardrobe::LEN);
    b.try_into().ok()
}

/// The nine worn cells `(cat, idx)` — `-1` = empty — RAW base (the page runs inside the
/// menu, where nothing about the gauges matters).
pub fn worn_cells(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<[(i16, i16); crate::atkspeed::WORN_SLOTS]> {
    let c = char_base_raw(read)?;
    let mut out = [(-1i16, -1i16); crate::atkspeed::WORN_SLOTS];
    for (i, slot) in out.iter_mut().enumerate() {
        let rec = c + WORN_BASE_OFF + i as u32 * WORN_STRIDE;
        *slot = (rd_u16(read, rec + WORN_CAT_OFF)? as i16, rd_u16(read, rec + WORN_IDX_OFF)? as i16);
    }
    Some(out)
}

/// The class byte (`CHAR+0xb6`) — the loader's shield gate (`0x4980a`) keys on it.
pub fn char_class_byte(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u8> {
    let c = char_base_raw(read)?;
    rd_u16(read, c + CHAR_CLASS_OFF).and_then(|v| u8::try_from(v).ok())
}

/// ★ 172차: the byte the game raises while EQUIPMENT's right-hand 장비장착 candidate list is
/// up (`CHAR+0x9c7a`, base-relative literal `0x9c52`): `0x483a0` and `0x484ee` write 1 as they
/// open the list (and zero the list's col/row at `+0x9c78`/`+0x9c79`), the list's CLR writes
/// 0 back (80차-J). Device log 2026-10-08 (S25): `0x9c7a 0 -> 1` the moment the list opened,
/// `1 -> 0` when it closed, untouched by every worn-grid cursor move.
const EQUIP_CANDIDATES_OFF: u32 = 0x9c7a;

/// The EQUIPMENT page in its WORN-GRID mode with nothing of the game's on top of it — the
/// only state the AVATAR page opens over, and stays over. The candidate list is
/// [`EQUIP_CANDIDATES_OFF`]; the action popup is the usual `POPUP_OPEN_OFF`; a message/choice
/// box is [`game_box_up`].
///
/// ⛔⛔ 172차: this used to require the `.bss 0x1512904` byte (`equip_mode_addr`) to be 0,
/// read as "0 = the worn grid, 1 = the candidate list". It is neither: its five writers all
/// sit right after `0x46b7c(id)` widget installs (2 at `0x47804` / `0x48780`, 1 at `0x486cc`,
/// a register at `0x479fc` / `0x4867c`), so it says "EQUIPMENT put a message or confirm
/// widget up" — and it is NOT cleared when that widget closes. Device log 2026-10-08 (S25):
/// after a 방패 장비해제 through its 예/아니오 widget the player's every later AVATAR open
/// saw `screen=3 · popup 0 · choice 0` and still gave up after 60 iterations ("EQUIPMENT never
/// came up"), while the game itself went on opening the candidate list from the same state.
/// The widget is already covered while it is up ([`game_box_up`]'s choice flag), so the byte
/// adds nothing and is not read here any more.
pub fn equipment_worn_grid_up(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    let Some(c) = char_base_raw(read) else { return false };
    overlay_screen(read) == Some(MENU_SCREEN_EQUIP)
        && rd_u8(read, c + POPUP_OPEN_OFF) == Some(0)
        && rd_u8(read, c + EQUIP_CANDIDATES_OFF) == Some(0)
        && !game_box_up(read)
}

/// The four readings [`equipment_worn_grid_up`] decides from, for the log line the wardrobe
/// leaves when EQUIPMENT "never came up" — so the next such report names the refusing term
/// instead of costing a disassembly session (172차).
pub fn equipment_gate_report(read: &impl Fn(u32, usize) -> Vec<u8>) -> String {
    let Some(c) = char_base_raw(read) else { return "char base unreadable".to_string() };
    format!(
        "screen={:?} popup={:?} candidates={:?} box={} mode_byte={:?}",
        overlay_screen(read),
        rd_u8(read, c + POPUP_OPEN_OFF),
        rd_u8(read, c + EQUIP_CANDIDATES_OFF),
        game_box_up(read),
        crate::guest_build::equip_mode_addr().and_then(|a| rd_u8(read, a))
    )
}

/// The SKILL page with no popup on it — a «1» there goes back towards the wardrobe.
pub fn skill_page_up(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    let Some(c) = char_base_raw(read) else { return false };
    overlay_screen(read) == Some(MENU_SCREEN_SKILL) && rd_u8(read, c + POPUP_OPEN_OFF) == Some(0) && !game_box_up(read)
}

/// ★ 168차: the six-row MENU list, up and idle (the field menu's overlay on its list page, an
/// in-world record, no popup, no message / choice box) — and its cursor as `(address, row)`.
/// `None` otherwise, and for a cursor outside the game's own six rows. The host's AVATAR row
/// (`avatar_host`) lives on this answer.
pub fn menu_list_cursor(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u32, u8)> {
    let st = menu_state(read)?;
    if !st.overlay || st.online || st.screen != MENU_SCREEN_LIST || game_box_up(read) {
        return None;
    }
    let addr = resolve_char(read)? + crate::guest_build::menu_cursors()?.list;
    let row = rd_u8(read, addr)?;
    (row < crate::avatar::MENU_GAME_ROWS).then_some((addr, row))
}

/// Did the player pick 「예」? `Some(addr)` = enter the tower **and clear that byte**.
///
/// Fails closed: an unreadable CHAR is `None`, which costs nothing because the flag is
/// sticky — the next poll sees it.
pub fn guardian_entry_flag(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u32> {
    let addr = guardian_flag_addr(read, GUARDIAN_FLAG_ENTER)?;
    if rd_u8(read, addr)? == 0 {
        return None;
    }
    Some(addr)
}

/// Is a dialogue CHOICE menu on screen right now ([`CHOICE_FLAG_OFF`])? `None` when
/// CHAR is unreadable. Used by the entrance to reset the cursor to 예 when the confirm
/// opens (2026-09-16): the game only resets the shared list cursor when it equals 2, so
/// after picking 아니오 (cursor 1) it stays there and OK keeps landing on 아니오.
pub fn guardian_choice_open(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<bool> {
    let c = char_base_raw(read)?;
    Some(rd_u8(read, c + CHOICE_FLAG_OFF)? != 0)
}

/// The address of the shared list cursor ([`CHOICE_CURSOR_OFF`], the byte `0x574c0`
/// reads and the choice VM uses to pick which branch runs). `None` when CHAR is
/// unreadable. The entrance writes 0 here so OK defaults to 예 (row 0).
pub fn guardian_choice_cursor_addr(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u32> {
    Some(char_base_raw(read)? + CHOICE_CURSOR_OFF)
}

/// Keep the HARD mirror equal to `want`. `Some((addr, want))` **only when it differs**,
/// so a settled session writes nothing at all.
pub fn guardian_hard_mirror(read: &impl Fn(u32, usize) -> Vec<u8>, want: u8) -> Option<(u32, u8)> {
    let addr = guardian_flag_addr(read, GUARDIAN_FLAG_HARD)?;
    if rd_u8(read, addr)? == want {
        return None;
    }
    Some((addr, want))
}

/// Keep the TOWER seal equal to `want`, same read-verify-then-write shape as the HARD
/// mirror above. See [`GUARDIAN_FLAG_TOWER`] for what it does.
pub fn guardian_tower_flag(read: &impl Fn(u32, usize) -> Vec<u8>, want: u8) -> Option<(u32, u8)> {
    let addr = guardian_flag_addr(read, GUARDIAN_FLAG_TOWER)?;
    if rd_u8(read, addr)? == want {
        return None;
    }
    Some((addr, want))
}

/// ★★ 143차 — THE WAKE LINE. Raised by the host the moment the finished tower's way home is
/// posted (under a fully white screen — `guardian::WakeStep::GoHome`); the entrance towns'
/// ENTRY row (`tools_re/patch_guardian_npc.py`, `FLAG_WAKE`) sees it at load, clears it
/// itself (`23 c4 00`, first), waits for the veil to lift and says 「의식이 몽롱하다...」 in
/// the game's own dialogue window.
///
/// ⛔ Raised BEFORE the warp, never after: the entry row runs once per load (probe
/// 2026-09-30 — re-arming `0xff` after the load ran nothing again). ⛔ Only for a
/// destination in [`GUARDIAN_ENTRANCE_SCENES`]: nowhere else has the row, and a flag
/// left up would speak at the NEXT visit to 엔자크 / 네오 솔티아, days later. The host
/// drops it itself if the line never came (`guardian::WakeStep::SayOnHost`).
/// ⛔ Saved with the flag array like the others — the script clears it before it says
/// anything, so a save made while the line is up does not carry it.
pub const GUARDIAN_FLAG_WAKE: u32 = 0xc4;

/// Keep the WAKE flag equal to `want` — read-verify-then-write like the seal.
pub fn guardian_wake_flag(read: &impl Fn(u32, usize) -> Vec<u8>, want: u8) -> Option<(u32, u8)> {
    let addr = guardian_flag_addr(read, GUARDIAN_FLAG_WAKE)?;
    if rd_u8(read, addr)? == want {
        return None;
    }
    Some((addr, want))
}

/// The script window (dialogue). It drives the shared list widget ONLY while
/// [`CHOICE_FLAG_OFF`] is set — see there.
const SCRIPT_WINDOW: u8 = 4;
const POPUP_COUNT_OFF: u32 = 0x9c85;
const POPUP_SEL_OFF: u32 = 0x9c86;
const POPUP_TAG_OFF: u32 = 0x9c87;
// The inventory page's pre-emptor bytes moved to `guest_build::popup_preempt_offsets()`
// (2026-08-15, 51차): they are KTF measurements (`0x9c89`, `0xa220`) needed only where
// the commit bypasses the game's own key router, and on LGT the second one reads a
// different live field — which is what kept every CONSUMABLE popup un-relabelled on
// the device while skills (no pre-emptor gate) worked. See the table for the argument.

/// Popup flavour tag. ONLY these two carry register rows; 0x64 (2 rows), 0x65
/// (equipment), 0x68 (skill invest-only) and 0x69 (3-row item) must be left alone.
const TAG_ITEM: u8 = 0x66;
const TAG_SKILL: u8 = 0x67;
/// The three-row item popup (사용하기 / 상세보기 / 버리기) a cash-tier consumable gets —
/// no register rows, so the 퀵슬롯등록 rewrite leaves it alone; the bag-item cells'
/// 열기 rewrite (`bagbox_popup`) is the only host use of it.
const TAG_ITEM_SHORT: u8 = 0x69;
const ROWS_ITEM_SHORT: i8 = 3;
/// Native row counts for those tags — part of the gate, because `CHAR+0x9c87` is
/// NEVER CLEARED (`free_popup` at guest 0x12d77c zeroes only the count), so a stale
/// tag from a closed popup would otherwise pass.
const ROWS_ITEM: i8 = 8;
const ROWS_SKILL: i8 = 7;
/// Row counts after our rewrite (used for the idempotency half of the gate).
const OUR_ROWS_ITEM: i8 = 4;
const OUR_ROWS_SKILL: i8 = 3;

/// Each label buffer is a 10-byte allocation (guest 0x12ce54 `movs r0,#0xa`).
const POPUP_LABEL_CAP: usize = 10;

// The commit mechanism — KTF's `popup_key(kind, -5)` call and LGT's synthesised `OK` —
// is a per-build table now: `guest_build::popup_commit()` / `PopupCommit`.

/// `퀵슬롯등록` in EUC-KR + NUL = 11 bytes (player, 2026-08-06: the feature is called
/// 퀵슬롯, not 퀵바, everywhere).
///
/// ★ This is ONE BYTE PAST the 10-byte label allocation (guest 0x12ce54 `movs r0,#0xa`),
/// and that is a deliberate, evidenced choice rather than an oversight. The GUEST
/// writes an 11-byte string into a buffer from this same allocator on every skill
/// popup it opens — `포인트투자` is also 5 syllables + NUL — so an 11-byte write here
/// has exactly the footprint the game itself has always had. A 10-byte-clean
/// alternative exists (`슬롯등록`, 9 bytes) and was rejected only because it is not
/// what was asked for; switch to it if this is ever suspected of corrupting a
/// neighbouring block.
const LABEL_QUICKSLOT: [u8; 11] = [0xc4, 0xfc, 0xbd, 0xbd, 0xb7, 0xd4, 0xb5, 0xee, 0xb7, 0xcf, 0x00];
/// `1키등록` + NUL — the label row 1 must ALREADY hold for the gate to pass. This is
/// the semantic check that no combination of stale bytes can fake.
const LABEL_KEY1: [u8; 8] = [0x31, 0xc5, 0xb0, 0xb5, 0xee, 0xb7, 0xcf, 0x00];

/// Skill "is this skill learned" table: `(i8)*(CHAR + 0x1a8 + (cat*9 + idx)*8)`.
/// `<= 0` means unlearned, and `skill_bind`'s guard at guest 0x145cbe routes that to
/// the UNBIND path — i.e. asking to register an unlearned skill silently clears the
/// slot instead. The host refuses instead of offering it.
const SKILL_LEARNED_BASE: u32 = 0x1a8;
const SKILL_CAT_OFF: u32 = 0x9c80;
const SKILL_CURSOR_BASE: u32 = 0x9c7d;

/// What the host needs to know about a live, rewritable register popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopupView {
    /// `kind` for `popup_key` / `popup_choose` (2 = item page, 4 = skill page).
    pub kind: u8,
    /// True for the item popup, false for the skill popup.
    pub item: bool,
    /// The DISPLAY row the guest is currently highlighting (0..rows-1).
    pub sel: i8,
    /// How many rows our rewritten list has (4 item / 3 skill).
    pub rows: i8,
    /// Whether row 1 (`퀵슬롯등록`) may actually be taken. False for a skill the
    /// character has not learned — see [`SKILL_LEARNED_BASE`].
    pub can_bind: bool,
}

/// Base of `popup_handle[8]`, or `None` when this build's array has not been located
/// (or, on KTF, when the slot does not hold a heap address yet).
///
/// Build-aware since 2026-08-11 (7차): KTF keeps the array behind a data slot, LGT
/// compiles the array itself into `.bss` — the same disagreement as the CHAR record.
/// See `guest_build::PopupHandles` for the LGT hunt and why it unblocks three matrix
/// rows at once.
fn popup_handles(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u32> {
    match crate::guest_build::popup_handles()? {
        crate::guest_build::PopupHandles::Pointer { global } => {
            let base = rd_u32(read, global)?;
            in_heap(base).then_some(base)
        }
        // A static array needs no validation of its own: it is always mapped, and
        // "is a popup open" is answered by the HANDLES being non-NULL, which is
        // exactly what `popup_label_addr` tests one line later.
        crate::guest_build::PopupHandles::Static(base) => Some(base),
    }
}

/// Payload address of label buffer `i` — `*(handle[i]) + 8` on KTF, the pointer
/// itself on LGT.
///
/// ★ Resolved through the HANDLE array every time, never through the `popup_text`
/// cache: `free_popup` NULLs the handles but leaves the text cache pointing at freed
/// memory, so the cache is a dangling read after any close.
///
/// The hop count is [`deref_container`]'s business, not this function's: it is the
/// carrier runtime's pointer convention, the same one that settles the inventory grid
/// and the AREA object. Verified on LGT by dumping the array with a popup open — the
/// pointers land directly on the EUC-KR text with no allocator header in front.
fn popup_label_addr(read: &impl Fn(u32, usize) -> Vec<u8>, handles: u32, i: u32) -> Option<u32> {
    deref_container(read, handles + i * 4)
}

/// Is the game showing a context popup we may rewrite, and in what state?
///
/// Every clause below is load-bearing; the two subtle ones are the row count (the
/// tag is never cleared, so tag alone lets a closed popup through) and the label
/// check (the only test that cannot be satisfied by stale state).
pub fn popup_view(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<PopupView> {
    let c = resolve_char(read)?;
    // 1) a menu overlay owns the screen. NOT the window id at CHAR+0xa0c0 — guest
    //    0x124208 tests this byte and branches away BEFORE the window id is loaded,
    //    so the window id can still read 3 (field) with a menu up.
    if rd_u8(read, c + SELECTOR_FLAG_OFF)? == 0 {
        return None;
    }
    // 2) the page, which is also `popup_key`'s `kind`.
    let kind = rd_u8(read, c + MENU_PAGE_OFF)?;
    if kind != PAGE_ITEM && kind != PAGE_SKILL {
        return None;
    }
    // 3) a popup is open.
    if rd_u8(read, c + POPUP_OPEN_OFF)? == 0 {
        return None;
    }
    // 4) …of a flavour that HAS register rows.
    let tag = rd_i8(read, c + POPUP_TAG_OFF)? as u8;
    let item = match (tag, kind) {
        (TAG_ITEM, PAGE_ITEM) => true,
        (TAG_SKILL, PAGE_SKILL) => false,
        _ => return None,
    };
    // 5) the row count is either the native one or the one WE wrote (idempotent).
    let (native_rows, our_rows) = if item {
        (ROWS_ITEM, OUR_ROWS_ITEM)
    } else {
        (ROWS_SKILL, OUR_ROWS_SKILL)
    };
    let rows = rd_i8(read, c + POPUP_COUNT_OFF)?;
    if rows != native_rows && rows != our_rows {
        return None;
    }
    // 6) none of this build's inventory-page pre-emptors is set. Per-build because the
    //    gate only serves a commit that BYPASSES the game's key router (KTF's direct
    //    call); LGT's synthetic OK rides the router itself, and its list is empty —
    //    the KTF offsets read unrelated LGT fields (see the anchors table).
    if item {
        for off in crate::guest_build::popup_preempt_offsets() {
            if rd_u8(read, c + off)? != 0 {
                return None;
            }
        }
    }
    // 7+8) the label buffers exist AND row 1 reads either the game's `1키등록` or the
    //      `퀵슬롯등록` we already wrote. This is the check that makes the whole gate
    //      trustworthy rather than merely plausible.
    let handles = popup_handles(read)?;
    let row1 = popup_label_addr(read, handles, 1)?;
    // Our label is 11 bytes — one past the allocation, see `LABEL_QUICKSLOT` — so the
    // idempotency read has to be long enough to see all of it, not `POPUP_LABEL_CAP`.
    let got = read(row1, POPUP_LABEL_CAP.max(LABEL_QUICKSLOT.len()));
    if got.len() < LABEL_KEY1.len() {
        return None;
    }
    let is_native = got[..LABEL_KEY1.len()] == LABEL_KEY1;
    let is_ours = got.len() >= LABEL_QUICKSLOT.len() && got[..LABEL_QUICKSLOT.len()] == LABEL_QUICKSLOT;
    if !is_native && !is_ours {
        return None;
    }
    // 9) items: the bag double-indirect must resolve — `popup_choose`'s item arm
    //    derefs it unguarded (guest 0x13c996/0x13c998).
    if item && resolve_grid_base(read, c).is_none() {
        return None;
    }
    // 9b) …and the cell must not be one of the three BAG-ITEM cells (2026-09-14): their
    //     popup is the host's one-row 열기 (`bagbox_popup`), and this rewrite must never
    //     put 사용하기 / 퀵슬롯등록 on it — row 0 forwarded would run the DELISTED cash
    //     arm the cell was renamed from.
    if item {
        let tab = rd_u8(read, c + INV_TAB_OFF)?;
        let slot = rd_u8(read, c + INV_CURSOR_BASE + u32::from(tab))?;
        if let Some(cell) = orb_cell(read, tab, slot)
            && crate::bagbox::is_bag_item(cell.cat, cell.idx)
        {
            return None;
        }
    }
    // 10) skills: may this one actually be bound?
    let can_bind = if item {
        true
    } else {
        let cat = u32::from(rd_u8(read, c + SKILL_CAT_OFF)?);
        let cursor = rd_i8(read, c + SKILL_CURSOR_BASE + cat)?;
        if cursor < 0 {
            return None;
        }
        let idx = cat * 9 + cursor as u32;
        rd_i8(read, c + SKILL_LEARNED_BASE + idx * 8).is_some_and(|v| v > 0)
    };
    Some(PopupView {
        kind,
        item,
        sel: rd_i8(read, c + POPUP_SEL_OFF)?,
        rows: our_rows,
        can_bind,
    })
}

/// The writes that turn the game's list into ours. Empty when it is already ours.
///
/// The tail labels (`상세보기` / `버리기` / `등록취소`) are COPIED OUT OF THE GUEST'S
/// OWN BUFFERS rather than hardcoded, so this survives a build whose string resource
/// differs. Returned as (address, bytes) so the caller does the poking and can log it.
pub fn popup_relabel(read: &impl Fn(u32, usize) -> Vec<u8>, view: &PopupView) -> Vec<(u32, Vec<u8>)> {
    // Same per-item gate as `popup_sel_addr`: never OFFER 퀵슬롯등록 on a build that
    // cannot then run it. Every write below is a data write — into the game's own label
    // allocations (reached through the per-build `popup_handles` anchor) and the count
    // byte — with no build-specific address anywhere.
    if crate::guest_build::popup_commit().is_none() {
        return Vec::new();
    }
    let Some(c) = resolve_char(read) else {
        return Vec::new();
    };
    let Some(handles) = popup_handles(read) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    // Row 1 becomes 퀵슬롯등록. Skipped when it already is — this runs every frame the
    // popup is up (other windows write the count byte), so it has to be idempotent.
    if let Some(a) = popup_label_addr(read, handles, 1) {
        let cur = read(a, LABEL_QUICKSLOT.len());
        if cur.len() != LABEL_QUICKSLOT.len() || cur != LABEL_QUICKSLOT {
            out.push((a, LABEL_QUICKSLOT.to_vec()));
        }
    }
    // The native tail rows slide down into rows 2.. .
    let moves: &[(u32, u32)] = if view.item { &[(2, 6), (3, 7)] } else { &[(2, 6)] };
    for (dst, src) in moves {
        let (Some(da), Some(sa)) = (popup_label_addr(read, handles, *dst), popup_label_addr(read, handles, *src)) else {
            continue;
        };
        let text = read(sa, POPUP_LABEL_CAP);
        // Copy up to and including the terminator; a buffer with no NUL in 10 bytes is
        // not something the game produced, so leave it alone.
        let Some(n) = text.iter().position(|b| *b == 0) else { continue };
        let bytes = text[..=n].to_vec();
        if read(da, bytes.len()) != bytes {
            out.push((da, bytes));
        }
    }
    // …and the count shrinks, which shrinks the drawn box and the cursor clamp too.
    let want = view.rows as u8;
    if rd_u8(read, c + POPUP_COUNT_OFF) != Some(want) {
        out.push((c + POPUP_COUNT_OFF, vec![want]));
    }
    out
}

/// Turn a DISPLAY row into the guest row `popup_choose` expects. `slot` is the GUEST
/// quickbar slot the binding is going into, and is only consulted for display row 1.
///
/// ⛔ Not a PLATE (2026-09-04): the game's own binder writes `CHAR+0x252+slot`, so a
/// plate index past the guest's five would be refused here — and the `< QB_SLOTS`
/// test below is what refuses it. The caller (`InputEvent::PopupBindSlot`) turns the
/// plate the player tapped into a guest slot with `quickpage::Pages::reserve` first.
///
/// Returns `None` for a row that should be passed through untouched (display 0) or
/// that does not exist.
pub fn popup_guest_row(view: &PopupView, display: i8, slot: u8) -> Option<u8> {
    match display {
        // ★ `then`, not `then_some`: `then_some` evaluates its argument EAGERLY, so
        // `slot + 1` overflowed for slot 255 before the range test could reject it
        // (debug panic; a silent wrap in release). Caught by the selftest.
        1 => (slot < QB_SLOTS as u8).then(|| slot + 1),
        2 => Some(6),
        3 => view.item.then_some(7),
        _ => None,
    }
}

/// The live ACTION POPUP, for any flavour — not just the two the host relabels.
///
/// ★ 79차-C. [`popup_view`] answers only for the register popups (tag `0x66`/`0x67`),
/// because everything it carries is about rewriting their rows. Tapping a popup row
/// needs far less and needs it for EVERY popup: how many rows are on screen (so the
/// pixel scan can be told what to look for) and which one is highlighted (so a second
/// tap on the same row can mean "take it").
///
/// The row count is also the gate: `CHAR+0x9c87` (the tag) is never cleared —
/// `free_popup` zeroes only the count — so "a popup is up" is `open != 0` AND a count
/// inside the label-buffer domain, never the tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopupList {
    /// Rows currently drawn (`CHAR+0x9c85`), 1..=8.
    pub rows: u8,
    /// The highlighted row (`CHAR+0x9c86`).
    pub sel: i8,
}

pub fn popup_list(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<PopupList> {
    // Same gate as every other popup read: no commit mechanism on this build means the
    // host must not drive the popup at all.
    crate::guest_build::popup_commit()?;
    let c = resolve_char(read)?;
    if rd_u8(read, c + POPUP_OPEN_OFF)? == 0 {
        return None;
    }
    let rows = rd_i8(read, c + POPUP_COUNT_OFF)?;
    // The painter allocates eight label buffers (`0x3fa46`) and draws `rows` of them,
    // so anything outside 1..=8 is not a popup this host understands.
    let rows = u8::try_from(rows)
        .ok()
        .filter(|r| (1..=crate::menutouch::POPUP_MAX_ROWS as u8).contains(r))?;
    Some(PopupList {
        rows,
        sel: rd_i8(read, c + POPUP_SEL_OFF)?,
    })
}

/// `(address, row)` for moving the popup highlight to `row`, or `None` when the popup
/// is gone or `row` is not one of its rows.
///
/// ⛔ REFUSES rather than clamps, and re-reads the live count to do it. The game's own
/// range check on this byte lives in its key arm; the register path writes it with no
/// bound at all (the same shape as the menu-cursor trap in `TOUCH_MENU_FEASIBILITY.md`
/// §7-1, which opened a screen unreachable from the menu).
pub fn popup_select_write(read: &impl Fn(u32, usize) -> Vec<u8>, row: u8) -> Option<(u32, u8)> {
    let live = popup_list(read)?;
    (row < live.rows).then_some(())?;
    Some((popup_sel_addr(read)?, row))
}

/// The address of the highlighted-row byte, for the commit poke. Separate from
/// [`popup_view`] so the caller re-resolves CHAR at commit time rather than trusting
/// a pointer captured a frame earlier.
pub fn popup_sel_addr(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u32> {
    // Gated on "does this build know how to COMMIT a row" rather than on the blanket
    // flag (2026-08-11, §3.6 discipline): the write itself needs no build-specific
    // address, only an offset, and `POPUP_SEL_OFF` is verified on both builds — LGT read
    // 0 with the cursor on row 0 and took a poked 1 as row 1, which is what made the
    // whole rewrite portable (see `guest_build::PopupCommit`). Offering the row and then
    // being unable to run it is the combination that must not ship, and that is exactly
    // what `popup_commit()` answers.
    crate::guest_build::popup_commit()?;
    resolve_char(read).map(|c| c + POPUP_SEL_OFF)
}

/// `Some((address, native_row_count))` when the popup's row count is currently OURS and
/// has to be put back before a [`crate::guest_build::PopupCommit::SyntheticOk`] commit.
///
/// ★ Only the synthetic-OK path needs this, and the reason is a TICK. Our relabel shrinks
/// `CHAR+0x9c85` to 4 (or 3), while the row we commit can be as high as 5 (plate 4 →
/// guest row 5). A direct guest call reads the row byte with no guest code running in
/// between; a synthesised keypress is delivered on the next tick, and the guest's own
/// cursor clamp could pull an out-of-range row back inside our shrunken count first.
/// Restoring the native count removes the window — the popup is about to close anyway,
/// and the count byte is the one this rewrite already owns.
pub fn popup_count_restore(read: &impl Fn(u32, usize) -> Vec<u8>, view: &PopupView) -> Option<(u32, u8)> {
    let c = resolve_char(read)?;
    let native = if view.item { ROWS_ITEM } else { ROWS_SKILL } as u8;
    (rd_u8(read, c + POPUP_COUNT_OFF)? != native).then_some((c + POPUP_COUNT_OFF, native))
}

// ── in-game 오브결합 / 소켓확장 ──────────────────────────────────────────────
//
// The rules and the record map live in `crate::orbcombine` (dependency-free, so the
// standalone selftest can `include!` it). Everything here is the part that needs guest
// memory: finding the popup, reading cells, and turning a decision into writes.
//
// ★ Why this can exist at all: the 60-byte BAG record carries the complete socket state
// (refine / total / orb id / filled), the save serialises exactly those for every
// equipment item, and the game's own `0x528f0` rebuilds the two derived halfwords from
// them. So the whole operation is three record bytes plus two calls into the game's own
// code — no server, no wire record, no warehouse.

/// The game's own item popup, when it is the two-row one an ORB opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrbPopup {
    pub tab: u8,
    pub slot: u8,
    /// The orb's own instance key. `(tab, slot)` is NOT an identity — the game compacts
    /// the bag (`0x5380c`, seventeen call sites) — so this is what the commit matches on.
    pub key: u64,
    /// The orb's DB index — `0x0f` is 소켓확장, everything else a real orb.
    pub orb_idx: u16,
    /// The stack in that cell, so the caller can spend exactly one unit.
    pub count: u16,
    /// Our extra row is already installed.
    pub ours: bool,
}

/// Is the game showing a bag ORB's context popup?
///
/// Every clause is load-bearing, and the shape mirrors [`popup_view`] — but this is a
/// DIFFERENT widget and must not be routed through that one. `popup_view` gates on tag
/// 0x66 with 8 native rows (the consumable popup 51차 rewrites); the inventory popup
/// builder sends every category except 0x12 down the TWO-row tag-0x64 branch instead
/// (`0x3fac4 cmp r3,#0x11; ble`), which is the one an orb gets.
///
/// The reason a host row is safe here is that `popup_choose` dispatches tag 0x64 for
/// rows 0 and 1 ONLY (`0x47590`-`0x4759e`) and then `popup_key` closes and FREES the
/// popup unconditionally (`0x47b3a`-`0x47b54`). So committing our row runs no guest code
/// and leaks no heap — we simply learn that it happened.
pub fn orb_popup(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<OrbPopup> {
    crate::guest_build::item_refresh()?;
    let c = resolve_char(read)?;
    // 1) a menu overlay owns the screen, 2) on the item page, 3) with a popup open.
    if rd_u8(read, c + SELECTOR_FLAG_OFF)? == 0 || rd_u8(read, c + MENU_PAGE_OFF)? != PAGE_ITEM || rd_u8(read, c + POPUP_OPEN_OFF)? == 0 {
        return None;
    }
    // 4) …of the plain two-row flavour.
    if rd_i8(read, c + POPUP_TAG_OFF)? as u8 != crate::orbcombine::POPUP_TAG_PLAIN {
        return None;
    }
    // 5) the row count is the game's own or the one WE wrote (idempotent — this runs
    //    every frame the popup is up).
    let rows = rd_i8(read, c + POPUP_COUNT_OFF)?;
    let ours = rows == crate::orbcombine::POPUP_ROWS_OURS;
    if rows != crate::orbcombine::POPUP_ROWS_PLAIN && !ours {
        return None;
    }
    // 6) the cell the popup is anchored to really holds an orb. `popup_choose` reads the
    //    cursor the same way (guest 0x13c99e/0x13c9b0), so this is the same cell the
    //    game would act on.
    let tab = rd_u8(read, c + INV_TAB_OFF)?;
    let slot = rd_u8(read, c + INV_CURSOR_BASE + u32::from(tab))?;
    let cell = orb_cell(read, tab, slot)?;
    if !cell.is_orb() {
        return None;
    }
    // 7) the label buffers exist — the only check stale bytes cannot fake.
    let handles = popup_handles(read)?;
    popup_label_addr(read, handles, crate::orbcombine::POPUP_ROW_COMBINE as u32)?;
    Some(OrbPopup {
        tab,
        slot,
        key: cell.key,
        orb_idx: cell.idx,
        count: cell.count,
        ours,
    })
}

/// The writes that add our 오브결합 row. Empty when it is already there.
///
/// Row 2's buffer is written first and the count byte last: until the count grows, the
/// paint loop never reads that buffer, so there is no frame on which the game could draw
/// whatever the allocator left in it.
pub fn orb_popup_writes(read: &impl Fn(u32, usize) -> Vec<u8>, p: &OrbPopup) -> Vec<(u32, Vec<u8>)> {
    if p.ours {
        return Vec::new();
    }
    let (Some(c), Some(handles)) = (resolve_char(read), popup_handles(read)) else {
        return Vec::new();
    };
    let Some(label) = popup_label_addr(read, handles, crate::orbcombine::POPUP_ROW_COMBINE as u32) else {
        return Vec::new();
    };
    vec![
        (label, crate::orbcombine::LABEL_ORB_COMBINE.to_vec()),
        (c + POPUP_COUNT_OFF, vec![crate::orbcombine::POPUP_ROWS_OURS as u8]),
    ]
}

/// Is the highlighted popup row OURS? Read on the OK key, before the guest sees it.
pub fn orb_row_selected(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    resolve_char(read)
        .and_then(|c| rd_i8(read, c + POPUP_SEL_OFF))
        .is_some_and(|sel| sel == crate::orbcombine::POPUP_ROW_COMBINE)
}

/// Decode one bag cell. `None` for an unresolvable grid, an out-of-range cell, or an
/// empty one — an empty cell is not a thing the player can pick or spend.
pub fn orb_cell(read: &impl Fn(u32, usize) -> Vec<u8>, tab: u8, slot: u8) -> Option<crate::orbcombine::Cell> {
    let c = resolve_char(read)?;
    let grid = resolve_grid_base(read, c)?;
    let rec = crate::orbcombine::rec_addr(grid, tab, slot)?;
    if rd_u8(read, rec + crate::orbcombine::REC_OCCUPIED)? == 0 {
        return None;
    }
    let name = read(rec + crate::orbcombine::REC_NAME, 0x1e);
    let name = name.split(|b| *b == 0).next().unwrap_or(&[]).to_vec();
    let key = {
        let b = read(rec + crate::orbcombine::REC_KEY, 8);
        if b.len() != 8 {
            return None;
        }
        u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    };
    Some(crate::orbcombine::Cell {
        tab,
        slot,
        key,
        cat: rd_u16(read, rec + crate::orbcombine::REC_CAT)?,
        idx: rd_u16(read, rec + crate::orbcombine::REC_IDX)?,
        count: rd_u16(read, rec + crate::orbcombine::REC_COUNT)?,
        refine: rd_i8(read, rec + crate::orbcombine::REC_REFINE)?,
        sockets: rd_u8(read, rec + crate::orbcombine::REC_SOCKETS)?,
        orb: rd_i8(read, rec + crate::orbcombine::REC_ORB)?,
        filled: rd_i8(read, rec + crate::orbcombine::REC_FILLED)?,
        // ⛔ READ ONLY. `rec+0x3c4` is the worn flag and `+0x3c7` the equip slot; a stray
        // non-`-1` there makes the item permanently undepositable (guest gate `0x6f28`).
        equipped: rd_i8(read, rec + crate::orbcombine::REC_EQUIPPED)? != -1,
        name,
    })
}

/// Every equipment item in the bag that the picker may OFFER, in bag order.
///
/// Ineligible pieces are INCLUDED — the dialog shows them greyed with the reason, because
/// "my sword is not in the list" is indistinguishable from a broken feature while "홈이
/// 없는 장비입니다" explains itself.
///
/// ★ 70차 (2026-08-25): WORN gear is included again. 69차 dropped it here on the same
/// day's earlier request; the follow-up feedback reversed that — *"착용중인 장비에
/// 사용하면 confirm팝업이 뜨도록 조치"* — so a worn item now shows with its `[장착]`
/// marker, disabled, and CONFIRMING it pops the game-style message box with the reason
/// (`orbcombine::Row::notice` → `app.rs::orb_notice`).
///
/// ⛔ This is PRESENTATION ONLY. `orb_apply` re-reads and re-validates through
/// `orbcombine::eligible`, which refuses `equipped` on its own — what a row looks like
/// must never be the thing that keeps a write from happening.
pub fn orb_targets(read: &impl Fn(u32, usize) -> Vec<u8>) -> Vec<crate::orbcombine::Cell> {
    let Some(c) = resolve_char(read) else { return Vec::new() };
    let Some(grid) = resolve_grid_base(read, c) else { return Vec::new() };
    // The LIVE bag count, clamped: `grid+0x1668` is where the count byte sits precisely
    // because `0x394 + 5*0x3c4` lands there, so a value above 5 would read into it.
    let bags = rd_i8(read, grid + crate::orbcombine::GRID_BAG_COUNT)
        .unwrap_or(0)
        .clamp(0, crate::orbcombine::MAX_TABS as i8) as u8;
    let mut out = Vec::new();
    for tab in 0..bags {
        for slot in 0..crate::orbcombine::SLOTS_PER_TAB {
            if let Some(cell) = orb_cell(read, tab, slot)
                && cell.is_equipment()
            {
                out.push(cell);
            }
        }
    }
    out
}

/// A snapshot of the player's bag for the offline engine: how many cells are free, and
/// what each occupied one holds.
///
/// ★★ Why the engine needs it (2026-08-25): a withdraw hands the bag a `05 04` frame and
/// the guest's `addItemToInventory` (0x131d0) fills a matching stack to 99 then spills into
/// a free cell — but with NEITHER it sets the bag-full flag at `grid+0x1669` and branches
/// straight to its epilogue (0x13252-0x1325a -> 0x13810), skipping the `ldr r0,[sp,#0x28]`
/// that loads the leftover. It therefore returns 0xff, the `05 04` handler's single retry
/// (0x13d28) hits the same wall, and its return is discarded — the units are gone, and the
/// host has already taken them out of the warehouse. Bounding the offer needs this.
///
/// `free_cells: None` = could not be read, which [`crate::offline::bag_room`] treats as
/// "no limit known" so a failed read can never refuse a legitimate withdraw.
pub fn bag_view(read: &impl Fn(u32, usize) -> Vec<u8>) -> crate::offline::BagView {
    let blank = crate::offline::BagView::default();
    let (Some(c),) = (resolve_char(read),) else { return blank };
    let Some(grid) = resolve_grid_base(read, c) else { return blank };
    let Some(bags) = rd_i8(read, grid + crate::orbcombine::GRID_BAG_COUNT) else {
        return blank;
    };
    let bags = bags.clamp(0, crate::orbcombine::MAX_TABS as i8) as u8;
    let mut cells = Vec::new();
    let mut free = 0u32;
    for tab in 0..bags {
        for slot in 0..crate::orbcombine::SLOTS_PER_TAB {
            let Some(rec) = crate::orbcombine::rec_addr(grid, tab, slot) else {
                continue;
            };
            let Some(occupied) = rd_u8(read, rec + crate::orbcombine::REC_OCCUPIED) else {
                // An unreadable cell makes the whole snapshot untrustworthy: counting it
                // as free would let the engine promise room that may not exist.
                return blank;
            };
            if occupied == 0 {
                free += 1;
                continue;
            }
            let (Some(cat), Some(idx), Some(n)) = (
                rd_u16(read, rec + crate::orbcombine::REC_CAT),
                rd_u16(read, rec + crate::orbcombine::REC_IDX),
                rd_u16(read, rec + crate::orbcombine::REC_COUNT),
            ) else {
                return blank;
            };
            cells.push((cat, idx, n.max(1)));
        }
    }
    crate::offline::BagView {
        free_cells: Some(free),
        cells,
    }
}

/// First `(cat, idx)` of `wanted` that is not already in the bag, or `None` if all are
/// present, the bag cannot be read, or there is no free cell for it. One call per guest
/// give (`config::Grant`, the worker's grant arm): the bag is re-read every poll, so the
/// give the game just performed is what stops the next one.
/// What the maintenance grant should do this poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantStep {
    /// Hand this row to the game's giver.
    Give(u16, u16),
    /// Every wanted row is in the bag.
    Done,
    /// Rows are still missing but the bag has no free cell (or cannot be read): keep
    /// the file, say so, and try again later.
    ///
    /// ★ 141차: this used to be folded into `None` together with `Done`, so a grant
    /// larger than the bag was declared complete — and its file deleted — the moment
    /// the bag filled up. The 신규 아이템 set (legacy 78 + 조합서 78 + …) is more than
    /// twice a bag, which is exactly that case.
    Wait,
}

/// Is a `(cat, idx)` in the bag right now? (The grant's "it landed" test.)
pub fn bag_has(read: &impl Fn(u32, usize) -> Vec<u8>, cat: u16, idx: u16) -> bool {
    bag_view(read).cells.iter().any(|&(c, i, _)| c == cat && i == idx)
}

pub fn grant_step(read: &impl Fn(u32, usize) -> Vec<u8>, wanted: &[(u16, u16)]) -> GrantStep {
    let view = bag_view(read);
    let missing = wanted.iter().copied().find(|&(cat, idx)| !view.cells.iter().any(|&(c, i, _)| c == cat && i == idx));
    match (missing, view.free_cells) {
        (None, _) => GrantStep::Done,
        (Some(row), Some(free)) if free > 0 => GrantStep::Give(row.0, row.1),
        _ => GrantStep::Wait,
    }
}

/// Put a `1` in every occupied bag cell whose stack count is 0. Returns the addresses to
/// write, so the caller owns the guest write and this stays testable from a `read`.
///
/// ★★★ 72차 — THE RETROACTIVE HALF OF THE «오브 0개» FIX.
///
/// The guest patch in `emulator::give_item_count_patch` stops NEW orbs from being born
/// with a count of 0, but a player who already used 오브원석 has those zeros in their save
/// (the serialiser carries occupied cells verbatim), and a 0-count orb is refused by
/// [`orb_apply`] — so it prints «0» and cannot be combined, forever.
///
/// This is safe precisely because the state is unreachable by the game itself: the only
/// code that lowers a count is `useItem`'s tail `0x55450`, which calls the cell-clear
/// `0x53648` the instant the count reaches 0. An occupied cell with a count of 0 is
/// therefore always damage, never a legitimate intermediate.
///
/// ⛔ Equipment (`cat <= 0x0c`) is excluded: its count is written as a literal 1 by
/// `0x52c5c` and it never stacks, so a zero there would mean something else entirely and
/// is not ours to guess at.
/// ⛔ Kept OUT of `orb_apply`'s write plan on purpose — `orbcombine_selftest`'s exhaustive
/// corpus asserts that combining touches only the six bytes the save carries, and this is
/// a different, unrelated repair.
pub fn zero_count_repairs(read: &impl Fn(u32, usize) -> Vec<u8>) -> Vec<u32> {
    let mut out = Vec::new();
    let Some(c) = resolve_char(read) else { return out };
    let Some(grid) = resolve_grid_base(read, c) else { return out };
    let Some(bags) = rd_i8(read, grid + crate::orbcombine::GRID_BAG_COUNT) else {
        return out;
    };
    for tab in 0..bags.clamp(0, crate::orbcombine::MAX_TABS as i8) as u8 {
        for slot in 0..crate::orbcombine::SLOTS_PER_TAB {
            let Some(rec) = crate::orbcombine::rec_addr(grid, tab, slot) else {
                continue;
            };
            let (Some(occupied), Some(cat), Some(n)) = (
                rd_u8(read, rec + crate::orbcombine::REC_OCCUPIED),
                rd_u16(read, rec + crate::orbcombine::REC_CAT),
                rd_u16(read, rec + crate::orbcombine::REC_COUNT),
            ) else {
                continue;
            };
            if occupied != 0 && n == 0 && cat > crate::orbcombine::CAT_EQUIP_MAX {
                out.push(rec + crate::orbcombine::REC_COUNT);
            }
        }
    }
    out
}

/// The whole mutation, as `(writes, refresh args)`.
///
/// `refresh args` is `(tab, slot, cat, idx)` for the TARGET — the argument tuple the
/// game's own save loader passes to `0x528f0` / `0x5283c` right after restoring these
/// same bytes (`0x1a3f4` / `0x1a40a`). The caller makes those two guest calls; doing the
/// arithmetic in the host instead would mean re-deriving the orb-power table and the
/// per-category item DB and hoping the two never drift.
///
/// ⛔⛔ BOTH SIDES ARE MATCHED ON THEIR INSTANCE KEY, not on `(tab, slot)`. The game
/// COMPACTS the bag — `0x5380c` bubbles empty slots to the end by swapping adjacent
/// 60-byte records and its wrapper `0x55048` is reached from seventeen places, including
/// the item popup's own close path — so the cell the player chose from can hold a
/// different item by the time they answer. Re-reading and re-checking "is there still an
/// orb here" is NOT enough; the key is the identity that travels with the record.
///
/// `None` for any mismatch, and the caller does nothing: the orb is kept.
#[allow(clippy::type_complexity)]
pub fn orb_apply(
    read: &impl Fn(u32, usize) -> Vec<u8>,
    orb_at: (u8, u8, u64),
    target_at: (u8, u8, u64),
) -> Option<(Vec<(u32, Vec<u8>)>, (u8, u8, u16, u16), crate::orbcombine::Kind)> {
    crate::guest_build::item_refresh()?;
    let c = resolve_char(read)?;
    let grid = resolve_grid_base(read, c)?;
    let orb = orb_cell(read, orb_at.0, orb_at.1)?;
    let target = orb_cell(read, target_at.0, target_at.1)?;
    if !orb.same_item(orb_at.2) || !target.same_item(target_at.2) {
        log::warn!("orb: the bag moved under the dialog — refusing");
        return None;
    }
    if !orb.is_orb() || orb.count == 0 {
        return None;
    }
    let kind = crate::orbcombine::kind_of(orb.idx);
    crate::orbcombine::eligible(&target, kind).ok()?;

    let orb_rec = crate::orbcombine::rec_addr(grid, orb.tab, orb.slot)?;
    let tgt_rec = crate::orbcombine::rec_addr(grid, target.tab, target.slot)?;
    let mut writes: Vec<(u32, Vec<u8>)> = crate::orbcombine::writes(tgt_rec, &target, kind, orb.idx)
        .into_iter()
        .map(|(a, v)| (a, vec![v]))
        .collect();
    if writes.is_empty() {
        return None;
    }
    // ★ The orb is spent LAST. If a write fails half way the player keeps the orb; the
    // other order would take it and give nothing back.
    writes.extend(crate::orbcombine::spend_one(orb_rec, orb.count));
    Some((writes, (target.tab, target.slot, target.cat, target.idx), kind))
}

// ── the bag boxes: 오브가방 · 재료가방 · 소비가방 (2026-09-14) ─────────────────
//
// The rules live in `crate::bagbox` (dependency-free). This is the half that needs guest
// memory: every occupied cell with its position, the canonical EMPTY record for a cell
// the sweep has moved out, the 열기 popup on a bag-item cell, the missing bag items, and
// a combine whose orb comes from the box rather than a bag cell.

/// Every occupied bag cell with its position. `None` when any cell is unreadable — a
/// sweep planned on a partial snapshot could clear a cell it never absorbed.
pub fn bag_cells(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<Vec<crate::bagbox::BagCell>> {
    let c = resolve_char(read)?;
    let grid = resolve_grid_base(read, c)?;
    let bags = rd_i8(read, grid + crate::orbcombine::GRID_BAG_COUNT)?.clamp(0, crate::orbcombine::MAX_TABS as i8) as u8;
    let mut out = Vec::new();
    for tab in 0..bags {
        for slot in 0..crate::orbcombine::SLOTS_PER_TAB {
            let rec = crate::orbcombine::rec_addr(grid, tab, slot)?;
            if rd_u8(read, rec + crate::orbcombine::REC_OCCUPIED)? == 0 {
                continue;
            }
            out.push(crate::bagbox::BagCell {
                tab,
                slot,
                cat: rd_u16(read, rec + crate::orbcombine::REC_CAT)?,
                idx: rd_u16(read, rec + crate::orbcombine::REC_IDX)?,
                count: rd_u16(read, rec + crate::orbcombine::REC_COUNT)?,
            });
        }
    }
    Some(out)
}

/// The writes that make `(tab, slot)` the game's own EMPTY record — byte for byte what
/// `removeItem 0x5340c` leaves (`0x53574`..`0x535c6`): the name zeroed, occupied 0,
/// `+0x20` 0 / `+0x21` -1, cat and idx `0x00ff`, count 1 (which is why a fresh cell
/// inherits "1", 72차), and the twenty bytes from the instance key on set to `0xff`.
///
/// Refuses (`None`) unless the cell still holds `(cat, idx)` with `count`: the sweep
/// read it a moment ago, and the game may have spent it since.
pub fn cell_clear_writes(read: &impl Fn(u32, usize) -> Vec<u8>, cell: &crate::bagbox::BagCell) -> Option<Vec<(u32, Vec<u8>)>> {
    use crate::orbcombine as o;
    let c = resolve_char(read)?;
    let grid = resolve_grid_base(read, c)?;
    let rec = o::rec_addr(grid, cell.tab, cell.slot)?;
    if rd_u8(read, rec + o::REC_OCCUPIED)? == 0
        || rd_u16(read, rec + o::REC_CAT)? != cell.cat
        || rd_u16(read, rec + o::REC_IDX)? != cell.idx
        || rd_u16(read, rec + o::REC_COUNT)? != cell.count
    {
        return None;
    }
    Some(vec![
        (rec + o::REC_NAME, vec![0u8; 0x1e]),
        (rec + o::REC_OCCUPIED, vec![0]),
        (rec + 0x20, vec![0, 0xff]),
        (rec + o::REC_CAT, vec![0xff, 0x00]),
        (rec + o::REC_IDX, vec![0xff, 0x00]),
        (rec + o::REC_COUNT, vec![1, 0]),
        (rec + o::REC_KEY, vec![0xffu8; 0x14]),
    ])
}

/// The game's item popup, open on one of the three BAG-ITEM cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BagBoxPopup {
    pub kind: crate::bagbox::Kind,
    /// Row 0 already reads 열기 and the count is 1.
    pub ours: bool,
}

/// Is the game showing the item popup for a bag-item cell? Same shape as
/// [`orb_popup`], on the OTHER flavour: a cat-0x12 cell opens the eight-row tag-0x66
/// popup (`0x3fac4 cmp r3,#0x11; ble` sends every other category down the two-row one),
/// which the 퀵슬롯등록 rewrite would otherwise take. This is checked first, and once the
/// count reads 1 `popup_view` no longer matches (it wants 8 or 4), so the two never fight.
pub fn bagbox_popup(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<BagBoxPopup> {
    crate::guest_build::popup_commit()?;
    let c = resolve_char(read)?;
    if rd_u8(read, c + SELECTOR_FLAG_OFF)? == 0 || rd_u8(read, c + MENU_PAGE_OFF)? != PAGE_ITEM || rd_u8(read, c + POPUP_OPEN_OFF)? == 0 {
        return None;
    }
    // ★ Two shapes (device, 2026-09-14 S25): a cash-tier consumable (the three renamed
    // rows are ones) opens the THREE-row tag-0x69 popup (사용하기 / 상세보기 / 버리기),
    // not the eight-row register one — `pop open=1 rows=3` in the focus probe while
    // the first build waited for eight and never relabelled.
    let tag = rd_i8(read, c + POPUP_TAG_OFF)? as u8;
    let rows = rd_i8(read, c + POPUP_COUNT_OFF)?;
    let ours = rows == crate::bagbox::POPUP_ROWS_OPEN as i8;
    let native = matches!((tag, rows), (TAG_ITEM, ROWS_ITEM) | (TAG_ITEM_SHORT, ROWS_ITEM_SHORT));
    if !native && !(ours && (tag == TAG_ITEM || tag == TAG_ITEM_SHORT)) {
        return None;
    }
    let tab = rd_u8(read, c + INV_TAB_OFF)?;
    let slot = rd_u8(read, c + INV_CURSOR_BASE + u32::from(tab))?;
    let cell = orb_cell(read, tab, slot)?;
    let kind = crate::bagbox::Kind::from_bag_item(cell.cat, cell.idx)?;
    // The label buffers exist — the one check stale bytes cannot fake.
    let handles = popup_handles(read)?;
    let row0 = popup_label_addr(read, handles, 0)?;
    let got = read(row0, crate::bagbox::LABEL_OPEN.len());
    let ours = ours && got == crate::bagbox::LABEL_OPEN;
    Some(BagBoxPopup { kind, ours })
}

/// The ONLINE screen's panel index — which of 창고's three grids owns the screen.
///
/// ★★★ Measured 2026-09-15 (static, LGT): the online key handler is
/// `0xaaba: ldr r0,[r0]; bl 0x74f8`, where `r0 = base + 0xa430` — a `u32` the «1» and «3»
/// badges rotate through 0..2 (`0xaa60` counts down and wraps to 2, `0xaa7a` counts up
/// and wraps to 0). `0x74f8` then branches on it as its first argument:
/// **`0` reads the BAG record** (`0x7586`: `grid + tab*0x3c4 + slot*0x3c`, occupied byte)
/// — 싱글가방; **`1` reads the warehouse row cache** (`0x75cc`) — 네트워크 창고;
/// anything else is the 캐시샵.
///
/// ⛔ This is the byte that keeps [`bagbox_cell_focused`] honest. All three grids share
/// ONE cell cursor (`CHAR+0x9c72 + page`, 66차-C / 80차-E), so a tap on warehouse row 1
/// lands on the same cursor value as a tap on bag cell 1 — and the bag cell 1 is a
/// bag-item cell the moment the pin has run.
pub const ONLINE_PANEL_OFF: u32 = 0xa458;
// The two panel values a box can open from (0 = the bag's grid, 1 = the warehouse's rows)
// are `bagbox::{ONLINE_PANEL_BAG, ONLINE_PANEL_WAREHOUSE}`, beside their one judge
// `bagbox::Screen::from_shell` — pure, so `bagbox_selftest` can pin what it refuses.

/// The client's cache of the warehouse page it is showing: `CHAR+0xa57c`, one 20-byte
/// WIRE RECORD per row (`cat +8`, `idx +9`, `count +10` — 72차), with the live row count
/// at `CHAR+0xa56a`.
///
/// ★★ Measured 2026-09-15 alongside [`ONLINE_PANEL_OFF`]: the warehouse arm of the online
/// key handler reads `base + 0xa39c + cursor*0x14 + 0x1c0` for the row's category
/// (`0x75f4`..`0x7602`) after refusing a cursor the count does not cover (`0x75d8`), and
/// `base + 0xa39c + 0x1a6` is that count. `base = CHAR + 0x28`, so the record base is
/// `CHAR + 0xa584 - 8 = 0xa57c` — which is the address the repo already had for these rows.
pub const WH_ROWS_OFF: u32 = 0xa57c;
pub const WH_COUNT_OFF: u32 = 0xa56a;
pub const WH_ROW_LEN: u32 = 20;

pub fn online_panel(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u32> {
    let c = resolve_char(read)?;
    rd_u32(read, c + ONLINE_PANEL_OFF)
}

/// `(cat, idx)` of the warehouse row the shared cell cursor is on, or `None` when the
/// cursor is past the rows the client has (the same bound the game checks at `0x75d8`).
pub fn wh_row_at_cursor(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u16, u16)> {
    let c = resolve_char(read)?;
    let tab = rd_u8(read, c + INV_TAB_OFF)?;
    let cursor = u32::from(rd_u8(read, c + INV_CURSOR_BASE + u32::from(tab))?);
    // ⛔ SIGNED, like the game's own `ldrsh` at `0x75d8`: a count read as unsigned would
    // let a negative one wave every cursor through.
    let count = rd_u16(read, c + WH_COUNT_OFF)? as i16;
    if count <= 0 || cursor >= count as u32 {
        return None;
    }
    let row = c + WH_ROWS_OFF + cursor * WH_ROW_LEN;
    Some((u16::from(rd_u8(read, row + 8)?), u16::from(rd_u8(read, row + 9)?)))
}

/// Is the game's own CELL CURSOR sitting on one of the three bag-item cells?
///
/// ★★ 2026-09-15 — this is what makes the boxes work in the NETWORK WAREHOUSE (user:
/// *"네트워크 창고에서도 3종 가방 동작해야하는데 … 싱글가방, 네트워크 창고가방 둘다"*).
/// [`bagbox_popup`] can only speak for the inventory page: the online screens run with
/// `MENU_PAGE_OFF == 0` (66차-C) and raise their own popup, whose tag this host has never
/// measured. This asks the one question that is true on every screen instead — "is the
/// cursor on our cell" — and the worker opens the window WITHOUT the game's popup at all.
///
/// ⛔ Reads the LIVE bytes, so the caller must already know the tap it is acting on hit
/// the focused cell (`MenuTapAction::Activate` does — that is the whole meaning of the
/// second tap). ⛔ The bag record is what decides, not the cursor value: a stale cursor
/// simply names a cell that is not one of the three.
/// [`bagbox_cell_focused`] plus the reason, for the one log line that has to answer
/// "why did tapping my 오브가방 cell not open it".
///
/// ★★ 2026-09-15. The 66차-C rule: a refusal that does not say which term refused it
/// costs a screenshot and a round trip (80차-B said the same thing about `menu_tap`'s
/// own REFUSED line). Every term of the gate is in the string.
pub fn bagbox_focus_report(read: &impl Fn(u32, usize) -> Vec<u8>) -> String {
    let shell = shell_state(read);
    let screen = shell_screen(read);
    let panel = online_panel(read);
    let tab = resolve_char(read).and_then(|c| rd_u8(read, c + INV_TAB_OFF));
    let cur = resolve_char(read)
        .zip(tab)
        .and_then(|(c, t)| rd_u8(read, c + INV_CURSOR_BASE + u32::from(t)));
    let cell = resolve_char(read)
        .zip(tab.zip(cur))
        .and_then(|(_, (t, s))| orb_cell(read, t, s))
        .map(|c| format!("{:#x}/{:#x}", c.cat, c.idx));
    let wh = wh_row_at_cursor(read).map(|(c, i)| format!("{c:#x}/{i:#x}"));
    let page = resolve_char(read).and_then(|c| rd_u8(read, c + MENU_PAGE_OFF));
    let selector = resolve_char(read).and_then(|c| rd_u8(read, c + SELECTOR_FLAG_OFF));
    let shop = (window_id(read) == Some(crate::bagbox::SHOP_WINDOW)).then(|| shop_state(read)).flatten();
    format!(
        "shell={shell} screen={screen:?} panel={panel:?} shop={shop:?} page={page:?} selector={selector:?} tab={tab:?} cur={cur:?} bagcell={} whrow={}",
        cell.as_deref().unwrap_or("-"),
        wh.as_deref().unwrap_or("-")
    )
}

///
/// ★★ 96차: the answer carries WHICH SCREEN, because that decides which SET of boxes the
/// cell opens — the bag's own, or (on the warehouse grid) the WAREHOUSE's, which is a view
/// of the offline store — and which verbs the rows get (`bagbox::verbs`).
pub fn bagbox_cell_focused(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(crate::bagbox::Screen, crate::bagbox::Kind)> {
    // ⛔ On the ONLINE screens the cell cursor is SHARED by all three grids
    // (`CHAR+0x9c72 + page`, 66차-C / 80차-E), so which one is on screen has to be asked
    // — see [`ONLINE_PANEL_OFF`]. Fail closed: a panel byte we cannot read there means
    // the tap goes to the game, which is what it did before this existed.
    //
    // ⛔⛔ And app state 3 alone is NOT the online screen (2026-09-16): the title, both
    // slot lists and the creation screens are app state 3 too (66차-C), and the panel
    // byte and the cell cursor keep their last value there. The shell's screen word
    // (`SHELL_SCREEN_ONLINE`, the same gate the slot-list tap arm keys on — 80차-P) is
    // what says the grids are on screen. The judge is `Screen::from_shell`, pure and
    // pinned by `bagbox_selftest`; the failure it guards against is written there.
    // ★ 105차: the window id joins the three facts, because IN THE WORLD it is what tells
    // the shop's sell grid (`bagbox::SHOP_WINDOW`) from the inventory page. It is read
    // even in the shell, where it is simply ignored — `from_shell` only looks at it on the
    // `!shell` arm, and keeping the call unconditional keeps this line one expression.
    let screen = crate::bagbox::Screen::from_shell(
        shell_state(read),
        shell_screen(read) == Some(SHELL_SCREEN_ONLINE),
        online_panel(read),
        window_id(read),
    )?;
    // ⛔⛔ 117차-D: the SHOP is two grids behind one window id and one shared cursor, and
    //   only the game's own state byte says which is up. FAIL CLOSED — unreadable, or
    //   anything but the sell grid (the buy grid = 2, its menus, its confirms), and the tap
    //   or key is the game's. See [`SHOP_STATE_OFF`].
    if screen == crate::bagbox::Screen::Shop && shop_state(read)? != SHOP_STATE_SELL_GRID {
        return None;
    }
    if screen == crate::bagbox::Screen::OnlineWarehouse {
        // 창고 grid: the three openers are VIRTUAL ROWS the host put at the head of
        // page 0 (`offline::opener_rows`), so the row's own `(cat, idx)` is the
        // answer — it is not a bag cell at all.
        let (cat, idx) = wh_row_at_cursor(read)?;
        let kind = crate::bagbox::Kind::from_bag_item(cat, idx)?;
        return Some((screen, kind));
    }
    let c = resolve_char(read)?;
    // ⛔⛔ IN-GAME, ONLY WHILE THE INVENTORY PAGE IS UP (2026-09-16, S9 vc331: a second
    // tap on the MENU list's focused 가방 row opened the 오브가방 window instead of the
    // inventory). The two cursor bytes below belong to the inventory page and keep
    // their last value on every other screen — and their DEFAULT is cell 0 of bag 1,
    // which is exactly where the pin (93차) seats the 오브가방. So without this gate
    // every "already focused -> OK" tap on the menu list, STATUS, SKILL … of a freshly
    // loaded character became "open the 오브가방". Same two bytes `inv_cursor_writes`
    // and `bagbox_popup` gate on.
    if screen == crate::bagbox::Screen::Inventory && (rd_u8(read, c + SELECTOR_FLAG_OFF)? == 0 || rd_u8(read, c + MENU_PAGE_OFF)? != PAGE_ITEM) {
        return None;
    }
    let tab = rd_u8(read, c + INV_TAB_OFF)?;
    let slot = rd_u8(read, c + INV_CURSOR_BASE + u32::from(tab))?;
    let cell = orb_cell(read, tab, slot)?;
    let kind = crate::bagbox::Kind::from_bag_item(cell.cat, cell.idx)?;
    Some((screen, kind))
}

/// The OK key is on its way to the guest and the cursor is on one of the three bag-item
/// cells of an ONLINE screen — so the host opens the box and the key is SWALLOWED.
///
/// ★★★ 96차-B (2026-09-15, 제보: *"패드 이용자들은 터치를 잘 이용 안 하기 때문에 OK 키로도
/// 열기가 가능해야 해"*). In the inventory the game's own popup is relabelled to 열기 and
/// [`bagbox_popup`] answers for it. **That cannot be done on the online screens**, and the
/// disassembly says why: the online key handler's OK arm (`0x7558`, panel 0 at `0x75b2`;
/// panel 1 at `0x76d0`) writes only the SHARED popup bytes —
/// `CHAR+0x9c84 = 1` ([`POPUP_OPEN_OFF`], the very byte `0x751a` gates the next key on)
/// and `CHAR+0x9c85 = 3` ([`POPUP_COUNT_OFF`], via the shared tail `0x77ae`) — and writes
/// **no tag and no labels at all**. Its three rows (상세보기 / 버리기 / 올리기) therefore
/// do not come from the eight popup label buffers the in-game relabel rewrites, so there
/// is nothing there to turn into 열기.
///
/// ⛔ So the key is taken BEFORE the guest runs that arm. Nothing is lost: on our cells
/// the game's own three verbs are all refused by the game itself (the three are priced 0,
/// which is what gates 버리기 / 올리기 — 72차), and on the warehouse grid they are the
/// VIRTUAL opener rows, whose key `05 04`/`05 05` the engine refuses with text (95차-B).
/// ⛔ The INVENTORY is deliberately not here — it already works through the relabel, and
/// this would turn its two-press flow into one.
/// ⛔ And only while the game has NO popup up (`POPUP_OPEN_OFF == 0`): with one open the
/// OK belongs to its rows, not to us.
pub fn bagbox_open_key(read: &impl Fn(u32, usize) -> Vec<u8>, cell_cursor: bool) -> Option<(crate::bagbox::Screen, crate::bagbox::Kind)> {
    let (screen, kind) = bagbox_cell_focused(read)?;
    // ⛔ The INVENTORY is still out, and for its own reason: the relabelled 열기 popup
    // already answers the key there, and taking it here would collapse its two presses
    // into one.
    if screen == crate::bagbox::Screen::Inventory {
        return None;
    }
    // ★★★ 109차 — THE SHOP IS IN, BUT ONLY WITH THE PIXEL PROOF (제보 2026-09-19:
    // *"아이템 판매시 3종 가방을 터치로 탭하면 가방이 열리는데 OK버튼으로는 「아이템을 팔 수
    // 없습니다.」 라고 뜨고있어"* — that message is the GAME refusing to sell a price-0 cell,
    // i.e. the key went straight through to it).
    //
    // 105차 refused this screen outright because 구매 and 판매 are the SAME window id and
    // share one cell cursor, so "the cursor names one of our three cells" is true on the
    // buy list too — and swallowing OK there would stop the player buying. That reasoning
    // is unchanged; what changes is that the key no longer has to guess. The UI publishes
    // whether the game is drawing a bag CELL cursor (`crate::cell_cursor_shown`), which is
    // the same evidence the tap path cross-checks against, and a row list never draws one.
    //
    // ⛔ FAIL CLOSED: no cursor drawn (or the flag never set — a build whose UI does not
    // publish it) means the key goes to the game, which is exactly 105차's behaviour.
    if screen == crate::bagbox::Screen::Shop && !cell_cursor {
        return None;
    }
    let c = resolve_char(read)?;
    if rd_u8(read, c + POPUP_OPEN_OFF)? != 0 {
        return None;
    }
    Some((screen, kind))
}

/// Turn the bag-item popup into the single 열기 row. Label first, count last (until the
/// count shrinks the painter draws the game's own eight rows, none of them ours).
pub fn bagbox_popup_writes(read: &impl Fn(u32, usize) -> Vec<u8>, p: &BagBoxPopup) -> Vec<(u32, Vec<u8>)> {
    if p.ours {
        return Vec::new();
    }
    let (Some(c), Some(handles)) = (resolve_char(read), popup_handles(read)) else {
        return Vec::new();
    };
    let Some(row0) = popup_label_addr(read, handles, 0) else {
        return Vec::new();
    };
    vec![
        (row0, crate::bagbox::LABEL_OPEN.to_vec()),
        (c + POPUP_COUNT_OFF, vec![crate::bagbox::POPUP_ROWS_OPEN]),
    ]
}

/// One line of everything that could have removed a bag-item cell — the popup (open?
/// tag / rows / selected row), the inventory cursor and what sits under it, the window id
/// and app state. Read at the moment a cell is seen to be GONE (`BagBoxHost::frame`), so
/// the next vanish report answers itself instead of costing a build (TODO `CP` ⓑ).
pub fn bagbox_vanish_report(read: &impl Fn(u32, usize) -> Vec<u8>) -> String {
    let c = resolve_char(read);
    let open = c.and_then(|c| rd_u8(read, c + POPUP_OPEN_OFF));
    let tag = c.and_then(|c| rd_i8(read, c + POPUP_TAG_OFF));
    let rows = c.and_then(|c| rd_i8(read, c + POPUP_COUNT_OFF));
    let sel = popup_list(read).map(|p| p.sel);
    format!(
        "popup open={open:?} tag={tag:?} rows={rows:?} sel={sel:?} | window={:?} field={} | {}",
        window_id(read),
        in_field_state(read),
        bagbox_focus_report(read)
    )
}

/// The bag-item cells the bag does NOT hold, in [`crate::bagbox::Kind::ALL`] order, and
/// how many free cells there are to put them in. `None` when the bag cannot be read.
///
/// ★ 2026-09-16: ALL of them at once, not the first — the grant used to give one cell per
/// 30-frame poll, so a freshly created character (potions swept, three cells to grant)
/// needed three quiet field polls before the third cell even existed, and a player who
/// opened the bag inside those two seconds saw the three "unpinned" (TODO `CP` ⓐ). The
/// caller gives as many as fit and pins them the moment they land.
pub fn bagbox_missing_all(read: &impl Fn(u32, usize) -> Vec<u8>, kinds: &[crate::bagbox::Kind]) -> Option<(Vec<(u16, u16)>, usize)> {
    let cells = bag_cells(read)?;
    let bags = {
        let c = resolve_char(read)?;
        let grid = resolve_grid_base(read, c)?;
        rd_i8(read, grid + crate::orbcombine::GRID_BAG_COUNT)?.clamp(0, crate::orbcombine::MAX_TABS as i8) as usize
    };
    let free = (bags * usize::from(crate::orbcombine::SLOTS_PER_TAB)).saturating_sub(cells.len());
    let missing = kinds
        .iter()
        .map(|k| k.bag_item())
        .filter(|&(cat, idx)| !cells.iter().any(|c| c.cat == cat && c.idx == idx))
        .collect();
    Some((missing, free))
}

/// The writes that SWAP two bag records, plus the quickbar fixups that swap goes with.
///
/// ★★ 2026-09-14 — what puts the three bag-item cells in bag 1, cells 1–3 (user: *"항상
/// 인벤토리의 맨첫번째 가방 1,2,3번 칸에 있어야해. 고정이야 이건"*). `giveItem` puts
/// them wherever `findSlot 0x529a0` finds room, so an existing character gets them behind
/// its equipment; [`crate::bagbox::pin_plan`] says which swaps fix that and this is how
/// they are made.
///
/// A record is `SLOT_STRIDE` bytes and carries everything about the item — name, flags,
/// instance key, sockets — so the swap is a straight byte exchange. Two things do NOT
/// travel inside the record and are fixed up here:
///
/// ⛔ **The quickbar's item refs are `(tab << 5) | slot`** (`0x466b4`, and the game's own
/// compaction rewrites them for exactly this reason). A plate pointing at either endpoint
/// has to follow its item, or it fires on whatever moved in.
///
/// Refuses (`None`) unless BOTH records still read as the caller last saw them — the
/// destination may legitimately be empty, which is why it is matched on `occupied` rather
/// than on a `(cat, idx)` the caller does not have.
pub fn cell_swap_writes(read: &impl Fn(u32, usize) -> Vec<u8>, from: (u8, u8), from_item: (u16, u16), to: (u8, u8)) -> Option<Vec<(u32, Vec<u8>)>> {
    use crate::orbcombine as o;
    let c = resolve_char(read)?;
    let grid = resolve_grid_base(read, c)?;
    let a = o::rec_addr(grid, from.0, from.1)?;
    let b = o::rec_addr(grid, to.0, to.1)?;
    if a == b {
        return None;
    }
    // The source must still be the item the plan named.
    if rd_u8(read, a + o::REC_OCCUPIED)? == 0 || rd_u16(read, a + o::REC_CAT)? != from_item.0 || rd_u16(read, a + o::REC_IDX)? != from_item.1 {
        return None;
    }
    let (ra, rb) = (read(a, o::SLOT_STRIDE as usize), read(b, o::SLOT_STRIDE as usize));
    if ra.len() != o::SLOT_STRIDE as usize || rb.len() != o::SLOT_STRIDE as usize {
        return None;
    }
    let mut writes = vec![(a, rb), (b, ra)];

    // ── the quickbar refs ──────────────────────────────────────
    let refb = |(tab, slot): (u8, u8)| (tab << 5) | (slot & 0x1f);
    let (ref_a, ref_b) = (refb(from), refb(to));
    for i in 0..QB_SLOTS {
        if rd_u8(read, c + QB_TYPE_OFF + i)? != TY_ITEM {
            continue;
        }
        let now = rd_u8(read, c + QB_ITEMREF_OFF + i)?;
        let next = if now == ref_a {
            ref_b
        } else if now == ref_b {
            ref_a
        } else {
            continue;
        };
        writes.push((c + QB_ITEMREF_OFF + i, vec![next]));
    }
    Some(writes)
}

// ── 136차 (2026-09-25): 정렬 ──────────────────────────────────────────────────

/// Which screen may offer the 「정렬」 button right now, or `None`.
///
/// In-game: the INVENTORY page with nothing over it ([`inventory_page_up`]: overlay up,
/// item page, no action popup, no window) and no game box or modal. On the shell: the
/// 네트워크 screen (3) with a loaded character, no popup, no modal, on the 싱글가방 (0) or
/// 창고 (1) panel — the cash shop (2) has nothing to sort. Fail closed everywhere: an
/// unreadable byte is `None`, and the worker asks again on the tap.
pub fn sort_screen(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<crate::bagsort::SortScreen> {
    use crate::bagsort::SortScreen;
    if modal_id(read).unwrap_or(1) != 0 {
        return None;
    }
    if shell_state(read) {
        if shell_screen(read) != Some(SHELL_SCREEN_ONLINE) || !online_screen(read) {
            return None;
        }
        let c = resolve_char(read)?;
        if rd_u8(read, c + POPUP_OPEN_OFF)? != 0 {
            return None;
        }
        return match online_panel(read)? {
            crate::bagbox::ONLINE_PANEL_BAG => Some(SortScreen::OnlineBag),
            crate::bagbox::ONLINE_PANEL_WAREHOUSE => Some(SortScreen::OnlineWarehouse),
            _ => None,
        };
    }
    (inventory_page_up(read) && !game_box_up(read)).then_some(SortScreen::Inventory)
}

/// Is the 네트워크 창고 panel the screen — the one place the `*` key opened the gold
/// deposit box (「금액을 설정하세요」) that 136차 retired with its `✱` hint?
pub fn warehouse_panel_up(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    shell_state(read)
        && shell_screen(read) == Some(SHELL_SCREEN_ONLINE)
        && online_screen(read)
        && online_panel(read) == Some(crate::bagbox::ONLINE_PANEL_WAREHOUSE)
}

/// What one bag sort writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BagSort {
    /// Record bytes, quickbar item refs and the cursor follow — apply in order.
    pub writes: Vec<(u32, Vec<u8>)>,
    /// Occupied cells, before.
    pub cells: usize,
    /// How many of them change place.
    pub moved: usize,
    /// Where the item under the cell cursor ends up, when the cursor was on one.
    pub cursor_to: Option<(u8, u8)>,
    /// `(from, to)` for every cell whose item changes place — what the host's own
    /// quickbar plates follow (`quickpage::Pages::remap_item_cells`).
    pub moves: Vec<((u8, u8), (u8, u8))>,
}

/// The writes that put the bag in `bagsort::layout` order — the whole bag in one pass.
///
/// Every occupied record is read in full (`SLOT_STRIDE` bytes) and lands, byte for byte,
/// at its new cell; a cell that ends up empty gets the game's own empty pattern applied to
/// its current bytes (what `removeItem 0x5340c` leaves — see [`cell_clear_writes`]). The
/// three bag-item cells are the pins (`bagbox::Kind::ALL`, seats 1·2·3 of bag 1). Two
/// things travel outside the record and are fixed up here, exactly as [`cell_swap_writes`]
/// does: the quickbar's item refs (`(tab<<5)|slot`) and the cell cursor, which follows the
/// item it was on. `None` when anything cannot be read — nothing is half-written.
///
/// ⛔ Read-then-write in ONE worker step: the guest only runs inside `tick`, so nothing
/// moves between the read and the write.
pub fn bag_sort_writes(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<BagSort> {
    use crate::orbcombine as o;
    let c = resolve_char(read)?;
    let grid = resolve_grid_base(read, c)?;
    let bags = rd_i8(read, grid + o::GRID_BAG_COUNT)?.clamp(1, o::MAX_TABS as i8) as u8;
    let stride = o::SLOT_STRIDE as usize;
    let mut recs: Vec<(u32, Vec<u8>)> = Vec::with_capacity(usize::from(bags) * 16);
    let mut cells: Vec<crate::bagsort::Cell> = Vec::new();
    for tab in 0..bags {
        for slot in 0..o::SLOTS_PER_TAB {
            let rec = o::rec_addr(grid, tab, slot)?;
            let bytes = read(rec, stride);
            if bytes.len() != stride {
                return None;
            }
            if bytes[o::REC_OCCUPIED as usize] != 0 {
                let u16at = |off: u32| u16::from_le_bytes([bytes[off as usize], bytes[off as usize + 1]]);
                let mut key = [0u8; 8];
                key.copy_from_slice(&bytes[o::REC_KEY as usize..o::REC_KEY as usize + 8]);
                cells.push(crate::bagsort::Cell {
                    tab,
                    slot,
                    cat: u16at(o::REC_CAT),
                    idx: u16at(o::REC_IDX),
                    count: u16at(o::REC_COUNT),
                    refine: bytes[o::REC_REFINE as usize] as i8,
                    key: u64::from_le_bytes(key),
                });
            }
            recs.push((rec, bytes));
        }
    }
    let pinned: Vec<(u16, u16)> = crate::bagbox::Kind::ALL.iter().map(|k| k.bag_item()).collect();
    let plan = crate::bagsort::layout(&cells, &pinned, bags, &crate::item_levels::item_level);
    if plan.len() != cells.len() {
        // Cannot happen (the cells came out of these very bags), but a plan that drops a
        // cell would be a lost item — refuse rather than write it.
        return None;
    }
    let at = |tab: u8, slot: u8| usize::from(tab) * usize::from(o::SLOTS_PER_TAB) + usize::from(slot);
    let mut writes = Vec::new();
    let mut moved = 0usize;
    for (dest, &(rec, ref cur)) in recs.iter().enumerate() {
        let (tab, slot) = ((dest / 16) as u8, (dest % 16) as u8);
        let want: Vec<u8> = match plan.iter().find(|(d, _)| *d == (tab, slot)) {
            Some((_, i)) => recs[at(cells[*i].tab, cells[*i].slot)].1.clone(),
            None => {
                // The game's own empty record, over this cell's present bytes.
                let mut e = cur.clone();
                e[..0x1e].fill(0);
                e[o::REC_OCCUPIED as usize] = 0;
                e[0x20] = 0;
                e[0x21] = 0xff;
                e[o::REC_CAT as usize..o::REC_CAT as usize + 2].copy_from_slice(&[0xff, 0x00]);
                e[o::REC_IDX as usize..o::REC_IDX as usize + 2].copy_from_slice(&[0xff, 0x00]);
                e[o::REC_COUNT as usize..o::REC_COUNT as usize + 2].copy_from_slice(&[1, 0]);
                e[o::REC_KEY as usize..].fill(0xff);
                e
            }
        };
        if want != *cur {
            writes.push((rec, want));
        }
    }
    let mut moves = Vec::new();
    for (d, i) in &plan {
        if (cells[*i].tab, cells[*i].slot) != *d {
            moved += 1;
            moves.push(((cells[*i].tab, cells[*i].slot), *d));
        }
    }
    // The quickbar's item refs follow their items (the compaction's own `0x466b4` rule).
    for i in 0..QB_SLOTS {
        if rd_u8(read, c + QB_TYPE_OFF + i)? != TY_ITEM {
            continue;
        }
        let now = rd_u8(read, c + QB_ITEMREF_OFF + i)?;
        let next = crate::bagsort::remap_ref(now, &cells, &plan);
        if next != now {
            writes.push((c + QB_ITEMREF_OFF + i, vec![next]));
        }
    }
    // The cell cursor follows the item it was on (same bytes on the inventory page and
    // the online 싱글가방 — `bagbox_cell_focused` reads them there too).
    let cur_tab = rd_u8(read, c + INV_TAB_OFF)?;
    let cur_slot = rd_u8(read, c + INV_CURSOR_BASE + u32::from(cur_tab))?;
    let cursor_to = crate::bagsort::moved_to(cur_tab, cur_slot, &cells, &plan);
    if let Some((t, s)) = cursor_to
        && (t, s) != (cur_tab, cur_slot)
    {
        writes.push((c + INV_TAB_OFF, vec![t]));
        writes.push((c + INV_CURSOR_BASE + u32::from(t), vec![s]));
    }
    Some(BagSort { writes, cells: cells.len(), moved, cursor_to, moves })
}

/// `Some((gameName, slotName))` when the host may run the full save chain on the
/// 네트워크 싱글가방 screen after a 정렬 (136차) — [`save_gate`]'s online twin.
///
/// Same chain, same record names, same slot-word bound; what differs is the place. The
/// field gate (`in_field_state`, the AREA byte) is replaced by: shell screen 3 with a
/// loaded character, the 싱글가방 panel, no popup — where the game itself runs `saveGame`
/// after every deposit, so the JVM/thread context is the one it uses. ⛔ Not on the two
/// slot lists (`summary_repair_gate`'s reason: the game's own OK is mid-`readSaveRecord`
/// there). The write still goes through `vault::write_check` like every other.
pub fn sort_save_gate(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u32, u32)> {
    let chain = crate::guest_build::save_chain()?;
    let c = resolve_char(read)?;
    if !shell_state(read) || shell_screen(read) != Some(SHELL_SCREEN_ONLINE) || !online_screen(read) {
        return None;
    }
    if online_panel(read)? != crate::bagbox::ONLINE_PANEL_BAG || rd_u8(read, c + POPUP_OPEN_OFF)? != 0 {
        return None;
    }
    if rd_u32(read, c + chain.slot_off)? > crate::save::SAVE_SLOT_MAX {
        return None;
    }
    let (game, slot) = match chain.names {
        crate::guest_build::SaveNames::SlPtrs { game, slot } => (rd_u32(read, game)?, rd_u32(read, slot)?),
        crate::guest_build::SaveNames::Direct { game, slot } => (game, slot),
    };
    (crate::save::plausible_ptr(game) && crate::save::plausible_ptr(slot)).then_some((game, slot))
}

/// The first bag cell holding `(cat, idx)` — where a materialised unit landed
/// (`giveItem` merges into an existing stack of the kind before it opens a cell, so
/// "first" is also "the stack the game would have topped up").
pub fn bag_cell_of(read: &impl Fn(u32, usize) -> Vec<u8>, cat: u16, idx: u16) -> Option<(u8, u8, u16)> {
    bag_cells(read)?
        .into_iter()
        .find(|c| c.cat == cat && c.idx == idx)
        .map(|c| (c.tab, c.slot, c.count))
}

/// Units of `(cat, idx)` the BAG holds, summed across cells — what a materialise
/// compares before and after, since `giveItem` may merge into an existing stack instead
/// of opening a cell. `None` when the bag cannot be read.
pub fn bag_units_of(read: &impl Fn(u32, usize) -> Vec<u8>, cat: u16, idx: u16) -> Option<u32> {
    Some(
        bag_cells(read)?
            .iter()
            .filter(|c| c.cat == cat && c.idx == idx)
            .map(|c| u32::from(c.count))
            .sum(),
    )
}

/// Free bag cells — how many stacks a materialise may push back. `None` when the bag
/// cannot be read (⛔ not 0: a failed read must not look like a full bag).
pub fn bag_free_cells(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<usize> {
    let cells = bag_cells(read)?;
    let c = resolve_char(read)?;
    let grid = resolve_grid_base(read, c)?;
    let bags = rd_i8(read, grid + crate::orbcombine::GRID_BAG_COUNT)?.clamp(0, crate::orbcombine::MAX_TABS as i8) as usize;
    Some((bags * usize::from(crate::orbcombine::SLOTS_PER_TAB)).saturating_sub(cells.len()))
}

/// Append `name` to the game's own ACQUISITION-NOTICE queue — the floating item name a
/// picked-up drop shows (★★★ 103차).
///
/// The queue lives in the inventory grid and `giveItem` / `addItemToInventory` append to
/// it AFTER placing, which is the half the bag-box entry hooks replace: an absorbed orb,
/// material or consumable went into its box without a word while equipment, which the
/// boxes never take, still announced itself. This is the host paying that back, writing
/// exactly the two things the game writes — the text at `queue[count]` and `count + 1`.
///
/// `None` when the grid cannot be read, the queue is full, or the name encodes to
/// nothing; the notice is then simply not shown, never half-written.
///
/// ⛔ Read the count fresh for every notice. Two drops absorbed in the same frame are two
/// ring entries, and a cached count would write both into the same slot.
pub fn bagbox_notice_writes(read: &impl Fn(u32, usize) -> Vec<u8>, name: &str) -> Option<Vec<(u32, Vec<u8>)>> {
    let c = resolve_char(read)?;
    let grid = resolve_grid_base(read, c)?;
    let count = rd_i8(read, grid + crate::bagbox::NOTICE_COUNT_OFF)?;
    let text = crate::gamefont::to_euckr(name);
    let (off, bytes, next) = crate::bagbox::notice_plan(count, &text)?;
    Some(vec![(grid + off, bytes), (grid + crate::bagbox::NOTICE_COUNT_OFF, vec![next as u8])])
}

// ── 127차: a boxed unit is used THROUGH the box's own bag-item cell ──────────

/// The bag-item cell a boxed unit was written over, and the record that goes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BagStage {
    pub tab: u8,
    pub slot: u8,
    /// Absolute guest address of the record — what the restage hook writes to.
    pub rec: u32,
    /// The bag-item record as it was, all `SLOT_STRIDE` bytes.
    pub snapshot: [u8; crate::orbcombine::SLOT_STRIDE as usize],
}

/// The item DB's own name for a non-equipment `(cat, idx)`, EUC-KR, without its NUL —
/// the string `giveItem` copies into a new record (`0x5303e`..`0x530fc`: `*(grid+0x10 +
/// (cat-0xd)*8)` is that category's row table, `row[0]` the name pointer). Row strides
/// are the ones `giveItem`'s six arms use (`0x53040 #0x1c` … `0x530ea #0x18`).
/// `None` for equipment (a different table), an index past what the table could hold, an
/// unreadable pointer, or an empty name.
fn item_db_name(read: &impl Fn(u32, usize) -> Vec<u8>, char_ptr: u32, cat: u16, idx: u16) -> Option<Vec<u8>> {
    let (handle_off, stride): (u32, u32) = match cat {
        0x0d => (0x10, 0x1c),
        0x0e => (0x18, 0x14),
        0x0f => (0x20, 0x20),
        0x10 => (0x28, 0x18),
        0x11 => (0x30, 0x14),
        0x12 => (ITEMDB_HANDLE_OFF, ITEMDB_STRIDE),
        _ => return None,
    };
    if idx > 0xff {
        return None;
    }
    let grid = resolve_grid_base(read, char_ptr)?;
    let db = deref_container(read, grid + handle_off)?;
    let name_ptr = rd_u32(read, db + u32::from(idx) * stride)?;
    // A pointer the emulator cannot read yields no bytes, and no bytes is no name.
    let raw = read(name_ptr, crate::bagbox::STAGE_NAME_LEN);
    let name: Vec<u8> = raw.split(|b| *b == 0).next().unwrap_or(&[]).to_vec();
    (!name.is_empty()).then_some(name)
}

/// The writes that put ONE unit of `(cat, idx)` over the bag-item cell of `kind` — the
/// record `giveItem` would have written (`bagbox::stage_record`), and the snapshot of
/// what was there. The cell is found by CONTENT (the bag-item `(cat, idx)`), and refused
/// (`None`) when the bag cannot be read, the cell is not in the bag, its record does not
/// read as the bag item, or the item DB has no name for the unit — nothing is ever
/// staged over a record this cannot describe.
pub fn bag_stage_writes(read: &impl Fn(u32, usize) -> Vec<u8>, kind: crate::bagbox::Kind, cat: u16, idx: u16) -> Option<(BagStage, Vec<(u32, Vec<u8>)>)> {
    use crate::orbcombine as o;
    let c = resolve_char(read)?;
    let grid = resolve_grid_base(read, c)?;
    let item = kind.bag_item();
    let cell = bag_cells(read)?.into_iter().find(|c| (c.cat, c.idx) == item)?;
    let rec = o::rec_addr(grid, cell.tab, cell.slot)?;
    let raw = read(rec, o::SLOT_STRIDE as usize);
    if raw.len() != o::SLOT_STRIDE as usize || !crate::bagbox::snapshot_is_bag_item(&raw, kind) {
        return None;
    }
    let mut snapshot = [0u8; o::SLOT_STRIDE as usize];
    snapshot.copy_from_slice(&raw);
    let name = item_db_name(read, c, cat, idx)?;
    let bytes = crate::bagbox::stage_record(&name, cat, idx);
    Some((
        BagStage {
            tab: cell.tab,
            slot: cell.slot,
            rec,
            snapshot,
        },
        vec![(rec, bytes.to_vec())],
    ))
}

/// What the staged cell holds now: `None` when the bag cannot be read (or the grid is
/// not the one the stage was taken in), `Some(None)` for an empty cell, else
/// `Some(Some((cat, idx, count)))` of the record there. The restore and the watch both
/// ask this.
pub fn bag_stage_cell(read: &impl Fn(u32, usize) -> Vec<u8>, st: &BagStage) -> Option<Option<(u16, u16, u16)>> {
    use crate::orbcombine as o;
    let c = resolve_char(read)?;
    let grid = resolve_grid_base(read, c)?;
    if o::rec_addr(grid, st.tab, st.slot)? != st.rec {
        // The grid moved (a different world): the address the stage was taken in is not
        // this bag's, and nothing about it can be read as ours.
        return None;
    }
    if rd_u8(read, st.rec + o::REC_OCCUPIED)? == 0 {
        return Some(None);
    }
    Some(Some((
        rd_u16(read, st.rec + o::REC_CAT)?,
        rd_u16(read, st.rec + o::REC_IDX)?,
        rd_u16(read, st.rec + o::REC_COUNT)?,
    )))
}

/// The write that puts the bag-item record back over a staged cell that STILL HOLDS the
/// staged `(cat, idx)` — the host's own restore, for a unit the game did not consume.
/// `None` when the cell reads as anything else (consumed and restored by the hook
/// already, cleared, moved): then there is nothing of ours to overwrite.
pub fn bag_stage_restore_writes(read: &impl Fn(u32, usize) -> Vec<u8>, st: &BagStage, cat: u16, idx: u16) -> Option<Vec<(u32, Vec<u8>)>> {
    match bag_stage_cell(read, st)? {
        Some((c, i, _)) if (c, i) == (cat, idx) => Some(vec![(st.rec, st.snapshot.to_vec())]),
        _ => None,
    }
}

/// Is there at least one free bag cell? `None` when the bag cannot be read.
pub fn bag_has_room(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<bool> {
    let cells = bag_cells(read)?;
    let c = resolve_char(read)?;
    let grid = resolve_grid_base(read, c)?;
    let bags = rd_i8(read, grid + crate::orbcombine::GRID_BAG_COUNT)?.clamp(0, crate::orbcombine::MAX_TABS as i8) as usize;
    Some(cells.len() < bags * usize::from(crate::orbcombine::SLOTS_PER_TAB))
}

/// Point the inventory's own cursor at `(tab, slot)` — the tab byte the page painter
/// reads (`CHAR+0x9c71`) and that tab's cell cursor (`CHAR+0x9c72 + tab`), the two bytes
/// `popup_choose` hands to `useItem` (guest `0x13ce22`). A synthetic OK then opens the
/// game's own popup on that cell. Refuses an out-of-range cell rather than clamping (the
/// 66차 rule: the cursor array is indexed by the page byte).
pub fn inv_cursor_writes(read: &impl Fn(u32, usize) -> Vec<u8>, tab: u8, slot: u8) -> Option<Vec<(u32, Vec<u8>)>> {
    if tab >= crate::orbcombine::MAX_TABS || slot >= crate::orbcombine::SLOTS_PER_TAB {
        return None;
    }
    let c = resolve_char(read)?;
    if rd_u8(read, c + SELECTOR_FLAG_OFF)? == 0 || rd_u8(read, c + MENU_PAGE_OFF)? != PAGE_ITEM {
        return None;
    }
    Some(vec![(c + INV_TAB_OFF, vec![tab]), (c + INV_CURSOR_BASE + u32::from(tab), vec![slot])])
}

/// Is the game's item popup open on `(tab, slot)` with `사용하기` as row 0 — i.e. is the
/// materialised cell ready to be used? Reads the same bytes [`cash_use_pending`] does.
pub fn popup_open_on(read: &impl Fn(u32, usize) -> Vec<u8>, tab: u8, slot: u8) -> bool {
    let Some(c) = resolve_char(read) else { return false };
    if rd_u8(read, c + POPUP_OPEN_OFF).unwrap_or(0) == 0 {
        return false;
    }
    rd_u8(read, c + INV_TAB_OFF) == Some(tab) && rd_u8(read, c + INV_CURSOR_BASE + u32::from(tab)) == Some(slot)
}

/// Is the game holding a box that takes the NEXT OK for itself — the shared choice /
/// message widget ([`CHOICE_FLAG_OFF`]) or a `showModal` box ([`modal_id`])?
///
/// ★★★ 110차 (2026-09-19): 오브원석's `useItem` arm ends by showing the orb it rolled in
/// that widget (S25: `choice=1 paint=195` right after a confirmed use, `0` again after the
/// next OK). The 소비가방 window covers the game panel, so the player never sees it, and the
/// NEXT use's synthetic OK went into closing it instead of opening the cell's popup —
/// `the popup never opened — the unit stays in bag 0,9`, every other use. A potion shows
/// nothing and chained fine, which is why only 오브원석 was reported.
///
/// `false` when CHAR cannot be read (nothing to dismiss is the safe answer: the job then
/// behaves exactly as before this check existed).
pub fn game_box_up(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    let Some(c) = char_base_raw(read) else { return false };
    if rd_u8(read, c + CHOICE_FLAG_OFF).unwrap_or(0) != 0 {
        return true;
    }
    modal_id(read).is_some_and(|m| m != 0)
}

/// [`orb_apply`] for an orb that lives in the 오브가방 rather than a bag cell: the same
/// target writes, and NO spend — the box is the worker's, spent after the writes land.
pub fn orb_apply_from_box(
    read: &impl Fn(u32, usize) -> Vec<u8>,
    orb_idx: u16,
    target_at: (u8, u8, u64),
) -> Option<(Vec<(u32, Vec<u8>)>, (u8, u8, u16, u16), crate::orbcombine::Kind)> {
    crate::guest_build::item_refresh()?;
    let c = resolve_char(read)?;
    let grid = resolve_grid_base(read, c)?;
    let target = orb_cell(read, target_at.0, target_at.1)?;
    if !target.same_item(target_at.2) {
        log::warn!("orb: the bag moved under the dialog — refusing");
        return None;
    }
    let kind = crate::orbcombine::kind_of(orb_idx);
    crate::orbcombine::eligible(&target, kind).ok()?;
    let tgt_rec = crate::orbcombine::rec_addr(grid, target.tab, target.slot)?;
    let writes: Vec<(u32, Vec<u8>)> = crate::orbcombine::writes(tgt_rec, &target, kind, orb_idx)
        .into_iter()
        .map(|(a, v)| (a, vec![v]))
        .collect();
    if writes.is_empty() {
        return None;
    }
    Some((writes, (target.tab, target.slot, target.cat, target.idx), kind))
}

// ── auto-loot ────────────────────────────────────────────────────────────────
//
// The game's "오토루팅(싱글)" cash item (category 0x12 index 0x0d, "아이템 자동 루팅
// (게임종료시 까지)") does exactly ONE thing: set this byte to 1 (guest 0x1540fc,
// the `useItem` jump-table arm for DB index 0x0d). Readers, both in the field:
//   * the per-tick player update (guest 0x141c60) runs the ground-pickup routine
//     0x1527fc when the byte is non-zero,
//   * the OK/'7' key row (guest 0x1242ec) SKIPS its manual pickup attempt when the
//     byte is set (the tick already does it).
// Writers in the whole image: the cash item (=1), CHAR_init at app start (=0), and
// the new-game reset (=0) — nothing else, no bit sharing, and it is NOT serialised
// into the save (hence the item's "until you exit the game" wording). So the host
// can simply keep it at 1: idempotent, and correct for New Game / Continue / death
// / map change without any ordering logic. [RE: workflow wf_477f0b16 QD]
//
// ★ 2026-08-13 — THE OFFSET IS NOW PER BUILD, and that is a bug fix, not tidying. This
// used to read a KTF constant behind `guest_code_addresses_valid()`, which is true only
// on KTF; the APK ships LGT, so on every real device this returned `None` and the flag
// was never set. Worse, the server had delisted the `오토루팅` cash item *because* the
// host was supposed to be doing this, so the player had no way to get auto-loot at all
// (player report 2026-08-13). The item is listed again (`hero3_protocol.AUTOLOOT_IDX`)
// and the offset now comes from `guest_anchors::autoloot_off`, which is `Some(0x2d7)` on
// KTF and `None` on LGT until the LGT byte is actually located — that note records the
// search. Asking the anchor table instead of the global flag is the §3.6 discipline:
// when LGT's offset lands, this function starts working with no other change.

/// `Some((address, value))` when auto-loot is currently OFF and should be forced on;
/// `None` when it is already on, when CHAR is not resolvable yet, or when this build's
/// flag is unknown (in which case the player buys the game's own `오토루팅` item).
///
/// Writes ONLY when the byte reads exactly 0. The value is per build (`1` on KTF, `0x81`
/// on LGT) and may well be a bitfield, so a non-zero byte is left alone rather than
/// overwritten wholesale — the game or an earlier write already owns it.
/// ★ 50차: THE FLAG IS AN ARRAY, indexed by the save slot (`CHAR+0xa410` on LGT — the
/// same word the save chain uses). The host used to write a fixed `CHAR+0x2d8`, which is
/// element **1**, so it only ever armed auto-loot for a character in save slot 1 and did
/// nothing for everyone else — the player report that started this. The address is now
/// computed the way the game's own `useItem` arm computes it
/// (`guest_anchors::AutolootFlag::offset_for`).
pub fn autoloot_to_force(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u32, u8)> {
    // Per-ITEM gate, not the blanket one: this is a guest WRITE into the character
    // record, so it needs the offset AND the value for the build that actually booted.
    let flag = crate::guest_build::autoloot_flag()?;
    let c = resolve_char(read)?;
    // Unreadable = refuse, rather than falling back to element 0: a wrong index here
    // writes a byte somewhere else in the record, which is the failure mode with no
    // symptom.
    let slot = flag.slot_off.map(|off| rd_u32(read, c + off));
    let addr = c + flag.offset_for(slot.unwrap_or(None))?;
    let current = rd_u8(read, addr)?;
    let desired = if crate::game_options::auto_loot() { current | flag.value } else { current & !flag.value };
    (current != desired).then_some((addr, desired))
}

/// Whether this build can have auto-loot forced by the host at all. False means the
/// player has to buy the game's own `오토루팅` item; the UI layer says so once at boot
/// rather than letting the feature vanish silently, which is how it went unnoticed.
pub fn autoloot_forcible() -> bool {
    crate::guest_build::autoloot_flag().is_some()
}

// ── the flag-window watcher, for closing an offset ON A DEVICE ────────────────
//
// Static RE got the LGT auto-loot byte down to a candidate and no further: the game's own
// base is `CHAR+0x28` (accessor thunk 0x1068 -> 0x1503ab0, 647 call sites) and exactly one
// `strb #1` reachable through it lands in `game+0x200..0x300` — `0x51152` into
// `CHAR+0x2dd` — but that arm belongs to a dispatcher that is NOT `useItem`, so it may be
// something else entirely. `guest_anchors::autoloot_off` therefore says `None` for LGT.
//
// The measurement that settles it needs no more disassembly: the byte is written by the
// game's OWN `오토루팅` arm, so buy the item, use it, and watch which byte flips. This
// snapshots a window of single-byte flags and reports every transition, so the answer
// appears in logcat named and timestamped:
//
//     adb -s <serial> logcat -s wie_android:* | findstr flagwatch
//
// Kept in the shipping build on purpose: it is read-only, it only speaks when a byte
// actually changes, and the same procedure closes any of §2.11's remaining 🔶 offsets.
/// ★ 2026-08-13 — the window is the WHOLE character record, not a guessed neighbourhood.
/// The first trial watched `0x280..0x300` because KTF's anchor is `0x2d7`, and the one
/// byte that moved in it (`0x2d7`, to exactly `0x01`, at exactly the right moment) turned
/// out NOT to be the flag — writing it from the host does nothing. A window chosen around
/// the answer you expect can only ever confirm or mislead; it cannot find. Reading 43 KB
/// twice a second is a memcpy, so there is no reason to narrow it.
const FLAGWATCH_LO: u32 = 0x000;
const FLAGWATCH_LEN: usize = 0xa800;
/// Cap on changes reported per sample. A save load rewrites the whole record, and one line
/// carrying thousands of entries is worse than useless; the count still comes through.
const FLAGWATCH_MAX_REPORT: usize = 48;
/// A byte that changes on this many consecutive samples is a COUNTER, not a flag, and is
/// suppressed from then on. Without this the report is unreadable: the first device trial
/// (2026-08-13, S25) drowned in `CHAR+0x2ba` and `CHAR+0x2d4`, which tick every sample.
/// A flag transitions once and so never reaches the limit.
const FLAGWATCH_COUNTER_RUN: u8 = 3;

struct FlagWatch {
    prev: Vec<u8>,
    /// Consecutive samples in which each byte changed; saturates at the limit.
    run: Vec<u8>,
}

static FLAGWATCH: std::sync::Mutex<Option<FlagWatch>> = std::sync::Mutex::new(None);

/// `Some(report)` listing every byte in the watched window that changed since the last
/// call. `None` on the first call (which seeds the baseline), when nothing moved, or when
/// CHAR is not resolvable yet.
///
/// Transitions to 1 from 0 are flagged `<== 0->1` because that is the auto-loot
/// signature; everything else is reported too, since a neighbour moving in step is how
/// you tell a real flag from a byte that belongs to a wider field.
pub fn flag_window_changes(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<String> {
    let c = resolve_char(read)?;
    let now = read(c + FLAGWATCH_LO, FLAGWATCH_LEN);
    if now.len() != FLAGWATCH_LEN {
        return None;
    }
    let mut guard = FLAGWATCH.lock().ok()?;
    let Some(w) = guard.as_mut() else {
        *guard = Some(FlagWatch {
            prev: now,
            run: vec![0; FLAGWATCH_LEN],
        });
        return None;
    };
    let mut hits = String::new();
    let mut muted = 0usize;
    let mut changed = 0usize;
    let mut shown = 0usize;
    for i in 0..FLAGWATCH_LEN {
        let (a, b) = (w.prev[i], now[i]);
        if a == b {
            w.run[i] = 0;
            continue;
        }
        w.run[i] = w.run[i].saturating_add(1);
        if w.run[i] >= FLAGWATCH_COUNTER_RUN {
            muted += 1; // a counter: it has now changed on N samples in a row
            continue;
        }
        changed += 1;
        if shown >= FLAGWATCH_MAX_REPORT {
            continue;
        }
        shown += 1;
        let off = FLAGWATCH_LO + i as u32;
        hits += &format!(
            " CHAR+{off:#05x}({:#x}) {a:#04x}->{b:#04x}{}",
            c + off,
            if a == 0 && b != 0 { " <== SET" } else { "" },
        );
    }
    w.prev = now;
    (changed > 0).then(|| {
        format!(
            "flagwatch ({}) CHAR={c:#x} +{FLAGWATCH_LO:#x}..+{:#x} \
             ({changed} changed, {muted} counters muted{}):{hits}",
            crate::guest_build::name(),
            FLAGWATCH_LO + FLAGWATCH_LEN as u32,
            if changed > shown {
                format!(", {} not shown", changed - shown)
            } else {
                String::new()
            },
        )
    })
}

/// The auto-loot flag's live value, the save slot that selects it, and the whole
/// slot array — because 50차 established that the "three refuted candidates" were never
/// three candidates at all, just elements 0, 6 and (the one being written) 1 of one
/// array. Printing the row is what makes that visible at a glance, and it is what says
/// whether the host armed the element the GAME is going to read.
pub fn autoloot_candidates(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<String> {
    let c = resolve_char(read)?;
    let flag = crate::guest_build::autoloot_flag();
    let slot = flag.and_then(|f| f.slot_off).and_then(|off| rd_u32(read, c + off));
    let live = match flag {
        Some(f) => match f.offset_for(slot) {
            Some(off) => format!("+{off:#x}={:?} (want {:#04x}, bit7 must be set)", rd_u8(read, c + off), f.value),
            None => "no usable slot yet".to_string(),
        },
        None => "unlocated".to_string(),
    };
    let base = flag.map_or(0x2d7, |f| f.base_off);
    let row: Vec<String> = (0..=7).map(|i| format!("{:?}", rd_u8(read, c + base + i))).collect();
    Some(format!(
        "autoloot flag {live} | save slot {slot:?} | slots[0..8] @+{base:#x} = [{}]",
        row.join(" ")
    ))
}

// ── the game's own top-HUD y origin ──────────────────────────────────────────
//
// Every y in the game's TOP band — the buff/status list at the left and the target
// monster's name/level/HP box at the right — is `Y0 + const`, where
// `Y0 = *(*(sl+0x428))`: a plain int global (image value 0). Two independent RE
// passes plus a hand check of all four referencing sites found ONLY READS
// (0x149be8, 0x149dc6, 0x14a716 and 0x159cae — the last one loads it into a local),
// so the host owns that value.
//
// The player asked for our gauges at the extreme top-left with the game's buff list
// BELOW them (2026-07-30). Rather than shoving our HUD sideways, we push the game's
// band down by writing this global. Side effect, and it is inherent: the target
// monster box shares the origin, so it moves down by the same amount.
// [RE: workflow wf_477f0b16 QA/QB (both CONFIRMED) + wf_3503d69e RB]
const SL: u32 = 0x1b_2c40;
const SL_HUD_Y_ORIGIN: u32 = SL + 0x428;

/// `Some(address)` of the y-origin int when it does not already hold `want`.
/// The address is read from the relocated pointer at `sl+0x428`, never hardcoded.
pub fn hud_y_origin_to_set(read: &impl Fn(u32, usize) -> Vec<u8>, want: u32) -> Option<u32> {
    // Guest CODE address / unverified-offset WRITE: KTF only until the LGT
    // equivalent is reverse engineered. See `guest_build`.
    if !crate::guest_build::guest_code_addresses_valid() {
        return None;
    }
    let p = rd_u32(read, SL_HUD_Y_ORIGIN)?;
    if !in_heap(p) {
        return None;
    }
    (rd_u32(read, p)? != want).then_some(p)
}

// ── move the game's buff band sideways ───────────────────────────────────────
//
// The band's x has NO origin global (unlike y): each of its three draws computes
// `LAY[0x30] * i + K` with its own one-byte immediate, where LAY[0x30] is the ui_bm
// frame-3 width (the pitch, 20) and K is a bare Thumb ADD-imm8:
//
//   guest 0x149c2e  `adds r1, #1`  (0x3101)  frame  -> x = w*i + 1
//   guest 0x149c8c  `adds r4, #3`  (0x3403)  icon   -> x = w*i + 3
//   guest 0x149d2c  `adds r1, #1`  (0x3101)  tick number -> x = w*i + w/2 + 1
//
// ALL THREE must move together or the band tears apart (first attempt moved only
// the frame and left the icons at the far left). The relative +2 of the icon is
// preserved. [RE: workflow wf_69d1cba7 RC1, HIGH]
//
// ⚠ Do NOT touch guest 0x149c48 (`movs r3,#0x14`): that multiply by i is the 20-byte
// BUFF RECORD stride, not a coordinate.
//
// ★ 2026-08-11 (10차): the per-build addresses moved into
// `guest_anchors::GuestBuild::band_x_sites`, which is where the LGT trio
// (`0x2372`/`0x23ec`/`0x2450`) and its derivation live. The reasoning above is about the
// GAME, so it stays here at the call site — the same split as `hud_bottom_patch`.

/// Every band-x immediate that is not already at `want` (+ its own delta), as
/// `(address, byte)`. Only instructions that still look like the original
/// `adds rN, #imm8` are offered, so a different build is left alone.
///
/// Not gated on `guest_code_addresses_valid` any more: the per-build table answers "do we
/// know this build's sites" precisely, and the opcode check below is what makes the write
/// safe either way. An empty table means no writes at all.
pub fn band_x_patches(read: &impl Fn(u32, usize) -> Vec<u8>, want: u8) -> Vec<(u32, u8)> {
    let mut out = Vec::new();
    for &(addr, opcode, delta) in crate::guest_build::band_x_sites() {
        let Some(cur) = rd_u16(read, addr) else { continue };
        if cur & 0xff00 != opcode {
            continue; // not the instruction we know
        }
        let target = want.saturating_add(delta);
        if (cur & 0xff) as u8 != target {
            out.push((addr, target));
        }
    }
    out
}

// ── move the game's target-monster box LEFT ──────────────────────────────────
//
// Player request (2026-07-30): "the minimap has to be at the very top-right and the
// monster name/HP/LV box to its LEFT". Both want the same corner, and the game's box
// is RIGHT-EDGE anchored, so the box is what moves.
//
// Its block is guest 0x149db2..0x149f78 inside the field-HUD renderer (entered by the
// `bgt` at 0x149dac; the else-path branches straight to 0x149f7a, the bottom-HUD block
// the host already skips). EVERY x in it is right-anchored off the same two values —
// W = the screen width at [sp,#0x100], and FW = the frame-4 width (85) at [r6]+0x40 —
// and each ends in its OWN add/sub-imm8 on r1:
//
//   0x149df2  subs r1,#2      x = W - FW - 2     the 85x26 frame          -> 0x11aa4c
//   0x149e1e  adds r1,#1      x = W - FW + 1     HP-bar background rect   -> 0x10defc
//   0x149e58  subs r1,#0x2c   x = W - 44         name / level text        -> 0x13ecfc
//   0x149ebc  adds r1,#0xf    x = W - FW + 15    second text run          -> 0x13ecfc
//   0x149f42  adds r1,#0x1f   x = W - FW + 31    HP-bar fill, left end    -> 0x10ec80
//   0x149f68  adds r1,#0x1f   x = W - FW + r7+31 HP-bar fill, right end   -> 0x10ec80
//
// ★ AND THE WHOLE BLOCK EXISTS TWICE. The selector at 0x149d5a-0x149d6a reads
// `(s8)CHAR[0x1ea]`: > 0 takes PATH A inline, otherwise `bl #0x14a6aa` acts as a LONG
// BRANCH (0x93a bytes is out of b.n range; LR was already saved by the entry push and
// the block never pops) into PATH B at 0x14a6aa..0x14a88c — a byte-identical second
// copy with its own six immediates, exiting via `b #0x149f7a` or `bl #0x149f6a` into
// path A's shared tail. Patching only path A would move the box for one target kind
// and not the other. [caught by the workflow's two independent derivations after the
// main loop's own sweep covered only path A - both agreed, and both were byte-verified]
//
// ALL TWELVE must move together - the buff band already taught us that patching a
// subset tears the widget apart (the icons stayed at the far left). There is no single
// shared source to patch instead: each site recomputes `W - FW` from scratch. The two
// values they DO share are FW - which is also the blitter's 4bpp source row stride and
// must never be touched (same rule as the band's cell width) - and the screen-width
// local at [sp,#0x100], whose only producer is `ldr r3,[r0,#0x48]; lsls #16; asrs #16;
// str r3,[sp,#0x100]` at 0x149ba6. Overwriting the sign-extend pair there would be a
// 2-halfword single-point fix, but it would also DISCARD the sign extension of a field
// the game itself treats as 16-bit-in-32, so the 12 immediates are the safe route.
//
// Shifting left by N just makes each site's signed constant `orig - N`, re-encoded as
// ADDS when it stays >= 0 and SUBS when it goes negative — both are 16-bit Thumb, so
// the instruction never changes length. Flags are safe: every one of the six is
// immediately followed by another flag-setting ALU op before any conditional.
// [RE: main loop, byte-verified against md5 3792a476; cross-checked by workflow
//  wf_6dac323a-daf]
//
// ★ 2026-08-11 (10차): the per-build address list moved into
// `guest_anchors::GuestBuild::target_box_x_sites`. LGT has TEN sites rather than twelve
// because its two HP-bar-fill draws are SHARED by both paths — see that table.

/// Largest left shift any site can still encode (site 3 binds: 255 - 44).
pub const TARGET_BOX_SHIFT_MAX: u32 = 211;

/// Encode `adds r1,#v` / `subs r1,#-v` for a signed constant, or `None` if it does
/// not fit an imm8.
fn enc_add_sub_r1(v: i32) -> Option<u16> {
    match v {
        0..=255 => Some(0x3100 | v as u16),
        -255..=-1 => Some(0x3900 | (-v) as u16),
        _ => None,
    }
}

/// Every target-box x immediate that is not already encoding a left shift of
/// `shift` guest px, as `(address, halfword_le)`. Idempotent, so the periodic poke
/// pass can re-assert it forever.
pub fn target_box_x_patches(read: &impl Fn(u32, usize) -> Vec<u8>, shift: u32) -> Vec<(u32, [u8; 2])> {
    let mut out = Vec::new();
    if shift > TARGET_BOX_SHIFT_MAX {
        return out;
    }
    for &(addr, orig, base) in crate::guest_build::target_box_x_sites() {
        let Some(cur) = rd_u16(read, addr) else { continue };
        let Some(want) = enc_add_sub_r1(base - shift as i32) else { continue };
        if cur == want {
            continue; // already applied
        }
        // Only rewrite what we recognise: the pristine instruction, or one of our own
        // earlier encodings of the same site (any add/sub-imm8 on r1 — 0x31xx/0x39xx).
        if cur != orig && (cur & 0xf700) != 0x3100 {
            continue;
        }
        out.push((addr, want.to_le_bytes()));
    }
    out
}

// ── shrink the game's buff band ──────────────────────────────────────────────
//
// The band cell's size is read at RUNTIME from the ui_bm frame-3 record:
//   LAY = *(CHAR+0xa384) ; w = u16 @LAY+0x30 (20) ; h = u16 @LAY+0x32 (25).
//
// Poking **h** is SAFE: the blitter uses it only as the DESTINATION ROW COUNT — it
// crops 1:1, it never scales (left-edge clip advances srcX by a destination delta;
// the row loop advances src by w/2 bytes per row) — plus it anchors the tick
// number's y. Nothing outside this loop reads frame 3.
//
// ⚠ NEVER poke **w** (LAY+0x30): it is the 4bpp SOURCE ROW STRIDE as well as the
// cell pitch and the tick-centring divisor. Lowering it shears the graphic
// progressively down the cell, and an odd value diverts into a different blitter.
// [RE: workflow wf_69d1cba7 RC2, HIGH]
//
// Floor: the icon is a hardcoded 15x15 blit at y+5, so below 20 it gets clipped.
// Shrinking also crops the frame's own bottom border row (the alternative — repacking
// the jar asset — would keep it, at the cost of touching the game archive).
const LAY_PTR_OFF: u32 = 0xa384;
const BAND_FRAME3_H_OFF: u32 = 0x32;
pub const BAND_H_MIN: u16 = 20;
pub const BAND_H_ORIG: u16 = 25;

/// `Some((address, bytes))` when the band's cell height is not already `want`.
/// Re-applied for free by the periodic poke pass, which also covers the ui_bm
/// loader rebuilding the record.
pub fn band_height_poke(read: &impl Fn(u32, usize) -> Vec<u8>, want: u16) -> Option<(u32, [u8; 2])> {
    // Guest CODE address / unverified-offset WRITE: KTF only until the LGT
    // equivalent is reverse engineered. See `guest_build`.
    if !crate::guest_build::guest_code_addresses_valid() {
        return None;
    }
    let c = resolve_char(read)?;
    let lay = rd_u32(read, c + LAY_PTR_OFF)?;
    if !in_heap(lay) {
        return None;
    }
    let addr = lay + BAND_FRAME3_H_OFF;
    let cur = rd_u16(read, addr)?;
    // Only ever touch a record that still looks like frame 3.
    if !(BAND_H_MIN..=BAND_H_ORIG).contains(&cur) {
        return None;
    }
    let want = want.clamp(BAND_H_MIN, BAND_H_ORIG);
    (cur != want).then_some((addr, want.to_le_bytes()))
}

// ── stop the game drawing its own BOTTOM field HUD ───────────────────────────
//
// ★ The world IS drawn underneath it. The field paint (guest 0x1248a6..0x1248ba)
// runs the map/world renderers first and calls the HUD renderer 0x149b30 LAST, and
// the HUD's box is only 170 px wide, centred — world pixels are visible either side
// of it on the same rows. So the box is an overlay on top of finished world, which
// is why hiding it by painting black looked wrong: there is real world under there,
// it just gets overwritten. Skipping the draw shows it.
//
// Guest 0x149f7a is exactly where the renderer, having finished the buff band and
// the target-monster box, starts the bottom block: it computes that box's origin
// (`(screen_w - 0xaa)/2 + 2`, `screen_h - 0x18`) and then paints the panel, the
// HP/SP/EXP bars and the 5 quickbar cells — every one of which our overlay already
// replaces. Branching from there to the function's own epilogue (0x14a324) drops
// exactly that block and keeps everything above it.
//
// Checked before adopting: the only non-stack stores in the skipped range
// (0x14a2a0 / 0x14a2bc / 0x14a2d0) are sl-relative HUD blink counters, not game
// state. Nothing else in the range writes CHAR.
//
// ★ 2026-08-10: the LGT build has this block too (renderer 0x22a4, site 0x2864), so
// the three numbers moved into `guest_build::hud_bottom_patch()` — that table carries
// the per-build addresses and how each was found. The reasoning above is about the
// GAME, not the compile, so it stays here at the call site.

/// `Some((address, bytes))` while the game's bottom field HUD is still being drawn.
/// `None` once patched, if the code is not what we expect, or on a build where the
/// block has not been located.
///
/// Not gated on `guest_code_addresses_valid` any more: the per-build table answers
/// "do we know this build's site" precisely, where the flag could only say "is this
/// KTF" — and the byte check below is what makes writing it safe either way.
pub fn hud_bottom_patch(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u32, [u8; 2])> {
    let (site, orig, branch) = crate::guest_build::hud_bottom_patch()?;
    let cur = rd_u16(read, site)?;
    (cur == orig).then_some((site, branch.to_le_bytes()))
}

/// The one-halfword network-warehouse deposit grade patch, `(address, bytes)`, while
/// the site still reads its original `cmp r3,#3`. `None` once patched (idempotent) or
/// on a build without the located site — the same contract as [`hud_bottom_patch`].
/// Lets 영웅-이상 (gold/white cash) equipment into the network warehouse; see
/// `guest_anchors::warehouse_grade_gate` for the full RE and the naming error string.
pub fn warehouse_grade_patch(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u32, [u8; 2])> {
    let (site, orig, patched) = crate::guest_build::warehouse_grade_gate()?;
    let cur = rd_u16(read, site)?;
    (cur == orig).then_some((site, patched.to_le_bytes()))
}

// ── may the host save right now? (see `crate::save`) ─────────────────────────

/// `Some((gameSaveName, slotSaveName))` when it is safe to run the save chain.
///
/// Every clause is the game's own condition, not a guess:
///
/// 1. **CHAR resolves.** Nothing to serialise before the character exists.
/// 2. **The AREA allows saving** — [`area_can_save`], i.e. `(s8)*(area+0x11d) != 0`,
///    which is what both of the game's own save paths test (guest 0x12c828 and
///    0x13c572) before answering "저장할 수 없는 지역입니다".
///
///    ★ Until 2026-08-09 this clause read `(s8)*(CHAR+0xa280) == 0` and it was wrong
///    twice over: wrong byte (that is the shared list widget's CURSOR ROW) and wrong
///    polarity. Because nothing ever resets that cursor, it sat non-zero for a whole
///    session and this gate refused nearly every autosave — device-measured on
///    2026-08-09, two autosaves in fifty minutes on a five-minute timer, with the
///    SAVE button and the minimap gone for thirteen straight minutes on an ordinary
///    field. The player found the giveaway before the disassembly did: opening any
///    menu and closing it brought both controls back, because opening a menu runs the
///    widget's constructor and that is the one thing that writes the cursor to 0.
///    See [`crate::save::AREA_CAN_SAVE_OFF`] for the full RE.
///
///    Unreadable (`None`) is treated as ALLOWED here, matching [`can_save_here`]: a
///    single unresolvable frame mid-map-change must not kill the save chain.
/// 3. **[`in_field_state`]** — real-time field free-roam owns the screen (no window
///    open, app state 4, selector overlay closed). The game's save runs from its own
///    save window; driving the primitives while another window is up would serialise a
///    state the player is in the middle of changing.
/// 4. **A sane slot number.** `writeDbRecord` formats the record name as `"%s_%d"` with
///    `*(CHAR+0xa3f8)`, so a wild value would create a record the loader never opens.
/// 5. **Both record-name pointers resolve** out of the guest's own `sl` table.
pub fn save_gate(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u32, u32)> {
    // Per-build table (MIGRATION_LGT.md §3.6): `None` = the chain is not ported to the
    // running build, so the primitives cannot be driven. This replaced the blanket
    // `guest_code_addresses_valid()` gate when LGT's chain landed (9차).
    let chain = crate::guest_build::save_chain()?;
    let c = resolve_char(read)?;
    if area_can_save(read) == Some(false) {
        return None;
    }
    if !in_field_state(read) {
        return None;
    }
    if rd_u32(read, c + chain.slot_off)? > crate::save::SAVE_SLOT_MAX {
        return None;
    }
    let (game, slot) = match chain.names {
        // KTF: the sl table holds POINTERS to the strings — read them.
        crate::guest_build::SaveNames::SlPtrs { game, slot } => (rd_u32(read, game)?, rd_u32(read, slot)?),
        // LGT: the strings' own `.text` addresses — nothing to read.
        crate::guest_build::SaveNames::Direct { game, slot } => (game, slot),
    };
    (crate::save::plausible_ptr(game) && crate::save::plausible_ptr(slot)).then_some((game, slot))
}

// ⛔ `backup_target` (the host-driven recovery backup's read gate) WAS HERE and is gone
// with the private server (99차, 2026-09-16). It was KTF-only anyway.

/// `Some((slotSaveName, slot))` when the host may rebuild a slot SUMMARY right now.
///
/// ★★★ 72차-D (TODO `BB` ⓑ). Deliberately NOT [`save_gate`]: that one is about running
/// the whole four-step chain, so it insists on [`in_field_state`] — and the paths that
/// lose a summary (창고 올리기/내리기, 캐시 지급, 제련) all happen with a window open or
/// in the online shell, where that is false by construction. Reusing it would mean the
/// repair fires only long after the fact, or never.
///
/// What a summary rebuild actually needs is much less, because `slotSummary` (`0x1a6d0`)
/// reads CHAR and allocates 16 bytes and touches nothing else — no window, no bag, no
/// AREA. So the clauses here are only the ones that can make the WRITE land wrong:
///
/// 1. **The chain is ported to this build**, and its `SlotSave` name pointer resolves.
/// 2. **A character is really loaded** — level in `1..=LEVEL_CAP`. A summary baked from a
///    zeroed CHAR is a row describing nobody, which is worse than the missing row it
///    would replace. Read through [`char_base_raw`] for the 71차-J reason
///    [`live_save_slot`] gives.
/// 3. ★★ **The guest's slot cursor equals the pin.** `writeSaveRecord` appends `_<slot>`
///    from `CHAR+0xa410` and cannot be told otherwise, so this is what makes the record
///    name the pinned character's own. It also keeps the write on `write_verdict`'s
///    `p == target => Allow` arm, i.e. off the redirect entirely.
///
/// ⛔ The result must NOT be latched for later use — see §72차-C §2, "살아남은 것 ②":
/// pre-authorising a write that runs afterwards is what `note_execute_pin` did, and the
/// pin can move in between. The caller re-asks on every attempt and the write still goes
/// through `vault::write_check` like any other.
pub fn summary_repair_gate(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u32, u8)> {
    let chain = crate::guest_build::save_chain()?;
    let c = char_base_raw(read)?;
    let level = rd_u16(read, c + LEVEL_OFF)?;
    if !(1..=LEVEL_CAP).contains(&level) {
        return None;
    }
    let row = u8::try_from(rd_u32(read, c + chain.slot_off)?).ok()?;
    // ★ 130차: the word is the guest's ROW; the pin — and `summary_needs_repair`'s
    // record name — are REAL slots. Convert first, compare second.
    let cursor = crate::slotwindow::real_slot(crate::slot_first(), row)?;
    if crate::vault::live_slot() != Some(cursor) {
        return None;
    }
    // ⛔ Screens 2 and 9: the game's own OK is mid `readSaveRecord`. A host
    // `slotSummary` here nests a guest call into that path and 게임시작 never
    // leaves the slot list. Warehouse (3) stays eligible — that is the 72차-D
    // reason this gate is not `save_gate`.
    if crate::savevault::on_slot_list(shell_screen(read)) {
        return None;
    }
    let slot_name = match chain.names {
        crate::guest_build::SaveNames::SlPtrs { slot, .. } => rd_u32(read, slot)?,
        crate::guest_build::SaveNames::Direct { slot, .. } => slot,
    };
    crate::save::plausible_ptr(slot_name).then_some((slot_name, cursor))
}

/// The save slot the live character occupies, when there IS one.
///
/// ★ This is the same word as the slot-list cursor (`CHAR+0xa410` on LGT) — the game
/// has no other. It only MEANS "the loaded character's slot" while the game is in play,
/// because every writer of it lives in the title/network shell screens; the caller is
/// responsible for that gate. See `savevault::write_verdict` — the whole save-slot
/// policy in one pure function — for what it is for.
///
/// ★★★ 71차-J: reads through [`char_base_raw`], not [`resolve_char`]. The liveness
/// gate (`hp <= hp_max` and friends) failed on the warehouse/slot screen in vc177
/// — `pinning slot` never logged, the pin stayed `None` until the first in-play
/// poll, and that poll adopted the drifted cursor. The slot word and the name are
/// valid without the gauge check; the pin is the one reader that cannot wait for it.
///
/// `None` when the word is not a real slot.
///
/// ★★★ 130차 — THE REAL SLOT, not the guest's row. The word holds the row of the three
/// the list shows (`0..=2`); the slot that row IS depends on where the save-slot WINDOW
/// sits (`slotwindow`). Every consumer of this value compares it with the pin, names a
/// record with it or keys a host file by it — all real-slot space — so the conversion
/// happens HERE, once. A reader that needs the guest's row as an ARRAY INDEX (the
/// autoloot flag, `slot_window_base`) reads the word itself.
pub fn live_save_slot(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u8> {
    let (_, row) = slot_window_base(read)?;
    crate::slotwindow::real_slot(crate::slot_first(), row)
}

/// `(CHAR, the guest's slot-list ROW)` — the raw cursor word, for the save-slot window's
/// own key arm (130차). `None` when the word is not one of the three rows.
pub fn slot_window_base(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u32, u8)> {
    let chain = crate::guest_build::save_chain()?;
    let c = char_base_raw(read)?;
    let row = rd_u32(read, c + chain.slot_off)?;
    let row = u8::try_from(row).ok().filter(|s| u32::from(*s) <= crate::savevault::GUEST_SLOT_MAX)?;
    Some((c, row))
}

/// CP949 string at `CHAR+0xb8`. LGT probe: `c4 c9 c0 cc` = "케이".
///
/// ⛔⛔ **THIS IS THE CLASS NAME, NOT THE CHARACTER'S NAME** (71차-L). The whole image
/// has ONE writer, `0x49e64`, with ONE caller, world init `0x3ab68`, and its index is
/// the class byte at `CHAR+0xb6` into `res/dat/char_dat` — so the value is 리츠 / 케이.
/// `deserialiseSave` (`0x1a964`) never touches it.
const CHAR_NAME_OFF: u32 = 0xb8;
const CHAR_NAME_LEN: usize = 16;

/// Identity of the loaded character, for the save-slot pin.
///
/// ⛔⛔ 71차-L — IT IS A CLASS NAME, AND THIS COMMENT USED TO SAY OTHERWISE.
///
/// The old wording ("the CP949 name … `loadSaveRecord` does [write it]") was wrong and
/// it misled four separate investigations of the 2026-08-26 save-destruction report.
/// [`CHAR_NAME_OFF`] carries the CLASS name (리츠 / 케이), written only by world init;
/// `deserialiseSave` leaves it alone. Two consequences that constrain the whole
/// save-slot policy:
///   * **two characters of the same class hash identically** — this cannot prove which
///     character is loaded, only which class is;
///   * a session that went 타이틀 → 온라인 → 창고 has never run world init, so the field
///     is all zeroes and this returns `None` for the entire session.
/// So the pin must come from an observed LOAD (`crate::vault::note_slot_loaded`), never
/// from this agreeing — see the tombstone on `crate::savevault::execute_pin_slot`.
/// `None` when the field is missing or all zeroes.
///
/// ★★★ 71차-J: same as [`live_save_slot`] — [`char_base_raw`], not [`resolve_char`].
/// Level 0 (title / no character) still returns `None` so a leftover name byte
/// cannot arm a pin; the HP/SP gauge check does not, because that is what went
/// blind on the warehouse screen.
pub fn live_char_identity(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u64> {
    let c = char_base_raw(read)?;
    let level = rd_u16(read, c + LEVEL_OFF).unwrap_or(0);
    if !(1..=LEVEL_CAP).contains(&level) {
        return None;
    }
    let raw = read(c + CHAR_NAME_OFF, CHAR_NAME_LEN);
    if raw.len() != CHAR_NAME_LEN || raw.iter().all(|&b| b == 0) {
        return None;
    }
    let name = raw.split(|&b| b == 0).next().unwrap_or(&raw);
    if name.is_empty() {
        return None;
    }
    Some(crate::savevault::identity_hash(name))
}

/// ★ 143차: which HERO is loaded (리츠 / 케이), for the tower's way home when the run has
/// no origin of its own (`crate::guardian::fallback_home`). The same class-name field as
/// [`live_char_identity`] — which is exactly the right thing here, because each class has
/// its own entrance town. `None` when the field reads as neither (a title-screen CHAR).
pub fn char_hero(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<crate::guardian::Hero> {
    let c = char_base_raw(read)?;
    let raw = read(c + CHAR_NAME_OFF, 5);
    // EUC-KR, NUL-terminated: 리츠 = `b8 ae c3 f7` · 케이 = `c4 c9 c0 cc`.
    match raw.as_slice() {
        [0xb8, 0xae, 0xc3, 0xf7, 0] => Some(crate::guardian::Hero::Ritz),
        [0xc4, 0xc9, 0xc0, 0xcc, 0] => Some(crate::guardian::Hero::Kei),
        _ => None,
    }
}

/// The 32-bit out-parameter the serialise step wrote (the payload length).
pub fn save_scratch(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u32> {
    rd_u32(read, crate::guest_build::save_chain()?.scratch)
}

/// The game's own AREA permission ALONE: is this a place the game would save in?
/// Deliberately ignores everything else [`save_gate`] checks (open windows, slot
/// sanity) — those are about "this exact moment", while this is about the PLACE.
/// A zero at `area+0x11d` is what makes the game answer "저장할 수 없는 지역입니다".
///
/// `true` when the chain does not resolve: boot/teardown is not a no-save AREA, and
/// the field HUD this feeds is gated on a live field elsewhere.
pub fn can_save_here(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    // Fail OPEN: an unreadable chain (boot, teardown, mid-map-change) is not a
    // no-save area, and answering "no" there is what made this feature erase two
    // controls for a whole session. See [`area_can_save`].
    area_can_save(read).unwrap_or(true)
}

// ── did a skill cast ACTUALLY start? ─────────────────────────────────────────
//
// From the wf_9fcbeeaf skill-dispatch RE (SKILL_BUTTON_FEASIBILITY.md, CONFIRMED
// items): the field quickslot dispatcher's skill branch (guest 0x1430dc) applies
// no effect itself — passing its gates ARMS the real-time combo state machine:
// it records the action CATEGORY in `CHAR+0x2c9` (the weapon-anim switch, values
// 0xa/0xb/0xc/0xd) and later frames consume it. The action-state gate 0x140024
// reads `CHAR+0x9bb5` and treats the top-bit modes (0x80/0x90) as "busy". A tap
// the gates REFUSE (not idle, no SP, no target in range) writes neither, so the
// PAIR — busy mode AND a skill category — is what distinguishes "the cast is
// running" from "the tap was swallowed". The cooldown watcher keys off this (or
// off SP dropping, whichever shows first) so a refused cast never starts one.

/// `CHAR`-relative `u8`: the action-state mode byte; `& 0x80` covers the 0x80/0x90
/// busy modes the game's own idle gate (guest 0x140024) tests.
const ACTION_MODE_OFF: u32 = 0x9bb5;
/// `CHAR`-relative `u8`: the action category the armed cast records — 0xa..=0xd
/// are the four skill weapon-anim switches (guest 0x1430dc).
const ACTION_CAT_OFF: u32 = 0x2c9;

/// Whether the combo state machine is running a SKILL action right now.
pub fn skill_cast_active(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    let Some(c) = resolve_char(read) else { return false };
    let busy = rd_u8(read, c + ACTION_MODE_OFF).is_some_and(|m| m & 0x80 != 0);
    busy && matches!(rd_u8(read, c + ACTION_CAT_OFF), Some(0x0a..=0x0d))
}

// ── cash-shop buffs the HOST owns (see `crate::buffs`) ───────────────────────
//
// Three of the original cash items had no client-side effect (their `useItem` index
// misses the jump table's `cmp #0x10` bound at guest 0x153f02 and lands on the shared
// tail 0x154542, which only decrements the stack). The game still consumes them, so
// the host supplies the effect. Two guest code rewrites carry it (a halfword for the
// EXP divisor, a halfword or an LGT two-halfword pair for the equip check), all
// size-neutral and all written only after their ORIGINAL value is confirmed — the
// `hud_bottom_patch` contract.

// ── the equip-level check, on both builds ─────────────────────────────────────
//
// (This block documents the SITES; the patch itself is `equip_relief_patch` below,
// and which shape a build takes is `guest_build::equip_relief_site()`.)
//
// KTF: `adds r1, r3, #0` at guest 0x13d0aa — the move of the item's REQUIRED LEVEL
// into the register the equip check compares against.
//
// The check is one site in the whole image (the only caller of `showMessage(0x67)` =
// "장비를 장착할수 없습니다."):
// ```text
//   13d0a4  ldrb r3,[r3,#2]     ; required level, from the inventory record
//   13d0a8  asrs r3,r3,#0x18    ; sign-extended
//   13d0aa  adds r1,r3,#0       ; <-- we make this `subs r1,r3,#N`
//   13d0ac  ldrh r3,[r4]        ; (kills r3, so the subtract costs nothing)
//   13d0b6  cmp  r2, r3         ; playerLevel vs r1
//   13d0b8  bge  pass
// ```
// Patching the MOVE rather than the branch is what makes this a "-N levels" item
// instead of "ignore the requirement": everything below it runs unchanged, so the
// condition becomes `playerLevel >= requiredLevel - N`.
//
// `r1` is dead after 0x13d0b2 and `r3` is overwritten at 0x13d0ac, so the rewrite has
// no other effect; `subs` sets flags but `cmp` re-sets them before the branch.
//
// ★ NOT YET LIVE ON LGT, and the reason is worth writing down: the check itself IS
// located (2026-08-10, via the only `movs r0,#0x67` that feeds a `showMessage` call —
// LGT `showMessage` = `0x46b7c`, 12 callers, ours at `0x477dc`):
// ```text
//   477cc  adds  r4,#0xba
//   477ce  movs  r3,#0
//   477d0  ldrsh r1,[r4,r3]     ; player level
//   477d2  movs  r3,#2
//   477d4  ldrsb r3,[r2,r3]     ; item's required level (record+2)
//   477d6  cmp   r1,r3
//   477d8  bge   0x477e2        ; pass
//   477da  movs  r0,#0x67
//   477dc  bl    0x46b7c        ; "장비를 장착할수 없습니다."
// ```
// but LGT's codegen compares the freshly-loaded byte DIRECTLY — there is no
// `adds r1,r3,#0` move to turn into a `subs`, so the one-halfword trick has no
// analogue. Two four-byte rewrites are available instead, and WHICH ONE is not a
// matter of taste (7차, 2026-08-11):
//
//   * REQUIREMENT side, `0x477d2`: `movs r3,#2 ; ldrsb r3,[r2,r3]` ->
//     `ldrb r3,[r2,#2] ; subs r3,#N`. This is what §2.8 proposed and 6차 plumbed, and
//     it is REJECTED: it turns a SIGNED load into an unsigned one, so any item whose
//     requirement byte is >= 0x80 (a `-1` "no requirement" sentinel is exactly what a
//     C `signed char` invites) flips from allowed to blocked. Nothing had checked the
//     item data for such a value, and "the feature shows the wrong answer" is the
//     silent-bug class this migration keeps finding.
//   * LEVEL side, `0x477ce`: `movs r3,#0 ; ldrsh r1,[r4,r3]` ->
//     `ldrh r1,[r4,#0] ; adds r1,#N`. Shipped. `r4` is `CHAR+0xe2` (`LEVEL_OFF`) —
//     `0x477cc adds r4,#0xba` on top of the accessor §2.8 showed returns `CHAR+0x28` —
//     and the game caps level at `LEVEL_CAP` (70, 99 since 111차), so `ldrh` and `ldrsh`
//     agree on every value the field can hold. The rewrite changes the comparison and
//     nothing else.
//
// Both sites' bytes were read back out of RUNNING guest memory before either decision
// (`wie_probe --poke`: `0x477ce` = `0023e15e`, `0x477d2` = `0223d356`, both exactly what
// the static disassembly predicted), and the poked run's frame hashes matched the clean
// run's. `guest_build::equip_relief_site()` says which shape a build takes.

/// `movs r1, #100`: the divisor of the kill-EXP formula in `addExpFromKill` (KTF guest
/// 0x14650c, whose single caller 0x120682 runs it for both party members and then calls
/// `checkLevelUp` 0x146ca8).
/// ```text
///   gain = (enemyLv*13 + 130) * enemy[0xb4] * (clamp(lvDiff,-9,9)*2 + 10) / 100
///   if (gain <= 0) gain = 1
///   CHAR[0xe4] += gain ;  CHAR[0xe4] += enemy[0xb4]
/// ```
/// Halving the divisor doubles `gain` exactly (100/50, 100/25 are integral), and the
/// level-up loop keeps running inside the game's own flow.
///
/// ⚠ Honest limit: the SECOND, flat `+= enemy[0xb4]` happens after the divide, so x2
/// is `2*gain + flat`, not `2*(gain + flat)`.
///
/// The ADDRESS is per-build (KTF 0x14654e / LGT 0x4d3cc) and lives in
/// `guest_build::exp_divisor_site()`; the formula above is the game's, not the
/// compile's, so it stays here.

/// `Some((address, bytes))` when the equip-level-relief instruction(s) are not already
/// what `levels` asks for. `levels == 0` restores the shipped code, so a lapsed buff
/// un-patches itself. The byte payload is one halfword on KTF and two on LGT — see the
/// site notes above.
///
/// Not gated on `guest_code_addresses_valid` any more (same per-item split as
/// [`hud_bottom_patch`]): `guest_build::equip_relief_site()` answers "is this build's
/// site known AND verified", and each arm refuses unless what is on site is either the
/// shipped original or one of our own rewrites — a different revision is left alone.
pub fn equip_relief_patch(read: &impl Fn(u32, usize) -> Vec<u8>, levels: u8) -> Option<(u32, Vec<u8>)> {
    match crate::guest_build::equip_relief_site()? {
        crate::guest_build::EquipReliefSite::MoveHalfword { site } => {
            let want = crate::buffs::equip_relief_halfword(levels)?;
            let cur = rd_u16(read, site)?;
            let orig = crate::buffs::equip_relief_halfword(0)?;
            // `subs r1,r3,#imm3` with imm3 masked out == the imm3=0 form of `subs`.
            let is_ours = (cur & !(7 << 6)) == (0x1E00 | (3 << 3) | 1);
            if cur != orig && !is_ours {
                return None;
            }
            (cur != want).then(|| (site, want.to_le_bytes().to_vec()))
        }
        crate::guest_build::EquipReliefSite::LevelBias { site } => {
            let cur = [rd_u16(read, site)?, rd_u16(read, site + 2)?];
            let want = equip_relief_bias_patch(cur, levels)?;
            Some((site, [want[0].to_le_bytes(), want[1].to_le_bytes()].concat()))
        }
    }
}

/// The pure half of the `LevelBias` arm: given the two halfwords currently on site,
/// the pair to WRITE, or `None` when nothing should be written (already right, or not
/// code we recognise). Split out so the decision is unit-testable without a guest.
fn equip_relief_bias_patch(cur: [u16; 2], levels: u8) -> Option<[u16; 2]> {
    let want = crate::buffs::equip_relief_level_bias(levels)?;
    let orig = crate::buffs::equip_relief_level_bias(0)?;
    // Ours: the exact `ldrh r1,[r4,#0]` followed by any `adds r1,#imm8`.
    let is_ours = cur[0] == 0x8821 && (cur[1] & 0xff00) == 0x3100;
    if cur != orig && !is_ours {
        return None;
    }
    (cur != want).then_some(want)
}

/// `Some((address, bytes))` when the EXP divisor is not already `100 / mult`.
/// `mult == 1` restores the shipped `movs r1,#100`.
///
/// Refuses unless the current halfword is a `movs r1,#imm8` whose imm8 divides 100
/// exactly — i.e. either the original or one of ours.
pub fn exp_multiplier_patch(read: &impl Fn(u32, usize) -> Vec<u8>, mult: u8) -> Option<(u32, [u8; 2])> {
    // Per-build site (`None` where the routine has not been located), then the same
    // shape check as before: only a `movs r1,#imm8` whose immediate divides 100 is ever
    // overwritten, so a revision whose codegen differs is left alone.
    let site = crate::guest_build::exp_divisor_site()?;
    let want = crate::buffs::exp_divisor_halfword(mult)?;
    let cur = rd_u16(read, site)?;
    let imm = cur & 0xff;
    if (cur & 0xff00) != 0x2100 || imm == 0 || 100 % u32::from(imm) != 0 {
        return None;
    }
    (cur != want).then_some((site, want.to_le_bytes()))
}

// ── "the player just confirmed 사용하기 on a buff item" ───────────────────────
//
// The inventory cursor, as `popup_choose` itself reads it (guest 0x13c99e/0x13c9b0):
//   tab  = u8 @ CHAR+0x9c71
//   slot = u8 @ CHAR+0x9c72 + tab      (a per-tab cursor array)
// and those are exactly the two arguments it hands to `useItem` (guest 0x13ce22), so
// the record they address is the item about to be consumed.
const INV_TAB_OFF: u32 = 0x9c71;
const INV_CURSOR_BASE: u32 = 0x9c72;

/// `사용하기` + NUL, from the game's own `dat/InGame_txt` id 0x21.
///
/// ★ The gate keys on the LABEL, not on the popup tag. The popup these items get is
/// built by guest 0x12d0ac, which writes a DYNAMIC tag (`*(r7-0x2d)`) and picks row
/// 0's text at 0x12d12a — id 0x21 `사용하기` for every item except 0x0b, else 0x20
/// `상세보기`. A tag test would therefore be guessing; the label is the thing that
/// actually tells us the highlighted row will run `useItem`.
const LABEL_USE: [u8; 9] = [0xbb, 0xe7, 0xbf, 0xeb, 0xc7, 0xcf, 0xb1, 0xe2, 0x00];

/// Is the player, RIGHT NOW, sitting on `사용하기` for a cash item? Returns that
/// item's `(category, index)`; the caller decides whether it is one the host
/// supplies an effect for (`buffs::grant_for` / `buffs::box_reward`) or merely one it
/// recognises to say where the real item lives (`buffs::is_orb_box` /
/// `buffs::is_socket_expand`).
///
/// Called from the worker the instant an OK keydown arrives, before it is forwarded —
/// so the read is never stale. (This is the same lesson as 17차's `PopupConfirm`: the
/// UI thread's copy of popup state is up to ~80 ms old, and here the answer decides
/// whether a week-long buff starts.)
///
/// `None` for every other popup and row. The host then forwards OK unchanged either
/// way — the GAME consumes the item, we only watch.
pub fn cash_use_pending(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u16, u16)> {
    let c = resolve_char(read)?;
    // A menu overlay owns the screen (same byte, and same reason, as `popup_view`
    // gate 1: the window id is not loaded yet when the game tests this).
    if rd_u8(read, c + SELECTOR_FLAG_OFF)? == 0 {
        return None;
    }
    // The ITEM page, not the skill page.
    if rd_u8(read, c + MENU_PAGE_OFF)? != PAGE_ITEM {
        return None;
    }
    // A popup is open, with a highlighted row inside its row count.
    if rd_u8(read, c + POPUP_OPEN_OFF)? == 0 {
        return None;
    }
    let rows = rd_i8(read, c + POPUP_COUNT_OFF)?;
    let sel = rd_i8(read, c + POPUP_SEL_OFF)?;
    if rows <= 0 || sel < 0 || sel >= rows {
        return None;
    }
    // That row really reads 사용하기. The label buffers are resolved through the HANDLE
    // array every time, never through the dangling `popup_text` cache.
    let handles = popup_handles(read)?;
    let label = popup_label_addr(read, handles, sel as u32)?;
    let got = read(label, LABEL_USE.len());
    if got.len() < LABEL_USE.len() || got[..] != LABEL_USE[..] {
        return None;
    }
    // …and the item under the cursor exists, with something left to spend.
    let tab = rd_u8(read, c + INV_TAB_OFF)?;
    let slot = rd_u8(read, c + INV_CURSOR_BASE + u32::from(tab))?;
    let (count, cat, idx) = item_record(read, c, tab, slot);
    if count == 0 {
        return None;
    }
    Some((cat, idx))
}

// ── 제련 확률 (71차-I) ────────────────────────────────────────────────────────
//
// The smith never tells the player their odds; 71차-H decoded them, and this is the
// read that makes them printable. Everything here is READ-ONLY — the feature writes no
// guest byte, calls no guest function and synthesises no key.

/// The game's own window id for 아이템제련 (`openWindow` 13, measured 66차-B).
const WINDOW_SMITH: u8 = 13;

/// Read the live 제련 odds, or `None` when the smith's stone spinner is not the thing
/// on screen (which is most of the time).
///
/// ── WHY THE STAGE BYTE AND NOT THE WINDOW ID ─────────────────────────────────
/// ⛔⛔ `window == 13` is true for the WHOLE flow, and the roll's epilogue zeroes the
/// stone count on its way back to the item picker (`0x25490`). Gating on the window id
/// alone therefore prints `성공 0 %` over the picker — a confident wrong number about an
/// action that can destroy the item. [`crate::refine::STAGE_SPINNER`] is the byte the
/// picker's OK arm sets in the same breath as `B = 1` (`0x237b4`/`0x237b6`).
///
/// ── WHY THE VIEW TABLE AND NOT (tab, slot) ───────────────────────────────────
/// ⛔⛔ On this screen the inventory cursor is NOT a bag coordinate. The smith lists
/// equipment only, and the cursor walks that filtered list — see the block comment on
/// [`crate::refine::VIEW_TABLE`]. Reusing [`cash_use_pending`]'s `(INV_TAB_OFF,
/// INV_CURSOR_BASE + tab)` pair here reads a different record and produces a plausible
/// number for the wrong item.
///
/// Every read is fail-closed: one unreadable byte anywhere and the caller draws nothing.
/// There is no last-good value and no default — a stale probability is worse than none.
pub fn refine_odds(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<crate::refine::Refine> {
    use crate::refine;
    if !crate::guest_build::refine_odds_ok() {
        return None;
    }
    let c = resolve_char(read)?;
    if rd_u8(read, c + WINDOW_ID_OFF)? != WINDOW_SMITH {
        return None;
    }
    let ctx = c + refine::CTX_OFF;
    if rd_u8(read, ctx + refine::CTX_STAGE)? != refine::STAGE_SPINNER {
        return None;
    }
    // `B` — the number the player is moving. The painter renormalises it to 1..=99 every
    // frame (`0x2102e` / `0x210a0`), and the DOWN arm has a dead edge that can leave
    // 0xff behind for one tick, so mirror the painter rather than refusing.
    let stones = rd_u8(read, ctx + refine::CTX_STONES)?.clamp(refine::STONE_MIN, refine::STONE_MAX);
    // `C` — a WORD, and only ever 0 / 1 / 3. Anything else means this is not the field
    // we think it is, so give up rather than guess.
    let streak = rd_u32(read, ctx + refine::CTX_STREAK)?;
    if streak > u32::from(refine::STREAK_MAX) {
        return None;
    }

    // The target, through the smith's own filtered equipment view.
    let grid = resolve_grid_base(read, c)?;
    let page = rd_i8(read, c + INV_TAB_OFF)?;
    if page < 0 {
        return None;
    }
    let cell = rd_i8(read, c + INV_CURSOR_BASE + page as u32)?;
    let index = refine::view_index(page, cell)?;
    let entry = refine::view_entry_addr(grid, index)?;
    let count = rd_i8(read, grid + refine::VIEW_COUNT)?;
    let (tab, slot) = refine::view_slot(rd_u16(read, entry)? as i16, rd_u16(read, entry + 2)? as i16, index, count)?;

    let rec = crate::orbcombine::rec_addr(grid, tab, slot)?;
    if rd_u8(read, rec + crate::orbcombine::REC_OCCUPIED)? == 0 {
        return None;
    }
    let cat = rd_u16(read, rec + crate::orbcombine::REC_CAT)?;
    let level = rd_i8(read, rec + crate::orbcombine::REC_REFINE)?;
    if level < 0 {
        return None;
    }
    Some(refine::evaluate(cat, level, stones, streak as u8))
}

/// The two data writes of a FULL HP+SP RESTORE — exactly what the game's own
/// revive / full-restore sites do (guest 0x124458 / 0x1540cc / 0x1570c4:
/// `HP = *(CHAR+0xec); SP = 1000`), so this invents no new state. Used as one of
/// the random-box payouts.
///
/// `None` when the character does not read as live (same plausibility gate as the
/// HUD) — a restore written into a half-built CHAR would be a corruption, not a heal.
///
/// ── NOT gated on `guest_code_addresses_valid` since 2026-08-11 (7차) ──────────
/// It never needed to be, and keeping it there was the blanket gate hiding a case it
/// does not describe. This function calls nothing and touches no build-specific
/// address: it writes [`HP_OFF`] and [`SP_OFF`], the two offsets the HUD gauges read,
/// which are VERIFIED on both builds (MIGRATION_LGT.md §2.7 read the status screen's
/// own HP 167/167 and SP back at exactly these, and §2.12 re-read them live on the
/// shipped archive). The value written is the game's own (`*(CHAR+0xec)` for HP,
/// the `AddSP` clamp constant for SP), so it invents no state either.
///
/// What still protects it is the thing that always did: [`resolve_char`] refuses a
/// record that does not read as a live character (level `1..=70`, `hp <= hp_max`,
/// `sp <= 1000`), so there is no build on which this can write into a half-built or
/// mis-anchored record.
pub fn full_restore_writes(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<[(u32, Vec<u8>); 2]> {
    let c = resolve_char(read)?;
    let s = read_stats(read)?;
    // HP is a s16 in the guest. Past 32767 it is a GAUGE (142차): the gauge max (32767)
    // is "full", and the HP hook re-derives the true HP from it — so the ceiling is right.
    let hp = s.hp_max.min(i16::MAX as u32) as u16;
    Some([
        (c + HP_OFF, hp.to_le_bytes().to_vec()),
        (c + SP_OFF, u32::from(SP_MAX).to_le_bytes().to_vec()),
    ])
}

/// The ONE write that pays the player `add` gold, and the balance it lands on (105차).
///
/// ★ READ-VERIFY-THEN-WRITE, like every other host write into CHAR: the current balance
/// is read here and the caller writes the sum, so a poll that cannot resolve CHAR pays
/// nothing rather than writing a guessed absolute.
///
/// ⛔ SATURATING, not wrapping. The field is a `u32` the game only ever `adds` to at the
/// shop, and a wrap would turn a sale into a wipe of everything the player has — the one
/// arithmetic in this file where the failure mode is indistinguishable from theft.
/// ⛔ `add == 0` still returns the write: the caller has already debited the box by then,
/// and refusing here would make a zero-价 sale look like a silent failure. The zero-price
/// case is refused EARLIER, by `bagbox::sell_refusal`, which is where it belongs.
pub fn gold_add_writes(read: &impl Fn(u32, usize) -> Vec<u8>, add: u32) -> Option<((u32, Vec<u8>), u32)> {
    let c = resolve_char(read)?;
    let now = rd_u32(read, c + GOLD_OFF)?;
    let next = now.saturating_add(add);
    Some(((c + GOLD_OFF, next.to_le_bytes().to_vec()), next))
}

/// The 천상의 영약's ONE data write: `exp (CHAR+0xe4) = expToNext (CHAR+0xe8) + 1`,
/// which is the exact state a kill that crosses the threshold leaves behind — the
/// caller then invokes the game's own `checkLevelUp` (`guest_build::level_up_fn`)
/// and everything downstream (points, HP/SP refill, the `LEVEL_CAP` clamp, visuals) is
/// game code. Returns `(write_addr, bytes, current_level)`.
///
/// `None` when the character does not read as live (same plausibility gate as
/// [`full_restore_writes`]) or is already at the cap — at the cap `doLevelUp` would
/// clamp-and-pin anyway, but declining here lets the caller keep the pending grant
/// instead of burning it on a no-op.
pub fn level_elixir_writes(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u32, Vec<u8>, u16)> {
    let c = resolve_char(read)?;
    let s = read_stats(read)?;
    if s.level >= LEVEL_CAP {
        return None;
    }
    let exp = s.exp_next.saturating_add(1);
    Some((c + EXP_OFF, exp.to_le_bytes().to_vec(), s.level))
}

/// The write that pays a cleared tower floor's EXP (112차, `guardian::Floor::exp`):
/// `exp += add` at `CHAR+0xe4`, saturating, for the caller to follow with the game's
/// own `checkLevelUp` (`guest_build::level_up_fn`) — the 천상의 영약 mechanism, so
/// points, refills, the level cap and the level-up effect are all game code, and a
/// grant worth several levels climbs them in one call (the loop runs up to 15).
///
/// Returns `(write_addr, bytes, exp_before, exp_after, level)`; `None` when no live
/// character reads (same plausibility gate as [`level_elixir_writes`]). ⛔ Not gated on
/// the level cap: at the cap `doLevelUp` pins `exp = expToNext` itself, so the write is
/// harmless there and the caller's bookkeeping stays simple.
pub fn tower_exp_writes(read: &impl Fn(u32, usize) -> Vec<u8>, add: u32) -> Option<(u32, Vec<u8>, u32, u32, u16)> {
    let c = resolve_char(read)?;
    let s = read_stats(read)?;
    let after = s.exp.saturating_add(add);
    Some((c + EXP_OFF, after.to_le_bytes().to_vec(), s.exp, after, s.level))
}

// ── 소켓확장 — the host-side write is GONE (44차) ────────────────────────────
//
// 38차 implemented this item here: walk the seven equip slots, take the first item with
// room, write `+0x3c6`. Every byte of that RE was right and it is not why it was
// removed. It was removed because the player's ask was about CHOOSING:
//
//   "네트워크 창고에서 소켓확장 아이템을 사용하면 **오브처럼** 소켓확장을 시킬 아이템을
//    선택할 수 있게 해야해"
//
// "the first equipped item with room" cannot express that, and no host UI could either
// without stacking a new picker on top of a guest modal. The game already owns a picker
// for exactly this shape of action — 오브결합 — so the item now travels through it:
// delivered into the network warehouse as category 0x10, targeted with the game's own
// "장비를 선택하세요" flow, resolved by the server's `_orb_combine`. The equip-slot walk,
// the `SocketExpand` plan struct and the socket offsets all went with it.
//
// The RE that made it possible is not lost: the instance offsets live in
// `MIGRATION_LGT.md`, and the wire-side socket bytes (rec[16]=total, rec[18]=filled)
// are what the server now edits instead.

// ── equipped secondary weapon (the key-'7' button icon) ──────────────────────
//
// Field key '7' is the SECONDARY-WEAPON attack: the shared attack arm at guest
// 0x1432d8 branches on the raw key and picks (hand 2, equip slot 6) for '7' versus
// (hand 1, equip slot 0) for OK/FIRE. Equip slot 6's item reference is what the
// secondary-weapon attack-power routine reads, which is what pins "slot 6 ==
// secondary weapon". [RE: workflow wf_477f0b16 QC]
//
// The reference is the same (tab, slot) pair into the inventory grid that the
// quickbar uses, so the icon resolves through the existing item-DB path.
const EQUIP6_TAB_OFF: u32 = 0x513; // i8, -1 = empty
const EQUIP6_SLOT_OFF: u32 = 0x514; // i8
const EQUIP6_PRESENT_OFF: u32 = 0x524; // i8, -1 = nothing equipped (the game's own dim test)
/// item_bm has 29 frames (0..28); anything else means we misread the record.
const ITEM_FRAME_MAX: u16 = 28;

/// The icon frame of the EQUIPPED SECONDARY WEAPON, for the key-'7' button.
/// `None` = nothing equipped (draw the button dimmed — the same condition under
/// which the game itself rejects '7') or the record did not look sane.
pub fn secondary_weapon_icon(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<IconRef> {
    let c = resolve_char(read)?;
    if rd_i8(read, c + EQUIP6_PRESENT_OFF)? < 0 {
        return None;
    }
    let tab = rd_i8(read, c + EQUIP6_TAB_OFF)?;
    let slot = rd_i8(read, c + EQUIP6_SLOT_OFF)?;
    if tab < 0 || slot < 0 {
        return None;
    }
    let base = resolve_grid_base(read, c)?;
    let rec = base + u32::from(tab as u8) * TAB_STRIDE + u32::from(slot as u8) * SLOT_STRIDE;
    // The grid record's DB index is a SIGNED halfword (the game reads it with
    // `ldrsh`), so -1 = "no item" must be rejected, not turned into 65535 and used
    // to index the DB.
    let idx = rd_u16(read, rec + 0x3b8)? as i16;
    let cat = rd_u16(read, rec + 0x3b6)?;
    if idx < 0 {
        return None;
    }
    icon_frame(read, c, cat, idx as u16).map(IconRef::Item)
}

/// The item_bm frame the game itself would draw for `(category, db index)` —
/// mirrors its icon draw at guest 0x1533a8:
/// ```text
///   if (cat == 0x12) frame = (s8)itemDB[idx].icon      // consumables only
///   else             frame = cat                        // one icon per category
///   if (cat == 0x0e && idx == 3) frame = 0x1c           // one special case
/// ```
/// ★ This is what the first version of the key-'7' button got wrong: it ran the
/// CONSUMABLE lookup for a weapon, so it indexed the consumable DB with a weapon's
/// id and blitted whatever byte happened to be there ("the icon is wrong",
/// 2026-07-30). Only category 0x12 goes through that table.
fn icon_frame(read: &impl Fn(u32, usize) -> Vec<u8>, char_ptr: u32, cat: u16, idx: u16) -> Option<u16> {
    let frame = if cat == CONSUMABLE_CAT {
        item_db_icon(read, char_ptr, idx)?
    } else if cat == 0x0e && idx == 3 {
        0x1c
    } else {
        cat
    };
    (frame <= ITEM_FRAME_MAX).then_some(frame)
}

/// Which overlay PAGE the field menu is showing — `None` while no menu is open.
///
/// `CHAR+0x9c6e` is the byte the game's two page tables are indexed by: KEY handlers at
/// `.data 0x14001d4` (dispatched at `0x46088`) and PAINT functions at `.data 0x14001fc`
/// (`0x3ff36`) — ★ 150차: two 10-entry tables back to back, NOT one 20-way table as 71차-F
/// read it. [`EQUIP_OVERLAY`] is the one the orb refresh and the set notice care about.
///
/// ★ 150차: answers only while the menu overlay is OPEN (`SELECTOR_FLAG_OFF != 0`). The page
/// byte keeps its last value after the menu closes, so a stale 3 on the field must not read
/// as "the equipment screen".
///
/// Reads through [`char_base_raw`]: the caller is deciding WHEN to re-run an idempotent
/// rebuild, and refusing to answer because a gauge looks odd is precisely the failure
/// this was added to fix.
pub fn overlay_screen(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u8> {
    let c = char_base_raw(read)?;
    if rd_u8(read, c + SELECTOR_FLAG_OFF)? == 0 {
        return None;
    }
    rd_u8(read, c + MENU_PAGE_OFF)
}

/// Overlay page of the 장비 screen — **3** (= [`MENU_SCREEN_EQUIP`], probe-measured 150차:
/// PAINT[3] = `0x42830`). Its socket pips read `rec+0x3c6` / `rec+0x3c9` live, and the
/// derived numbers beside them come from `rec+0x3cc` / `rec+0x3ce`, which only `0x528f0`
/// rebuilds — which is why entering this screen is worth a sweep.
///
/// ⛔⛔ It was **17** from 71차-F to 149차, and 17 is not a page: it was the FLAT index of
/// PAINT[7] (`0x4503c`, the 상세보기 painter) in what was read as one 20-entry table. The
/// page byte never held 17, so the 장비-screen orb refresh (71차-L) and the 142차
/// set-completion box never fired once (both found by the 150차 RE, three reports agreeing).
pub const EQUIP_OVERLAY: u8 = MENU_SCREEN_EQUIP;

/// Overlay page of the 상세보기 window (painter `0x4503c`, keys `0x46340` — CLR only).
pub const PAGE_DETAIL: u8 = 7;
/// The page 상세보기 returns to on CLR (`CHAR+0x9c6f` ← the page it was opened from,
/// `0x47974`): [`PAGE_ITEM`] = the bag, [`MENU_SCREEN_EQUIP`] = the 장비 candidate list.
const MENU_RETURN_PAGE_OFF: u32 = 0x9c6f;

// ── 공격속도 / 스킬사용속도: reading the stat off the worn gear (2026-09-23) ───
//
// `0x4f014`, the game's own stat recompute, is the reference implementation. For each of
// the nine worn slots it reads the slot's `(cat, idx)` and then the item's DB row:
//
//   EQUIPPED[i]      = base + 0x364 + i * 0x3c        (a copy of the bag record)
//   cat, idx         = (i16)rec+0x22, (i16)rec+0x24   (-1 = the slot is empty)
//   row              = *(*(grid + 0) + cat * 4) + idx * 0x20
//   addStat((i8)row[0x1a], (i8)row[0x1b]); addStat((i8)row[0x1c], (i8)row[0x1d])
//
// `*(grid+0)` is an array of 13 table pointers, one per equipment category — measured on
// the probe (cat 0x00..0x0c are heap pointers, 0x0d on is zero). ⛔ It is NOT `grid + cat*4`:
// the non-equipment tables live at `grid+0x10..0x38` and the two would collide.
//
// ⛔ Read the option pairs at the runtime offset, never at an asset byte number: a 20-byte
// equipment row carries them at `b16..b19` and an 18-byte accessory row at `b14..b17`, and
// the loader lands BOTH on runtime `+0x1a` (which is why the guest can read them without
// knowing the category at all).
//
// We read the same two pairs and keep the ones tagged [`crate::atkspeed::ATTACK_SPEED_STAT`],
// which the guest's own `addStat` throws away (its gate is `id - 2 <= 0x15`).

/// `EQUIPPED[0]`, CHAR-relative (`base+0x364`, and `base = CHAR + 0x28`).
const WORN_BASE_OFF: u32 = 0x28 + 0x364;
/// One worn record, the same 0x3c the bag uses.
const WORN_STRIDE: u32 = 0x3c;
/// `(i16)` category and index inside a worn record; either at `-1` means "empty".
const WORN_CAT_OFF: u32 = 0x22;
const WORN_IDX_OFF: u32 = 0x24;
/// The equipment DB row: `*(grid + 0)` is the per-category table array.
const EQUIP_TABLES_OFF: u32 = 0x00;
const EQUIP_ROW_STRIDE: u32 = 0x20;
/// The two `(statId, value)` option pairs the recompute feeds to `addStat`.
const EQUIP_ROW_OPTS_OFF: u32 = 0x1a;
/// Equipment is `cat <= 0x0c` — the game's own gate, and the length of the table array.
const EQUIP_CAT_MAX: i16 = 0x0c;

/// Every `(statId, value)` option pair the player is currently WEARING.
///
/// Empty when there is no character, no grid, or the table array will not resolve — the
/// caller then publishes 0, which is the original timing. ⛔ Fail soft, never a guess: a
/// wrong sum here would change how the game feels for a reason nobody could see.
pub fn worn_option_pairs(read: &impl Fn(u32, usize) -> Vec<u8>) -> Vec<(u8, i8)> {
    let Some(c) = resolve_char(read) else { return Vec::new() };
    let Some(grid) = resolve_grid_base(read, c) else { return Vec::new() };
    let Some(tables) = rd_u32(read, grid + EQUIP_TABLES_OFF).filter(|p| in_heap(*p)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for i in 0..crate::atkspeed::WORN_SLOTS as u32 {
        let rec = c + WORN_BASE_OFF + i * WORN_STRIDE;
        let (Some(cat), Some(idx)) = (
            rd_u16(read, rec + WORN_CAT_OFF).map(|v| v as i16),
            rd_u16(read, rec + WORN_IDX_OFF).map(|v| v as i16),
        ) else {
            continue;
        };
        // The game's own two tests, in its own order: an empty slot is -1 in both.
        if cat < 0 || cat > EQUIP_CAT_MAX || idx < 0 {
            continue;
        }
        let Some(table) = rd_u32(read, tables + cat as u32 * 4).filter(|p| in_heap(*p)) else {
            continue;
        };
        let row = table + idx as u32 * EQUIP_ROW_STRIDE;
        let b = read(row + EQUIP_ROW_OPTS_OFF, 4);
        if b.len() != 4 {
            continue;
        }
        out.push((b[0], b[1] as i8));
        out.push((b[2], b[3] as i8));
        // ★ 150차: the reworked accessories' THIRD option lives outside the asset row
        // (`acc_options.rs`); the one the HOST sums rides here as a third pair — today only
        // 타이탄의 파편's 「공격속도 +50」, which `atkspeed::sum_options` adds like any worn
        // pair. ⛔ Never the `Stat` thirds: the wear-recompute hook (`acc_third_stat_lgt`)
        // feeds those to the game's own `addStat`, and adding them here too would count them
        // twice (and `addStat` would ignore a host copy anyway).
        if let Some(pair) = wie_util::stats::host_third_pair(cat as u16, idx as u16) {
            out.push(pair);
        }
    }
    out
}

/// ★ 150차 레아의목걸이 「5초당 HP회복 +N%」: the summed `N` of every worn option pair with
/// id [`wie_util::acc_options::REGEN_STAT`] (0x78 — `addStat` ignores it, so the guest does
/// nothing with it and the host heals). Duplicates count, like the game sums pairs (two 레아
/// = 40). Clamped to 0..=100: a negative sum is nothing to heal, and more than the whole
/// max per tick means nothing. `0` = no regen worn, or nothing readable.
pub fn regen_pct(read: &impl Fn(u32, usize) -> Vec<u8>) -> u32 {
    let sum: i32 = worn_option_pairs(read)
        .iter()
        .filter(|(id, _)| *id == wie_util::acc_options::REGEN_STAT)
        .map(|(_, v)| i32::from(*v))
        .sum();
    sum.clamp(0, 100) as u32
}

/// May the 5-second regen heal right now? The FIELD only (no window, no menu, not the
/// shell — `in_field_state`), with nothing of the game's asking (`game_box_up`), and a
/// player who is not dying (state 16) or being shown GAME OVER (state 7): a heal there
/// would race the game's own death path. Every read failure answers `false`.
pub fn regen_may_heal(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    if !in_field_state(read) || game_box_up(read) {
        return false;
    }
    let Some(c) = char_base_raw(read) else { return false };
    matches!(
        read(c + PLAYER_STATE_OFF, 1).first().copied(),
        Some(s) if s != PLAYER_STATE_DYING && s != PLAYER_STATE_GAME_OVER
    )
}

// ── 150차 — the 상세보기 (item detail) note ────────────────────────────────────
//
// The game's own detail window is overlay page 7 (painter `0x4503c`). The host redraws its
// text BODY (`itemnote.rs`, drawn by the UI over the game frame): the 11 reworked
// accessories lose 「기절 저항 +0」 and gain their third option, and every set piece shows its
// set. This half answers WHICH item the window shows (or is about to show) and builds the
// note; the UI guards the draw pixel by pixel (`menutouch::detail_page` + `itemnote::shows`),
// so a note here is only ever a PROPOSAL.
//
// Where the painter takes the item from (`0x450a6..0x45122`, re-read for this round):
//   return page 3 (the 장비 candidate list): entry = base + 0x580 + s * 0x12c0 + r * 0x3c,
//       s = `(s8) CHAR+0x9c77` with 8 → 2 (both accessory slots share one list), r =
//       `(s8) CHAR+0x9c7b`; the entry's `+0x1f` / `+0x20` are the BAG cell (tab, slot) and
//       its `+0x22` / `+0x24` the copied `(cat, idx)`;
//   anything else (the bag): tab = `CHAR+0x9c71`, slot = `CHAR+0x9c72 + tab`;
// then the bag record `grid + tab * 0x3c4 + slot * 0x3c` (`0x45298`). `base = CHAR + 0x28`.
//
// ★ The note is published BEFORE page 7 opens too — on the bag page (2) for the cell under
// the cursor, on the 장비 page (3) for the candidate under it. The window's first frame then
// finds its note already on the UI side; a note that only followed page 7 would arrive after
// the frame (the frame goes out inside the tick that opens the window) and the game's own
// body — 「기절 저항 +0」 included — would show for a frame first.

/// The candidate list's first entry (`base + 0x580`), CHAR-relative.
const CAND_BASE_OFF: u32 = 0x28 + 0x580;
/// One worn slot's candidate list (80 entries of 0x3c), and one entry.
const CAND_LIST_STRIDE: u32 = 0x12c0;
const CAND_ENTRY_STRIDE: u32 = 0x3c;
const CAND_MAX: i8 = (CAND_LIST_STRIDE / CAND_ENTRY_STRIDE) as i8;
/// Inside a candidate entry: the bag cell it copies, and the copied `(cat, idx)`.
const CAND_TAB_OFF: u32 = 0x1f;
const CAND_SLOT_OFF: u32 = 0x20;
const CAND_CAT_OFF: u32 = 0x22;
const CAND_IDX_OFF: u32 = 0x24;
/// The 장비 cursor: the worn slot whose candidates are listed, and the candidate row.
const EQUIP_SLOT_CURSOR_OFF: u32 = 0x9c77;
const EQUIP_CAND_CURSOR_OFF: u32 = 0x9c7b;
/// The second accessory slot (8) lists the first one's candidates (2) — the painter's own
/// `cmp r3,#8` / `movs r1,#2`.
const WORN_ACC2: i8 = 8;
const WORN_ACC1: i8 = 2;
/// An orb's label in the string table: `InGame_txt[orb + 0xab]` (「@투신의」 …), the
/// painter's `adds r1,#0xab` (`0x4546a`) — the same bias `orbsum.rs` uses.
const ORB_LABEL_BASE: u16 = 0xab;
/// The 20-byte armour row's `b11` at runtime (`+0x10..+0x14` = b7..b11): the game's own
/// four-piece set id (`0x4f270` reads `[row, #0x14]`).
const EQUIP_ROW_NATIVE_SET_OFF: u32 = 0x14;

/// The item a 상세보기 window shows, or is about to show (see the section notes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoteTarget {
    pub tab: u8,
    pub slot: u8,
    pub cat: u16,
    pub idx: u16,
    /// The record's instance key (`orbcombine::REC_KEY`).
    pub key: u64,
    /// `(i16) rec+0x3ce` — the derived stat; for an accessory, the 「기절 저항 +N」 it prints.
    pub derived: i16,
    pub sockets: u8,
    /// The orb set in it, `-1` = none.
    pub orb: i8,
    pub refine: i8,
    /// The window is up NOW (page 7), not merely one tap away.
    pub open: bool,
}

impl NoteTarget {
    /// The identity the UI keys its scroll on: a different item resets it to the top.
    pub fn identity(&self) -> u64 {
        let cell = u64::from(self.tab) << 8 | u64::from(self.slot);
        let what = u64::from(self.cat) << 16 | u64::from(self.idx);
        self.key.rotate_left(17) ^ (cell << 48) ^ (what << 16) ^ 0x9e37_79b9_7f4a_7c15
    }
}

/// Which item the 상세보기 window shows — or, on the bag / 장비 pages, would show if opened
/// from the cell under the cursor. `None` (FAIL CLOSED) without an open menu, outside the
/// three pages, for anything that is not equipment (`cat > 0x0c` — no note exists for it),
/// and on any unreadable or out-of-range byte.
pub fn item_note_target(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<NoteTarget> {
    let c = char_base_raw(read)?;
    // Two reads on every frame of normal play: the menu flag, and that is the end of it.
    if rd_u8(read, c + SELECTOR_FLAG_OFF)? == 0 {
        return None;
    }
    let page = rd_u8(read, c + MENU_PAGE_OFF)?;
    let from = match page {
        PAGE_DETAIL => rd_u8(read, c + MENU_RETURN_PAGE_OFF)?,
        PAGE_ITEM | MENU_SCREEN_EQUIP => page,
        _ => return None,
    };
    // A live record, in the game (the online screens are app state 3 and paint their own
    // DETAIL screen — `menutouch::detail_page` refuses it, and so does this).
    if !in_game_state(read) {
        return None;
    }
    let c = resolve_char(read)?;
    let (tab, slot, copied) = match from {
        PAGE_ITEM => {
            let tab = rd_u8(read, c + INV_TAB_OFF)?;
            if tab >= crate::orbcombine::MAX_TABS {
                return None;
            }
            (tab, rd_u8(read, c + INV_CURSOR_BASE + u32::from(tab))?, None)
        }
        MENU_SCREEN_EQUIP => {
            let s = match rd_i8(read, c + EQUIP_SLOT_CURSOR_OFF)? {
                WORN_ACC2 => WORN_ACC1,
                s => s,
            };
            let r = rd_i8(read, c + EQUIP_CAND_CURSOR_OFF)?;
            if !(0..crate::atkspeed::WORN_SLOTS as i8).contains(&s) || !(0..CAND_MAX).contains(&r) {
                return None;
            }
            let entry = c + CAND_BASE_OFF + s as u32 * CAND_LIST_STRIDE + r as u32 * CAND_ENTRY_STRIDE;
            let copied = (rd_u16(read, entry + CAND_CAT_OFF)?, rd_u16(read, entry + CAND_IDX_OFF)?);
            (rd_u8(read, entry + CAND_TAB_OFF)?, rd_u8(read, entry + CAND_SLOT_OFF)?, Some(copied))
        }
        _ => return None,
    };
    // `orb_cell` range-checks (tab, slot) and refuses an empty record.
    let cell = orb_cell(read, tab, slot)?;
    if !cell.is_equipment() {
        return None;
    }
    // A candidate entry is a COPY of the bag record; one that disagrees is stale (the
    // cursor is on the worn grid, or the list is being rebuilt) — answer nothing.
    if copied.is_some_and(|ci| ci != (cell.cat, cell.idx)) {
        return None;
    }
    let grid = resolve_grid_base(read, c)?;
    let rec = crate::orbcombine::rec_addr(grid, tab, slot)?;
    let derived = rd_u16(read, rec + crate::orbcombine::REC_EFFECTIVE)? as i16;
    Some(NoteTarget {
        tab,
        slot,
        cat: cell.cat,
        idx: cell.idx,
        key: cell.key,
        derived,
        sockets: cell.sockets,
        orb: cell.orb,
        refine: cell.refine,
        open: page == PAGE_DETAIL,
    })
}

/// The equipment DB row of `(cat, idx)` (`*(*(grid + 0) + cat * 4) + idx * 0x20` — the same
/// table the wear recompute and [`worn_option_pairs`] walk). `None` past the equipment
/// categories or on a pointer outside the heap.
fn equip_row(read: &impl Fn(u32, usize) -> Vec<u8>, grid: u32, cat: u16, idx: u16) -> Option<u32> {
    if cat > EQUIP_CAT_MAX as u16 || idx > 0xff {
        return None;
    }
    let tables = rd_u32(read, grid + EQUIP_TABLES_OFF).filter(|p| in_heap(*p))?;
    let table = rd_u32(read, tables + u32::from(cat) * 4).filter(|p| in_heap(*p))?;
    Some(table + u32::from(idx) * EQUIP_ROW_STRIDE)
}

/// A NUL-terminated EUC-KR string the game keeps at `addr`, without its NUL; `None` when it
/// cannot be read, is empty, or runs past `cap` bytes without ending.
fn game_cstr(read: &impl Fn(u32, usize) -> Vec<u8>, addr: u32, cap: usize) -> Option<Vec<u8>> {
    if !in_heap(addr) {
        return None;
    }
    let raw = read(addr, cap);
    let end = raw.iter().position(|b| *b == 0)?;
    (end > 0).then(|| raw[..end].to_vec())
}

/// An equipment row's name, EUC-KR, its colour escape still on (row `+0`, the pointer the
/// painter's `"%s"` reads — `0x45422`).
fn equip_row_name(read: &impl Fn(u32, usize) -> Vec<u8>, row: u32) -> Option<Vec<u8>> {
    game_cstr(read, rd_u32(read, row)?, 0x40)
}

/// `InGame_txt[id]` — the game's `getString 0x1bdcc`: `base + u16_le(base + 4 + id * 2)`,
/// `base = *string_table_ptr` (non-zero only while a menu is open).
fn ingame_text(read: &impl Fn(u32, usize) -> Vec<u8>, id: u16) -> Option<Vec<u8>> {
    let base = rd_u32(read, crate::guest_build::string_table_ptr()?).filter(|p| in_heap(*p))?;
    let off = rd_u16(read, base + 4 + u32::from(id) * 2)?;
    game_cstr(read, base + u32::from(off), 0x40)
}

/// The note for `t`, or `None` when the host changes nothing about its window (an ordinary
/// item) or anything it needs cannot be read. `worn` = [`worn_items`] — what the set section
/// marks as worn, and how many of the set count.
///
/// The NAME is composed exactly as the painter composes it (`0x45410..0x454ee`), because the
/// UI only draws once the window's name box reads back pixel-exact: an orb's label and a
/// space before the row's name (`"%s %s"`), `+N` after it when refined (`"%s+%d"`), each
/// colour escape switching the ink where it stands (the orb labels are all `@` = white, and
/// a name without its own escape stays white after one).
pub fn item_note(read: &impl Fn(u32, usize) -> Vec<u8>, t: &NoteTarget, worn: &[(u16, u16)]) -> Option<crate::itemnote::ItemNote> {
    use crate::itemnote as n;
    let c = resolve_char(read)?;
    let grid = resolve_grid_base(read, c)?;
    let row = equip_row(read, grid, t.cat, t.idx)?;
    let escape_of = |raw: &[u8]| raw.first().copied().filter(|b| crate::gamefont::COLOUR_ESCAPES.contains(b));
    // The orb label first: the painter's branch is `orb + 1 == 0` → none.
    let prefix = match t.orb {
        -1 => None,
        o if o >= 0 => {
            let label = ingame_text(read, ORB_LABEL_BASE + o as u16)?;
            Some((crate::gamefont::from_euckr(&label), escape_of(&label)))
        }
        _ => return None,
    };
    let raw = equip_row_name(read, row)?;
    let own = escape_of(&raw);
    let mut name = crate::gamefont::from_euckr(&raw);
    if t.refine > 0 {
        name.push_str(&format!("+{}", t.refine));
    }
    // The ink the name is drawn in: its own escape, else the one the orb label left set.
    let escape = own.or(prefix.as_ref().and_then(|(_, e)| *e));
    let third = wie_util::acc_options::third_of(t.cat, t.idx);
    let body = if t.cat == 0x0c {
        // 「기절 저항 +0」 goes for the 11 only, and only while it really reads 0 (a refined
        // accessory prints its refine there, and keeps the line).
        n::GameBody::Accessory { drop_resist: third.is_some() && t.derived == 0 }
    } else {
        n::GameBody::Equipment { sockets: t.sockets > 0, orb: t.orb != -1 }
    };
    let sid = crate::setbonus::set_of(t.cat, t.idx, crate::set_items::SET_ITEMS);
    // The set's pieces, named off the item table (no escape): 투구 · 갑옷 · 장갑 · 신발 · 방패 ·
    // 장신구. One unreadable name and the section is not drawn half-blank — no note at all.
    let states = match sid {
        Some(s) => crate::setbonus::piece_states(s, worn, crate::set_items::SET_ITEMS),
        None => Vec::new(),
    };
    let mut names = Vec::with_capacity(states.len());
    for &((pc, pi), _) in &states {
        let r = equip_row(read, grid, pc, pi)?;
        names.push(crate::gamefont::from_euckr(&equip_row_name(read, r)?));
    }
    let pieces: Vec<n::Piece> = states
        .iter()
        .zip(&names)
        .map(|(&((pc, _), w), nm)| n::Piece { slot: crate::setbonus::slot_word(pc), name: nm.as_str(), worn: w })
        .collect();
    let set = sid.and_then(|s| {
        let info = crate::setbonus::SETS.get(usize::from(s))?;
        Some(n::SetView {
            name: info.name,
            bonus: info.bonus,
            text: info.text,
            pieces: &pieces,
            worn: crate::setbonus::piece_counts(worn, crate::set_items::SET_ITEMS)[usize::from(s)],
            need: crate::setbonus::PIECES_FOR_BONUS,
        })
    });
    // The game's own four-armour-piece set (b11, 150차: 8..13 per legacy set): a set ARMOUR
    // piece whose row really carries an id.
    let native_set =
        t.cat <= 3 && sid.is_some() && rd_u8(read, row + EQUIP_ROW_NATIVE_SET_OFF).is_some_and(|b| (b as i8) > 0);
    let mut note = n::build(&n::NoteInput { name: &name, escape, body, third: third.map(|a| a.detail), set, native_set })?;
    note.name_prefix = prefix.map(|(text, e)| (format!("{text} "), n::name_rgb(e)));
    Some(note)
}

/// Every worn `(cat, idx)` — the same nine slots [`worn_option_pairs`] walks, for the 85제
/// set effects (142차, `setbonus::active_mask`). Empty when there is no character (no set).
pub fn worn_items(read: &impl Fn(u32, usize) -> Vec<u8>) -> Vec<(u16, u16)> {
    let Some(c) = resolve_char(read) else { return Vec::new() };
    let mut out = Vec::new();
    for i in 0..crate::atkspeed::WORN_SLOTS as u32 {
        let rec = c + WORN_BASE_OFF + i * WORN_STRIDE;
        let (Some(cat), Some(idx)) = (
            rd_u16(read, rec + WORN_CAT_OFF).map(|v| v as i16),
            rd_u16(read, rec + WORN_IDX_OFF).map(|v| v as i16),
        ) else {
            continue;
        };
        if cat < 0 || cat > EQUIP_CAT_MAX || idx < 0 {
            continue;
        }
        out.push((cat as u16, idx as u16));
    }
    out
}

/// The 공격속도 percent the player's equipment adds up to right now.
pub fn attack_speed_equipment_pct(read: &impl Fn(u32, usize) -> Vec<u8>) -> u32 {
    crate::atkspeed::sum_options(&worn_option_pairs(read))
}

/// Every OCCUPIED equipment cell in the bag, as the `(tab, slot, cat, idx)` tuple the
/// game's own derived-field refresh takes.
///
/// ★★★ 71차-F. The in-game combine calls `0x528f0(tab, slot, cat, idx)` for the cell it
/// just wrote, and that is not enough — the player reported the equip screen's candidate
/// list still showing the pre-combine numbers (2026-08-26). Two things can defeat a
/// single targeted call and BOTH are real:
///
/// * **the bag compacts.** `0x5380c` (wrapper `0x55048`, 23 call sites by 93차's count, the
///   popup CLOSE path among them) moves 0x3c-byte records to fill a gap — and the combine makes
///   a gap whenever it spends the last orb of a stack. The recalc is spawned
///   asynchronously with the tuple captured BEFORE that, so it can land on whatever slid
///   into that cell instead. 69차 wrote this risk down and judged the window "one tick";
///   the popup close is inside that tick.
/// * **the call can simply not have run.** It is `system.spawn`ed; a failure is a
///   `tracing::warn` nobody is watching on a release device.
///
/// So the fix does not chase the cell: it rebuilds the derived halfwords for EVERY
/// equipment record, which is exactly what the save loader does on every load
/// (`0x1a3a4`-`0x1a40a` walks all of them). The call is a pure function of the record's
/// own six bytes, so re-running it on an untouched item is a no-op — idempotent, and
/// immune to anything having moved.
pub fn equipment_cells(read: &impl Fn(u32, usize) -> Vec<u8>) -> Vec<(u8, u8, u16, u16)> {
    let Some(c) = resolve_char(read) else { return Vec::new() };
    let Some(grid) = resolve_grid_base(read, c) else { return Vec::new() };
    let bags = rd_i8(read, grid + crate::orbcombine::GRID_BAG_COUNT)
        .unwrap_or(0)
        .clamp(0, crate::orbcombine::MAX_TABS as i8) as u8;
    let mut out = Vec::new();
    for tab in 0..bags {
        for slot in 0..crate::orbcombine::SLOTS_PER_TAB {
            // `orb_cell` already refuses an unoccupied record (`REC_OCCUPIED`), and
            // `is_equipment` is the game's own `cat <= 0x0c` — the same precondition the
            // save loader gates the refresh on.
            if let Some(cell) = orb_cell(read, tab, slot)
                && cell.is_equipment()
                // ⛔ 69차 §3: a record whose socket bytes disagree is the one thing not to
                // touch. The rebuild indexes the orb table unchecked.
                && crate::orbcombine::consistent(&cell)
            {
                out.push((cell.tab, cell.slot, cell.cat, cell.idx));
            }
        }
    }
    out
}

/// The combined item's derived cache, found by its instance key rather than by a
/// `(tab, slot)` that may have moved: `(tab, slot, sockets, filled, bonus%, stat)`.
/// For the one diagnostic line that says whether the refresh actually landed.
pub fn derived_probe(read: &impl Fn(u32, usize) -> Vec<u8>, key: u64) -> Option<(u8, u8, u8, i8, u16, u16)> {
    let c = resolve_char(read)?;
    let grid = resolve_grid_base(read, c)?;
    let bags = rd_i8(read, grid + crate::orbcombine::GRID_BAG_COUNT)
        .unwrap_or(0)
        .clamp(0, crate::orbcombine::MAX_TABS as i8) as u8;
    for tab in 0..bags {
        for slot in 0..crate::orbcombine::SLOTS_PER_TAB {
            let Some(cell) = orb_cell(read, tab, slot) else { continue };
            if !cell.same_item(key) {
                continue;
            }
            let rec = crate::orbcombine::rec_addr(grid, tab, slot)?;
            let bonus = rd_u16(read, rec + crate::orbcombine::REC_BONUS_PCT)?;
            let stat = rd_u16(read, rec + crate::orbcombine::REC_EFFECTIVE)?;
            return Some((tab, slot, cell.sockets, cell.filled, bonus, stat));
        }
    }
    None
}

/// The `item_bm` frame the game itself would draw for `(cat, idx)`, with the generic
/// frame as a last resort.
///
/// ★ 71차-D: the in-game 오브결합 picker draws this in its left column, so a row says
/// 투구 / 갑옷 / 검 / 방패 at a glance (player request 2026-08-26: *"투구인지 갑옷인지
/// 무기인지 장신구인지 등 아이콘도 혹시 맨왼쪽에 표시가능해?"*). It is the SAME rule the
/// quickbar uses — [`icon_frame`] — precisely so the two never disagree about what an
/// item looks like. For equipment that rule reduces to `frame = cat`, which is why the
/// atlas's first thirteen frames are 투구·갑옷·장갑·신발·스피어·스워드·나이프·단총·
/// 캐논·암흑성·광휘석·방패·혼 in category order.
pub fn item_frame(read: &impl Fn(u32, usize) -> Vec<u8>, cat: u16, idx: u16) -> u16 {
    resolve_char(read)
        .and_then(|c| icon_frame(read, c, cat, idx))
        .unwrap_or(ITEM_ICON_FALLBACK)
}

/// Diagnostic snapshot for a periodic logcat line: (CHAR ptr, field?, window id,
/// per-slot type bytes, app state). Aids device validation without a UI.
/// ★ Also returns the three bytes the HUD's whole mode decision is made of, because
/// "메뉴를 열면 우측 퀵바가 사라진다" (player, 2026-08-04) cannot be resolved by reading
/// the code — every candidate needs a value:
///
/// * **app state** `CHAR+state_off()` (KTF `0x7c` / LGT `0x78`, §2.7) — `in_game_state`
///   is `state == 4` and that is what decides whether the slot list is published at
///   all. This helper reads the per-build offset, not a hardcoded `0x7c`: on an LGT
///   boot a maintainer must look at `0x78`.
/// * **window id** `CHAR+0xa0c0` — `cutscene_open` is `window == 4`, and a menu taken for
///   the SCRIPT window makes `quick_buttons` return nothing by design.
/// * **selector / menu-overlay flag** `CHAR+0x9c6c` — `in_field_state` demands it be 0.
///   17차's RE is the reason to log it next to the window id: `0x124208` branches on THIS
///   byte before it ever reads the window id, so **a menu can be up while the window id
///   still reads 3 (= "field")**. The two bytes disagree, and which one the host trusts
///   decides the mode.
pub fn debug_probe(read: &impl Fn(u32, usize) -> Vec<u8>) -> (Option<u32>, bool, Option<u8>, [u8; 5], Option<u32>, Option<u8>) {
    let char_ptr = resolve_char(read);
    let field = in_field_state(read);
    let mut types = [0u8; 5];
    if let Some(c) = char_ptr {
        for s in 0..QB_SLOTS as usize {
            types[s] = rd_u8(read, c + QB_TYPE_OFF + s as u32).unwrap_or(0);
        }
    }
    let state = char_ptr.and_then(|c| rd_u32(read, c + crate::guest_build::state_off()));
    let selector = char_ptr.and_then(|c| rd_u8(read, c + SELECTOR_FLAG_OFF));
    (char_ptr, field, window_id(read), types, state, selector)
}

/// The three numbers [`resolve_char`] judges, read raw and **without any gate**, so a
/// refusal can say WHICH term refused it.
///
/// ⛔ THIS EXISTS BECAUSE THE REFUSAL WAS SILENT. `resolve_char` returning `None` takes
/// the whole host blind — HUD, quick plates, every touch target that resolves against
/// CHAR — and the only evidence was `char=None`, which cannot tell "no character has
/// been created" from "a live record failed one plausibility test". 71차-L already paid
/// for that once (`hp <= hp_max` refused every unequipped character), and 80차-E paid
/// again in the network warehouse. Same lesson as the difficulty byte (78차-C): fail
/// closed, but SAY SO.
///
/// `None` on a build with no static anchor (there is nothing to read).
pub fn char_raw_stats(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u32, Option<u16>, Option<u32>, Option<u32>)> {
    let crate::guest_build::CharAnchor::Static(base) = crate::guest_build::char_anchor()? else {
        return None;
    };
    Some((
        base,
        rd_u16(read, base + LEVEL_OFF),
        rd_u32(read, base + HP_MAX_OFF),
        rd_u32(read, base + SP_OFF),
    ))
}

/// Every CHAR offset this crate reads that is **not yet verified on the running
/// build**, on one line, next to the offsets that ARE verified.
///
/// ── WHY THIS EXISTS (MIGRATION_LGT.md §2.11) ─────────────────────────────────
/// The LGT port has a class of risk the gates cannot cover: reads that are switched
/// ON with offsets lifted from KTF. If one of those moved — and two provably did,
/// the state byte `+0x7c`→`+0x78` and the HUD's `+0xa384`→`+0xa370` — the feature
/// does not disappear, it shows a WRONG value. §2.11 lists five such rows
/// (quickbar / badges / secondary weapon / no-save area / skill cooldown).
///
/// `wie_probe --verify-offsets` prints the same numbers, but the probe cannot reach
/// the states that make them informative: a level-up (badges are 0 at level 1), a
/// bound quick slot (nothing is bound in a fresh game), an equipped secondary, an
/// open item popup. A human playing on the device reaches all of them in minutes, so
/// this is the same audit taken to where the evidence is. Compare each field with
/// what is on screen; a REF block that matches while a 🔶 field does not is exactly
/// a moved offset.
///
/// The popup cluster is here for a second reason: it is the whole trigger path for
/// every cash buff. Until 2026-08-11 [`cash_use_pending`] resolved the label buffers
/// through KTF's `sl`-relative slot, which on LGT is not even mapped (device-measured
/// `handles=None`), so no buff could start and the EXP x2/x4 and level-relief patches
/// were unreachable however correct their sites were. `handles` is now the build's own
/// anchor (`guest_build::popup_handles`), and `label` is the nine bytes at the
/// highlighted row — `bbe7bfebc7cfb1e200` is `사용하기`, the string that has to be
/// there before a buff is granted. A resolving `handles` with a readable `label` while
/// an item popup is open is what says the port is live.
///
/// Read-only and non-panicking: every access goes through the bounds-checked `rd_*`
/// helpers, so an unmapped address yields `None` rather than a fault.
/// Base of the game's OPTION block — 16 bytes, and `[0]`/`[1]` are the two sound levels.
///
/// ★★★ 2026-08-28 — RESOLVED FROM `saveOptions`/`loadOptions`, not guessed.
/// `saveOptions 0x1b150` is `malloc(0x10)` -> `memcpy(buf, 0x150e980, 0x10)` ->
/// `writeSaveRecord("OptionSave_11"(0x5ca1c), buf, 0x10)`, and `loadOptions 0x1b18c`
/// fills DEFAULTS into the same block when the record is missing:
///
/// ```text
///   0x1b1b4  movs r3,#3 ; strb r3,[r1]      <- [0] = 3   배경음
///   0x1b1b8             ; strb r3,[r1,#1]   <- [1] = 3   효과음
///   0x1b1b0  movs r3,#8 ; strb r3,[r1,#2]
///   0x1b1ba  movs r3,#2 ; strb r3,[r1,#4]
/// ```
///
/// Both defaults are **3**, which is exactly the game's documented default for the two
/// sliders (the probe sees `MC_mdaClipSetVolume(clip, 60)` = 3 x 20 on both buses), and
/// the record name is one this repo already knows (`savevault.rs` treats `OptionSave_11`
/// as a non-character record).
///
/// ⛔ `0x150eb50` is NOT this block — that is the audio module's clip pointer plus the
/// current BGM track id, and a device dump showed its bytes never following the slider.
pub const AUDIO_OPTS: u32 = 0x0150_e980;

/// Highest level either sound slider can hold (5 bars).
pub const SOUND_LEVEL_MAX: u8 = 5;

/// `(배경음 level, 효과음 level)`, each `0..=SOUND_LEVEL_MAX`.
///
/// ★★★ 2026-08-28 — **`[1]` IS 배경음 AND `[0]` IS 효과음**, and that order is the
/// opposite of what the `loadOptions` default block suggests. It is not a guess: with the
/// player moving ONLY the 배경음 row one step, the device printed these four lines in a
/// row (S9, vc196):
///
/// ```text
///   audio opts 0x150e980: 03 02 ...      <- [1] went 3 -> 2
///   audio: music bus level 40 (gain 0.160)          <- the guest's own path: 40 = 2 x 20
///   audio: music bus level 60 ... from the 배경음 option  <- THIS function, reading [0] = 3
///   music bus level 40 adopted at putData           <- and the guest putting 40 back
/// ```
///
/// So the guest itself derives **40** from `[1]`, which settles it. The block also carries
/// the two derived volumes — `[7] = [0] * 20` and `[8] = [1] * 20` — and those are checked
/// below, because a layout drift that silently swapped the pair again is exactly the
/// mistake this comment exists to prevent.
///
/// `None` when the block has not been initialised yet (all sixteen bytes zero, i.e. before
/// `loadOptions` ran), when either level is out of range, or when the derived volumes do
/// not agree with the levels — all three fail toward "do not touch the mix".
pub fn audio_option_levels(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u8, u8)> {
    let b = read(AUDIO_OPTS, 16);
    if b.len() < 16 || b.iter().all(|&x| x == 0) {
        return None;
    }
    let (bgm, sfx) = (b[1], b[0]);
    if bgm > SOUND_LEVEL_MAX || sfx > SOUND_LEVEL_MAX {
        return None;
    }
    // The game's own `level * 20`, which it keeps alongside. If these have drifted the
    // layout is not what this function thinks it is — say nothing rather than guess.
    if b[8] != bgm.saturating_mul(20) || b[7] != sfx.saturating_mul(20) {
        return None;
    }
    Some((bgm, sfx))
}

/// Raw dump of the option block, for the change-gated worker log.
pub fn audio_opts_audit(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<String> {
    let b = read(AUDIO_OPTS, 16);
    if b.len() < 16 {
        return None;
    }
    let hex: Vec<String> = b.iter().map(|x| format!("{x:02x}")).collect();
    Some(format!(
        "audio opts {AUDIO_OPTS:#x}: {} | 배경음 {} (vol {}) 효과음 {} (vol {})",
        hex.join(" "),
        b[1],
        b[8],
        b[0],
        b[7],
    ))
}

pub fn offset_audit(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<String> {
    let c = resolve_char(read)?;
    let mut s = format!("offset audit ({}) CHAR={c:#x}", crate::guest_build::name());
    // VERIFIED reference block: if these do not match the status screen the base is
    // wrong and nothing after it means anything.
    s += &format!(
        " | REF lv={:?} exp={:?}/{:?} hp={:?}/{:?} sp={:?} win={:?} state={:?} sel={:?}",
        rd_u16(read, c + LEVEL_OFF),
        rd_u32(read, c + EXP_OFF),
        rd_u32(read, c + EXP_NEXT_OFF),
        rd_u16(read, c + HP_OFF),
        rd_u32(read, c + HP_MAX_OFF),
        rd_u32(read, c + SP_OFF),
        rd_u8(read, c + WINDOW_ID_OFF),
        rd_u32(read, c + crate::guest_build::state_off()),
        rd_u8(read, c + SELECTOR_FLAG_OFF),
    );
    // 🔶 #6 badges — compare with the two "+" badges on the status screen.
    s += &format!(
        " | #6 badge stat={:?} skill={:?}",
        rd_u16(read, c + STAT_POINTS_OFF).map(|v| v as i16),
        rd_u8(read, c + SKILL_POINTS_OFF),
    );
    // 🔶 #5 quickbar — compare with the five cells the overlay draws.
    for slot in 0..QB_SLOTS {
        s += &format!(
            " | #5 qb{slot} t={:?} i={:?} s={:?} cell={:?}",
            rd_u8(read, c + QB_TYPE_OFF + slot),
            rd_u8(read, c + QB_ITEMREF_OFF + slot),
            rd_u8(read, c + QB_SKILLREF_OFF + slot),
            rd_i8(read, c + QB_SKILLIDX_OFF + slot),
        );
    }
    // 🔶 #5b on-field sheet ids — the byte `skill_icon` gates the atlas on (4..=10,
    // atlas index = value-4; anything else draws the empty box of §2.18-(2)). The
    // OFFSET is closed on LGT by static RE (loader fn 0x4957c reads
    // (CHAR+0x28)+0x8c+0x1d+i = CHAR+0xd1+i, same as KTF); this line closes the VALUE.
    s += &format!(
        " | #5b sheet={:?}/{:?}",
        rd_u8(read, c + SKILL_SHEET_OFF),
        rd_u8(read, c + SKILL_SHEET_OFF + 1),
    );
    // 🔶 #7 secondary weapon — compare with the EQUIPMENT screen's 보조무기 row.
    s += &format!(
        " | #7 equip present={:?} tab={:?} slot={:?}",
        rd_i8(read, c + EQUIP6_PRESENT_OFF),
        rd_i8(read, c + EQUIP6_TAB_OFF),
        rd_i8(read, c + EQUIP6_SLOT_OFF),
    );
    // 🔶 #8 no-save AREA. The object itself is the one the minimap renders from, so a
    // resolving `area` means the container hop is right and only `+0x11d` is open.
    let area = resolve_area_base(read, c);
    s += &format!(
        " | #8 area={area:#x?} can_save={:?}",
        area.and_then(|a| rd_i8(read, a + crate::save::AREA_CAN_SAVE_OFF)),
    );
    // 🔶 #18 skill cooldown — mode's bit 7 plus a 0x0a..0x0d category is "casting".
    s += &format!(
        " | #18 cast mode={:?} cat={:?}",
        rd_u8(read, c + ACTION_MODE_OFF),
        rd_u8(read, c + ACTION_CAT_OFF),
    );
    // #17 popup cluster — the gate every cash buff has to pass. `label` is the 9 bytes
    // at the highlighted row; `bbe7bfebc7cfb1e200` is 사용하기 (`LABEL_USE`).
    let handles = popup_handles(read);
    s += &format!(
        " | #17 pop page={:?} open={:?} rows={:?} sel={:?} tag={:?} tab={:?} handles={handles:#x?}",
        rd_u8(read, c + MENU_PAGE_OFF),
        rd_u8(read, c + POPUP_OPEN_OFF),
        rd_i8(read, c + POPUP_COUNT_OFF),
        rd_i8(read, c + POPUP_SEL_OFF),
        rd_i8(read, c + POPUP_TAG_OFF),
        rd_u8(read, c + INV_TAB_OFF),
    );
    if let (Some(h), Some(sel)) = (handles, rd_i8(read, c + POPUP_SEL_OFF)) {
        let label = (sel >= 0)
            .then(|| popup_label_addr(read, h, sel as u32))
            .flatten()
            .map(|a| read(a, LABEL_USE.len()));
        s += &format!(" label={:02x?}", label.as_deref());
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn reader(map: HashMap<u32, u8>) -> impl Fn(u32, usize) -> Vec<u8> {
        move |addr, len| {
            let mut v = Vec::with_capacity(len);
            for i in 0..len as u32 {
                match map.get(&(addr + i)) {
                    Some(b) => v.push(*b),
                    None => return Vec::new(),
                }
            }
            v
        }
    }
    fn put_u16(m: &mut HashMap<u32, u8>, a: u32, v: u16) {
        for (i, b) in v.to_le_bytes().into_iter().enumerate() {
            m.insert(a + i as u32, b);
        }
    }
    fn put_u32(m: &mut HashMap<u32, u8>, a: u32, v: u32) {
        for (i, b) in v.to_le_bytes().into_iter().enumerate() {
            m.insert(a + i as u32, b);
        }
    }

    // The KTF pair, which is the build these tests exercise; the live values are
    // `guest_build::char_anchor()`.
    const CHAR_PTR_GLOBAL: u32 = 0x1b_3084;
    const CHAR: u32 = 0x1b_7b6c;
    const HANDLE: u32 = 0x4020_0000;
    const BLOCK: u32 = 0x4021_0000;
    /// KTF's container payload offset (`*(handle) + 8`) — these tests set the KTF
    /// build, so `deref_container` takes that arm. See `guest_build::container_data_off`.
    const KTF_DATA_ADJ: u32 = 8;
    fn gbase() -> u32 {
        BLOCK + KTF_DATA_ADJ
    }

    /// Seed the new-save quickbar: slot0=skill, slot1=skill, slot2=item(tab0,slot5),
    /// slots3,4=empty; item stack count 13 in the inventory grid.
    fn seed(m: &mut HashMap<u32, u8>, state: u32) {
        // `resolve_char` asks which carrier booted; nothing sets that in a unit test.
        crate::guest_build::set_ktf();
        put_u32(m, CHAR_PTR_GLOBAL, CHAR);
        put_u32(m, CHAR + crate::guest_build::state_off(), state);
        m.insert(CHAR + SELECTOR_FLAG_OFF, 0);
        // No window open = the field owns the screen (see WINDOW_ID_OFF).
        m.insert(CHAR + WINDOW_ID_OFF, WIN_NONE);
        // types
        m.insert(CHAR + QB_TYPE_OFF, TY_SKILL);
        m.insert(CHAR + QB_TYPE_OFF + 1, TY_SKILL);
        m.insert(CHAR + QB_TYPE_OFF + 2, TY_ITEM);
        m.insert(CHAR + QB_TYPE_OFF + 3, 0);
        m.insert(CHAR + QB_TYPE_OFF + 4, 0);
        // refs
        m.insert(CHAR + QB_SKILLREF_OFF, 0x6a); // skill id 0x6a&0x1f = 0x0a
        m.insert(CHAR + QB_SKILLREF_OFF + 1, 0x2d);
        m.insert(CHAR + QB_ITEMREF_OFF + 2, 5); // tab0 slot5
        // inventory grid for count
        put_u32(m, CHAR + GRID_HANDLE_OFF, HANDLE);
        put_u32(m, HANDLE, BLOCK);
        put_u16(m, gbase() + 0 * TAB_STRIDE + 5 * SLOT_STRIDE + COUNT_OFF, 13);
    }

    /// The whole PLATE row is resolved from the STORE, and an item plate carries its
    /// live stack count from the bag — which is the one thing the store cannot hold,
    /// because it changes every time the player drinks one.
    #[test]
    fn plate_views_resolve_every_plate_and_the_live_item_count() {
        use crate::quickpage::{Entry, Window};
        let mut m = HashMap::new();
        seed(&mut m, STATE_FIELD);
        let r = reader(m);
        let mut plates = [Entry::Empty; crate::quickpage::PLATES];
        plates[0] = Entry::Skill {
            cat: 1,
            cell: 5,
            class: 7,
            sref: Some(0x6a),
        };
        // The seeded bag cell (0,5) holds count 13 with cat/idx 0 — so this is the
        // stamp that MATCHES, and the plate resolves.
        plates[2] = Entry::Item {
            tab: 0,
            slot: 5,
            cat: 0,
            idx: 0,
        };
        // ...and a plate PAST the guest's five is resolved exactly like the rest.
        plates[6] = Entry::Item {
            tab: 0,
            slot: 5,
            cat: 0,
            idx: 0,
        };
        let mut w = Window::new();
        w.seat(0, 0);
        let v = plate_views(&r, &plates, &w, &|_, _| 0);
        assert_eq!(v.len(), crate::quickpage::PLATES);
        assert_eq!(v[0].kind, SlotKind::Skill);
        assert_eq!(v[0].guest, Some(0), "plate 0 is the one that is resident");
        assert_eq!(v[1].kind, SlotKind::Empty);
        assert_eq!(v[2].kind, SlotKind::Item);
        assert_eq!(v[2].count, 13);
        assert_eq!(v[2].guest, None, "resolved without being in the guest's bar");
        assert_eq!(v[6].kind, SlotKind::Item);
        assert_eq!(v[6].count, 13);
        assert_eq!((v[0].slot, v[2].slot, v[6].slot), (0, 2, 6));
    }

    /// An item plate whose bag cell no longer holds the stamped record draws EMPTY.
    /// A non-resident plate gets no help from the game's own unbind sweep, so this is
    /// the only thing standing between the player and a potion icon that is really
    /// somebody else's item.
    #[test]
    fn a_plate_whose_item_moved_draws_empty() {
        use crate::quickpage::{Entry, Window};
        let mut m = HashMap::new();
        seed(&mut m, STATE_FIELD);
        let r = reader(m);
        let mut plates = [Entry::Empty; crate::quickpage::PLATES];
        plates[5] = Entry::Item {
            tab: 0,
            slot: 5,
            cat: 0x12,
            idx: 0x03,
        };
        let v = plate_views(&r, &plates, &Window::new(), &|_, _| 0);
        assert_eq!(v[5].kind, SlotKind::Empty);
        assert_eq!(v[5].count, 0);
    }

    #[test]
    fn hotkey_map_matches_re() {
        use wie_backend::KeyCode;
        assert_eq!(crate::quickslot_hotkey(0), Some((KeyCode::NUM1, '1')));
        assert_eq!(crate::quickslot_hotkey(1), Some((KeyCode::NUM3, '3')));
        assert_eq!(crate::quickslot_hotkey(2), Some((KeyCode::NUM9, '9')));
        assert_eq!(crate::quickslot_hotkey(3), Some((KeyCode::NUM0, '0')));
        assert_eq!(crate::quickslot_hotkey(4), Some((KeyCode::HASH, '#')));
        assert_eq!(crate::quickslot_hotkey(5), None);
    }

    #[test]
    fn field_gate_requires_no_window_state4_and_clear_selector() {
        let mut ok = HashMap::new();
        seed(&mut ok, STATE_FIELD);
        assert!(in_field_state(&reader(ok)));

        let mut menu = HashMap::new();
        seed(&mut menu, 3);
        assert!(!in_field_state(&reader(menu)));

        // A window (map / inventory / save / cutscene) leaves CHAR+0x7c at 4, so the
        // window id is the only thing that catches it.
        for win in [WIN_MAP, 4, 14, 15, 16] {
            let mut m = HashMap::new();
            seed(&mut m, STATE_FIELD);
            m.insert(CHAR + WINDOW_ID_OFF, win);
            assert!(!in_field_state(&reader(m.clone())), "window {win} is not the field");
            assert_eq!(window_id(&reader(m)), Some(win));
        }
    }

    #[test]
    fn gauges_use_the_corrected_hp_sp_offsets() {
        // Regression for the swap that made the red bar ignore HP potions: HP is the
        // s16 at CHAR+0xf2 over the s32 max at CHAR+0xec; SP is CHAR+0xf8 over 1000.
        let mut m = HashMap::new();
        seed(&mut m, STATE_FIELD);
        put_u16(&mut m, CHAR + LEVEL_OFF, 12);
        put_u32(&mut m, CHAR + EXP_OFF, 1234);
        put_u32(&mut m, CHAR + EXP_NEXT_OFF, 15000);
        put_u32(&mut m, CHAR + HP_MAX_OFF, 632); // ComputeMaxHP(level 12, vit 20)
        put_u16(&mut m, CHAR + HP_OFF, 632); // full
        put_u32(&mut m, CHAR + SP_OFF, 400);
        // 19차 added the badge fields to `read_stats`, and a map that omits them
        // reads as "not a character" (`rd_u16` -> None). This test could never run
        // on the Windows host (the test binary dies at startup there), so the gap
        // only surfaced when `cargo test` first ran on a Linux host (6차).
        put_u16(&mut m, CHAR + STAT_POINTS_OFF, 3);
        m.insert(CHAR + SKILL_POINTS_OFF, 1);
        let s = read_stats(&reader(m)).expect("plausible");
        assert_eq!((s.hp, s.hp_max), (632, 632), "full HP must read full");
        assert_eq!((s.sp, s.sp_max), (400, SP_MAX));
        assert_eq!((s.stat_points, s.skill_points), (3, 1));
    }

    #[test]
    fn read_sp_survives_hp_above_max() {
        // 73차: SkillWatch used `read_stats`, which still requires hp<=hp_max.
        // Unequip produces that state; the watch then saw None/None and never
        // confirmed a cast. `read_sp` is CHAR + the SP word only.
        //
        // ⛔⛔ 80차-E TOOK THAT GATE OUT, and this assertion was left behind asserting the
        // old rule — the exact trap the same 회차 caught in `items_selftest.rs`'s hand-written
        // mirror (`hp_max == 0` / `hp > hp_max` were both asserted to be REFUSED). `hp_max`
        // is a DERIVED field with three writers, none of which runs on the network-warehouse
        // path, so refusing a reading because of it turned the live character into "no
        // character" and took half of `menu_tap_decide` with it. `level` is the only witness
        // now, and `bar()` clamps with `cur.min(max)` when it draws. So: `read_stats` MUST
        // succeed here, and `read_sp` — the 73차 point of this test — still answers.
        let mut m = HashMap::new();
        seed(&mut m, STATE_FIELD);
        put_u16(&mut m, CHAR + LEVEL_OFF, 12);
        put_u32(&mut m, CHAR + EXP_OFF, 1234);
        put_u32(&mut m, CHAR + EXP_NEXT_OFF, 15000);
        put_u32(&mut m, CHAR + HP_MAX_OFF, 400);
        put_u16(&mut m, CHAR + HP_OFF, 632);
        put_u32(&mut m, CHAR + SP_OFF, 400);
        put_u16(&mut m, CHAR + STAT_POINTS_OFF, 3);
        m.insert(CHAR + SKILL_POINTS_OFF, 1);
        let s = read_stats(&reader(m.clone())).expect("hp>hp_max is a clamp, not a refusal (80차-E)");
        assert_eq!((s.hp, s.hp_max), (632, 400), "both numbers are reported as they are read");
        assert_eq!(read_sp(&reader(m)), Some(400));
    }

    #[test]
    fn read_cd_reduce_pct_is_the_addstat_word() {
        let mut m = HashMap::new();
        seed(&mut m, STATE_FIELD);
        put_u16(&mut m, CHAR + LEVEL_OFF, 12);
        put_u32(&mut m, CHAR + EXP_OFF, 1234);
        put_u32(&mut m, CHAR + EXP_NEXT_OFF, 15000);
        put_u32(&mut m, CHAR + HP_MAX_OFF, 400);
        put_u16(&mut m, CHAR + HP_OFF, 632);
        put_u32(&mut m, CHAR + SP_OFF, 400);
        put_u16(&mut m, CHAR + STAT_POINTS_OFF, 3);
        m.insert(CHAR + SKILL_POINTS_OFF, 1);
        put_u16(&mut m, CHAR + CD_REDUCE_OFF, 18);
        assert_eq!(read_cd_reduce_pct(&reader(m.clone())), Some(18));
        // ★ 158차 P2: the same cap the guest hook applies (섬광 45 + 오딘 38 + 굴베이그 30 = 113).
        let mut over = m.clone();
        put_u16(&mut over, CHAR + CD_REDUCE_OFF, 113);
        assert_eq!(read_cd_reduce_pct(&reader(over)), Some(cd_reduce_cap().min(113) as u16), "main 70 · vanilla ring 100");
        let mut neg = m.clone();
        put_u16(&mut neg, CHAR + CD_REDUCE_OFF, (-20i16) as u16);
        assert_eq!(read_cd_reduce_pct(&reader(neg)), Some(0), "a negative sum lengthens nothing on the ring");
        let mut empty = m;
        empty.remove(&(CHAR + CD_REDUCE_OFF));
        empty.remove(&(CHAR + CD_REDUCE_OFF + 1));
        assert_eq!(read_cd_reduce_pct(&reader(empty)), None);
    }

    #[test]
    fn no_char_yields_empty() {
        crate::guest_build::set_ktf();
        let mut m = HashMap::new();
        put_u32(&mut m, CHAR_PTR_GLOBAL, 0x10); // out-of-heap
        let plates = [crate::quickpage::Entry::Empty; crate::quickpage::PLATES];
        assert!(plate_views(&reader(m), &plates, &crate::quickpage::Window::new(), &|_, _| 0).is_empty());
    }

    // ── skill_icon_of = (on-field hero sheet, cell) — mirrors 0x14a51a ────────
    //
    // ★ The argument used to be a quickbar SLOT and the two bytes were read out of
    //   `CHAR+0x25c`/`+0x26c`. It is now `(cat, cell)` — the same two values, taken
    //   from the plate's stored identity instead, because most plates are not in the
    //   guest's bar to be read from (`quickpage.rs`). `cat` IS the ref's top two bits,
    //   so the device ground truth below carries over unchanged.
    #[test]
    fn skill_icon_uses_member_sheet_and_cell() {
        // Live SLOT3 ground truth: slot0 ref 0x6a idx 5, slot1 ref 0x2d idx 5;
        // hero0 sheet 7 (s7/atlas3), hero1 sheet 8 (s8/atlas4).
        let mut m = HashMap::new();
        m.insert(CHAR + SKILL_SHEET_OFF, 7); // member 0 sheet = s7
        m.insert(CHAR + SKILL_SHEET_OFF + 1, 8); // member 1 sheet = s8
        let r = reader(m);
        // 0x6a >> 6 = 1 -> member 1; 0x2d >> 6 = 0 -> member 0.
        assert_eq!(skill_icon_of(&r, CHAR, 1, 5), Some(IconRef::Skill(4, 5))); // s8 cell5 = 일심
        assert_eq!(skill_icon_of(&r, CHAR, 0, 5), Some(IconRef::Skill(3, 5))); // s7 cell5 = 장탄
    }

    #[test]
    fn skill_icon_cell_past_the_strip_is_none() {
        let mut m = HashMap::new();
        m.insert(CHAR + SKILL_SHEET_OFF, 7);
        assert_eq!(skill_icon_of(&reader(m), CHAR, 0, SKILL_STRIP_CELLS as u8), None);
    }

    #[test]
    fn skill_icon_bad_sheet_is_none() {
        let mut m = HashMap::new();
        m.insert(CHAR + SKILL_SHEET_OFF + 1, 0); // sheet 0 -> invalid (< 4)
        assert_eq!(skill_icon_of(&reader(m), CHAR, 1, 3), None);
    }

    #[test]
    fn skill_icon_clamps_to_two_onfield_sheets() {
        // The game tests `(ref as i8) >> 6 != 0`, so refs with top bits 2 or 3
        // still resolve to the SECOND sheet (CHAR+0xd2) — not 0xd3 / 0xd4.
        let mut m = HashMap::new();
        m.insert(CHAR + SKILL_SHEET_OFF, 7);
        m.insert(CHAR + SKILL_SHEET_OFF + 1, 8);
        m.insert(CHAR + SKILL_SHEET_OFF + 3, 10); // must NOT be picked
        assert_eq!(skill_icon_of(&reader(m), CHAR, 3, 2), Some(IconRef::Skill(4, 2)));
    }

    /// The `LevelBias` arm of the equip relief, exercised through its pure half.
    #[test]
    fn equip_relief_bias_patch_rewrites_only_code_it_recognises() {
        const ORIG: [u16; 2] = [0x2300, 0x5EE1]; // movs r3,#0 ; ldrsh r1,[r4,r3]
        const OURS5: [u16; 2] = [0x8821, 0x3105]; // ldrh r1,[r4,#0] ; adds r1,#5
        // The shipped pair takes the -5 rewrite; the rewrite restores on 0.
        assert_eq!(equip_relief_bias_patch(ORIG, 5), Some(OURS5));
        assert_eq!(equip_relief_bias_patch(OURS5, 0), Some(ORIG));
        // Already what is asked for -> no write (the idempotence emulator.rs relies
        // on to make this block a no-op for a player with no live buff).
        assert_eq!(equip_relief_bias_patch(ORIG, 0), None);
        assert_eq!(equip_relief_bias_patch(OURS5, 5), None);
        // Re-tiering without restoring first: ours -> ours.
        assert_eq!(equip_relief_bias_patch(OURS5, 3), Some([0x8821, 0x3103]));
        // Anything else on site is a different revision: refuse both directions. The
        // last two are the REQUIREMENT-side pair 7차 rejected — if a table ever aimed
        // this arm four bytes further on, it must refuse rather than write an `ldrb`
        // where the game has an `ldrsb`.
        for alien in [
            [0x2300u16, 0x5EE2],
            [0x2301, 0x5EE1],
            [0x8821, 0x3205],
            [0, 0],
            [0x2302, 0x56D3],
            [0x7893, 0x3B05],
        ] {
            assert_eq!(equip_relief_bias_patch(alien, 5), None, "{alien:04x?}");
            assert_eq!(equip_relief_bias_patch(alien, 0), None, "{alien:04x?}");
        }
        // Out-of-bound levels are the encoder's refusal, passed through.
        assert_eq!(equip_relief_bias_patch(ORIG, 8), None);
    }
}

// ── minimap: RENDER IT OURSELVES from the map arrays ─────────────────────────
//
// ★ REPLACES the old "mirror the guest's offscreen framebuffer" pipeline entirely
// (2026-08-03). That pipeline could only ever produce a picture the GUEST had
// painted, and the guest paints its minimap buffer in exactly one place — the map
// window's ENTER handler. So keeping the panel current meant driving that window
// open and closed behind a frozen screen, which is the flicker the player reported
// ("방에 입장하면 깜빡하면서 미니맵이 떴다가 꺼진다"), and identifying "which
// buffer is the current map's" was guesswork that sometimes latched a picture of
// the field instead ("미니맵 자리에 실제 게임 맵이 스크린샷돼서 뜬다").
//
// None of that is necessary: the map window does not have any private data. It
// paints from the SAME two arrays the movement code walks on, which are live on
// every map at every moment. So the host can draw the identical image at any time,
// from RAM, with no window, no key presses and no screen freeze.
//
// ── the guest routine, decoded (static disasm 2026-08-03) ────────────────────
// `openWindow(8)` -> guest 0x12c938 -> **0x17ba44**:
//     MAP   = *(*(CHAR+0x9c90)) + 8      KTF indirect handle (the `+8` data offset
//                                        the inventory grid uses -- GRID_DATA_ADJ)
//     w,h   = (i32)MAP+0x48 / +0x4c      in TILES
//     fb    = MC_grpCreateOffScreenFrameBuffer(w*2, h*2)
//     FillRect(fb, 0,0, w*2,h*2, 0,0,0)  then for every pixel:
//         c = pixel_colour(px, py)                          (0x17b8e4)
//         FillRect(fb, px, py, 1,1, c>>16, c>>8 & 0xff, c & 0xff)
// `pixel_colour(px, py)` — 2x supersample with a corner blend (0x17b8e4):
//     tx,ty = px>>1, py>>1 ; base = tile_colour(tx, ty)
//     nx = px odd ? tx+1 : tx-1 ;  ny = py odd ? ty+1 : ty-1
//     c1 = tile_colour(nx, ty) ; c2 = tile_colour(tx, ny)   (-1 -> use base)
//     blend only when base differs from BOTH, and base != the global at 0x1a4bc4;
//     then each channel = (base + c1 + c2) / 3   (0x19f874 = signed divide)
// `tile_colour(tx, ty)` (0x17b874), out of bounds -> -1:
//     blocked = (attr[ty*w+tx] & 0x80) != 0                 (0x17a6f8)
//     c = blocked ? 0x5b5b5b : 0xd9d4be
//     if !(tx==0 && ty==0) && (i8)event[ty*w+tx] > 0 -> c = 0x225acc   (0x17a6b4)
// with `attr = *(*(MAP+0x60)) + 8` (u8[w*h]) and `event = *(*(MAP+0x68)) + 8`
// (i8[w*h]).
//
// ── why those arrays are safe to read at any time ────────────────────────────
// They are not map-window state: `0x17a6f8` has five callers and `0x17a6b4` six,
// and only one each belongs to the minimap — the rest are the field's own
// collision and warp checks, which run while the player walks. If these were ever
// stale the player would walk through walls.
//
// ── verified, not assumed ────────────────────────────────────────────────────
// `wie_probe --map-check` renders this and diffs it against the buffer the guest
// itself painted, on the real save: **0 of 5040 pixels differ** (70x72 = a 35x36
// tile map), and the render is already correct at a frame where the map window has
// never been opened (`last_offscreen_copy() == None`).

// The renderer itself, in a dependency-free file so `wie_probe --map-check` can
// `include!` the SHIPPING source and diff its output against the buffer the guest
// paints. Measured on the real save: 0 of 5040 pixels differ on the first map and
// 0 of 1224 on a second one reached by walking through a door — and both renders
// are already correct at frames where the map window has never been opened.
// ★ 141차: the picture renderer, the player-tile reader and the stamp in there are the
// probe's now (`wie_probe --map-check` proves the pointer chain against the game's own
// paint); the app draws tile classes. Same file, so they stay one source — the
// probe-only items carry `#[allow(dead_code)]` in it.
include!("minimap_render.rs");

/// The DRAW corner the player's sprite is blitted from, the box the arrival placement
/// sizes it with, and the kind byte that picks the vertical rule — all in the same
/// object as the position pair above. See [`crate::guardian::draw_offsets`] for the
/// disassembly; the short version is that `+0x16`/`+0x18` are what the painter reads and
/// NOTHING derives them from `+0x10`/`+0x12` except the walk step.
const PLAYER_DRAW_X_OFF: u32 = 0x16;
const PLAYER_DRAW_Y_OFF: u32 = 0x18;
const PLAYER_BOX_W_OFF: u32 = 0x1c;
const PLAYER_BOX_H_OFF: u32 = 0x20;
const PLAYER_KIND_OFF: u32 = 0x04;

/// EVERY WRITE IT TAKES TO STAND THE PLAYER ON `(nx, ny)` — tile bytes, draw corner and
/// pixel pair.
///
/// ⛔⛔ ALL THREE PAIRS, AND THIS IS THE ONLY PLACE THAT KNOWS THAT. The pixel words
/// are what the game's movement and collision work in, the tile bytes are what the map
/// window and the walk check read, and the draw corner is what the painter blits from;
/// the host must keep all three consistent. Arrival visibility also requires a valid
/// facing, supplied by `guardian_warp_writes` before the scene is loaded.
///
/// ⛔ The pixel pair goes LAST, the same ordering rule as `guardian_fork_writes` and
/// `MenuTapAction::FocusEquip`: a torn read then still sees a self-consistent pair.
/// Measured convention: `x = tile_x * 16 + 8`, `y = tile_y * 16`.
fn player_move_writes(read: &impl Fn(u32, usize) -> Vec<u8>, obj: u32, nx: u8, ny: u8) -> Option<Vec<(u32, Vec<u8>)>> {
    let i16at = |a: u32| -> Option<i16> {
        let b = read(a, 2);
        (b.len() == 2).then(|| i16::from_le_bytes([b[0], b[1]]))
    };
    let (px, py) = (i16::from(nx) * 16 + 8, i16::from(ny) * 16);
    let w = i16at(obj + PLAYER_BOX_W_OFF)?;
    let h = i16at(obj + PLAYER_BOX_H_OFF)?;
    let kind = *read(obj + PLAYER_KIND_OFF, 1).first()? as i8;
    let (dx, dy) = crate::guardian::draw_offsets(px, py, w, h, kind);
    Some(vec![
        (obj + PLAYER_TILE_X_OFF, vec![nx]),
        (obj + PLAYER_TILE_Y_OFF, vec![ny]),
        (obj + PLAYER_DRAW_X_OFF, dx.to_le_bytes().to_vec()),
        (obj + PLAYER_DRAW_Y_OFF, dy.to_le_bytes().to_vec()),
        (obj + PLAYER_PX_X_OFF, px.to_le_bytes().to_vec()),
        (obj + PLAYER_PX_Y_OFF, py.to_le_bytes().to_vec()),
    ])
}

/// Drop the writes whose bytes are already in place. `true` in the second slot means
/// nothing at all is left to do, which is the steady state of a settled arrival.
fn prune_settled(read: &impl Fn(u32, usize) -> Vec<u8>, writes: Vec<(u32, Vec<u8>)>) -> (Vec<(u32, Vec<u8>)>, bool) {
    let left: Vec<_> = writes.into_iter().filter(|(addr, bytes)| read(*addr, bytes.len()) != *bytes).collect();
    let settled = left.is_empty();
    (left, settled)
}

/// Where the player object says the player is, and how big the map they are on is.
///
/// `(obj, x, y, tile_x, tile_y, map_w, map_h)` — the two `i16`s are the PIXEL pair the
/// game's own arrival placement writes and reads (`0x56134` loads `(i16)[obj+0x10]` and
/// `(i16)[obj+0x12]` before handing them to `0x560a4`), and the two bytes after them are
/// the tile the map window draws its marker from. They are not redundant: a write to one
/// pair leaves the other stale, which is exactly how "my character disappeared" reads
/// from the outside.
///
/// ⛔ Diagnostic only, and it must stay LOUD. A player who cannot see their character
/// has no way to tell "off the map", "inside a wall" and "never spawned" apart, and
/// neither does anyone reading their logcat — that is a whole session of screenshots
/// (2026-09-11) for a line that costs one poll every five seconds.
pub fn player_position(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u32, i16, i16, u8, u8, u32, u32)> {
    let c = char_base_raw(read)?;
    let chain = map_chain();
    let u32at = |a: u32| -> Option<u32> {
        let b = read(a, 4);
        (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let deref = |p: u32| -> Option<u32> {
        let h = u32at(p).filter(|v| (MAP_HEAP_LO..MAP_HEAP_HI).contains(v))?;
        match chain.data_off {
            Some(off) => u32at(h).map(|v| v.wrapping_add(off)).filter(|v| (MAP_HEAP_LO..MAP_HEAP_HI).contains(v)),
            None => Some(h),
        }
    };
    let obj = deref(c + PLAYER_OBJ_HANDLE_OFF)?;
    let i16at = |a: u32| -> Option<i16> {
        let b = read(a, 2);
        (b.len() == 2).then(|| i16::from_le_bytes([b[0], b[1]]))
    };
    let u8at = |a: u32| -> Option<u8> { read(a, 1).first().copied() };
    let map = deref(c + MAP_HANDLE_OFF)?;
    Some((
        obj,
        i16at(obj + PLAYER_PX_X_OFF)?,
        i16at(obj + PLAYER_PX_Y_OFF)?,
        u8at(obj + PLAYER_TILE_X_OFF)?,
        u8at(obj + PLAYER_TILE_Y_OFF)?,
        u32at(map + MAP_W_OFF)?,
        u32at(map + MAP_H_OFF)?,
    ))
}

/// Put the player somewhere they can stand, when a tower warp has left them off the
/// map. READ, COMPARE, then write only when it is actually wrong — the steady state of
/// a floor is an empty plan.
///
/// Returns `(writes, from_tile, to_tile)`. See [`crate::guardian::landing_tile`] for
/// why this is needed at all; the short version is that the two-byte warp primitive
/// carries no destination coordinates, so the player keeps the tile they stood on in
/// the map they came from.
///
/// ⛔ BOTH PAIRS. The pixel words (`+0x10`/`+0x12`) are what the game's own movement
/// and drawing work in, and the tile bytes (`+0x27`/`+0x28`) are what the map window
/// and the collision check read; neither is derived from the other on the fly, so
/// writing one and not the other leaves a player who is drawn in one place and
/// collides in another. Measured convention: `x = tile_x * 16 + 8`, `y = tile_y * 16`.
///
/// ⛔ The pixel pair goes LAST, the same ordering rule as `guardian_fork_writes` and
/// `MenuTapAction::FocusEquip`: a torn read then still sees a self-consistent pair.
pub fn guardian_landing_writes(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(Vec<(u32, Vec<u8>)>, (u8, u8), (u8, u8))> {
    let c = char_base_raw(read)?;
    let chain = map_chain();
    let u32at = |a: u32| -> Option<u32> {
        let b = read(a, 4);
        (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let deref = |p: u32| -> Option<u32> {
        let h = u32at(p).filter(|v| (MAP_HEAP_LO..MAP_HEAP_HI).contains(v))?;
        match chain.data_off {
            Some(off) => u32at(h).map(|v| v.wrapping_add(off)).filter(|v| (MAP_HEAP_LO..MAP_HEAP_HI).contains(v)),
            None => Some(h),
        }
    };
    let obj = deref(c + PLAYER_OBJ_HANDLE_OFF)?;
    let map = deref(c + MAP_HANDLE_OFF)?;
    let mw = u32at(map + MAP_W_OFF)? as i32;
    let mh = u32at(map + MAP_H_OFF)? as i32;
    if !(1..=200).contains(&mw) || !(1..=200).contains(&mh) {
        return None;
    }
    let n = (mw * mh) as usize;
    let attr = {
        let p = deref(map + MAP_ATTR_OFF)?;
        let b = read(p, n);
        (b.len() == n).then_some(b)?
    };
    let tx = *read(obj + PLAYER_TILE_X_OFF, 1).first()?;
    let ty = *read(obj + PLAYER_TILE_Y_OFF, 1).first()?;
    let (nx, ny) = crate::guardian::landing_tile(mw, mh, &attr, (i32::from(tx), i32::from(ty)))?;
    Some((player_move_writes(read, obj, nx, ny)?, (tx, ty), (nx, ny)))
}

/// WHERE A TOWER FLOOR STARTS — the arrival placement (2026-09-21).
///
/// Unlike [`guardian_landing_writes`] this does not ask whether the player is stuck: a
/// host-posted warp carries no coordinates, so "where they were on the last floor" is
/// not a position anybody chose. See [`crate::guardian::arrival_tile`] for the report
/// this answers. Read-compare-write, so re-offering it on the next frame is free.
///
/// `Some((writes, from, to))`; an empty `writes` means the player is already standing
/// and being drawn exactly there.
pub fn guardian_arrival_writes(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(Vec<(u32, Vec<u8>)>, (u8, u8), (u8, u8))> {
    let c = char_base_raw(read)?;
    let chain = map_chain();
    let u32at = |a: u32| -> Option<u32> {
        let b = read(a, 4);
        (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let deref = |p: u32| -> Option<u32> {
        let h = u32at(p).filter(|v| (MAP_HEAP_LO..MAP_HEAP_HI).contains(v))?;
        match chain.data_off {
            Some(off) => u32at(h).map(|v| v.wrapping_add(off)).filter(|v| (MAP_HEAP_LO..MAP_HEAP_HI).contains(v)),
            None => Some(h),
        }
    };
    let obj = deref(c + PLAYER_OBJ_HANDLE_OFF)?;
    let map = deref(c + MAP_HANDLE_OFF)?;
    let mw = u32at(map + MAP_W_OFF)? as i32;
    let mh = u32at(map + MAP_H_OFF)? as i32;
    if !(1..=200).contains(&mw) || !(1..=200).contains(&mh) {
        return None;
    }
    let n = (mw * mh) as usize;
    let attr = {
        let p = deref(map + MAP_ATTR_OFF)?;
        let b = read(p, n);
        (b.len() == n).then_some(b)?
    };
    let tx = *read(obj + PLAYER_TILE_X_OFF, 1).first()?;
    let ty = *read(obj + PLAYER_TILE_Y_OFF, 1).first()?;
    let (nx, ny) = crate::guardian::arrival_tile(mw, mh, &attr)?;
    let (writes, _) = prune_settled(read, player_move_writes(read, obj, nx, ny)?);
    Some((writes, (tx, ty), (nx, ny)))
}

// ── the player's death state (2026-09-17, static + S9 vc339) ───────────────────
//
// The field state machine `0x5089c` switches on the PLAYER STATE byte `base+0x9b8e`
// (`base = getBase() = CHAR+0x28`, table `0x5e224`, 42 cases). Dying is
// `0x50e3a`: `0x492cc(0)` · state = 16 · facing (`base+0x9b8c`) = 1 · pose
// (`base+0x9b8d`, `0x4a0f8`) = 5. Case 16 counts the death animation and then sets
// state 7 (`0x50edc`); case 7 (`0x50ee2`) logs "게임오버" and `openWindow(7)` — the
// GAME OVER window, whose KEY (`0x3b3d6`) is `setAppState(3)` = the title.
//
// ⛔ None of that is in the player OBJECT: the scene reload the tower's leave warp
// triggers recreates the object and the state byte survives it (measured: the return
// landed, HP was refilled, and window 7 still opened a second later, then the title).
// So "죽어서 마을 복귀" needs what the game's own revive does. Window 17's 「예」 arm
// (`0x3b510`) is: HP = hp_max · SP = 1000 · `setState(0)` (`0x4a060` → `strb` at
// `base+0x9b8e`) · `setPose(0)` (`0x4a0f8` → `0x56094`: `strb` at `base+0x9b8d` AND
// `strh 0` at `base+0x9ba0`, the pose frame counter) · `0x50774()` (the idle reset —
// buffs, `0x4f014`, not mirrored here) · `*(u16)(base+0xa1d0) = 0`.
// `guardian_revive_writes` mirrors the four plain stores; the caller pairs it with
// `full_restore_writes`, HP FIRST — case 0 (idle) re-enters dying on `hp <= 0`.
/// The player state byte (`base+0x9b8e`), relative to CHAR.
pub const PLAYER_STATE_OFF: u32 = 0x28 + 0x9b8e;
/// The pose byte (`base+0x9b8d`), relative to CHAR.
pub const PLAYER_POSE_OFF: u32 = 0x28 + 0x9b8d;
/// The pose frame counter (`base+0x9ba0`, u16), relative to CHAR.
pub const PLAYER_POSE_TICK_OFF: u32 = 0x28 + 0x9ba0;
/// The u16 the revive zeroes (`base+0xa1d0`), relative to CHAR — the game-over
/// painter's neighbour word (`base+0xa1d2` is the "key accepted" flag it raises).
pub const PLAYER_DEATH_WORD_OFF: u32 = 0x28 + 0xa1d0;
/// State 16 = the death animation, 7 = the game-over window is being opened.
pub const PLAYER_STATE_DYING: u8 = 16;
pub const PLAYER_STATE_GAME_OVER: u8 = 7;
/// State 0 = standing still (1 = walking). Measured (probe, 2026-09-20): the byte is 0
/// before any key, 1 for the whole of a held-direction walk, and **0 again the moment the
/// walk ends** — at a wall, and after anything that interrupts it. The game's own keyup
/// handler (`0x3b614`) tests the same byte against 1 before it stops a walk.
pub const PLAYER_STATE_IDLE: u8 = 0;

/// ★ 142차-C 붉은마녀 「마녀의 생명력」: the player's stat block (`CHAR+0xb4`, what
/// `ComputeMaxHP 0x49000` takes in `r0`) when a live character is loaded — `None` in the
/// title shell or on any read failure (the world's own init recomputes the max there).
pub fn player_stat_block(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u32> {
    if shell_state(read) {
        return None;
    }
    resolve_char(read).map(|c| c + 0xb4)
}

/// Is the player STANDING STILL on the field with nothing asking them anything — i.e.
/// would a direction key start a walk right now?
///
/// ★★★ 2026-09-20 — THE WALK IS AN EDGE, NOT A LEVEL. Player: "조이스틱 이동 중(손 계속
/// 안떼고 있음) 이상태에서 OK 및 스킬 사용 하면 사용후에 안떼고있는 방향으로 이동을 이어서
/// 진행했으면 좋겠어". Measured, and it is the whole of the problem:
///   * ONE keydown walks tile after tile until the matching keyup — 17 tiles on one press.
///   * `handleCletEvent` (`0x10f8`) dispatches **only** `0x1f6` (pressed) and `0x1f7`
///     (released); `0x1f8` (REPEATED) falls off the end and does nothing. So the worker's
///     `Event::Keyrepeat` — which fires every `KEYREPEAT_MS` for every held key — cannot
///     restart anything. Proved: an injected extra keyup mid-walk stopped the character
///     for good, with the direction still "held" and repeats still firing.
///   * Nothing polls a held direction. The only walk-start is that keydown edge.
/// ⇒ whatever ends the walk (an attack, a skill, a wall), the game is waiting for a key
/// the finger is already holding down and will never send again. A stick is a LEVEL
/// control, so the host has to supply the missing edge — see `InputEvent::Walk`.
///
/// `false` on every read failure: a re-press we are not sure about is a key the player
/// did not ask for.
pub fn walk_idle_on_field(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    // ⛔ `game_box_up` as well as `in_field_state`: a dialogue CHOICE and a `showModal`
    // box leave the window id at `WIN_NONE` (117차-C measured `win=3` for the 귀환서
    // 예/아니오), so the field gate alone still reads true while the game is asking a
    // question — and there a direction key MOVES THE CURSOR.
    if !in_field_state(read) || game_box_up(read) {
        return false;
    }
    let Some(c) = char_base_raw(read) else { return false };
    read(c + PLAYER_STATE_OFF, 1).first().copied() == Some(PLAYER_STATE_IDLE)
}

/// One read for the held-attack-key rule (`atkspeed::AttackHold`, 132차-B): is the player
/// idle on the field, and does anything other than the field own OK right now?
///
/// ⛔ `ui` is `!in_field_state || game_box_up`, so a CHAR we cannot resolve reads as UI —
/// the fail-safe direction, because a spoiled hold only ever sends FEWER keydowns.
pub fn attack_hold_view(read: &impl Fn(u32, usize) -> Vec<u8>) -> crate::atkspeed::HoldView {
    let field = in_field_state(read);
    let boxed = game_box_up(read);
    let base = char_base_raw(read);
    let state = base.and_then(|c| read(c + PLAYER_STATE_OFF, 1).first().copied());
    let idle = field && !boxed && state == Some(PLAYER_STATE_IDLE);
    // ★ 132차-C: the combo. While the attack script runs (state 2) an OK APPENDS the next
    //   step to `CHAR+0x23f[8]` — so "room to chain" is: a live step whose next slot is
    //   still the `-1` terminator. Unreadable bytes read as "no room" (fail closed).
    let combo_open = match (field && !boxed && state == Some(PLAYER_STATE_ACTION), base) {
        (true, Some(c)) => {
            let step = read(c + ATTACK_STEP_OFF, 1).first().map(|b| *b as i8);
            let script = read(c + ATTACK_SCRIPT_OFF, ATTACK_SCRIPT_LEN);
            match step {
                Some(s) if s >= 0 && (s as usize) + 1 < ATTACK_SCRIPT_LEN && script.len() == ATTACK_SCRIPT_LEN => {
                    (script[s as usize + 1] as i8 == -1).then_some(s)
                }
                _ => None,
            }
        }
        _ => None,
    };
    crate::atkspeed::HoldView { idle, ui: !field || boxed, combo_open }
}

/// The player state while the action pose script runs — a basic attack or a skill cast
/// (`0x5089c`'s arm 2, `binary_patches::atkspeed::ACTION_STATES`).
pub const PLAYER_STATE_ACTION: u8 = 2;
/// The action's pose script, `i8[8]`, `-1`-terminated: one pose id per combo step. An OK
/// during the swing appends the next step (132차-C, traced: `[2]` -> `[2,3]` -> `[2,3,4]`).
const ATTACK_SCRIPT_OFF: u32 = 0x23f;
const ATTACK_SCRIPT_LEN: usize = 8;
/// The script step running right now (`-1` before the first pose is set).
const ATTACK_STEP_OFF: u32 = 0x24f;

// ── the field key gate (164차, static + probe) ─────────────────────────────────
//
// The field key handler `0x502ec` asks `0x4ace4` first and, on a yes, drops the key
// ("조작불가", `0x5e3f0`, through the no-op debug print `0x1c220`) — the hotkey handler
// `0x4fed0` never runs. `0x4ace4` (`base = getBase() = CHAR+0x28`) is busy when:
// the word `base+0x100` is 1, 2 or 5 · the player state `base+0x9b8e` is 16 / 7 / 8
// (dying, game over, knock-back) · `base+0x28d` (`0x4acc8`) is set · `base+0x2b2` is set
// ("스킬공격 유지중이다~~", `0x5e2f0`) · or, last, `base+0x35f` is set.
// `base+0x35f` is the SKILL CAST flag: the key handler raises it when a skill cast is
// accepted (`0x50596`, `r5 = base+0x8c`, `+0x2d3`) and the idle reset `0x50774` clears it
// with `base+0x2b2` (`0x507d8`·`0x50814`). So from a skill's wind-up to the end of its
// follow-through every field key is dropped, a potion hotkey included.
// Probe (2026-10-05, potion x5 at bag 0,6 on quickbar slot 0, HP 50/167): key '1' =
// 5 -> 4 and HP 167 · same with `base+0x35f = 1` = nothing (5, HP 50) · same flag plus
// `useItem(0, 6)` = 5 -> 4 and HP 167.
const FIELD_GATE_WORD_OFF: u32 = 0x28 + 0x100;
const FIELD_GATE_LOCK_OFF: u32 = 0x28 + 0x28d;
const FIELD_GATE_HELD_SKILL_OFF: u32 = 0x28 + 0x2b2;
const FIELD_GATE_CAST_OFF: u32 = 0x28 + 0x35f;
const PLAYER_STATE_KNOCKBACK: i8 = 8;

/// What `0x4ace4` would answer for a field key right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKeyGate {
    /// The key reaches the hotkey handler.
    Open,
    /// Dropped ONLY because a skill is being cast (`base+0x2b2` / `base+0x35f`).
    Casting,
    /// Dropped for any other reason — the game's own refusal, left alone.
    Busy,
}

/// The game's field-key gate, read in the order `0x4ace4` tests it. `None` on any read
/// failure — a caller must not act on a gate it could not read.
pub fn field_key_gate(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<FieldKeyGate> {
    let c = char_base_raw(read)?;
    let word = rd_u32(read, c + FIELD_GATE_WORD_OFF)?;
    if word <= 5 && (0x26u32 >> word) & 1 != 0 {
        return Some(FieldKeyGate::Busy);
    }
    let state = rd_i8(read, c + PLAYER_STATE_OFF)?;
    if state == PLAYER_STATE_DYING as i8 || state == PLAYER_STATE_GAME_OVER as i8 || state == PLAYER_STATE_KNOCKBACK {
        return Some(FieldKeyGate::Busy);
    }
    if rd_u8(read, c + FIELD_GATE_LOCK_OFF)? != 0 {
        return Some(FieldKeyGate::Busy);
    }
    if rd_u8(read, c + FIELD_GATE_HELD_SKILL_OFF)? != 0 || rd_u8(read, c + FIELD_GATE_CAST_OFF)? != 0 {
        return Some(FieldKeyGate::Casting);
    }
    Some(FieldKeyGate::Open)
}

/// The game's own revive, minus the HP/SP pair (`full_restore_writes`) and the idle
/// reset call: state 0, pose 0, pose counter 0, death word 0. `Some(empty)` when the
/// player is not dying — read-compare-write, so it is free to ask on every poll.
/// `None` when CHAR cannot be resolved.
pub fn guardian_revive_writes(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(Vec<(u32, Vec<u8>)>, u8)> {
    let c = char_base_raw(read)?;
    let state = *read(c + PLAYER_STATE_OFF, 1).first()?;
    if state != PLAYER_STATE_DYING && state != PLAYER_STATE_GAME_OVER {
        return Some((vec![], state));
    }
    Some((
        vec![
            (c + PLAYER_POSE_TICK_OFF, vec![0, 0]),
            (c + PLAYER_DEATH_WORD_OFF, vec![0, 0]),
            (c + PLAYER_POSE_OFF, vec![0]),
            // The state LAST: once it reads 0 the machine runs case 0 on the next tick.
            (c + PLAYER_STATE_OFF, vec![0]),
        ],
        state,
    ))
}

/// Stand the player on `tile` — the tower's return placement (2026-09-17). The leave
/// warp carries no coordinates any more than the entry warp does, so the player came
/// back to town on the tile they had on the last floor: not in front of the passer-by
/// they talked to, and (the floor arenas being 20x20 in a 35x36 town) somewhere that
/// happens to be walkable, so the landing fix — which only moves a TRAPPED player —
/// left them there. This writes the tile the run remembered (`guardian::Run::origin_tile`)
/// with the same four fields, in the same order, as [`guardian_landing_writes`].
///
/// READ, COMPARE, then write: `Some((vec![], from))` means the player is already
/// there. `None` = the chain could not be read, or `tile` is outside THIS map (the
/// caller checks the scene first; this is the second lock, so a placement meant for
/// the town can never be applied to a floor).
pub fn guardian_place_writes(read: &impl Fn(u32, usize) -> Vec<u8>, tile: (u8, u8)) -> Option<(Vec<(u32, Vec<u8>)>, (u8, u8))> {
    let c = char_base_raw(read)?;
    let chain = map_chain();
    let u32at = |a: u32| -> Option<u32> {
        let b = read(a, 4);
        (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let deref = |p: u32| -> Option<u32> {
        let h = u32at(p).filter(|v| (MAP_HEAP_LO..MAP_HEAP_HI).contains(v))?;
        match chain.data_off {
            Some(off) => u32at(h).map(|v| v.wrapping_add(off)).filter(|v| (MAP_HEAP_LO..MAP_HEAP_HI).contains(v)),
            None => Some(h),
        }
    };
    let obj = deref(c + PLAYER_OBJ_HANDLE_OFF)?;
    let map = deref(c + MAP_HANDLE_OFF)?;
    let mw = u32at(map + MAP_W_OFF)?;
    let mh = u32at(map + MAP_H_OFF)?;
    let (nx, ny) = tile;
    if !(1..=200).contains(&mw) || !(1..=200).contains(&mh) || u32::from(nx) >= mw || u32::from(ny) >= mh {
        return None;
    }
    let tx = *read(obj + PLAYER_TILE_X_OFF, 1).first()?;
    let ty = *read(obj + PLAYER_TILE_Y_OFF, 1).first()?;
    // ⛔ The settled test covers the DRAW corner too: a player who is standing in the
    // right place but being painted at the last map's corner is not settled, they are
    // invisible (2026-09-21).
    let (writes, _) = prune_settled(read, player_move_writes(read, obj, nx, ny)?);
    Some((writes, (tx, ty)))
}

/// The build-specific half of the map pointer chain, from the shared anchor table.
/// One line, because `minimap_render.rs` is `include!`d into two crates and so cannot
/// name `crate::guest_build` itself.
fn map_chain() -> MapChain {
    MapChain {
        data_off: crate::guest_build::container_data_off(),
        noblend_global: crate::guest_build::minimap_noblend_global(),
    }
}

// ── 메뉴 터치화 (66차) ────────────────────────────────────────────────────────
//
// The tap router's guest half. Every byte it reads is one THIS MODULE already owns:
//   * `SELECTOR_FLAG_OFF`  — a menu overlay owns the keypad (`popup_view` gate 1)
//   * `MENU_PAGE_OFF`      — which overlay page (`PAGE_ITEM` is the 인벤토리 grid)
//   * `POPUP_OPEN_OFF`     — the game's own action popup is on TOP of that page
//   * `INV_TAB_OFF` / `INV_CURSOR_BASE` — the per-bag cell cursor, i.e. the two bytes
//     `cash_use_pending` reads to know "the item about to be consumed"
// plus exactly ONE address from the per-build table, the MENU list cursor
// (`guest_build::menu_row_cursor`), because that one is new.
//
// ⛔ Do NOT re-declare those five offsets anywhere else, and do not copy them into the
// anchor table "for symmetry". A second copy in the same crate is how the WATCHER
// (`popup_view`, `cash_use_pending`) and the WRITER (here) end up pointing at
// different bytes after someone re-measures one of them — a divergence no selftest
// can see, because both halves keep passing their own.

/// Overlay page indices, as the game numbers them (`screen = 메뉴커서 + 1`, so the MENU
/// list itself is 0). Measured on the device (66차-B) by reading `MENU_PAGE_OFF` on each
/// screen. [`PAGE_ITEM`] (2) and [`PAGE_SKILL`] (4) already existed.
/// The shared list widget's axis byte. Measured both ways on the device (66차-B): the
/// shop's row lists read 1 and DOWN moves the cursor, the purchase CONFIRM (`예/아니오`)
/// reads 0 and RIGHT moves it while DOWN does nothing. ⛔ Any other value is a layout we
/// have not seen, and the router refuses it rather than guessing which key moves what.
const LIST_AXIS_VERTICAL: u8 = 1;
const LIST_AXIS_HORIZONTAL: u8 = 0;

/// The game's own windows that we have SEEN driving the shared list widget. 12 = 상점 and
/// 13 = 제련 were measured on the device (66차-B); the window table itself is in
/// `TOUCH_MENU_FEASIBILITY.md §1`. ⛔ Add a window here only after watching `0xa284` move
/// on it — a stale row count on an unrelated window would make the host write bytes into
/// a screen that is not a list at all.
const LIST_WINDOWS: [u8; 2] = [12, 13];

pub const MENU_SCREEN_LIST: u8 = 0;
pub const MENU_SCREEN_STATUS: u8 = 1;
pub const MENU_SCREEN_EQUIP: u8 = 3;
/// ★ 167차: the SKILL page — its «1» leads back to the AVATAR page (the wardrobe sits
/// between EQUIPMENT and SKILL in the «1»«3» ring).
pub const MENU_SCREEN_SKILL: u8 = 4;
pub const MENU_SCREEN_QUEST: u8 = 5;
pub const MENU_SCREEN_SYSTEM: u8 = 6;

/// 메뉴 터치화 (66차): what the tap router needs to know about the live screen.
///
/// `None` means "do not touch anything", and it covers four different reasons:
/// the build has no measured cursor, the game is not in a live playthrough, no
/// overlay screen owns the keypad, or one of the GAME's own popups is on top of it.
pub struct MenuState {
    /// The field-menu OVERLAY owns the keypad. When false the game's own window does,
    /// and `window` says which — on 상점/제련 the overlay flag reads 0 (measured).
    pub overlay: bool,
    /// An ONLINE screen (창고 · 캐시샵) rather than one of the in-world menus — see
    /// [`online_screen`]. Those run in app state 3 and report `MENU_PAGE_OFF == 0`,
    /// which is [`MENU_SCREEN_LIST`]'s value, so without this flag the six-row MENU
    /// table would be applied to a 16-cell warehouse grid.
    pub online: bool,
    /// Overlay page: [`MENU_SCREEN_LIST`] = the six-row MENU list, [`PAGE_ITEM`] = the
    /// 인벤토리 grid (`screen = 메뉴커서 + 1`). Meaningless unless `overlay`.
    pub screen: u8,
    /// The live bag page. Meaningless unless `screen` is the grid.
    pub page: u8,
    /// The game's own window id (`openWindow`): 12 = 상점, 13 = 제련, …
    pub window: u8,
    /// The SHARED list widget's axis: [`LIST_AXIS_VERTICAL`] for a row list,
    /// [`LIST_AXIS_HORIZONTAL`] for a `예/아니오` confirm.
    pub list_axis: u8,
    /// A dialogue CHOICE is on screen right now ([`CHOICE_FLAG_OFF`]). The script
    /// window is up for the whole conversation, so this — not the window id — is what
    /// says the widget's rows belong to something the player can pick.
    pub choice: bool,
    /// …its live item count, 0 when the widget is not in use, and its cursor. These are
    /// what make every widget screen tappable from one measurement.
    pub list_rows: u8,
    pub list_cursor: u8,
}

/// Read [`MenuState`] out of the live record. Re-read on every use — the game clears
/// these on its own transitions (63차-B §7-2), so a cached copy is a wrong answer
/// waiting to happen.
///
/// ★ The gate is these bytes and NOT [`window_id`]: the main menu and the inventory
/// are not `openWindow` windows (measured `state=5 selflag=1 window=3`), which is
/// exactly why the guest's own key callback tests the overlay flag before it looks at
/// any window id.
pub fn menu_state(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<MenuState> {
    crate::guest_build::menu_cursors()?;
    let c = resolve_char(read)?;
    // ⛔ In a LIVE playthrough only. `Mode::Menu` on the UI side is merely "not field,
    // not cutscene", which is also true of the title and the slot screen, and the
    // overlay flag can still be set in a record that is on its way out of a game. The
    // host must not write cursors into a record nobody is playing.
    // ⛔ A LIVE record only — but "live" is not the same as "in the world". The online
    // screens (창고 · 캐시샵) are app state 3, which `in_game_state` rejects; what must
    // stay rejected is the title / slot / class shell, and that is [`online_screen`]'s
    // liveness test rather than the state word (66차-C).
    let online = online_screen(read);
    if !in_game_state(read) && !online {
        return None;
    }
    // Either the field-menu overlay owns the keypad, or one of the game's own windows
    // does. ★ Both are real: 상점/제련 run with the overlay flag at 0 and a window id of
    // 12/13 (measured 66차-B), and the six overlay screens run with the flag set and the
    // window id still reading 3. So this is not a gate any more, it is a BRANCH.
    let overlay = rd_u8(read, c + SELECTOR_FLAG_OFF)? != 0;
    let window = rd_u8(read, c + WINDOW_ID_OFF)?;
    // ⛔⛔ …and NOTHING of the game's own is on top of it. This is the gate that keeps a
    // second tap from USING or DROPPING an item: with the action popup open the state is
    // still "overlay + item page" (that is precisely `popup_view`'s / `cash_use_pending`'s
    // gate), so without this a tap meant to pick another cell would move the cell cursor
    // and then hand the popup an OK — and the popup's row 0 is `사용하기`. Do not
    // "optimise" this check away: the popup is where the irreversible actions live.
    if rd_u8(read, c + POPUP_OPEN_OFF)? != 0 {
        return None;
    }
    let cur = crate::guest_build::menu_cursors()?;
    // The shared widget is "in use" when it says how many rows it has. ⛔ The axis byte
    // has to be the one every measured screen showed (1 = vertical): a horizontal list
    // would need its keys the other way round, and we have never seen one.
    let axis = rd_u8(read, c + cur.widget_axis)?;
    let known_axis = axis == LIST_AXIS_VERTICAL || axis == LIST_AXIS_HORIZONTAL;
    let list_rows = if known_axis { rd_u8(read, c + cur.widget_rows)? } else { 0 };
    Some(MenuState {
        online,
        overlay,
        choice: rd_u8(read, c + CHOICE_FLAG_OFF)? != 0,
        screen: rd_u8(read, c + MENU_PAGE_OFF)?,
        page: rd_u8(read, c + INV_TAB_OFF)?,
        window,
        list_axis: axis,
        list_rows,
        list_cursor: rd_u8(read, c + cur.widget)?,
    })
}

/// 메뉴 터치화, 4단계 실측 (66차-B): WHICH byte holds the focus on the screen that is up
/// right now. `None` on the field / title, one line otherwise.
///
/// This is the standard way this repo closes a 🔶 offset — the same trick as
/// [`offset_audit`]: print the candidates next to each other while a HUMAN drives the
/// game to a state the probe cannot reach (a shop's buy/sell list, the smith's bag
/// window, the network warehouse), then read which one tracks the highlight. The
/// candidates are:
///   * `list*` — the SHARED list widget (`TOUCH_MENU_FEASIBILITY.md §2.1`): cursor
///     `CHAR+0xa284`, row count `+0xa285`, axis pick `+0xa276`. About ten screens are
///     said to use it, none of them verified on a device yet.
///   * `menu` — the six-row MENU list cursor, already shipped.
///   * `inv*` — the per-bag grid cursor, already shipped.
///   * `pop*` — the action popup (open / rows / selected), already shipped.
/// ⛔ Read-only. Delete this once §2.1's screens are all measured.
pub fn focus_probe(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<String> {
    let c = resolve_char(read)?;
    let overlay = rd_u8(read, c + SELECTOR_FLAG_OFF)?;
    let window = rd_u8(read, c + WINDOW_ID_OFF)?;
    // The field with nothing open is not interesting and would spam.
    if overlay == 0 && window == 3 {
        return None;
    }
    let page = rd_u8(read, c + INV_TAB_OFF);
    // 79차-E: the dialogue-choice flag, so the next session can see it move.
    let choice = rd_u8(read, c + CHOICE_FLAG_OFF).map_or(-1i32, i32::from);
    Some(format!(
        "focus probe: overlay={overlay:?} screen={:?} win={window:?} state={:?} | list cur={:?} rows={:?} axis={:?} paint={:?} | menu={:?} | inv page={:?} cell={:?} | pop open={:?} rows={:?} sel={:?} | choice={choice} | stat={:?} | equip idx={:?} col={:?} row={:?}",
        rd_u8(read, c + MENU_PAGE_OFF),
        rd_u32(read, c + crate::guest_build::state_off()),
        rd_i8(read, c + 0xa284),
        rd_i8(read, c + 0xa285),
        rd_u8(read, c + 0xa276),
        rd_u8(read, c + 0xa249),
        rd_i8(read, c + 0x9c6d),
        page,
        page.and_then(|p| rd_i8(read, c + INV_CURSOR_BASE + u32::from(p))),
        rd_u8(read, c + POPUP_OPEN_OFF),
        rd_i8(read, c + POPUP_COUNT_OFF),
        rd_i8(read, c + POPUP_SEL_OFF),
        // ★ 79차-C: 상태보기's stat cursor, now driven, printed so a device report can
        //   still show it disagreeing with the cyan marker.
        rd_i8(read, c + 0x9c70),
        // ★ 79차-D: the EQUIPMENT cursor, which is three bytes (index / column / row).
        //   Probe-measured on a bare character (`DOWN` moved the index by 3, `RIGHT` by
        //   1, so the grid is three columns) — what is still missing is whether the
        //   index SKIPS unusable slots on a character that actually wears things, and
        //   that is what these three answer while a human walks the grid.
        rd_i8(read, c + 0x9c77),
        rd_i8(read, c + 0x9c78),
        rd_i8(read, c + 0x9c79),
    ))
}

/// What a tap inside the game's own panel should do (66차-B).
///
/// ★★ TWO STEPS, because that is what the game's own keys do and what the player asked
/// for (2026-08-23: *"누르면 아이템 포커스. 그리고 포커스된 아이템을 한번 더 누르면 그때
/// 상세보기같은게 뜨는걸 원하는거야"*). The first version of this feature wrote the cursor
/// AND synthesised OK in one go, which is the game's "OK pressed twice" behaviour: it
/// skipped the focus step entirely, so the player could never look at an item before
/// acting on it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuTapAction {
    /// Move the game's own focus there — one byte, NO key. The game repaints the
    /// highlight by itself on the next frame (probe-proven, 66차).
    Focus { addr: u32, byte: u8 },
    /// The focus is ALREADY on what the finger hit, so this tap means "go": the same
    /// OK the handset's key would send, and every rule that follows is the game's.
    Activate,
    /// 장비 only: the cursor is THREE bytes (index / column / row at `addr` / `+1` /
    /// `+2`), so it gets its own action — one tap, one write, no keys (80차-I).
    ///
    /// ⛔ It replaced 79차-D's `Walk`, which pressed the game's own RIGHT once per worker
    /// poll and is what the player saw: *"장비창에서 신발을 터치하면 포커스가 갑자기
    /// 다른칸들을 쭈르르르륵 순식간에 돈다음에 신발로 포커스이동"*. The walk existed only
    /// because the repo's "the cursor SKIPS empty slots" warning could not be disproved
    /// with a character wearing all nine pieces. It has now been disproved on one with
    /// four empty slots — see `menutouch::EQUIP`.
    ///
    FocusEquip { addr: u32, cell: u8 },
    /// 장비's CANDIDATE pane (the 3x3 「장비장착」 opens on the right). Same three bytes,
    /// but it needs the game's own RIGHT to finish the move — see the worker's arm.
    ///
    /// ⛔ It is NOT [`MenuTapAction::FocusEquip`] with a different address. Device
    /// (vc259): writing column/row on this pane DOES take — the next RIGHT continued
    /// from `(1,1)` to `(2,1)` — but the yellow border **did not repaint**. The left
    /// grid paints its marker from those bytes live; this one caches its drawn position
    /// and only recomputes it when a key arrives. So the move is: park the cursor one
    /// cell BEFORE the target and send a single RIGHT.
    ///
    /// ⚠ KNOWN LIMIT (review, 2026-09-06): the first tap does not know how many
    /// candidates the pane holds — `Grid::holds` only checks the nine cells — and whether
    /// the game's RIGHT wraps at the candidate COUNT or at nine is unmeasured. On a pane
    /// with three candidates a tap on cell 8 parks at 7 and sends RIGHT, which may land
    /// the marker somewhere other than 8. Nothing irreversible follows: the SECOND tap is
    /// `Activate` only when the yellow marker's cell equals the live cursor bytes
    /// (`mark.cell == live`), so a mis-landed first tap costs one more tap, not an action.
    FocusEquipCandidate { addr: u32, cell: u8 },
    /// 장비: the candidate pane is up and the finger went back to the WORN slots —
    /// send the game's own `CLR`, which is how a player returns from it (80차-J).
    ///
    /// ⛔ A key, not a write. The pane is selected by a module global the host has no
    /// anchor for, and `CLR` is the game's own answer: device (vc260, on the candidate
    /// pane) — the marker went back to the left grid, onto the slot that had been
    /// opened, with `idx/col/row` left consistent (`4 / 1 / 1`). Nothing to guess.
    /// ⛔ Only ever while the marker IS on the candidate grid: on the worn grid the same
    /// key CLOSES the equipment screen.
    LeaveEquipCandidates,
}

/// One tap on an ITEM GRID (the bag on any screen, 일반조합, …), given the grid the UI
/// derived from the game's own pixels.
///
/// ⛔⛔ The cross-check is the safety rail: the UI computed which cell the yellow cursor
/// sits on FROM PIXELS, and this refuses unless that matches the cursor byte the game
/// actually holds. Without it a wrong grid writes an out-of-range index, and the game
/// then draws another item's details with no selection box at all — which is exactly the
/// 일반조합 bug (player, 2026-08-23). Verified across six device frames: the derived index
/// equalled the live byte every time (0 · 0 · 1 · 1 · 3 · 11).
fn item_grid_action(
    read: &impl Fn(u32, usize) -> Vec<u8>,
    addr: u32,
    page: u8,
    dx: i32,
    dy: i32,
    item: Option<(crate::menutouch::Grid, u8)>,
) -> Option<MenuTapAction> {
    let (grid, drawn) = item?;
    let live = rd_u8(read, addr)?;
    if drawn != live {
        return None;
    }
    let cell = grid.hit(dx, dy)?;
    grid.holds(cell).then_some(())?;
    // The page still bounds the WRITE: the cursor array is indexed by it.
    (page < crate::menutouch::PAGES).then_some(())?;
    Some(if live == cell {
        MenuTapAction::Activate
    } else {
        MenuTapAction::Focus { addr, byte: cell }
    })
}

/// 메뉴 터치화 (66차): what a tap at panel-relative `(dx, dy)` means right now.
///
/// `None` on every screen whose geometry is not measured, on a build with no measured
/// cursor, on a point that is neither a row nor a cell, and in every state
/// [`menu_state`] refuses. That is the whole failure budget: nothing happens, and the
/// player taps again or uses the keys.
///
/// ⛔ The clamp is not the caller's job. A row is clamped twice
/// ([`crate::menutouch::row_at`] can only answer 0..=5, and
/// [`crate::guest_build::GuestBuild::MENU_ROW_MAX`] again here) because the game clamps
/// this byte only inside its KEY arms — a 6 opens a screen the menu itself cannot reach
/// (measured, `TOUCH_MENU_FEASIBILITY.md §7-1`). A CELL is not clamped but REFUSED
/// (see `item_grid_action`): the bag page is an INDEX into the cursor array, so a page
/// byte outside the game's own bag count ([`crate::menutouch::PAGES`]) would move the
/// write to another byte of the record entirely.
pub fn menu_tap_action(
    read: &impl Fn(u32, usize) -> Vec<u8>,
    dx: i32,
    dy: i32,
    item: Option<(crate::menutouch::Grid, u8)>,
    // `status` = which cell 상태보기's cyan marker is on, from the UI's frame scan
    // (`menutouch::status_marker`); `None` = no marker on screen. A CROSS-CHECK, not a
    // hint: that screen's second tap spends a stat point and cannot be undone.
    status: Option<u8>,
    // `equip` = the same thing for 장비's yellow slot border (`menutouch::equip_marker`),
    // and since 80차-I it also says WHICH of that screen's two 3x3 grids the border is on.
    equip: Option<crate::menutouch::EquipMark>,
    // `cell_cursor` = a 17x17 CELL cursor is on screen (`menutouch::cell_box`), i.e. the
    // game IS drawing an item grid even when `item` could not work out its shape.
    cell_cursor: bool,
) -> Option<MenuTapAction> {
    let cur = crate::guest_build::menu_cursors()?;
    let st = menu_state(read)?;
    let c = resolve_char(read)?;
    // ★★★ 80차-G — THE GAME'S OWN `예 / 아니오` CONFIRM, AND IT COMES FIRST BECAUSE IT CAN
    // BE RAISED OVER ANY SCREEN.
    //
    // Player, 2026-09-05: *"인게임 -> 시스템 -> 세이브 … 예 혹은 아니오 선택해도 팝업창에서
    // 동작을 안하고 갑자기 뒤에있는 화면에 터치가돼 … 이건 보스방 입장시도 마찬가지야"*.
    // The box is the SHARED LIST WIDGET in its horizontal mode — device `focus probe` with
    // it up: `overlay=1 screen=Some(9) win=3 | list cur=Some(0) rows=Some(2) axis=Some(0)
    // | choice=1` — so it is NOT a window, NOT a modal and NOT the field-menu overlay's
    // own screen. Until now the only horizontal branch lived under `!st.overlay` behind
    // `LIST_WINDOWS`, which meant every confirm raised over an overlay screen (시스템 →
    // 세이브) or over the FIELD (boss room) fell through to whatever was drawn behind it
    // and moved ITS cursor. That is the report, exactly.
    //
    // The gate is the game's own pair: [`CHOICE_FLAG_OFF`] (`CHAR+0xa224`, which the game
    // SETS when a selection is pending and CLEARS when it applies one — 79차-E) plus the
    // horizontal axis. ⛔ The axis alone is not enough: the widget keeps the last screen's
    // bytes (see `menu_state`), and `choice` is what makes "a selection is pending" a fact
    // rather than a leftover.
    //
    // ⛔ A MISS RETURNS `None`: the box owns the panel. Letting a miss through is the
    // half of the report about the background reacting.
    // ★ In practice the UI's own pixel arm (`App::confirm_tap_hit` ->
    // `menutouch::confirm_pair_widget`) answers first and this never runs; it is here so
    // that a frame the scanner cannot read still cannot move anything behind the box.
    if st.choice && st.list_axis == LIST_AXIS_HORIZONTAL {
        // ★ 117차-D: under a TWO-LINE question the pair sits 10 px lower (귀환서's 「최근
        //   귀환장소로 이동하겠습니까?」, measured dy 81..94) — both rows are tried.
        let row = [crate::menutouch::CONFIRM_Y, crate::menutouch::CONFIRM_Y_2LINE]
            .into_iter()
            .find_map(|y0| {
                let g = crate::menutouch::confirm_grid_at(st.list_rows, y0)?;
                let r = g.hit(dx, dy)?;
                g.holds(r).then_some(r)
            })?;
        return Some(if st.list_cursor == row {
            MenuTapAction::Activate
        } else {
            MenuTapAction::Focus {
                addr: c + cur.widget,
                byte: row,
            }
        });
    }
    // ★★★ …and the same net for the game's own MODALS (80차-G). `showModal` (`0x4684`)
    // writes the box's id into the one state word 48차 established, and `closeModal`
    // zeroes it — that word is already this repo's "is the cash confirm up" test
    // (`cash_buy_confirm_open`), so it is known to be cleared rather than sticky.
    //
    // A modal is by definition on top of whatever raised it, so nothing below may move
    // while one is up. The UI's pixel arms answer the ones we can draw targets for
    // (`confirm_pair`, `ok_button`) BEFORE the point ever reaches this function; what is
    // left here is the boxes we do not recognise, and for those the right answer is
    // nothing at all rather than a cursor write behind the box.
    //
    // ⛔ FAIL CLOSED, BUT SAY SO: the caller logs `… is not a row or a cell on this
    // screen` for a `None`, and the worker already prints `modal N opened` on every
    // change. If a build ever leaves this word stale, those two lines together say so on
    // the first tap instead of presenting as "the menu stopped responding".
    if modal_id(read).is_some_and(|id| id != 0) {
        return None;
    }
    // ── the game's own WINDOWS: one rule for all of them ────────────────────────────
    //
    // Every window that shows a list uses the SHARED widget, and the widget tells us how
    // many rows it has — so the geometry is computed, not tabulated
    // (`menutouch::choice_grid`). That is why 상점/제련 needed no new measurement each.
    //
    // ⛔ Only windows we have actually SEEN doing this. A window that shows something
    // else entirely (a text box, the map) must not have its bytes written because the
    // widget happens to hold a stale count.
    // ★★ ONLINE screens first (창고 · 캐시샵). They set the overlay flag and report
    // `MENU_PAGE_OFF == 0` — indistinguishable, in memory, from the six-row MENU list —
    // so a `st.screen` match would drive them with `menutouch::MENU` and the menu cursor.
    // What they actually are is an item grid on the SAME per-bag cursor array as the
    // inventory: device-measured 66차-C on 싱글 창고, RIGHT moved `0x9c72` 0 -> 1 while
    // the drawn yellow box moved with it, and the pixel-derived index matched both times.
    //
    // ⛔ Nothing else on those screens is ours. Their tabs («1» SINGLE «3») and pages
    // («7» 가방 1 «9») are BADGES the game printed, which the UI already claims without
    // any guest anchor, and everything past that is the online module's own key handler.
    if st.online {
        return item_grid_action(read, c + INV_CURSOR_BASE + u32::from(st.page), st.page, dx, dy, item);
    }
    if !st.overlay {
        // ★★★ 대화 선택지 (79차-E, TODO `BO` 를 닫는다). The script window is not in
        // `LIST_WINDOWS` because it is not a list window MOST of the time — it is the
        // dialogue box, and the shared widget under it holds the previous choice's row
        // count for the rest of the conversation (device: `win=4 … rows=4 axis=1` long
        // after the answer). The game's own flag says which it is right now, and it is
        // the same byte the game gates its painter and its OK on, so "the flag is set"
        // and "an OK would pick a row" are the same statement.
        //
        // Everything after this is the shared-widget path the shop and the smith
        // already use — same painter (`0x575bc`, which takes NO arguments, so one
        // geometry for every caller), same cursor byte, same two-tap rule.
        // ⛔ Handled HERE and returned, never falling through to the item-grid tail:
        // a stray yellow box during a conversation must not turn a choice tap into a
        // write to the inventory cursor array.
        if st.window == SCRIPT_WINDOW {
            if !st.choice || st.list_axis != LIST_AXIS_VERTICAL {
                return None;
            }
            let grid = crate::menutouch::choice_grid(st.list_rows)?;
            let row = grid.hit(dx, dy)?;
            grid.holds(row).then_some(())?;
            // The 66차-B two-tap rule, and here it is also the safety net: this path has
            // no pixel cross-check, so the FIRST tap only moves the game's own highlight
            // and the player sees where it landed before the second tap confirms.
            return Some(if st.list_cursor == row {
                MenuTapAction::Activate
            } else {
                MenuTapAction::Focus {
                    addr: c + cur.widget,
                    byte: row,
                }
            });
        }
        if !LIST_WINDOWS.contains(&st.window) {
            return None;
        }
        // ★★ The same window shows BOTH a row list and a bag grid (상점: 구매/판매 ->
        // category -> the bag you sell from; 제련: its list -> the material bag). MEMORY
        // CANNOT TELL THEM APART: the widget's cursor and count keep the last list's
        // values, and `list_painting` (`0xa249`) reads 0 in the shop's sell window but 12
        // in the smith's material window — which is exactly how every tap in that window
        // became "row 0 = the cursor" and fired OK on whatever was focused (player,
        // 2026-08-23: "다른 아이템을 눌러도 포커스된 아이템에서 OK 누른것처럼").
        //
        // So the GAME's own cell cursor decides: if it is drawing the yellow 17x17
        // border, this is a bag grid. ⛔ Do not go back to a memory-only test.
        if item.is_none() {
            // ⛔⛔ …unless the game is drawing a CELL cursor, in which case this window is
            // showing a BAG and `item_grid` merely failed to measure it. 79차-E gave the
            // inventory a fallback for the full-bag case (no empty frames left to derive
            // from) but it only knows the INVENTORY's origin — the shop's sell window and
            // the smith's material window sit 4 px higher (66차-B measured cell 0's cursor
            // box at dy 18 there against the inventory's dy 22). So on a FULL bag those
            // two fell through to here, where the shared widget's STALE row count made a
            // tap on bag row 1 (dy 37..54) resolve to choice row 0 (dy 37..57) and, when
            // it matched the stale cursor, synthesise OK — the exact 66차-B failure the
            // yellow-border test was introduced to kill. Refusing is the whole budget:
            // nothing happens and the player uses the keys.
            if cell_cursor {
                return None;
            }
            // Vertical -> a row list; horizontal -> a `예/아니오` confirm. The count byte
            // means rows in one case and columns in the other, which is why the axis
            // picks the builder rather than being folded into it.
            let grid = match st.list_axis {
                LIST_AXIS_VERTICAL => crate::menutouch::choice_grid(st.list_rows)?,
                LIST_AXIS_HORIZONTAL => crate::menutouch::confirm_grid(st.list_rows)?,
                _ => return None,
            };
            let row = grid.hit(dx, dy)?;
            grid.holds(row).then_some(())?;
            return Some(if st.list_cursor == row {
                MenuTapAction::Activate
            } else {
                MenuTapAction::Focus {
                    addr: c + cur.widget,
                    byte: row,
                }
            });
        }
        // An ITEM GRID, driven by the very same per-bag cursor array as the inventory
        // screen (measured: RIGHT moved `0x9c72` in 상점's 판매 AND in 일반조합). Its shape
        // and origin come from the game's own pixels, because they are not constant: the
        // bag is 8x2 at pitch 19, 일반조합 is 5x1 at pitch 29.
        return item_grid_action(read, c + INV_CURSOR_BASE + u32::from(st.page), st.page, dx, dy, item);
    }
    // ── the field-menu OVERLAY screens: one table each ──────────────────────────────
    //
    // Which grid is on screen, and which byte holds its focus. Every pair here is
    // device-measured (66차-B); a screen that is not in this match is one whose geometry
    // or cursor we do not know, and it gets nothing rather than a guess.
    let (grid, addr) = match st.screen {
        MENU_SCREEN_LIST => (&crate::menutouch::MENU, c + cur.list),
        PAGE_SKILL => (&crate::menutouch::SKILL, c + cur.skill),
        MENU_SCREEN_QUEST => (&crate::menutouch::QUEST, c + cur.quest),
        MENU_SCREEN_SYSTEM => (&crate::menutouch::SYSTEM, c + cur.system),
        // The inventory's cursor is an ARRAY indexed by the live bag page, and that byte
        // is `items.rs`'s own (`cash_use_pending` reads the same one to know which item
        // is about to be consumed) — so it is not in the anchor table.
        PAGE_ITEM => {
            return item_grid_action(read, c + INV_CURSOR_BASE + u32::from(st.page), st.page, dx, dy, item);
        }
        // ★★ 상태보기's stat grid (79차-C). 66차-B could not measure it because the list
        // only exists while there are points to spend; the probe forged some
        // (`--poke CHAR+0x18a = 10`) and then the whole screen fell out at once — the
        // cursor from a full-record diff (`CHAR+0x9c70`, one byte, 0 -> 2 on one DOWN),
        // the 2x2 shape from that "2", and the geometry from poking all four values.
        //
        // ⛔ This is the ONE screen here whose second tap is IRREVERSIBLE — it spends a
        // stat point — so it does not run on memory alone. `status` is the cell the
        // GAME's own cyan marker is drawn on, scanned from the frame by the UI, and it
        // has to agree with the live cursor before anything is written. When there is
        // nothing to spend the game draws no marker, `status` is `None`, and the tap
        // does nothing at all.
        MENU_SCREEN_STATUS => {
            let cell = crate::menutouch::STATUS.hit(dx, dy)?;
            crate::menutouch::STATUS.holds(cell).then_some(())?;
            let addr = c + cur.status;
            let live = rd_u8(read, addr)?;
            // The pixels and the byte must be the same cell, or we are not looking at
            // the screen we think we are.
            (status? == live).then_some(())?;
            return Some(if live == cell {
                MenuTapAction::Activate
            } else {
                MenuTapAction::Focus { addr, byte: cell }
            });
        }
        // ★★ 장비 (79차-D, 80차-I). The cursor really is three bytes — index / column /
        // row at `CHAR+0x9c77`/`+8`/`+9` — and BOTH of the screen's grids share them:
        // the left 3x3 of worn slots, and the right 3x3 of candidates that 「장비장착」
        // opens. Which one is live is read from the GAME's own yellow border, and that
        // same reading is the cross-check (the 상태보기 contract).
        //
        // ★ 80차-I: one WRITE, not 79차-D's walk. See `MenuTapAction::FocusEquip`.
        // ⛔ On the candidate pane the index byte is NOT ours: the game leaves it at the
        // slot number being filled and moves only column/row (measured — RIGHT x3 and
        // DOWN x1 moved `col`/`row` while `idx` stayed 0). So the cross-check there is
        // against `row * 3 + col`, and `index: false` keeps the write off that byte.
        MENU_SCREEN_EQUIP => {
            let mark = equip?;
            // ★★ 80차-J — the candidate pane is up and the finger went back to the WORN
            // slots. Player: *"왼쪽 장비칸에서 장비장착 눌러서 오른쪽으로 포커스가
            // 이동되었을떄 왼쪽 장비칸쪽 레이블을 클릭하면 다시 포커스가 왼쪽 장비칸쪽으로
            // 이동되도록"*. Before this the tap fell out of the arm entirely and the
            // worker printed `is not a row or a cell on this screen` — which is what the
            // player's own device log showed.
            //
            // ⛔ It returns focus and NOTHING ELSE: `CLR` puts the marker back on the
            // slot that was opened, not on the cell that was tapped, and moving it from
            // there would mean racing a key the guest has not consumed yet (79차-D's
            // lesson). The next tap moves it, which is the two-tap rule everywhere else.
            if mark.candidates && crate::menutouch::EQUIP.hit(dx, dy).is_some() {
                return Some(MenuTapAction::LeaveEquipCandidates);
            }
            let grid = if mark.candidates {
                &crate::menutouch::EQUIP_CAND
            } else {
                &crate::menutouch::EQUIP
            };
            let cell = grid.hit(dx, dy)?;
            grid.holds(cell).then_some(())?;
            let live = if mark.candidates {
                let col = rd_u8(read, c + cur.equip + 1)?;
                let row = rd_u8(read, c + cur.equip + 2)?;
                row.checked_mul(grid.cols as u8)?.checked_add(col)?
            } else {
                rd_u8(read, c + cur.equip)?
            };
            (mark.cell == live).then_some(())?;
            let addr = c + cur.equip;
            return Some(match (live == cell, mark.candidates) {
                (true, _) => MenuTapAction::Activate,
                (false, true) => MenuTapAction::FocusEquipCandidate { addr, cell },
                (false, false) => MenuTapAction::FocusEquip { addr, cell },
            });
        }
        _ => return None,
    };
    let byte = grid.hit(dx, dy)?;
    // Belt and braces: `hit` cannot answer outside the grid, and this says so again at
    // the one place that turns an index into a WRITE.
    grid.holds(byte).then_some(())?;
    // The second tap is only a second tap if the GAME agrees the focus is there — read
    // it back, never remember it. (The game moves this byte itself: arrows, page flips
    // and its own transitions all write it, `TOUCH_MENU_FEASIBILITY.md §7-2`.)
    Some(if rd_u8(read, addr)? == byte {
        MenuTapAction::Activate
    } else {
        MenuTapAction::Focus { addr, byte }
    })
}

/// The game's minimap, rendered from the live map arrays — pixel-identical to what
/// the guest's own map window would paint, and available without opening it.
/// `None` when the map object is not resolvable (title screen, mid-load).
/// Includes the player marker — the red 2x2 dot the game's own map window draws
/// over the same picture (player request 2026-08-03). Without it the panel is a
/// room outline with no "you are here", which is most of what a minimap is for.
pub fn render_minimap(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<crate::Minimap> {
    let char_ptr = resolve_char(read)?;
    // ★ 141차: the hero5-style view wants TILE CLASSES, not the game's picture — the
    // picture renderer (`render_minimap_rgb`) stays for `wie_probe --map-check`, which
    // proves the pointer chain against the game's own paint.
    let (w, h, cells) = minimap_tiles(read, char_ptr, map_chain())?;
    Some(crate::Minimap { w, h, cells })
}

/// The minimap's moving marks (141차): the player in tiles (fractional, tile centre)
/// and every live monster the same way. `None` when the map is not resolvable.
///
/// A monster's position is its actor's map object (`*(rec+actor)` → `+0x1c` → the
/// object whose `+0x10`/`+0x12` the corpse-hide writes park off the map), so a parked
/// corpse and a dead record are both left out.
pub fn minimap_marks(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(Option<(f32, f32)>, Vec<(f32, f32)>)> {
    let char_ptr = resolve_char(read)?;
    let player = player_marker(read, char_ptr, map_chain());
    let mut mobs = Vec::new();
    if let Some(sw) = crate::guest_build::scene_world()
        && let Some(c) = char_base_raw(read)
        && let Some(arr) = rd_u32(read, c + sw.creatures_off).filter(|a| *a != 0)
        && let Some(n) = spawned_count(read, c, &sw)
    {
        for i in 0..n.clamp(0, 256) {
            let rec = arr.wrapping_add(i as u32 * sw.creature_stride);
            let alive = rd_u16(read, rec + sw.rec.hp_cur).is_some_and(|hp| (hp as i16) > 0);
            if !alive {
                continue;
            }
            let Some(obj) = rd_u32(read, rec + sw.rec.actor).filter(|a| *a != 0).and_then(|a| rd_u32(read, a + 0x1c)).filter(|o| *o != 0) else {
                continue;
            };
            let (Some(px), Some(py)) = (rd_u16(read, obj + 0x10).map(|v| v as i16), rd_u16(read, obj + 0x12).map(|v| v as i16)) else {
                continue;
            };
            if px < 0 || py < 0 {
                continue; // parked corpse (`guardian::CORPSE_HIDE_POS`) or garbage
            }
            mobs.push((f32::from(px) / 16.0, f32::from(py) / 16.0 + 0.5));
        }
    }
    Some((player, mobs))
}

// ── the portrait capture window (141차 · 153차) ─────────────────────────────────
//
// The rules are `portrait.rs` (dependency-free, shared with `wie_probe --portrait`);
// this is the guest half: where the player's draw corner is on the framebuffer, whether
// the frame just drawn is one the ring may be taken from, and what the worn look is.
include!("portrait.rs");

// ★ The blitter side (`wie_util::stats::portrait_takes`) carries its OWN copy of the
// shadow rule — `wie_util` cannot see this file. These pin the two copies together and
// check the rule against a measured frame at compile time.
const _: () = assert!(
    SHADOW_W == wie_util::stats::SHADOW_W
        && SHADOW_H == wie_util::stats::SHADOW_H
        && SHADOW_SLOP == wie_util::stats::SHADOW_SLOP
        && CAPTURE_RUN == wie_util::stats::CAPTURE_RUN
);
const _: () = assert!(is_shadow(171, 86, SHADOW_W, SHADOW_H, (171, 86)) && !is_shadow(171 + 16, 86, SHADOW_W, SHADOW_H, (171, 86)));
// ★ 153차: the face square must lie in the window `wie_util` fills, and the helmet
// (corner + (0, −39), 18 × 26) wholly inside it — or the blitter never copies it.
const _: () = {
    let (fx, fy) = face_origin();
    assert!(fx >= 0 && fy >= 0);
    assert!(fx as u32 + FACE_SIZE <= wie_util::stats::PORTRAIT_W && fy as u32 + FACE_SIZE <= wie_util::stats::PORTRAIT_H);
    assert!(PORTRAIT_UP >= 39 && (PORTRAIT_UP - 39 + 26) as u32 <= wie_util::stats::PORTRAIT_H);
};
const _: () = assert!(IDLE_STATE == PLAYER_STATE_IDLE && FACING_VIEWER == PLAYER_FACE_DOWN);

/// The two cameras the field painter scrolls by (`0x16a90`): the TARGET
/// (`.bss 0x150d914`/`0x150d918` = `base+0x9e64`) and the SMOOTHED one that eases
/// after it (`0x150d91c`/`0x150d920` = `base+0x9e6c`) — the 「화면 이동」 option
/// (`.bss 0x150e985`, `ldrsb … == 1`) picks the smoothed one, anything else the target.
/// Clamped (and a small map centred) by `field_scroll` it is the exact world →
/// framebuffer offset. LGT only — gated below on the direct-pointer map chain.
/// ⛔ 141차 read the smoothed pair unconditionally: right with the default option, a
/// window beside the player with the other.
const CAMERA_TARGET_X_ADDR: u32 = 0x150d914;
const CAMERA_TARGET_Y_ADDR: u32 = 0x150d918;
const CAMERA2_X_ADDR: u32 = 0x150d91c;
const CAMERA2_Y_ADDR: u32 = 0x150d920;
/// The option byte that picks the camera: the options block (`.bss 0x150e980`, saved
/// as `OptionSave_11`) + 5.
const CAMERA_SMOOTH_OPTION_ADDR: u32 = 0x150e985;
/// The player's FACING (`base+0x9b8c` = `CHAR+0x9bb4`, 132차): 1 up · 2 down · 3 left ·
/// 4 right. The portrait wants 2 — the face towards the viewer.
const PLAYER_FACE_OFF: u32 = 0x28 + 0x9b8c;
const PLAYER_FACE_DOWN: u8 = 2;
/// The class byte (`CHAR+0xb6`) — which hero's base body the paper doll falls back to.
const CHAR_CLASS_OFF: u32 = 0xb6;

/// The camera the field painter follows this frame — see [`CAMERA_SMOOTH_OPTION_ADDR`].
fn portrait_camera(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(i32, i32)> {
    let smooth = rd_i8(read, CAMERA_SMOOTH_OPTION_ADDR)? == 1;
    let (ax, ay) = if smooth { (CAMERA2_X_ADDR, CAMERA2_Y_ADDR) } else { (CAMERA_TARGET_X_ADDR, CAMERA_TARGET_Y_ADDR) };
    Some((rd_u32(read, ax)? as i32, rd_u32(read, ay)? as i32))
}

/// Where to arm the portrait window this frame — `(window origin, shadow anchor)` in
/// framebuffer px for a `screen_w × screen_h` guest framebuffer — or `None` when the
/// player is not on the field: then the window is disarmed and the blitter's extra
/// test is one atomic load per blit.
pub fn portrait_arm_window(read: &impl Fn(u32, usize) -> Vec<u8>, screen_w: u32, screen_h: u32) -> Option<((i32, i32), (i32, i32))> {
    if map_chain().data_off.is_some() || !in_field_state(read) {
        return None;
    }
    let (obj, _, _, _, _, map_w, map_h) = player_position(read)?;
    let dx = rd_u16(read, obj + PLAYER_DRAW_X_OFF)? as i16;
    let dy = rd_u16(read, obj + PLAYER_DRAW_Y_OFF)? as i16;
    let cam = portrait_camera(read)?;
    let (sx, sy) = field_scroll(cam, (map_w as i32 * 16, map_h as i32 * 16), (screen_w as i32, screen_h as i32));
    let (draw_x, draw_y) = (i32::from(dx) + sx, i32::from(dy) + sy);
    Some((portrait_origin(draw_x, draw_y), shadow_anchor(draw_x, draw_y)))
}

/// Is the frame just drawn one the ring may be taken from — the player on the field,
/// standing still, facing the viewer, on the idle loop's FIRST frame
/// ([`canonical_frame`])? The idle pose breathes over four frames; any one of them
/// taken at random was the 141차 ring that never stood still.
pub fn portrait_canonical(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    let Some(c) = char_base_raw(read) else { return false };
    let (Some(state), Some(face), Some(tick)) =
        (rd_u8(read, c + PLAYER_STATE_OFF), rd_u8(read, c + PLAYER_FACE_OFF), rd_u16(read, c + PLAYER_POSE_TICK_OFF))
    else {
        return false;
    };
    canonical_frame(state, face, tick) && in_field_state(read)
}

/// The worn look ([`look_key`]): the class byte and the nine worn slots' `(cat, idx)` — and,
/// ★ 167차, the wardrobe's six worn costume bytes ([`costume_key`]).
/// `None` when there is no character or a read fails — then nothing is decided.
pub fn portrait_look(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u64> {
    let c = resolve_char(read)?;
    let class = rd_u16(read, c + CHAR_CLASS_OFF)?;
    let mut worn = [(0i16, 0i16); crate::atkspeed::WORN_SLOTS];
    for (i, slot) in worn.iter_mut().enumerate() {
        let rec = c + WORN_BASE_OFF + i as u32 * WORN_STRIDE;
        *slot = (rd_u16(read, rec + WORN_CAT_OFF)? as i16, rd_u16(read, rec + WORN_IDX_OFF)? as i16);
    }
    let costume = avatar_bytes(read)?;
    Some(costume_key(look_key(class, &worn), &costume[..crate::avatar_rules::SLOTS]))
}

/// Where the player stands (world px, `obj+0x10`/`+0x12`) — the "spot" a picture was
/// confirmed at.
pub fn portrait_spot(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(i32, i32)> {
    let (_, x, y, ..) = player_position(read)?;
    Some((i32::from(x), i32::from(y)))
}

/// The face square out of the capture window as the last tick left it, or `None` when
/// that picture must not be used: a run still open (the tick ended between the player's
/// shadow and the last piece), a blended blit copied into it (the player see-through, or
/// someone else's effect), the face off the `screen_w × screen_h` framebuffer (a head the
/// game clipped), or nothing drawn in the square.
pub fn portrait_take(screen_w: u32, screen_h: u32) -> Option<crate::Portrait> {
    let (_, run, blended) = wie_util::stats::portrait_run_state();
    if run != 0 || blended {
        return None;
    }
    if !face_on_screen(wie_util::stats::portrait_origin()?, screen_w as i32, screen_h as i32) {
        return None;
    }
    let (w, h) = (wie_util::stats::PORTRAIT_W, wie_util::stats::PORTRAIT_H);
    let px = wie_util::stats::portrait_snapshot();
    let (cw, ch, crop) = face_crop(&px, w, h)?;
    Some(crate::Portrait { w: cw as u16, h: ch as u16, px: crop })
}

// ── cash-shop purchase quantity (60차) ───────────────────────────────────────
//
// Player: "캐시템선택하고 구매하기 눌렀을때 수량을 선택할 수있게하는걸 말하는거였어".
//
// The client has no quantity control of its own on the CASH tab — but it does not
// need one, because the number it hands over is a FIELD OF THE ROW, and the row it
// reads at grant time is its own CACHED copy of what our `05 3f` sent:
//
//     guest 0x13b74   memcpy(CHAR + 0xa6bc + i*20, &body[11 + i*21], 20)
//     guest 0x13b88   *(u16*)(CHAR + 0xa6a8) = N          ; rows in the cache
//
// So the host can offer a picker and write the chosen count into `+10` of that
// cached row while the purchase confirm is on screen; the client then grants it.
// Device-proven that the count is honoured: the shop's own 제련석 row ships
// `rec[0x0a] = 99` and the client both draws "99" and hands over the stack.
//
// ⚠ LGT offsets. `cash_row_base` returns `None` on any other build rather than
// poking an address that means something else there.

// ★ Both are derived from the MODAL STATE WORD, not from CHAR. They live in the same
// singleton the online module hangs everything off, and the only address of it this
// codebase has ever verified on a device is `NetDisconnectModal::state_word`
// (48차) — documented there as `base + 0xa438`. The record cache is `base + 0xa6bc`
// (guest 0x13afe: `records = [sp] + 0xa6bc`) and the row count `base + 0xa6a8`, so
// both are a fixed distance from the word we already trust. Deriving them from CHAR
// instead was the first attempt and it silently found nothing.
/// `state_word + this` = the 16-row cache the `05 3f` handler fills (16 x 20 B).
const CASH_ROWS_FROM_MODAL: u32 = 0xa6bc - 0xa438;
/// `state_word + this` = `u16` rows currently in that cache.
const CASH_ROWS_N_FROM_MODAL: u32 = 0xa6a8 - 0xa438;
/// Row field: the stack size handed over on purchase.
const CASH_ROW_QTY: u32 = 10;
/// The client's array is a fixed 16 (`enc_cash_list` caps N to the same number).
const CASH_ROWS_MAX: u32 = 16;
/// Modal id of the cash purchase confirm (`showModal(text, 0xd, 1, 1)`, guest
/// 0x741c / 0x7390 — the two sites that build "…구매하시겠습니까?").
const MODAL_CASH_BUY: u32 = 0xd;

/// Is the cash-shop purchase confirm on screen?
///
/// Reads the same modal state word 48차 established for the disconnect box
/// (`guest_anchors::NetDisconnectModal::state_word`, `0` = no modal), so this needs
/// no new anchor and follows the per-build table.
pub fn cash_buy_confirm_open(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    let Some(m) = crate::guest_build::net_disconnect_modal() else {
        return false;
    };
    rd_u32(read, m.state_word) == Some(MODAL_CASH_BUY)
}

/// `state_word + this` = the confirm box's 예/아니오 byte. Negative: it sits just BELOW
/// the state word (`base + 0xa434` against the word's `base + 0xa438`).
const CASH_CHOICE_FROM_MODAL: i32 = -4;

/// Address of the cash-buy confirm's 예/아니오 byte, or `None` on a build without the
/// anchor.
///
/// ── WHY THIS EXISTS (2026-08-26) ─────────────────────────────────────────────
/// Player report: "구매창이 뜨면 무조건 구매하는 방법밖에없고 CLR눌러도 아무 동작을 안해".
/// Both halves are the game's, and reading `modalKey[13]` (`0xc3a4`, reconstructed from
/// the module's own table initialiser `0x59f0`) says why:
///
/// ```text
///   key + 4 <= 1        ; keys -4 / -3, i.e. a DIRECTION key
///        -> *(base+0xa434) ^= 1                  ; toggle 예 <-> 아니오
///   key == -5           ; OK
///        -> *(base+0xa434) == 0 ? closeModal(0)  ; 아니오 — this is the CANCEL
///                              : send `05 42`    ; 예 — buy
///   anything else       -> ignored               ; CLR included: the box has no CLR arm
/// ```
///
/// So the box CAN be cancelled — but only by a direction key followed by OK, and the
/// quantity spinner drawn over it eats every direction key. That is what left the player
/// with no way out. Writing this byte to 0 and synthesising OK runs the game's own
/// `closeModal(0)` arm, which is the cancel it already had.
pub fn cash_confirm_choice_addr() -> Option<u32> {
    let m = crate::guest_build::net_disconnect_modal()?;
    m.state_word.checked_add_signed(CASH_CHOICE_FROM_MODAL)
}

/// Address of the cash row cache, or `None` when this build has no anchor for it.
fn cash_row_base(_read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u32> {
    let m = crate::guest_build::net_disconnect_modal()?;
    Some(m.state_word + CASH_ROWS_FROM_MODAL)
}

/// Address of the row count that goes with it.
fn cash_row_count_addr() -> Option<u32> {
    let m = crate::guest_build::net_disconnect_modal()?;
    Some(m.state_word + CASH_ROWS_N_FROM_MODAL)
}

/// The modal id currently on screen (`0` = none). Logged by the worker so an id we
/// have not seen before can be identified from a device rather than guessed.
pub fn modal_id(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u32> {
    let m = crate::guest_build::net_disconnect_modal()?;
    rd_u32(read, m.state_word)
}

/// Items a stack makes sense for — the player's own list, and nothing else.
///
/// ⛔ NOT 여행가방(0x0e) or 창고열쇠(0x13): both are capped expansions granted once
/// per purchase, and NOT equipment, whose per-instance sockets and refine make a
/// stack meaningless. Everything absent from this list buys one at a time.
pub fn cash_qty_allowed(cat: u8, idx: u8) -> bool {
    cat == 0x12 && matches!(idx, 0x07 | 0x0a | 0x0b | 0x0c | 0x15 | 0x19 | 0x1a)
}

/// …and of those, the ones whose count must be written into the CLIENT'S ROW.
///
/// ⛔ NOT 오브상자 (`0x19`) or 소켓확장 (`0x1a`). Those two are the only purchases whose
/// real item the HOST delivers — `offline::Engine::cash_buy` hands the bag a real cat-0x10
/// ORB with a `05 04` frame. Poking their cash row on top of that makes the client grant N
/// copies of the (cat 0x12) SHOP item as well, and those copies do nothing. So the count
/// goes to the engine for these two and to the row for the other five —
/// `offline::Env::cash_qty` and this function are the two halves of one rule.
///
/// ★ 2026-08-25 — the delivery moved from the network WAREHOUSE to the bag. It only went
/// to the warehouse because that was the one place the client's 오브결합 picker could
/// reach it (44차); the host now runs the combine from the bag (69차), so the detour is
/// gone. Nothing about THIS rule changed: the host still must not write their row.
///
/// ★ 2026-08-22 — and now the client grants those two nothing at all: the engine answers
/// their `05 42` with a message (`offline::notice`) instead of an ack, so the `05 40`
/// commit that carries the grant is never sent. The player was seeing that inert copy
/// ("네트워크창고에 99개 생기고 싱글창고에 1개가 생겨"). ★ 오브상자 `0x19` could only join
/// once the host stopped paying it a premium buff the same day — its bag copy was that
/// buff's only trigger (`buffs::box_reward`, now `buffs::is_orb_box`). ⛔ And it stays
/// doubly wrong to add either index HERE: row+10 is the granted STACK SIZE, so even a 0
/// would leave a zero-quantity phantom row, not nothing.
pub fn cash_qty_row_write(cat: u8, idx: u8) -> bool {
    cash_qty_allowed(cat, idx) && !matches!(idx, 0x19 | 0x1a)
}

// ── WHEN the count is written, and why it is not while the box is up ──────────
//
// Player, 2026-08-21: "수량을 변경하면 캐시샵 아이콘 우측하단에 숫자가 입력한 수량으로
// 바뀌는데 … 이걸 안 바꾸고 싶다는 거야. 그렇지만 선택한 수량으로 구매는 되고".
//
// The drawn number and the granted count ARE THE SAME BYTE. Guest `0xff80` does
// `ldrb r2,[row,#2]` (row+10 through a rows+20i+8 pointer) and sprintf's it as the
// icon's corner number, and the grant at `0x13e4c` copies the whole 20-byte row and
// consumes `+10` inside `0x131d0` (`ldrb r5,[r6,#0xa]`). Nothing else carries the
// number: both `05 42` (built at `0xc3c6`) and `05 40` (`0x13de0`) send only
// row[0..8] and row[12..16], and the reply the engine sends back is a status byte —
// so the count CANNOT be delivered over the wire, and the two uses can only be
// separated in TIME.
//
// The first version wrote the count on a `frame % 3` poll for as long as the confirm
// box was open, which is precisely why the whole page read 99: `0xff80` re-renders
// the poked value on every repaint and the box is drawn OVER a live grid.
//
// ★ So the write moved to the ONE moment the code can order exactly: the commit key,
// applied in `apply_input` BEFORE the key is handed to the guest. `handle_event` only
// pushes onto the event queue (`wie_lgt/src/emulator.rs`), so the guest cannot observe
// the key — and therefore cannot run `0xc3a4`, send `05 42`, or reach the grant —
// until a later tick. That ordering is guaranteed rather than likely, which matters
// here: the whole round trip (key → `05 42` → ack → `05 40` → ack → grant) can
// complete inside a SINGLE worker iteration, so any scheme that writes late and
// restores on a later poll has no window at all. Same mechanism as
// `full_restore_writes` and `popup_count_restore`, both of which already ship.

/// `state_word + this` = the grid's cursor byte (signed). `base + 0x9c49`.
const CASH_CURSOR_FROM_MODAL: i32 = 0x9c49 - 0xa438;
/// `state_word + this` = the per-cell row-index table the cursor indexes.
/// `base + 0x9c4a`.
const CASH_INDEX_TABLE_FROM_MODAL: i32 = 0x9c4a - 0xa438;

/// Which cached row the grid cursor is on, i.e. the row the grant will read.
///
/// The grant does NOT use the instance key it just echoed — `0x13e7e`/`0x13e8c`
/// re-derive the index from the on-screen cursor at reply time:
/// `rowIdx = (i8)*(base + 0x9c4a + (i8)*(base + 0x9c49))`. The same expression appears
/// at `0xc3dc` (building `05 42`) and `0x13df6`, three agreeing sites.
///
/// Returned only when it lands inside the cache the client actually filled, so a
/// garbage cursor can never move a write outside the 16-row array.
fn cash_cursor_row(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u32> {
    let m = crate::guest_build::net_disconnect_modal()?;
    let rows = cash_row_base(read)?;
    let n = u32::from(rd_u16(read, cash_row_count_addr()?)?.min(CASH_ROWS_MAX as u16));
    let cursor_at = m.state_word.checked_add_signed(CASH_CURSOR_FROM_MODAL)?;
    let table_at = m.state_word.checked_add_signed(CASH_INDEX_TABLE_FROM_MODAL)?;
    let sel = rd_u8(read, cursor_at)? as i8;
    let table_idx = rd_u8(read, table_at + u32::from(sel.max(0) as u8))? as i8;
    Some(rows + cash_row_from_cursor(sel, table_idx, n)? * 20)
}

/// The pure half of [`cash_cursor_row`]: turn the two signed cursor bytes into a row
/// number inside the cache, or `None`.
///
/// Both bytes are read `ldrsb` by the guest, so both can be negative, and `n` is
/// whatever the last `05 3f` filled. Returning `None` rather than clamping is the
/// point: a count that is wrong grants the wrong number of items, so an unreadable
/// cursor must fall back to the wide write instead of guessing a row.
pub fn cash_row_from_cursor(sel: i8, table_idx: i8, n: u32) -> Option<u32> {
    if sel < 0 || i64::from(sel) >= i64::from(CASH_ROWS_MAX) {
        return None;
    }
    let idx = u32::try_from(table_idx).ok()?;
    (idx < n.min(CASH_ROWS_MAX)).then_some(idx)
}

/// The row the grid cursor is on, with its `(cat, idx)` — i.e. the item the player is
/// actually buying.
///
/// `None` when the cursor cannot be resolved. Every caller treats that as "do nothing":
/// this is the item's identity, and acting on a guess about WHICH item is being bought
/// is how a quantity ends up attached to the wrong thing.
pub fn cash_cursor_item(read: &impl Fn(u32, usize) -> Vec<u8>) -> Option<(u32, u8, u8)> {
    let row = cash_cursor_row(read)?;
    Some((row, rd_u8(read, row + 8)?, rd_u8(read, row + 9)?))
}

/// The write that makes the client hand over `qty` — and the byte it replaces.
///
/// Returns `(addr, new, old)` so the caller can put the row back exactly as the client
/// filled it. That matters: one shipped catalog row is legitimately a stack (cat `0x0e`
/// idx `0x03` 제련석 carries `+10 = 99`).
///
/// ⚠ **ONLY the row the cursor is on, and only if that row is eligible.** Two earlier
/// versions were wider and both were wrong:
///   * writing `1` into every ineligible row "to normalise them" turned the 제련석 row
///     into a single stone — the grant reads the same byte, so the player paid for 99
///     and got one;
///   * writing `qty` into every eligible row on the page needed no cursor, but it
///     touched rows the player had not selected, and it paired with a page-wide offer
///     test that showed the picker for items it could not apply to (device-reported
///     2026-08-21: "창고열쇠 수량 변경해서 샀는데도 1개만 사지는데").
/// One row, chosen by identity, is the whole rule now.
pub fn cash_qty_writes(read: &impl Fn(u32, usize) -> Vec<u8>, qty: u8) -> Vec<(u32, u8, u8)> {
    let want = qty.max(1);
    let (row, cat, idx) = match cash_cursor_item(read) {
        Some(v) => v,
        None => return Vec::new(),
    };
    if !cash_qty_row_write(cat, idx) {
        return Vec::new();
    }
    match rd_u8(read, row + CASH_ROW_QTY) {
        Some(old) if old != want => vec![(row + CASH_ROW_QTY, want, old)],
        _ => Vec::new(),
    }
}

/// May a quantity be chosen for the item the player is buying RIGHT NOW?
///
/// ★ Per ITEM, not per page (2026-08-21). This used to answer "does this page hold
/// anything eligible", which on the cash shop's page 0 is always yes — so the picker
/// appeared over 창고열쇠 and 여행가방 too, took a number, and then the purchase
/// correctly handed over one. A picker that cannot affect the purchase it is drawn on
/// is worse than no picker: the player believes they bought five.
///
/// Fails CLOSED. If the cursor cannot be read we show nothing rather than showing a
/// control that might be lying — see [`cash_cursor_item`].
pub fn cash_qty_offerable(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    matches!(cash_cursor_item(read), Some((_, cat, idx)) if cash_qty_allowed(cat, idx))
}

// ── hysteresis for the two flags that decide whether the arrows appear ───────
//
// Both live here rather than inline in the poll so they can be tested: the bug they
// exist for is a SEQUENCE, not a value, and a sequence is only checkable if the rule
// is a function. See `CUTSCENE_EXIT_POLLS` / `CHOICE_CONFIRM_POLLS` for the reasoning
// about why consecutive samples and not elapsed time.

/// Should the cutscene control set still be shown? `gone` counts consecutive polls
/// that saw no script window and is owned by the caller.
///
/// A page turn inside a dialogue drops the window id for a poll or two and puts it
/// straight back; a scene that ended never does. So the hold is given up only after
/// `exit_polls` polls agree — and, crucially, **nothing else may cancel it**. The
/// version that did (a fast cancel when the field looked free) is what let the MENU
/// control set flash between dialogue pages.
/// The scene hold, with the one cancel that IS legitimate: **another window opened**.
///
/// ★ 66차-B (player: *"컷신상태가 아닌데도 상점이나 제련창에서 아무데나 눌러도 OK버튼과
/// 같이 동작하는거같이 보여"*). The hold exists to absorb the blip where the script
/// window's id drops to `WIN_NONE` between text pages — that is a window id of 3, and
/// nothing else. When the id becomes a REAL other window (12 상점, 13 제련, 15 인벤, …)
/// the scene is over as a matter of fact: the game has put a different screen on top.
/// Holding on for another `exit_polls` there gave every window opened from a dialogue a
/// ~0.5 s head start in which `overlay::tap_anywhere_key` answered OK for a tap
/// ANYWHERE — which is exactly what the player saw, and it could confirm a purchase.
///
/// ⛔ Do NOT widen this cancel back to "the field looks free" (the 2026-08-21 version):
/// `in_field_state` is true during a page turn, and that cancel is what made the menu
/// arrows flash mid-scene. The cancel here is the opposite case — a window is up, not
/// nothing is up.
pub fn cutscene_hold_gated(gone: &mut u32, script_open: bool, other_window: bool, exit_polls: u32) -> bool {
    if script_open {
        *gone = 0;
        return true;
    }
    if other_window {
        *gone = exit_polls;
        return false;
    }
    *gone = gone.saturating_add(1);
    *gone < exit_polls
}

/// Is a window OTHER than the script window on screen? `false` for the script window
/// itself and for "no window at all", which is the blip [`cutscene_hold_gated`] absorbs.
pub fn other_window_open(read: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    match window_id(read) {
        Some(w) => w != WIN_NONE && w != WIN_SCRIPT,
        None => false,
    }
}

/// Should the dialogue CHOICE arrows be shown? `seen` counts consecutive polls that
/// saw the flag set.
///
/// The flag is written around the script window's own open/close, so one sample can
/// catch a 1 that no choice box is behind. A real choice holds the flag until the
/// player answers, so requiring agreement cannot hide one.
pub fn choice_hold(seen: &mut u32, flag: bool, confirm_polls: u32) -> bool {
    if !flag {
        *seen = 0;
        return false;
    }
    *seen = seen.saturating_add(1);
    *seen >= confirm_polls
}

/// Modal id of the 획득/안내 box the grant opens (`showModal(…, 7, …)` at `0x13eea`).
/// Seeing it means the client has already copied the row, so the count can go back.
pub const MODAL_GRANT_NOTICE: u32 = 7;

// ── the slot screen's `#` (60차) ─────────────────────────────────────────────
//
// `#` there is TWO features chosen by occupancy: 초기화 on a slot that holds a
// character, 복구 on an empty one. Only the restore is retired, and the host does NOT
// try to tell them apart from guest state — an attempt to do that read the occupancy
// array through the online module's singleton and got `[1,1,1]` for a screen showing
// one character, i.e. the two modules do not share the base the anchor was derived
// from, and the cursor had no verified anchor at all.
//
// It does not need one. **A restore is defined by the network request it sends**
// (`04 3e`), and a delete is purely local — so the refusal lives where the request
// arrives (`offline::Engine::recovery`), which cannot be wrong about which of the two
// the player asked for.
