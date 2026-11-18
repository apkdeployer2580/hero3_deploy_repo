// Standalone self-test of items.rs quickbar logic (the wie_android test binary
// segfaults at startup on Windows host due to linked winit/android deps).
//   rustc --edition 2021 --test items_selftest.rs -o t.exe && ./t.exe

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind { Empty, Skill, Item }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuickSlot { pub slot: u8, pub kind: SlotKind, pub count: u16, pub ref_id: u16 }

const CHAR_PTR_GLOBAL: u32 = 0x1b_3084;
const CHAR_FALLBACK: u32 = 0x1b_7b6c;
const HEAP_LO: u32 = 0x10_0000;
const HEAP_HI: u32 = 0x8000_0000;
const QB_SLOTS: u32 = 5;
const QB_TYPE_OFF: u32 = 0x252;
const QB_ITEMREF_OFF: u32 = 0x257;
const QB_SKILLREF_OFF: u32 = 0x25c;
const TY_SKILL: u8 = 1;
const TY_ITEM: u8 = 2;
const GRID_HANDLE_OFF: u32 = 0x9e28;
const GRID_DATA_ADJ: u32 = 8;
const TAB_STRIDE: u32 = 0x3c4;
const SLOT_STRIDE: u32 = 0x3c;
const COUNT_OFF: u32 = 0x3ba;
const STATE_OFF: u32 = 0x7c;
const STATE_FIELD: u32 = 4;
const SELECTOR_FLAG_OFF: u32 = 0x9c6c;

fn rd_u16(r: &impl Fn(u32, usize) -> Vec<u8>, a: u32) -> Option<u16> { let b=r(a,2); (b.len()==2).then(||u16::from_le_bytes([b[0],b[1]])) }
fn rd_u32(r: &impl Fn(u32, usize) -> Vec<u8>, a: u32) -> Option<u32> { let b=r(a,4); (b.len()==4).then(||u32::from_le_bytes([b[0],b[1],b[2],b[3]])) }
fn rd_u8(r: &impl Fn(u32, usize) -> Vec<u8>, a: u32) -> Option<u8> { let b=r(a,1); (b.len()==1).then(||b[0]) }
fn in_heap(p: u32) -> bool { (HEAP_LO..HEAP_HI).contains(&p) }

fn resolve_char(r: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u32> {
    if let Some(p)=rd_u32(r,CHAR_PTR_GLOBAL) { if in_heap(p) { return Some(p); } }
    (r(CHAR_FALLBACK,4).len()==4).then_some(CHAR_FALLBACK)
}
// Mirror of items.rs::deref_container — the per-build container convention
// (MIGRATION_LGT.md §2.8): `Some(off)` = the field is a HANDLE and the payload is
// `*(handle) + off` (KTF, off 8); `None` = the field is already the payload address
// (LGT). items.rs reads the convention from `guest_build::container_data_off()`; this
// copy has no build atomic, so the convention is a parameter and the KTF call sites
// pass `Some(GRID_DATA_ADJ)`.
fn deref_container(r: &impl Fn(u32, usize) -> Vec<u8>, field: u32, data_adj: Option<u32>) -> Option<u32> {
    let first = rd_u32(r, field)?;
    if !in_heap(first) { return None; }
    let Some(off) = data_adj else { return Some(first) };
    let block = rd_u32(r, first)?;
    if !in_heap(block) { return None; }
    let base = block.checked_add(off)?;
    in_heap(base).then_some(base)
}
fn resolve_grid_base(r: &impl Fn(u32, usize) -> Vec<u8>, c: u32) -> Option<u32> {
    deref_container(r, c + GRID_HANDLE_OFF, Some(GRID_DATA_ADJ))
}
fn item_count(r: &impl Fn(u32, usize) -> Vec<u8>, c: u32, tab: u8, slot: u8) -> u16 {
    let Some(base)=resolve_grid_base(r,c) else { return 0 };
    rd_u16(r, base + u32::from(tab)*TAB_STRIDE + u32::from(slot)*SLOT_STRIDE + COUNT_OFF).unwrap_or(0)
}
// The `scan_quickbar` mirror that used to live here is GONE, with the original
// (2026-09-04). There are more host PLATES than guest slots now, so the row the HUD
// draws is resolved from the page store (`items::plate_views`) and a five-wide scan
// can no longer describe it. What the guest bar still answers for on its own -- one
// bound cell's stack count -- is `item_count` above.
// ── window id + field gate (mirror of items.rs; RE wf_6d270fb0 Q3) ───────────
// CHAR+0x7c does NOT change when one of the game's windows opens, so the window
// id at CHAR+0xa0c0 (sole writer openWindow @0x12c6a4) is the discriminator.
const WINDOW_ID_OFF: u32 = 0xa0c0;
pub const WIN_NONE: u8 = 3;
pub const WIN_MAP: u8 = 8;

pub fn window_id(r: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u8> {
    let c = resolve_char(r)?;
    rd_u8(r, c + WINDOW_ID_OFF)
}

pub fn in_field_state(r: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    let Some(c)=resolve_char(r) else { return false };
    matches!(
        (rd_u8(r,c+WINDOW_ID_OFF), rd_u32(r,c+STATE_OFF), rd_u8(r,c+SELECTOR_FLAG_OFF)),
        (Some(WIN_NONE), Some(STATE_FIELD), Some(0))
    )
}

// ── cutscene vs menu, and "is there a character at all" ──────────────────────
//
// Window id 4 = SCRIPT playback (story scene / NPC dialogue) — a DIFFERENT id from
// every menu window, which is what makes the player's 2026-08-03 question ("컷신과
// 메뉴오픈상태 등을 구분이 가능한지") answerable: yes, by one byte.
//
// `in_game_state` is app state 4 regardless of window, i.e. "a live character owns
// the world". The quickbar is scanned for the whole of it (the plates stay on screen
// with a window open) but NOT on the title/boot shell, where CHAR is not a character
// and the slot bytes are whatever the heap last held.
pub const WIN_SCRIPT: u8 = 4;

pub fn cutscene_open(r: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    window_id(r) == Some(WIN_SCRIPT)
}

pub fn in_game_state(r: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    let Some(c)=resolve_char(r) else { return false };
    rd_u32(r, c + STATE_OFF) == Some(STATE_FIELD)
}

// State 3 is TWO different things (66차-C): the title/slot/class shell AND every
// ONLINE screen (창고 · 캐시샵). `resolve_char` is what separates them — on the
// Static-anchor build it refuses below level 1, so the shell cannot resolve.
pub const STATE_ONLINE: u32 = 3;

pub fn online_screen(r: &impl Fn(u32, usize) -> Vec<u8>) -> bool {
    let Some(c)=resolve_char(r) else { return false };
    rd_u32(r, c + STATE_OFF) == Some(STATE_ONLINE)
}

// ── inventory consumables, for the potion buttons (player request 2026-08-03) ─
//
// The record arithmetic is `useItemEffect`'s own (guest 0x153e08, re-disassembled
// 2026-08-03): rec = grid_base + tab*0x3c4 + slot*0x3c, category u16 @rec+0x3b6
// compared against 0x12, index u16 @rec+0x3b8, count u16 @rec+0x3ba. The call the
// host makes is `useItemEffect(r0 = tab, r1 = slot)` — the caller at 0x1430c6 does
// `r0 = ref>>5; r1 = ref&0x1f` on the quickbar's item-ref byte.
//
// tab comes out of 3 bits and the tab stride holds exactly 16 slot records, so a
// full 8x16 sweep can never MISS a stack — only read past the end of a shorter
// grid. Hence the strict per-record validation: exact category, plausible count,
// and an item-DB icon frame the atlas actually has.
// (the item-DB chain — ITEMDB_HANDLE_OFF / STRIDE / ICON_OFF and `item_db_icon` —
// is already mirrored further down for the secondary-weapon icon; same lookup.)
const CONSUMABLE_CAT: u16 = 0x12;
const CAT_OFF: u32 = 0x3b6;
const IDX_OFF: u32 = 0x3b8;
const SCAN_TABS: u8 = 8;
const SCAN_SLOTS: u8 = 16;
const COUNT_CAP: u16 = 999;
/// `useItemEffect` @ guest 0x153e08, Thumb (low bit set for `call_guest_function`).
pub const USE_ITEM_EFFECT: u32 = 0x0015_3e08 | 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvItem { pub tab:u8, pub slot:u8, pub idx:u16, pub count:u16, pub icon:u16 }

fn item_record(r: &impl Fn(u32, usize) -> Vec<u8>, c: u32, tab: u8, slot: u8) -> (u16,u16,u16) {
    let Some(base)=resolve_grid_base(r,c) else { return (0,0,0) };
    let rec = base + u32::from(tab)*TAB_STRIDE + u32::from(slot)*SLOT_STRIDE;
    (rd_u16(r,rec+COUNT_OFF).unwrap_or(0), rd_u16(r,rec+CAT_OFF).unwrap_or(0), rd_u16(r,rec+IDX_OFF).unwrap_or(0))
}

/// `has_icon` stands in for `atlas().item(frame).is_some()` — the real filter, kept
/// injectable here because the atlas is a 81 KB embedded blob.
pub fn scan_consumables(r: &impl Fn(u32, usize) -> Vec<u8>, has_icon: &impl Fn(u16)->bool) -> Vec<InvItem> {
    let Some(c)=resolve_char(r) else { return Vec::new() };
    if resolve_grid_base(r,c).is_none() { return Vec::new(); }
    let mut out: Vec<InvItem> = Vec::new();
    for tab in 0..SCAN_TABS {
        for slot in 0..SCAN_SLOTS {
            let (count,cat,idx)=item_record(r,c,tab,slot);
            if cat != CONSUMABLE_CAT || !(1..=COUNT_CAP).contains(&count) { continue; }
            let Some(icon)=item_db_icon(r,c,idx).filter(|f| has_icon(*f)) else { continue };
            match out.iter_mut().find(|e| e.idx==idx) {
                Some(e) if e.count >= count => {}
                Some(e) => *e = InvItem{tab,slot,idx,count,icon},
                None => out.push(InvItem{tab,slot,idx,count,icon}),
            }
        }
    }
    out.sort_by_key(|e| e.idx);
    out
}

pub fn find_consumable(r: &impl Fn(u32, usize) -> Vec<u8>, idx: u16) -> Option<(u8,u8)> {
    let c=resolve_char(r)?;
    resolve_grid_base(r,c)?;
    for tab in 0..SCAN_TABS { for slot in 0..SCAN_SLOTS {
        let (count,cat,id)=item_record(r,c,tab,slot);
        if id==idx && cat==CONSUMABLE_CAT && (1..=COUNT_CAP).contains(&count) { return Some((tab,slot)); }
    }}
    None
}

pub fn consumable_count(r: &impl Fn(u32, usize) -> Vec<u8>, idx: u16) -> u16 {
    let Some(c)=resolve_char(r) else { return 0 };
    let mut total=0u16;
    for tab in 0..SCAN_TABS { for slot in 0..SCAN_SLOTS {
        let (count,cat,id)=item_record(r,c,tab,slot);
        if id==idx && cat==CONSUMABLE_CAT && (1..=COUNT_CAP).contains(&count) { total=total.saturating_add(count); }
    }}
    total
}

// ── player gauges (mirror of items.rs::read_stats; RE wf_6d270fb0 Q1) ────────
// ★ The old mapping had HP and SP SWAPPED: it read CHAR+0xf8 (= SP, 0..1000) as
// "HP" over a hardcoded 1000, so an HP potion never moved the red bar.
//   AddHP @0x1467a8: cur = (s16)CHAR[0xf2]; clamp to (s32)CHAR[0xec] and to 0.
//   AddSP @0x1467d0: cur = (s32)CHAR[0xf8]; clamp 0..1000.
//   ComputeMaxHP @0x145f4c (sole writer of CHAR+0xec).
const HP_OFF: u32 = 0xf2;
const HP_MAX_OFF: u32 = 0xec;
const SP_OFF: u32 = 0xf8;
const SP_MAX: u16 = 1000;
const EXP_OFF: u32 = 0xe4;
const EXP_NEXT_OFF: u32 = 0xe8;
const LEVEL_OFF: u32 = 0xe2;
const LEVEL_CAP: u16 = 99; // 111차: the LGT clamp is patched 70 -> 99
// 142차: the GAME's max-HP field is `min(true max, 32767)` since `hp_max_lgt`, so the
// liveness ceiling is the i16 ceiling (was 30000). The HUD's true max is a separate value.
const HP_CAP: u16 = 32767;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats { pub hp:u16, pub hp_max:u16, pub sp:u16, pub sp_max:u16, pub exp:u32, pub exp_next:u32, pub level:u16 }

pub fn read_stats(r: &impl Fn(u32, usize) -> Vec<u8>) -> Option<Stats> {
    let c = resolve_char(r)?;
    let s = Stats {
        hp: (rd_u16(r, c + HP_OFF)? as i16).max(0) as u16,
        hp_max: rd_u32(r, c + HP_MAX_OFF)?.try_into().unwrap_or(u16::MAX),
        sp: rd_u32(r, c + SP_OFF)?.min(u32::from(SP_MAX)) as u16,
        sp_max: SP_MAX,
        exp: rd_u32(r, c + EXP_OFF)?,
        exp_next: rd_u32(r, c + EXP_NEXT_OFF)?,
        level: rd_u16(r, c + LEVEL_OFF)?,
    };
    plausible_stats(&s).then_some(s)
}

// ⛔ 80차-E: `hp_max` is a CEILING, not a floor, and `hp <= hp_max` is not a liveness
// test at all — see items.rs::plausible_stats. Both terms refused live records.
fn plausible_stats(s: &Stats) -> bool {
    (1..=LEVEL_CAP).contains(&s.level) && s.hp_max <= HP_CAP && s.sp <= s.sp_max
}

/// Mirror of items.rs::char_live — the LGT static anchor's liveness rule.
fn char_live(level: u16, hp_max: u32, sp: u32) -> bool {
    (1..=LEVEL_CAP).contains(&level) && hp_max <= u32::from(HP_CAP) && sp <= u32::from(SP_MAX)
}

/// The game's own max-HP formula (ComputeMaxHP @0x145f4c) — used by the test to
/// build a character whose numbers the engine itself would have produced.
pub fn compute_max_hp(level: i32, vit: i32, pct: i32) -> i32 {
    (36 * level + 10 * vit) * (100 + pct) / 100
}


// ── the game's own context popup -> "퀵바등록" (mirror of items.rs) ────────────
//
// The host rewrites the guest's OWN label buffers and lets the guest draw its list;
// registration is then done by the game itself (write the highlighted-row byte, call
// popup_key(kind, -5)). Drawing over the popup is impossible — draw_popup keeps x/y
// in its stack frame — and patching the row table is fatal on the skill side
// (skill_bind's 2nd caller 0x154462 hardcodes slot 5). RE: workflow wf_95a21c44,
// 5 questions x independent adjudicator.
const MENU_PAGE_OFF: u32 = 0x9c6e;
pub const PAGE_ITEM: u8 = 2;
pub const PAGE_SKILL: u8 = 4;
const POPUP_OPEN_OFF: u32 = 0x9c84;
const POPUP_COUNT_OFF: u32 = 0x9c85;
const POPUP_SEL_OFF: u32 = 0x9c86;
const POPUP_TAG_OFF: u32 = 0x9c87;
// 51차: the pre-emptor bytes became `guest_build::popup_preempt_offsets()` — a per-build
// LIST, because they are KTF measurements serving a commit that bypasses the game's key
// router. LGT's synthetic-OK commit rides the router itself and its list is EMPTY (the
// KTF offsets read unrelated LGT fields; that stray non-zero is what kept consumables
// on the native 1키등록 list on the device while skills relabelled fine). This mirror
// models the KTF shape, so `popup_view` uses the KTF list; `popup_view_on` exposes the
// list so the LGT semantics (empty = gate skipped) are testable too.
const POPUP_PREEMPT_A_OFF: u32 = 0x9c89;
const POPUP_PREEMPT_B_OFF: u32 = 0xa220;
const POPUP_PREEMPT_KTF: [u32; 2] = [POPUP_PREEMPT_A_OFF, POPUP_PREEMPT_B_OFF];
const TAG_ITEM: u8 = 0x66;
const TAG_SKILL: u8 = 0x67;
const ROWS_ITEM: i8 = 8;
const ROWS_SKILL: i8 = 7;
const OUR_ROWS_ITEM: i8 = 4;
const OUR_ROWS_SKILL: i8 = 3;
const POPUP_HANDLES_SLOT: u32 = 0x1b_2df4;
const POPUP_PAYLOAD_ADJ: u32 = 8;
const POPUP_LABEL_CAP: usize = 10;
pub const POPUP_KEY_FN: u32 = 0x0013_a86c | 1;
pub const POPUP_KEY_SELECT: u32 = (-5i32) as u32;
/// `퀵바등록` EUC-KR + NUL = 9 B, inside the 10 B buffer. The SPACED form is 11 B and
/// would overrun — the guest itself does that with 포인트투자, which is why not.
const LABEL_QUICKBAR: [u8; 9] = [0xc4, 0xfc, 0xb9, 0xd9, 0xb5, 0xee, 0xb7, 0xcf, 0x00];
const LABEL_KEY1: [u8; 8] = [0x31, 0xc5, 0xb0, 0xb5, 0xee, 0xb7, 0xcf, 0x00];
const SKILL_LEARNED_BASE: u32 = 0x1a8;
const SKILL_CAT_OFF: u32 = 0x9c80;
const SKILL_CURSOR_BASE: u32 = 0x9c7d;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopupView { pub kind: u8, pub item: bool, pub sel: i8, pub rows: i8, pub can_bind: bool }

fn popup_handles(r: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u32> {
    let b = rd_u32(r, POPUP_HANDLES_SLOT)?; in_heap(b).then_some(b)
}
fn popup_label_addr(r: &impl Fn(u32, usize) -> Vec<u8>, handles: u32, i: u32) -> Option<u32> {
    let h = rd_u32(r, handles + i*4)?; if !in_heap(h) { return None; }
    let b = rd_u32(r, h)?; if !in_heap(b) { return None; }
    let p = b.checked_add(POPUP_PAYLOAD_ADJ)?; in_heap(p).then_some(p)
}

pub fn popup_view(r: &impl Fn(u32, usize) -> Vec<u8>) -> Option<PopupView> {
    popup_view_on(r, &POPUP_PREEMPT_KTF)
}

pub fn popup_view_on(r: &impl Fn(u32, usize) -> Vec<u8>, preempts: &[u32]) -> Option<PopupView> {
    let c = resolve_char(r)?;
    if rd_u8(r, c + SELECTOR_FLAG_OFF)? == 0 { return None; }
    let kind = rd_u8(r, c + MENU_PAGE_OFF)?;
    if kind != PAGE_ITEM && kind != PAGE_SKILL { return None; }
    if rd_u8(r, c + POPUP_OPEN_OFF)? == 0 { return None; }
    let tag = rd_i8(r, c + POPUP_TAG_OFF)? as u8;
    let item = match (tag, kind) {
        (TAG_ITEM, PAGE_ITEM) => true,
        (TAG_SKILL, PAGE_SKILL) => false,
        _ => return None,
    };
    let (native_rows, our_rows) = if item { (ROWS_ITEM, OUR_ROWS_ITEM) } else { (ROWS_SKILL, OUR_ROWS_SKILL) };
    let rows = rd_i8(r, c + POPUP_COUNT_OFF)?;
    if rows != native_rows && rows != our_rows { return None; }
    if item {
        for off in preempts {
            if rd_u8(r, c + off)? != 0 { return None; }
        }
    }
    let handles = popup_handles(r)?;
    let row1 = popup_label_addr(r, handles, 1)?;
    let got = r(row1, POPUP_LABEL_CAP);
    if got.len() < LABEL_KEY1.len() { return None; }
    if got[..LABEL_KEY1.len()] != LABEL_KEY1 && got[..LABEL_QUICKBAR.len()] != LABEL_QUICKBAR { return None; }
    if item && resolve_grid_base(r, c).is_none() { return None; }
    let can_bind = if item { true } else {
        let cat = u32::from(rd_u8(r, c + SKILL_CAT_OFF)?);
        let cur = rd_i8(r, c + SKILL_CURSOR_BASE + cat)?;
        if cur < 0 { return None; }
        rd_i8(r, c + SKILL_LEARNED_BASE + (cat*9 + cur as u32)*8).is_some_and(|v| v > 0)
    };
    Some(PopupView { kind, item, sel: rd_i8(r, c + POPUP_SEL_OFF)?, rows: our_rows, can_bind })
}

pub fn popup_relabel(r: &impl Fn(u32, usize) -> Vec<u8>, v: &PopupView) -> Vec<(u32, Vec<u8>)> {
    let Some(c) = resolve_char(r) else { return Vec::new() };
    let Some(handles) = popup_handles(r) else { return Vec::new() };
    let mut out = Vec::new();
    if let Some(a) = popup_label_addr(r, handles, 1) {
        let cur = r(a, LABEL_QUICKBAR.len());
        if cur.len() != LABEL_QUICKBAR.len() || cur != LABEL_QUICKBAR { out.push((a, LABEL_QUICKBAR.to_vec())); }
    }
    let moves: &[(u32,u32)] = if v.item { &[(2,6),(3,7)] } else { &[(2,6)] };
    for (dst, src) in moves {
        let (Some(da), Some(sa)) = (popup_label_addr(r, handles, *dst), popup_label_addr(r, handles, *src)) else { continue };
        let text = r(sa, POPUP_LABEL_CAP);
        let Some(n) = text.iter().position(|b| *b == 0) else { continue };
        let bytes = text[..=n].to_vec();
        if r(da, bytes.len()) != bytes { out.push((da, bytes)); }
    }
    let want = v.rows as u8;
    if rd_u8(r, c + POPUP_COUNT_OFF) != Some(want) { out.push((c + POPUP_COUNT_OFF, vec![want])); }
    out
}

pub fn popup_guest_row(v: &PopupView, display: i8, slot: u8) -> Option<u8> {
    match display {
        // `then`, not `then_some` — the latter evaluates `slot + 1` eagerly and
        // overflows for slot 255 before the range test rejects it.
        1 => (u32::from(slot) < QB_SLOTS).then(|| slot + 1),
        2 => Some(6),
        3 => v.item.then_some(7),
        _ => None,
    }
}

// ── auto-loot + equipped secondary weapon (RE wf_477f0b16 QD / QC) ───────────
// Auto-loot: u8 @ CHAR+0x2d7, set to 1 by the 오토루팅 cash item (guest 0x1540fc);
// cleared only by CHAR_init and the new-game reset; NOT serialised. The host keeps
// it at 1 (idempotent — three writers in the whole image, no bit sharing).
// ★ 2026-08-15 (50차): the real `items.rs` no longer has an offset here AT ALL. The flag
// is a byte ARRAY at `CHAR+0x2d7` indexed by the SAVE SLOT (`CHAR+0xa410`), because that
// is what the game's own `useItem` arm `0x5523c` computes and what the reader `0x3ac02`
// reads back. `CHAR+0x2d7` below is therefore ELEMENT 0, not "the flag".
//
// ⚠ WHAT THIS FILE DOES AND DOES NOT COVER. It is a hand-written mirror (see the header),
// so it deliberately keeps only the build-independent rule — "write ONLY while the byte
// reads 0". The slot indexing, the out-of-range refusal and the bit-7 value live in
// `guest_anchors::AutolootFlag` and are asserted by `anchors_selftest.rs`, which
// `include!`s the real source and therefore cannot drift from it. Do not re-implement the
// indexing here: a second copy of it is exactly how this file's warning gets earned again.
const AUTOLOOT_OFF: u32 = 0x2d7;
// Equip slot 6 = the SECONDARY weapon (field key '7' = hand 2 / slot 6, guest
// 0x143466). Its (tab, slot) indexes the same inventory grid the quickbar uses.
const EQUIP6_TAB_OFF: u32 = 0x513;
const EQUIP6_SLOT_OFF: u32 = 0x514;
const EQUIP6_PRESENT_OFF: u32 = 0x524;
const ITEM_FRAME_MAX: u16 = 28;
const ITEMDB_HANDLE_OFF: u32 = 0x38;
const ITEMDB_STRIDE: u32 = 0x18;
const ITEMDB_ICON_OFF: u32 = 0x14;

pub fn autoloot_to_force(r: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u32> {
    let c = resolve_char(r)?;
    let addr = c + AUTOLOOT_OFF;
    (rd_u8(r, addr)? == 0).then_some(addr)
}

fn item_db_icon(r: &impl Fn(u32, usize) -> Vec<u8>, c: u32, idx: u16) -> Option<u16> {
    let base = resolve_grid_base(r, c)?;
    let handle = rd_u32(r, base + ITEMDB_HANDLE_OFF)?;
    if !in_heap(handle) { return None; }
    let block = rd_u32(r, handle)?;
    if !in_heap(block) { return None; }
    rd_u8(r, block + GRID_DATA_ADJ + u32::from(idx) * ITEMDB_STRIDE + ITEMDB_ICON_OFF).map(u16::from)
}

/// `Some(item_bm frame)` for the equipped secondary weapon; `None` = none equipped.
pub fn secondary_weapon_icon(r: &impl Fn(u32, usize) -> Vec<u8>) -> Option<u16> {
    let c = resolve_char(r)?;
    if rd_i8(r, c + EQUIP6_PRESENT_OFF)? < 0 { return None; }
    let tab = rd_i8(r, c + EQUIP6_TAB_OFF)?;
    let slot = rd_i8(r, c + EQUIP6_SLOT_OFF)?;
    if tab < 0 || slot < 0 { return None; }
    let base = resolve_grid_base(r, c)?;
    let rec = base + u32::from(tab as u8) * TAB_STRIDE + u32::from(slot as u8) * SLOT_STRIDE;
    let idx = rd_u16(r, rec + 0x3b8)? as i16;
    let cat = rd_u16(r, rec + 0x3b6)?;
    if idx < 0 { return None; }
    icon_frame(r, c, cat, idx as u16)
}

/// Mirrors items.rs::icon_frame = the game's own rule at guest 0x1533a8:
/// consumables (cat 0x12) go through the item DB, everything else uses the
/// CATEGORY as the frame, with one special case.
pub fn icon_frame(r: &impl Fn(u32, usize) -> Vec<u8>, c: u32, cat: u16, idx: u16) -> Option<u16> {
    let frame = if cat == 0x12 {
        item_db_icon(r, c, idx)?
    } else if cat == 0x0e && idx == 3 {
        0x1c
    } else {
        cat
    };
    (frame <= ITEM_FRAME_MAX).then_some(frame)
}

// ── skill icon (mirror of items.rs::skill_icon_of, model = guest 0x14a51a) ───
// ★ 2026-09-04: takes `(cat, cell)` rather than a quickbar slot, exactly like the
// original now does -- most plates are not in the guest's bar to be read from, so the
// two bytes come from the plate's stored identity instead. `cat` IS the skill-ref's
// top two bits, so every ground-truth case below carries over unchanged.
const SKILL_SHEET_OFF: u32 = 0xd1;
const SKILL_STRIP_CELLS: i8 = 9;
fn rd_i8(r: &impl Fn(u32, usize) -> Vec<u8>, a: u32) -> Option<i8> { let b=r(a,1); (b.len()==1).then(||b[0] as i8) }
/// -> (atlas sheet 0..6, cell 0..8)
pub fn skill_icon_of(r: &impl Fn(u32, usize) -> Vec<u8>, c: u32, cat: u8, cell: u8) -> Option<(u8,u8)> {
    if i16::from(cell) >= i16::from(SKILL_STRIP_CELLS) { return None; }
    let sheet_id = rd_u8(r, c + SKILL_SHEET_OFF + u32::from(cat != 0))?;
    let atlas = sheet_id.checked_sub(4).filter(|&s| s <= 6)?;
    Some((atlas, cell))
}


// ── target-monster box left shift (mirrors items::target_box_x_patches) ───────
const TARGET_BOX_X_SITES: [(u32, u16, i32); 12] = [
    // PATH A - CHAR[0x1ea] > 0
    (0x14_9df2, 0x3902, -2),
    (0x14_9e1e, 0x3101, 1),
    (0x14_9e58, 0x392c, -44),
    (0x14_9ebc, 0x310f, 15),
    (0x14_9f42, 0x311f, 31),
    (0x14_9f68, 0x311f, 31),
    // PATH B - the outlined else arm, same six elements, same six immediates
    (0x14_a73e, 0x3902, -2),
    (0x14_a76a, 0x3101, 1),
    (0x14_a7a4, 0x392c, -44),
    (0x14_a808, 0x310f, 15),
    (0x14_a862, 0x311f, 31),
    (0x14_a888, 0x311f, 31),
];
pub const TARGET_BOX_SHIFT_MAX: u32 = 211;

fn enc_add_sub_r1(v: i32) -> Option<u16> {
    match v {
        0..=255 => Some(0x3100 | v as u16),
        -255..=-1 => Some(0x3900 | (-v) as u16),
        _ => None,
    }
}

pub fn target_box_x_patches(read: &impl Fn(u32, usize) -> Vec<u8>, shift: u32) -> Vec<(u32, [u8; 2])> {
    let mut out = Vec::new();
    if shift > TARGET_BOX_SHIFT_MAX {
        return out;
    }
    for (addr, orig, base) in TARGET_BOX_X_SITES {
        let Some(cur) = rd_u16(read, addr) else { continue };
        let Some(want) = enc_add_sub_r1(base - shift as i32) else { continue };
        if cur == want {
            continue;
        }
        if cur != orig && (cur & 0xf700) != 0x3100 {
            continue;
        }
        out.push((addr, want.to_le_bytes()));
    }
    out
}

// ── 소켓확장's host mirror is GONE (44차) ─────────────────────────────────────
//
// This file used to carry a hand mirror of `items::socket_expand_write` plus nine tests
// pinning its equip-slot walk. Both are deleted because the function is: the item no
// longer has a host-side effect at all — it is delivered into the network warehouse as
// category 0x10 and resolved by the server's `_orb_combine`, so that the player picks
// the target through the game's own 오브결합 flow. The coverage moved with the logic,
// to `hero3_server/selftest.py` (search 소켓확장 there).
//
// ⚠ A MIRROR THAT OUTLIVES ITS ORIGINAL STAYS GREEN FOREVER. That is why this note is
// here rather than nothing: the mirror is not an `include!`, so deleting the real
// function would not have failed a single test in this file.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    fn reader(m: HashMap<u32,u8>) -> impl Fn(u32,usize)->Vec<u8> {
        move |a,l| { let mut v=Vec::new(); for i in 0..l as u32 { match m.get(&(a+i)){Some(b)=>v.push(*b),None=>return Vec::new()} } v }
    }
    fn pu16(m:&mut HashMap<u32,u8>,a:u32,v:u16){for(i,b)in v.to_le_bytes().into_iter().enumerate(){m.insert(a+i as u32,b);}}
    fn pu32(m:&mut HashMap<u32,u8>,a:u32,v:u32){for(i,b)in v.to_le_bytes().into_iter().enumerate(){m.insert(a+i as u32,b);}}
    const CHAR:u32=0x1b_7b6c; const HANDLE:u32=0x4020_0000; const BLOCK:u32=0x4021_0000;
    fn gbase()->u32{BLOCK+GRID_DATA_ADJ}
    fn seed(m:&mut HashMap<u32,u8>,state:u32){
        pu32(m,CHAR_PTR_GLOBAL,CHAR); pu32(m,CHAR+STATE_OFF,state); m.insert(CHAR+SELECTOR_FLAG_OFF,0);
        m.insert(CHAR+WINDOW_ID_OFF,WIN_NONE); // no window open = the field owns the screen
        m.insert(CHAR+QB_TYPE_OFF,TY_SKILL); m.insert(CHAR+QB_TYPE_OFF+1,TY_SKILL); m.insert(CHAR+QB_TYPE_OFF+2,TY_ITEM);
        m.insert(CHAR+QB_TYPE_OFF+3,0); m.insert(CHAR+QB_TYPE_OFF+4,0);
        m.insert(CHAR+QB_SKILLREF_OFF,0x6a); m.insert(CHAR+QB_SKILLREF_OFF+1,0x2d); m.insert(CHAR+QB_ITEMREF_OFF+2,5);
        pu32(m,CHAR+GRID_HANDLE_OFF,HANDLE); pu32(m,HANDLE,BLOCK);
        pu16(m,gbase()+5*SLOT_STRIDE+COUNT_OFF,13);
    }

    /// The bag read a bound item plate's stack count comes from. (The five-slot scan
    /// that used to be tested here went with the original -- see the note above.)
    #[test] fn the_bound_cells_stack_count_is_read_from_the_bag(){
        let mut m=HashMap::new(); seed(&mut m,STATE_FIELD);
        let r=reader(m);
        let c=resolve_char(&r).unwrap();
        assert_eq!(item_count(&r,c,0,5),13);
        assert_eq!(item_count(&r,c,0,6),0);
    }
    #[test] fn gate(){
        let mut ok=HashMap::new(); seed(&mut ok,STATE_FIELD); assert!(in_field_state(&reader(ok)));
        let mut mn=HashMap::new(); seed(&mut mn,3); assert!(!in_field_state(&reader(mn)));
    }
    #[test] fn no_char(){ let mut m=HashMap::new(); pu32(&mut m,CHAR_PTR_GLOBAL,0x10); assert!(resolve_char(&reader(m)).is_none()); }

    // ── the container convention, both carriers (MIGRATION_LGT.md §2.8) ──────
    // The silent bug this pins: LGT containers are DIRECT pointers, and running the
    // KTF double-dereference on one still "resolves" whenever the payload's first
    // word happens to look like a heap address — onto the wrong bytes. items.rs
    // fixed that by routing every container through deref_container; this copy
    // mirrors it so the shape cannot drift unnoticed.
    #[test] fn container_convention_ktf_hops_and_lgt_does_not(){
        let field=0x4030_0000u32; let payload=0x4031_0000u32;
        let mut m=HashMap::new();
        pu32(&mut m,field,HANDLE); pu32(&mut m,HANDLE,BLOCK);
        // KTF: field -> handle -> block+8.
        assert_eq!(deref_container(&reader(m.clone()),field,Some(GRID_DATA_ADJ)),Some(BLOCK+GRID_DATA_ADJ));
        // LGT: the SAME field value IS the payload — no hop, no +8.
        assert_eq!(deref_container(&reader(m),field,None),Some(HANDLE));
        // The §2.8 failure mode, explicitly: LGT-direct data whose first word is
        // heap-plausible. The direct convention returns the field's value; the KTF
        // convention would happily follow it one hop too far.
        let mut d=HashMap::new();
        pu32(&mut d,field,payload); pu32(&mut d,payload,0x4032_0000);
        assert_eq!(deref_container(&reader(d.clone()),field,None),Some(payload));
        assert_eq!(deref_container(&reader(d),field,Some(GRID_DATA_ADJ)),Some(0x4032_0000+GRID_DATA_ADJ),
            "this resolving is WHY the convention must come from the build table");
        // Both conventions refuse a first word outside the heap window.
        let mut bad=HashMap::new(); pu32(&mut bad,field,0x10);
        assert_eq!(deref_container(&reader(bad.clone()),field,Some(GRID_DATA_ADJ)),None);
        assert_eq!(deref_container(&reader(bad),field,None),None);
        // KTF also refuses when the SECOND hop leaves the heap.
        let mut h2=HashMap::new(); pu32(&mut h2,field,HANDLE); pu32(&mut h2,HANDLE,0x20);
        assert_eq!(deref_container(&reader(h2),field,Some(GRID_DATA_ADJ)),None);
    }

    // ── cutscene vs menu vs field (player question 2026-08-03) ───────────────

    /// The three overlay modes come out of TWO bytes, and every game window that is
    /// not the script window has to read as "menu", not as "cutscene".
    #[test] fn window_id_separates_cutscene_from_every_other_window(){
        let mode = |win: u8, state: u32| {
            let mut m=HashMap::new(); seed(&mut m,state); m.insert(CHAR+WINDOW_ID_OFF,win);
            let r=reader(m);
            if in_field_state(&r) { "field" } else if cutscene_open(&r) { "cutscene" } else { "menu" }
        };
        assert_eq!(mode(WIN_NONE, STATE_FIELD), "field");
        assert_eq!(mode(WIN_SCRIPT, STATE_FIELD), "cutscene", "id 4 = script playback");
        for other in [7u8, 8, 14, 15, 16, 17, 18] {
            assert_eq!(mode(other, STATE_FIELD), "menu", "window {other} must not read as a cutscene");
        }
        // The title/slot shell has no window open but is not the field either.
        assert_eq!(mode(WIN_NONE, 3), "menu");
    }

    /// The quickbar plates stay on screen with a game window open, so they are
    /// scanned for the whole of app state 4 — but NOT on boot or the title shell,
    /// where the slot bytes are heap leftovers.
    #[test] fn quickbar_is_scanned_for_every_in_world_window(){
        for win in [WIN_NONE, WIN_SCRIPT, 8u8, 15] {
            let mut m=HashMap::new(); seed(&mut m,STATE_FIELD); m.insert(CHAR+WINDOW_ID_OFF,win);
            assert!(in_game_state(&reader(m)), "window {win} is still in-world");
        }
        for state in [2u32, 3] {
            let mut m=HashMap::new(); seed(&mut m,state);
            assert!(!in_game_state(&reader(m)), "state {state} is not a live character");
        }
    }

    /// ★★ The 창고/캐시샵 gate (66차-C). The bug this pins: state 3 was read as "out of
    /// world", so every online screen refused touch — the UI's own log said
    /// `REFUSED … shell=true` on a live 싱글 창고 frame. `online_screen` must be true
    /// there and false on the shell, and the ONLY thing that separates them is whether
    /// the record resolves (level >= 1).
    #[test] fn online_screen_is_state_3_with_a_live_record(){
        let mut m=HashMap::new(); seed(&mut m,STATE_ONLINE);
        assert!(online_screen(&reader(m.clone())), "state 3 with a live record IS an online screen");
        assert!(!in_game_state(&reader(m)), "…and it is not the in-world state");
        // ⚠ The OTHER half — "the title/slot shell must stay out" — cannot be asserted
        // here, and that is a property of this mirror, not of the predicate. The shell is
        // kept out by `resolve_char` REFUSING to resolve below level 1, which is the
        // Static-anchor (LGT) build's behaviour; this mirror models the KTF pointer-global
        // resolve, which succeeds for any record the map happens to hold. The real gate is
        // `guest_build::char_anchor` — see `items::online_screen`'s doc and `shell_state`'s
        // "must NOT resolve CHAR through resolve_char".
        let mut dead=HashMap::new(); seed(&mut dead,STATE_ONLINE);
        for off in 0..2u32 { dead.insert(CHAR + LEVEL_OFF + off, 0); }
        assert!(online_screen(&reader(dead)), "the mirror resolves regardless of level (see note)");
        // …and the in-world states are not "online" either.
        for st in [STATE_FIELD, 5u32] {
            let mut m=HashMap::new(); seed(&mut m,st);
            assert!(!online_screen(&reader(m)), "state {st} is in-world, not online");
        }
    }

    // ── inventory consumables + the potion buttons (player request 2026-08-03) ──

    const ITEMDB_HANDLE: u32 = 0x4022_0000;
    const ITEMDB_BLOCK: u32 = 0x4023_0000;

    /// Put a stack of `count` x item `idx` (category `cat`) at (tab, slot).
    fn put_item(m:&mut HashMap<u32,u8>, tab:u8, slot:u8, cat:u16, idx:u16, count:u16){
        let rec = gbase() + u32::from(tab)*TAB_STRIDE + u32::from(slot)*SLOT_STRIDE;
        pu16(m,rec+CAT_OFF,cat); pu16(m,rec+IDX_OFF,idx); pu16(m,rec+COUNT_OFF,count);
    }
    /// Give item-DB index `idx` an icon frame, the way the game's own icon draw
    /// resolves it (grid_base+0x38 -> handle -> block+8, records of 0x18, frame @+0x14).
    fn put_db_icon(m:&mut HashMap<u32,u8>, idx:u16, frame:u8){
        pu32(m,gbase()+ITEMDB_HANDLE_OFF,ITEMDB_HANDLE); pu32(m,ITEMDB_HANDLE,ITEMDB_BLOCK);
        m.insert(ITEMDB_BLOCK+GRID_DATA_ADJ+u32::from(idx)*ITEMDB_STRIDE+ITEMDB_ICON_OFF, frame);
    }

    #[test] fn consumable_scan_keeps_only_real_consumable_stacks(){
        let mut m=HashMap::new(); seed(&mut m,STATE_FIELD);
        put_db_icon(&mut m, 3, 0x21); put_db_icon(&mut m, 9, 0x22); put_db_icon(&mut m, 4, 0x23);
        put_item(&mut m, 0, 2, CONSUMABLE_CAT, 9, 5);   // MP potion
        put_item(&mut m, 1, 0, CONSUMABLE_CAT, 3, 20);  // HP potion
        put_item(&mut m, 1, 7, 0x0f, 4, 2);             // EQUIPMENT — wrong category
        put_item(&mut m, 2, 1, CONSUMABLE_CAT, 3, 0);   // empty stack
        put_item(&mut m, 3, 1, CONSUMABLE_CAT, 4, 5000);// implausible count = heap noise
        let got = scan_consumables(&reader(m), &|_| true);
        assert_eq!(got.len(), 2, "{got:?}");
        // Sorted by item index, so the picker's grid does not reshuffle between openings.
        assert_eq!((got[0].idx, got[0].count, got[0].icon), (3, 20, 0x21));
        assert_eq!((got[1].idx, got[1].count, got[1].tab, got[1].slot), (9, 5, 0, 2));
    }

    /// An item the ATLAS cannot draw is dropped: it is the last filter standing
    /// between a stray heap word and a phantom entry in the picker.
    #[test] fn consumable_scan_drops_items_with_no_atlas_icon(){
        let mut m=HashMap::new(); seed(&mut m,STATE_FIELD);
        put_db_icon(&mut m, 3, 0x21); put_db_icon(&mut m, 9, 0xfe);
        put_item(&mut m, 0, 0, CONSUMABLE_CAT, 3, 1);
        put_item(&mut m, 0, 1, CONSUMABLE_CAT, 9, 1);
        let got = scan_consumables(&reader(m), &|f| f < 0x80);
        assert_eq!(got.iter().map(|e| e.idx).collect::<Vec<_>>(), vec![3]);
    }

    /// The BIGGEST stack of a type wins, so the button's count is not whichever
    /// leftover single the sweep happened to reach first.
    #[test] fn duplicate_stacks_collapse_to_the_largest(){
        let mut m=HashMap::new(); seed(&mut m,STATE_FIELD);
        put_db_icon(&mut m, 3, 0x21);
        put_item(&mut m, 0, 0, CONSUMABLE_CAT, 3, 4);
        put_item(&mut m, 0, 5, CONSUMABLE_CAT, 3, 30);
        let r=reader(m);
        let got = scan_consumables(&r, &|_| true);
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].count, got[0].slot), (30, 5));
        // …but the total carried is what the button prints.
        assert_eq!(consumable_count(&r, 3), 34);
    }

    /// The binding stores an item INDEX, so the (tab, slot) has to be resolved fresh
    /// at press time — a stack that moved must still be found, and one that ran out
    /// must resolve to nothing rather than to whatever slid into its cell.
    #[test] fn use_target_is_resolved_from_the_item_index(){
        let mut m=HashMap::new(); seed(&mut m,STATE_FIELD);
        put_db_icon(&mut m, 3, 0x21);
        put_item(&mut m, 2, 11, CONSUMABLE_CAT, 3, 7);
        assert_eq!(find_consumable(&reader(m.clone()), 3), Some((2, 11)));
        assert_eq!(find_consumable(&reader(m.clone()), 9), None, "an item we do not carry");
        put_item(&mut m, 2, 11, CONSUMABLE_CAT, 3, 0);
        assert_eq!(find_consumable(&reader(m), 3), None, "an empty stack is not a target");
    }

    /// The Thumb bit matters: `call_guest_function` branches to this address, and a
    /// cleared low bit would enter ARM mode on a Thumb routine.
    #[test] fn use_item_effect_address_carries_the_thumb_bit(){
        assert_eq!(USE_ITEM_EFFECT, 0x0015_3e09);
    }

    // ── skill_icon: the game's own 2-way on-field-hero pick + strip cell ──────
    #[test] fn skill_icon_ground_truth(){
        // SLOT3 save: ref 0x6a cell 5 -> hero1/s8 = 일심; ref 0x2d cell 5 -> hero0/s7 = 장탄.
        // (0x6a >> 6 = 1 and 0x2d >> 6 = 0 -- the `cat` the plate stores.)
        let mut m=HashMap::new();
        m.insert(CHAR+SKILL_SHEET_OFF,7);    m.insert(CHAR+SKILL_SHEET_OFF+1,8);
        let r=reader(m);
        assert_eq!(skill_icon_of(&r,CHAR,1,5),Some((4,5)));
        assert_eq!(skill_icon_of(&r,CHAR,0,5),Some((3,5)));
    }
    #[test] fn skill_icon_bad_sheet(){
        let mut m=HashMap::new();
        m.insert(CHAR+SKILL_SHEET_OFF+1,0);
        assert_eq!(skill_icon_of(&reader(m),CHAR,1,3),None);
    }
    #[test] fn skill_icon_clamps_to_two_sheets(){
        // cat 3 still picks the SECOND loaded sheet (CHAR+0xd2), never 0xd4.
        let mut m=HashMap::new();
        m.insert(CHAR+SKILL_SHEET_OFF,7); m.insert(CHAR+SKILL_SHEET_OFF+1,8); m.insert(CHAR+SKILL_SHEET_OFF+3,10);
        assert_eq!(skill_icon_of(&reader(m),CHAR,3,2),Some((4,2)));
    }

    // ---- the "quickbar register" popup rewrite (player request 2026-08-03) ----

    const PH_ARRAY: u32 = 0x4030_0000;   // popup_handle[8]
    const PH_HANDLE0: u32 = 0x4031_0000; // handle[i] -> block
    const PH_BLOCK0: u32 = 0x4032_0000;  // block; payload = block+8

    fn label_addr(i: u32) -> u32 { PH_BLOCK0 + i * 0x100 + 8 }

    // EUC-KR labels the game actually builds.
    const L_USE: &[u8]    = &[0xbb,0xe7,0xbf,0xeb,0xc7,0xcf,0xb1,0xe2,0x00]; // 사용하기
    const L_KEY1: &[u8]   = &[0x31,0xc5,0xb0,0xb5,0xee,0xb7,0xcf,0x00];      // 1키등록
    const L_DETAIL: &[u8] = &[0xbb,0xf3,0xbc,0xbc,0xba,0xb8,0xb1,0xe2,0x00]; // 상세보기
    const L_DROP: &[u8]   = &[0xb9,0xf6,0xb8,0xae,0xb1,0xe2,0x00];           // 버리기
    const L_UNREG: &[u8]  = &[0xb5,0xee,0xb7,0xcf,0xc3,0xeb,0xbc,0xd2,0x00]; // 등록취소

    /// Seed a live popup of `tag` on page `page` with `rows` native rows.
    fn seed_popup(m: &mut HashMap<u32,u8>, page: u8, tag: u8, rows: i8) {
        pu32(m, CHAR_PTR_GLOBAL, CHAR);
        m.insert(CHAR + SELECTOR_FLAG_OFF, 1);      // a menu overlay owns the screen
        m.insert(CHAR + MENU_PAGE_OFF, page);
        m.insert(CHAR + POPUP_OPEN_OFF, 1);
        m.insert(CHAR + POPUP_TAG_OFF, tag);
        m.insert(CHAR + POPUP_COUNT_OFF, rows as u8);
        m.insert(CHAR + POPUP_SEL_OFF, 0);
        m.insert(CHAR + POPUP_PREEMPT_A_OFF, 0);
        m.insert(CHAR + POPUP_PREEMPT_B_OFF, 0);
        // the bag (items only, but harmless for skills)
        pu32(m, CHAR + GRID_HANDLE_OFF, HANDLE); pu32(m, HANDLE, BLOCK);
        // skill cursor + "learned" table
        m.insert(CHAR + SKILL_CAT_OFF, 0);
        m.insert(CHAR + SKILL_CURSOR_BASE, 3);
        m.insert(CHAR + SKILL_LEARNED_BASE + 3*8, 1);
        // popup_handle[8] -> handle -> block ; payloads 0x100 apart
        pu32(m, POPUP_HANDLES_SLOT, PH_ARRAY);
        for i in 0..8u32 {
            pu32(m, PH_ARRAY + i*4, PH_HANDLE0 + i*0x10);
            pu32(m, PH_HANDLE0 + i*0x10, PH_BLOCK0 + i*0x100);
        }
        let native: [&[u8]; 8] = [L_USE, L_KEY1, L_KEY1, L_KEY1, L_KEY1, L_KEY1, L_DETAIL, L_DROP];
        for (i, lbl) in native.iter().enumerate() {
            // The whole 10-byte allocation, zero-padded: `reader` is all-or-nothing, and
            // the real code reads the full buffer to find the terminator.
            for k in 0..POPUP_LABEL_CAP as u32 {
                m.insert(label_addr(i as u32) + k, lbl.get(k as usize).copied().unwrap_or(0));
            }
        }
    }

    /// Apply a (addr, bytes) write list to the fake memory. Named apart from the
    /// older `apply` helper below, which counts target-box patches.
    fn apply_writes(m: &mut HashMap<u32,u8>, writes: &[(u32, Vec<u8>)]) {
        for (a, bytes) in writes { for (k,b) in bytes.iter().enumerate() { m.insert(a + k as u32, *b); } }
    }

    #[test] fn popup_gate_accepts_only_the_two_register_flavours(){
        let mut m=HashMap::new(); seed_popup(&mut m, PAGE_ITEM, TAG_ITEM, ROWS_ITEM);
        let v = popup_view(&reader(m)).expect("item popup");
        assert!(v.item); assert_eq!(v.kind, PAGE_ITEM); assert_eq!(v.rows, OUR_ROWS_ITEM);

        let mut m=HashMap::new(); seed_popup(&mut m, PAGE_SKILL, TAG_SKILL, ROWS_SKILL);
        let v = popup_view(&reader(m)).expect("skill popup");
        assert!(!v.item); assert_eq!(v.rows, OUR_ROWS_SKILL); assert!(v.can_bind);

        // ...and everything else is refused. 0x64/0x65/0x68/0x69 have no register rows,
        // and a tag on the WRONG page is stale state (0x9c87 is never cleared).
        for (page, tag) in [(PAGE_ITEM,0x64u8),(PAGE_ITEM,0x69),(PAGE_SKILL,0x68),(PAGE_ITEM,TAG_SKILL),(PAGE_SKILL,TAG_ITEM),(3,TAG_ITEM)] {
            let mut m=HashMap::new(); seed_popup(&mut m, page, tag, ROWS_ITEM);
            assert!(popup_view(&reader(m)).is_none(), "page {page} tag {tag:#x} must be refused");
        }
    }

    /// The count check exists because `CHAR+0x9c87` is NEVER cleared: `free_popup`
    /// zeroes only the count. Without it a closed popup's stale tag passes the gate.
    #[test] fn a_closed_popup_with_a_stale_tag_is_refused(){
        let mut m=HashMap::new(); seed_popup(&mut m, PAGE_ITEM, TAG_ITEM, ROWS_ITEM);
        m.insert(CHAR + POPUP_COUNT_OFF, 0);            // free_popup ran
        assert!(popup_view(&reader(m.clone())).is_none(), "count 0 with a stale tag");
        m.insert(CHAR + POPUP_COUNT_OFF, ROWS_ITEM as u8);
        m.insert(CHAR + POPUP_OPEN_OFF, 0);             // and the open flag down
        assert!(popup_view(&reader(m)).is_none(), "closed popup");
    }

    /// The label check is the one clause no combination of stale bytes can fake.
    #[test] fn the_gate_needs_row_one_to_really_say_1_key_register(){
        let mut m=HashMap::new(); seed_popup(&mut m, PAGE_ITEM, TAG_ITEM, ROWS_ITEM);
        for k in 0..8u32 { m.insert(label_addr(1)+k, 0x41); } // "AAAAAAAA"
        assert!(popup_view(&reader(m)).is_none(), "row 1 is not a register row");
    }

    /// The two page pre-emptors (guest 0x13aa24 / 0x13aa56): with either set the page
    /// never routes a key into popup_key, so the popup is not really live. KTF only —
    /// its commit BYPASSES that router, so the host must apply the refusal itself.
    #[test] fn inventory_page_preemptors_refuse_the_popup(){
        for off in [POPUP_PREEMPT_A_OFF, POPUP_PREEMPT_B_OFF] {
            let mut m=HashMap::new(); seed_popup(&mut m, PAGE_ITEM, TAG_ITEM, ROWS_ITEM);
            m.insert(CHAR + off, 1);
            assert!(popup_view(&reader(m)).is_none(), "pre-emptor {off:#x}");
        }
    }

    /// 51차, the LGT shape: an EMPTY pre-emptor list must admit the item popup even
    /// with the KTF offsets non-zero. On LGT those bytes are unrelated fields of a
    /// different compile, and checking them is exactly what kept every consumable on
    /// the native 1키등록 list on the device (skills, which have no pre-emptor gate,
    /// relabelled fine — the asymmetry that named the bug). The skill path must be
    /// unaffected by the list either way.
    #[test] fn an_empty_preempt_list_ignores_the_ktf_bytes(){
        let mut m=HashMap::new(); seed_popup(&mut m, PAGE_ITEM, TAG_ITEM, ROWS_ITEM);
        m.insert(CHAR + POPUP_PREEMPT_A_OFF, 0x5a);
        m.insert(CHAR + POPUP_PREEMPT_B_OFF, 0xa5);
        let v = popup_view_on(&reader(m.clone()), &[]).expect("LGT: stray bytes must not gate the item popup");
        assert!(v.item);
        // ...while the same memory is refused under the KTF list — the two builds
        // genuinely disagree here, which is why the list is per-build at all.
        assert!(popup_view(&reader(m.clone())).is_none(), "KTF still refuses");
        // The skill popup never consulted the list on either build.
        let mut m=HashMap::new(); seed_popup(&mut m, PAGE_SKILL, TAG_SKILL, ROWS_SKILL);
        m.insert(CHAR + POPUP_PREEMPT_A_OFF, 0x5a);
        assert!(popup_view(&reader(m)).is_some(), "skills skip the pre-emptor gate");
    }

    /// An UNLEARNED skill must not be offered: `skill_bind`'s guard at guest 0x145cbe
    /// routes it to the UNBIND path, so "register" would silently clear the slot.
    #[test] fn an_unlearned_skill_cannot_be_bound(){
        let mut m=HashMap::new(); seed_popup(&mut m, PAGE_SKILL, TAG_SKILL, ROWS_SKILL);
        m.insert(CHAR + SKILL_LEARNED_BASE + 3*8, 0);
        assert!(!popup_view(&reader(m)).unwrap().can_bind);
    }

    /// The rewrite: row 1 becomes the quickbar row, the native tail slides up, the
    /// count shrinks -- and re-running it is a NO-OP (it runs every frame).
    #[test] fn relabel_is_correct_and_idempotent(){
        let mut m=HashMap::new(); seed_popup(&mut m, PAGE_ITEM, TAG_ITEM, ROWS_ITEM);
        let v = popup_view(&reader(m.clone())).unwrap();
        let w = popup_relabel(&reader(m.clone()), &v);
        assert!(!w.is_empty());
        apply_writes(&mut m, &w);
        let r = reader(m.clone());
        assert_eq!(r(label_addr(1), 9), LABEL_QUICKBAR.to_vec(), "row 1 = quickbar register");
        assert_eq!(r(label_addr(2), L_DETAIL.len()), L_DETAIL.to_vec(), "row 2 = details");
        assert_eq!(r(label_addr(3), L_DROP.len()), L_DROP.to_vec(), "row 3 = drop");
        assert_eq!(rd_u8(&r, CHAR + POPUP_COUNT_OFF), Some(OUR_ROWS_ITEM as u8));
        // idempotent, and the gate still passes on OUR list
        let v2 = popup_view(&r).expect("gate must accept our own rewritten list");
        assert!(popup_relabel(&r, &v2).is_empty(), "second pass must write nothing");
    }

    #[test] fn skill_relabel_keeps_unregister_as_row_two(){
        let mut m=HashMap::new(); seed_popup(&mut m, PAGE_SKILL, TAG_SKILL, ROWS_SKILL);
        for (k,b) in L_UNREG.iter().enumerate() { m.insert(label_addr(6)+k as u32, *b); }
        let v = popup_view(&reader(m.clone())).unwrap();
        let w = popup_relabel(&reader(m.clone()), &v);
        apply_writes(&mut m, &w);
        let r = reader(m);
        assert_eq!(r(label_addr(1), 9), LABEL_QUICKBAR.to_vec());
        assert_eq!(r(label_addr(2), L_UNREG.len()), L_UNREG.to_vec(), "unregister stays reachable");
        assert_eq!(rd_u8(&r, CHAR + POPUP_COUNT_OFF), Some(OUR_ROWS_SKILL as u8));
    }

    /// THE mis-fire guard. `popup_choose` dispatches on the RAW row index and the
    /// skill path has no bounds check at all, so the display->guest map must not
    /// drift: display 1 = the chosen plate, and the tail rows must land on the guest's
    /// ORIGINAL indices (6 / 7), not on their new display slots.
    #[test] fn display_rows_map_back_to_the_guest_rows(){
        let item = PopupView { kind: PAGE_ITEM, item: true, sel: 0, rows: OUR_ROWS_ITEM, can_bind: true };
        let skill = PopupView { kind: PAGE_SKILL, item: false, sel: 0, rows: OUR_ROWS_SKILL, can_bind: true };
        for slot in 0..5u8 {
            assert_eq!(popup_guest_row(&item, 1, slot), Some(slot + 1));
            assert_eq!(popup_guest_row(&skill, 1, slot), Some(slot + 1));
        }
        // an out-of-range plate is REFUSED, not clamped -- the skill dispatcher would
        // write one byte past every 5-entry quickbar array.
        for slot in [5u8, 6, 9, 255] {
            assert_eq!(popup_guest_row(&item, 1, slot), None, "plate {slot}");
            assert_eq!(popup_guest_row(&skill, 1, slot), None, "plate {slot}");
        }
        assert_eq!(popup_guest_row(&item, 2, 0), Some(6));   // details
        assert_eq!(popup_guest_row(&item, 3, 0), Some(7));   // drop
        assert_eq!(popup_guest_row(&skill, 2, 0), Some(6));  // unregister
        assert_eq!(popup_guest_row(&skill, 3, 0), None, "skills have no 4th row");
        // display 0 is passed through, never remapped
        assert_eq!(popup_guest_row(&item, 0, 0), None);
        assert_eq!(popup_guest_row(&skill, 0, 0), None);
    }

    #[test] fn popup_key_address_and_select_key(){
        assert_eq!(POPUP_KEY_FN, 0x0013_a86d, "Thumb bit set");
        assert_eq!(POPUP_KEY_SELECT as i32, -5, "the popup confirm arm @0x13a8be");
    }

    /// The label must fit the guest's 10-byte allocation (guest 0x12ce54 movs r0,#0xa).
    #[test] fn the_quickbar_label_fits_the_guest_buffer(){
        assert!(LABEL_QUICKBAR.len() <= POPUP_LABEL_CAP, "{} > {POPUP_LABEL_CAP}", LABEL_QUICKBAR.len());
        assert_eq!(*LABEL_QUICKBAR.last().unwrap(), 0, "NUL terminated");
        // the SPACED form would be 11 bytes and overrun -- that is why it is not used
        assert_eq!(LABEL_QUICKBAR.len() + 1, 10);
    }

    // ── gauges: the HP/SP swap that made the red bar ignore potions ──────────

    /// Seed a level-12 hero the way the engine would: maxHP from ComputeMaxHP,
    /// current HP at CHAR+0xf2, SP (0..1000) at CHAR+0xf8.
    fn seed_stats(m:&mut HashMap<u32,u8>, hp:i16, sp:u32){
        let max = compute_max_hp(12, 20, 0); // (36*12 + 10*20) * 100/100 = 632
        pu32(m,CHAR_PTR_GLOBAL,CHAR);
        pu16(m,CHAR+LEVEL_OFF,12);
        pu32(m,CHAR+EXP_OFF,1234); pu32(m,CHAR+EXP_NEXT_OFF,15000);
        pu32(m,CHAR+HP_MAX_OFF,max as u32);
        pu16(m,CHAR+HP_OFF,hp as u16);
        pu32(m,CHAR+SP_OFF,sp);
    }

    #[test] fn gauges_read_hp_from_0xf2_over_maxhp_at_0xec(){
        let mut m=HashMap::new();
        seed_stats(&mut m, 316, 400);
        let s=read_stats(&reader(m)).expect("plausible");
        assert_eq!((s.hp,s.hp_max),(316,632),"HP is CHAR+0xf2 over CHAR+0xec");
        assert_eq!((s.sp,s.sp_max),(400,1000),"SP is CHAR+0xf8 over the constant 1000");
        assert_eq!(s.level,12);
    }

    /// The reported bug, as a regression: a full-HP hero must read FULL. Under the
    /// old mapping the "HP" bar was SP/1000 and sat at 40 % no matter what.
    #[test] fn full_hp_reads_full_and_a_heal_moves_it(){
        let max = compute_max_hp(12,20,0) as i16;
        let mut m=HashMap::new(); seed_stats(&mut m, max, 400);
        let full=read_stats(&reader(m)).expect("plausible");
        assert_eq!(full.hp, full.hp_max, "full HP => full bar");

        // Take half the pool off, then heal it back (AddHP clamps at maxHP).
        let mut m2=HashMap::new(); seed_stats(&mut m2, max/2, 400);
        let hurt=read_stats(&reader(m2)).expect("plausible");
        assert!(hurt.hp * 2 <= hurt.hp_max + 1 && hurt.hp * 2 >= hurt.hp_max - 1, "half: {hurt:?}");
        let mut m3=HashMap::new(); seed_stats(&mut m3, max, 400);
        assert_eq!(read_stats(&reader(m3)).unwrap().hp, max as u16, "healed back to full");
        // SP is a separate pool and must NOT be what the HP bar shows.
        let mut m4=HashMap::new(); seed_stats(&mut m4, max, 1000);
        assert_eq!(read_stats(&reader(m4)).unwrap().hp, max as u16, "SP full does not fill HP");
    }

    #[test] fn dead_hero_reads_empty_not_garbage(){
        // The death gates test (s16)CHAR[0xf2] <= 0, so HP can be negative.
        let mut m=HashMap::new(); seed_stats(&mut m, -40, 0);
        let s=read_stats(&reader(m)).expect("plausible");
        assert_eq!(s.hp,0,"negative HP clamps to an empty bar, not 65496");
    }

    /// ⛔⛔ **TWO OF THIS TEST'S ASSERTIONS WERE GUARDING THE BUG** (80차-E) — the same
    /// trap 71차-C wrote down: a hand-written mirror pins whatever the source said,
    /// including a wrong premise. `hp_max == 0` and `hp > hp_max` are BOTH states a live
    /// character legitimately reaches (the network warehouse, where `ComputeMaxHP` has
    /// not run; and unequipping, which drops max HP below current HP), and rejecting
    /// them froze the published gauges at level 0 — see
    /// `the_warehouses_derived_max_hp_is_not_evidence_of_a_dead_record`.
    ///
    /// What is left is what the rule can honestly test: `level` witnesses liveness, and
    /// the derived fields get ceilings.
    #[test] fn implausible_reading_is_rejected_so_the_caller_keeps_the_last_good(){
        // Level 0 = not a live character (boot, torn-down pointer, uncommitted class).
        let mut m=HashMap::new(); seed_stats(&mut m, 100, 0); pu16(&mut m,CHAR+LEVEL_OFF,0);
        assert!(read_stats(&reader(m)).is_none());
        // Level cap is LEVEL_CAP (guest 0x146c3a = 70; LGT patched to 99 in 111차).
        let mut m=HashMap::new(); seed_stats(&mut m, 100, 0); pu16(&mut m,CHAR+LEVEL_OFF,LEVEL_CAP+1);
        assert!(read_stats(&reader(m)).is_none());
        // …and the cap itself is a live level (the whole point of 111차).
        let mut m=HashMap::new(); seed_stats(&mut m, 100, 0); pu16(&mut m,CHAR+LEVEL_OFF,LEVEL_CAP);
        assert!(read_stats(&reader(m)).is_some());
        // A max HP past the sanity ceiling means we are reading the wrong bytes.
        let mut m=HashMap::new(); seed_stats(&mut m, 100, 0); pu32(&mut m,CHAR+HP_MAX_OFF,u32::from(HP_CAP)+1);
        assert!(read_stats(&reader(m)).is_none());
        // ★ No SP case here: `read_stats` CLAMPS sp with `.min(SP_MAX)` before the gate,
        // so an over-range SP is unobservable in `Stats`. `char_live` reads it raw and
        // `a_zeroed_record_is_still_not_a_character` covers that side.
    }

    #[test] fn compute_max_hp_matches_the_guest_formula(){
        assert_eq!(compute_max_hp(1,10,0), 136);
        assert_eq!(compute_max_hp(12,20,0), 632);
        assert_eq!(compute_max_hp(12,20,25), 790); // +25 % equip bonus at CHAR+0xf0
    }

    // ── window id: the field gate the overlay switches on ────────────────────

    #[test] fn field_gate_needs_no_window_open(){
        // A menu/map window leaves CHAR+0x7c at 4, so the old gate said "field"
        // and the HUD drew over the window (and its gauges read as 0).
        let mut m=HashMap::new(); seed(&mut m,STATE_FIELD); m.insert(CHAR+WINDOW_ID_OFF,WIN_NONE);
        assert!(in_field_state(&reader(m)));

        let mut m=HashMap::new(); seed(&mut m,STATE_FIELD); m.insert(CHAR+WINDOW_ID_OFF,WIN_MAP);
        assert!(!in_field_state(&reader(m)),"map window is not the field");
        assert_eq!(window_id(&reader({let mut m=HashMap::new(); seed(&mut m,STATE_FIELD); m.insert(CHAR+WINDOW_ID_OFF,WIN_MAP); m})),Some(WIN_MAP));

        for win in [4u8,7,14,15,16,17,18] {
            let mut m=HashMap::new(); seed(&mut m,STATE_FIELD); m.insert(CHAR+WINDOW_ID_OFF,win);
            assert!(!in_field_state(&reader(m)),"window {win} is not the field");
        }
        // Selector overlay (state 5) is not the field either.
        let mut m=HashMap::new(); seed(&mut m,5); m.insert(CHAR+WINDOW_ID_OFF,WIN_NONE);
        assert!(!in_field_state(&reader(m)));
    }

    // ── auto-loot ────────────────────────────────────────────────────────────

    #[test] fn autoloot_is_forced_only_while_off(){
        let mut m=HashMap::new(); seed(&mut m,STATE_FIELD); m.insert(CHAR+AUTOLOOT_OFF,0);
        assert_eq!(autoloot_to_force(&reader(m)), Some(CHAR+AUTOLOOT_OFF), "off -> host writes it");
        let mut m=HashMap::new(); seed(&mut m,STATE_FIELD); m.insert(CHAR+AUTOLOOT_OFF,1);
        assert_eq!(autoloot_to_force(&reader(m)), None, "already on -> no write");
        // Any non-zero counts as on (the game tests `cmp #0`, not a bit).
        let mut m=HashMap::new(); seed(&mut m,STATE_FIELD); m.insert(CHAR+AUTOLOOT_OFF,0x5a);
        assert_eq!(autoloot_to_force(&reader(m)), None);
        // No CHAR yet (boot) -> nothing to do.
        let mut m=HashMap::new(); pu32(&mut m,CHAR_PTR_GLOBAL,0x10);
        assert_eq!(autoloot_to_force(&reader(m)), None);
    }

    // ── equipped secondary weapon icon (key-'7' button) ──────────────────────

    /// Seed equip slot 6 -> inventory (tab,slot) -> DB index -> icon frame.
    fn seed_secondary(m:&mut HashMap<u32,u8>, present:i8, tab:i8, slot:i8, idx:u16, frame:u8){
        seed_secondary_cat(m, present, tab, slot, idx, frame, 0x12)
    }

    #[allow(clippy::too_many_arguments)]
    fn seed_secondary_cat(m:&mut HashMap<u32,u8>, present:i8, tab:i8, slot:i8, idx:u16, frame:u8, cat:u16){
        pu32(m,CHAR_PTR_GLOBAL,CHAR);
        m.insert(CHAR+EQUIP6_PRESENT_OFF, present as u8);
        m.insert(CHAR+EQUIP6_TAB_OFF, tab as u8);
        m.insert(CHAR+EQUIP6_SLOT_OFF, slot as u8);
        pu32(m,CHAR+GRID_HANDLE_OFF,HANDLE); pu32(m,HANDLE,BLOCK);
        let rec = gbase() + (tab.max(0) as u32)*TAB_STRIDE + (slot.max(0) as u32)*SLOT_STRIDE;
        pu16(m, rec+0x3b8, idx);
        pu16(m, rec+0x3b6, cat);
        // item DB: *(*(grid+0x38))+8 + idx*0x18 + 0x14
        pu32(m, gbase()+ITEMDB_HANDLE_OFF, HANDLE+0x100); pu32(m, HANDLE+0x100, BLOCK+0x1000);
        m.insert(BLOCK+0x1000+GRID_DATA_ADJ + u32::from(idx)*ITEMDB_STRIDE + ITEMDB_ICON_OFF, frame);
    }

    #[test] fn secondary_icon_resolves_through_the_item_db(){
        let mut m=HashMap::new(); seed_secondary(&mut m, 3, 1, 4, 0x12, 7);
        assert_eq!(secondary_weapon_icon(&reader(m)), Some(7));
    }

    #[test] fn secondary_icon_none_when_nothing_equipped(){
        // present byte = -1 is the game's own "no secondary" test (dim the button).
        let mut m=HashMap::new(); seed_secondary(&mut m, -1, 1, 4, 0x12, 7);
        assert_eq!(secondary_weapon_icon(&reader(m)), None);
        // negative tab/slot -> reject rather than index backwards
        let mut m=HashMap::new(); seed_secondary(&mut m, 3, -1, 4, 0x12, 7);
        assert_eq!(secondary_weapon_icon(&reader(m)), None);
    }

    /// The reported bug: a WEAPON must not be looked up in the consumable DB — its
    /// icon frame is its category. (The old code returned whatever byte happened to
    /// sit at itemDB[weapon_idx], i.e. a random icon.)
    #[test] fn secondary_icon_of_a_weapon_is_its_category(){
        let mut m=HashMap::new();
        // cat 0x0f (equipment), DB index 0x21 -> frame 0x0f, NOT itemDB[0x21]
        seed_secondary_cat(&mut m, 3, 1, 4, 0x21, 99, 0x0f);
        assert_eq!(secondary_weapon_icon(&reader(m)), Some(0x0f));
        // the game's one special case
        let mut m=HashMap::new();
        seed_secondary_cat(&mut m, 3, 0, 0, 3, 99, 0x0e);
        assert_eq!(secondary_weapon_icon(&reader(m)), Some(0x1c));
        // …and a consumable still goes through the DB
        let mut m=HashMap::new();
        seed_secondary_cat(&mut m, 3, 1, 4, 0x12, 7, 0x12);
        assert_eq!(secondary_weapon_icon(&reader(m)), Some(7));
    }

    #[test] fn secondary_icon_rejects_out_of_range_frame(){
        // item_bm has 29 frames; a bogus record must not blit garbage.
        let mut m=HashMap::new(); seed_secondary(&mut m, 3, 0, 0, 5, 200);
        assert_eq!(secondary_weapon_icon(&reader(m)), None);
    }

    #[test] fn skill_icon_cell_out_of_strip(){
        let mut m=HashMap::new();
        m.insert(CHAR+SKILL_SHEET_OFF,7);
        assert_eq!(skill_icon_of(&reader(m),CHAR,0,9),None);
    }

    // ── target-monster box shift ────────────────────────────────────────────
    fn pristine_box() -> HashMap<u32, u8> {
        let mut m = HashMap::new();
        for (a, hw, _) in TARGET_BOX_X_SITES {
            m.insert(a, (hw & 0xff) as u8);
            m.insert(a + 1, (hw >> 8) as u8);
        }
        m
    }
    fn hw_at(m: &HashMap<u32, u8>, a: u32) -> u16 {
        u16::from_le_bytes([m[&a], m[&(a + 1)]])
    }
    fn apply(m: &mut HashMap<u32, u8>, shift: u32) -> usize {
        let ps = target_box_x_patches(&reader(m.clone()), shift);
        let n = ps.len();
        for (a, b) in ps {
            m.insert(a, b[0]);
            m.insert(a + 1, b[1]);
        }
        n
    }

    /// All SIX sites move, and each keeps its ORIGINAL offset relative to the
    /// others — that relative geometry is the box's internal layout, and getting
    /// it wrong is exactly how the buff band tore apart.
    #[test] fn box_shift_moves_every_site_by_the_same_amount(){
        for shift in [1u32, 2, 15, 16, 31, 32, 57, 64, 120, TARGET_BOX_SHIFT_MAX] {
            let mut m = pristine_box();
            assert_eq!(apply(&mut m, shift), 12, "shift {shift} did not touch all twelve");
            for (a, orig, base) in TARGET_BOX_X_SITES {
                let hw = hw_at(&m, a);
                let v = if hw & 0x0800 != 0 { -((hw & 0xff) as i32) } else { (hw & 0xff) as i32 };
                assert_eq!(v, base - shift as i32, "site {a:#x} (was {orig:#06x}) wrong at shift {shift}");
                assert_eq!(hw & 0xf700, 0x3100, "site {a:#x} stopped being an add/sub-imm on r1");
            }
        }
    }

    /// Re-applying is a no-op: the poke pass runs every 30 frames forever.
    #[test] fn box_shift_is_idempotent(){
        let mut m = pristine_box();
        assert_eq!(apply(&mut m, 57), 12);
        assert_eq!(apply(&mut m, 57), 0, "second pass must find nothing to write");
        // …and it can still be re-aimed (window resize / rotation).
        assert_eq!(apply(&mut m, 20), 12);
        assert_eq!(apply(&mut m, 20), 0);
        // shift 0 restores the pristine instructions exactly.
        assert_eq!(apply(&mut m, 0), 12);
        for (a, orig, _) in TARGET_BOX_X_SITES {
            assert_eq!(hw_at(&m, a), orig, "shift 0 must restore {a:#x}");
        }
    }

    /// A different build (or a site we do not recognise) is left completely alone.
    #[test] fn box_shift_refuses_unknown_instructions(){
        let mut m = pristine_box();
        m.insert(TARGET_BOX_X_SITES[2].0, 0x00);
        m.insert(TARGET_BOX_X_SITES[2].0 + 1, 0x48); // ldr r0,[pc,#0] — not ours
        let ps = target_box_x_patches(&reader(m), 57);
        assert_eq!(ps.len(), 11, "the unknown site must be skipped, the rest still move");
        assert!(!ps.iter().any(|(a, _)| *a == TARGET_BOX_X_SITES[2].0));
    }

    /// Above the encodable range nothing is written at all (never a half-move).
    #[test] fn box_shift_beyond_the_imm8_range_is_refused(){
        let m = pristine_box();
        assert!(target_box_x_patches(&reader(m), TARGET_BOX_SHIFT_MAX + 1).is_empty());
    }

    /// The binding site is `subs r1,#44`: 44 + max == 255.
    #[test] fn box_shift_max_is_the_binding_sites_headroom(){
        assert_eq!(TARGET_BOX_SHIFT_MAX, 255 - 44);
        assert!(enc_add_sub_r1(-44 - TARGET_BOX_SHIFT_MAX as i32).is_some());
        assert!(enc_add_sub_r1(-44 - (TARGET_BOX_SHIFT_MAX as i32 + 1)).is_none());
    }

    /// The box renderer exists TWICE (CHAR[0x1ea] picks the copy). The two lists
    /// must be identical element-for-element, or one target kind draws a torn box.
    #[test] fn both_copies_of_the_box_have_the_same_shape(){
        let (a, b) = TARGET_BOX_X_SITES.split_at(6);
        assert_eq!(a.len(), b.len());
        for (i, ((_, ha, ba), (_, hb, bb))) in a.iter().zip(b).enumerate() {
            assert_eq!(ha, hb, "element {i}: the two copies disagree on the encoding");
            assert_eq!(ba, bb, "element {i}: the two copies disagree on the offset");
        }
        // …and path B really is a separate address range, not a duplicate entry.
        for (aa, _, _) in a {
            assert!(!b.iter().any(|(ab, _, _)| ab == aa), "{aa:#x} listed twice");
        }
    }

    // ── cash-shop purchase quantity (60차, reworked 2026-08-21) ────────────────
    //
    // ⚠ MIRROR, like the rest of this file: these two functions are copied from
    // `items.rs`, not `include!`d. If you delete them there, delete them here.
    // They are worth mirroring anyway because both encode a rule whose violation is
    // silent — the player pays and receives the wrong number of items.

    const CASH_ROWS_MAX: u32 = 16;

    fn cash_qty_allowed(cat: u8, idx: u8) -> bool {
        cat == 0x12 && matches!(idx, 0x07 | 0x0a | 0x0b | 0x0c | 0x15 | 0x19 | 0x1a)
    }

    fn cash_qty_row_write(cat: u8, idx: u8) -> bool {
        cash_qty_allowed(cat, idx) && !matches!(idx, 0x19 | 0x1a)
    }

    /// The picker is offered for the SELECTED row only, and fails closed.
    fn cash_qty_offerable(cursor: Option<(u8, u8)>) -> bool {
        matches!(cursor, Some((cat, idx)) if cash_qty_allowed(cat, idx))
    }

    fn cash_row_from_cursor(sel: i8, table_idx: i8, n: u32) -> Option<u32> {
        if sel < 0 || i64::from(sel) >= i64::from(CASH_ROWS_MAX) {
            return None;
        }
        let idx = u32::try_from(table_idx).ok()?;
        (idx < n.min(CASH_ROWS_MAX)).then_some(idx)
    }

    /// ★★ THE REGRESSION TEST. The first version of this feature normalised every
    /// INELIGIBLE row to `1`, and one shipped catalog row is legitimately a stack:
    /// cat `0x0e` idx `0x03` (제련석) carries `+10 = 99` and sits on page 0 with all
    /// seven eligible rows. So opening the confirm box on page 0 rewrote it to 1 and —
    /// because the grant reads that same byte at `0x13e4c` — the player paid for 99
    /// stones and got one. Eligibility must therefore be an ALLOW list that is never
    /// used to write anything outside itself.
    #[test] fn the_quantity_rule_never_claims_a_row_it_is_not_about(){
        for idx in [0x07, 0x0a, 0x0b, 0x0c, 0x15, 0x19, 0x1a] {
            assert!(cash_qty_allowed(0x12, idx), "cat 0x12 idx {idx:#x} is one of the player's seven");
        }
        // The stacked 제련석 row, and the two capped expansions the list must exclude.
        assert!(!cash_qty_allowed(0x0e, 0x03), "제련석 ships a 99 stack — writing it corrupts the grant");
        assert!(!cash_qty_allowed(0x12, 0x0e), "여행가방 is a capped expansion");
        assert!(!cash_qty_allowed(0x12, 0x13), "창고열쇠 is a capped expansion");
        // Equipment categories are never eligible whatever the index.
        for cat in [0x00u8, 0x01, 0x0c, 0x0e, 0x10, 0x11, 0x13] {
            for idx in 0..=0x20u8 {
                assert!(!cash_qty_allowed(cat, idx), "cat {cat:#x} must never be eligible");
            }
        }
    }

    /// The cursor bytes are both read `ldrsb` by the guest (`0x13e7e`/`0x13e8c`), so
    /// both can be negative, and the row count is whatever the last `05 3f` filled.
    /// A `None` here means "fall back to the wide write", which is correct; a wrong
    /// `Some` would poke a row the grant does not read (buys 1) or, without the
    /// bounds, a byte outside the array.
    #[test] fn a_garbage_cursor_never_names_a_row(){
        assert_eq!(cash_row_from_cursor(0, 0, 16), Some(0));
        assert_eq!(cash_row_from_cursor(3, 7, 16), Some(7));
        assert_eq!(cash_row_from_cursor(15, 15, 16), Some(15));
        // Negative cursor, negative table entry: the two `ldrsb` cases.
        assert_eq!(cash_row_from_cursor(-1, 4, 16), None);
        assert_eq!(cash_row_from_cursor(4, -1, 16), None);
        assert_eq!(cash_row_from_cursor(-128, -128, 16), None);
        // Past the client's own array, and past what this page actually holds.
        assert_eq!(cash_row_from_cursor(16, 0, 16), None);
        assert_eq!(cash_row_from_cursor(127, 0, 16), None);
        assert_eq!(cash_row_from_cursor(0, 8, 8), None, "row 8 of an 8-row page does not exist");
        assert_eq!(cash_row_from_cursor(0, 7, 8), Some(7));
        assert_eq!(cash_row_from_cursor(0, 0, 0), None, "an empty page has no rows to poke");
        // A count larger than the client's array cannot widen the window.
        assert_eq!(cash_row_from_cursor(0, 20, 99), None);
        for n in 0..=99u32 {
            for sel in i8::MIN..=i8::MAX {
                for t in [i8::MIN, -1, 0, 1, 15, 16, i8::MAX] {
                    if let Some(r) = cash_row_from_cursor(sel, t, n) {
                        assert!(r < CASH_ROWS_MAX, "row {r} is outside the client's 16-row cache");
                        assert!(r < n, "row {r} is outside the {n} rows this page holds");
                    }
                }
            }
        }
    }

    // ── cutscene / choice hysteresis (2026-08-21) ──────────────────────────────
    //
    // ⚠ MIRROR of `items::cutscene_hold` / `items::choice_hold`.

    const CUTSCENE_EXIT_POLLS: u32 = 3;
    const CHOICE_CONFIRM_POLLS: u32 = 2;

    fn cutscene_hold(gone: &mut u32, script_open: bool, exit_polls: u32) -> bool {
        cutscene_hold_gated(gone, script_open, false, exit_polls)
    }

    fn cutscene_hold_gated(gone: &mut u32, script_open: bool, other_window: bool, exit_polls: u32) -> bool {
        if script_open { *gone = 0; return true; }
        if other_window { *gone = exit_polls; return false; }
        *gone = gone.saturating_add(1);
        *gone < exit_polls
    }

    /// ★★ 66차-B. The hold absorbs the page-turn blip (window id 3), but when ANOTHER
    /// window opens the scene is over as a fact — the game put a different screen on top.
    /// Holding on there gave every window opened from a dialogue (상점 · 제련 · 창고) half
    /// a second in which `overlay::tap_anywhere_key` answered OK for a tap ANYWHERE, which
    /// is what the player reported and could confirm a purchase.
    #[test] fn another_window_ends_the_scene_at_once(){
        let mut gone = 0;
        assert!(cutscene_hold_gated(&mut gone, true, false, CUTSCENE_EXIT_POLLS), "the scene is playing");
        assert!(!cutscene_hold_gated(&mut gone, false, true, CUTSCENE_EXIT_POLLS), "a shop opened: not a scene");
        // …and it stays given up, even for the polls the blip counter would have covered.
        for _ in 0..CUTSCENE_EXIT_POLLS {
            assert!(!cutscene_hold_gated(&mut gone, false, false, CUTSCENE_EXIT_POLLS));
        }
        // The page-turn blip (id 3, other_window = false) is still absorbed.
        let mut gone = 0;
        assert!(cutscene_hold_gated(&mut gone, true, false, CUTSCENE_EXIT_POLLS));
        assert!(cutscene_hold_gated(&mut gone, false, false, CUTSCENE_EXIT_POLLS), "the blip must not drop it");
    }

    fn choice_hold(seen: &mut u32, flag: bool, confirm_polls: u32) -> bool {
        if !flag { *seen = 0; return false; }
        *seen = seen.saturating_add(1);
        *seen >= confirm_polls
    }

    /// ★★ THE BUG THIS EXISTS FOR. The player: "대화1 -> 대화2로 넘어갈 때 그 사이 찰나의
    /// 순간에 떠서 깜빡깜빡 뜨는 느낌이야". A page turn drops the script window id for a
    /// poll or two, and the FIRST version of the hold threw itself away on that blip
    /// (it cancelled whenever the field looked free, and the field looks free exactly
    /// when no window is open). The very next poll was then neither field nor script —
    /// which is MENU, and MENU draws the arrows.
    ///
    /// So: no blip of any length up to the exit threshold may drop the hold, and a
    /// scene that really ended must still give it up promptly.
    #[test] fn a_page_turn_never_drops_the_cutscene_hold(){
        // 4 4 4 (0) (3) 4 4 — the page turn, as the poll sees it.
        let seq = [true, true, true, false, false, true, true];
        let mut gone = CUTSCENE_EXIT_POLLS;
        for (i, open) in seq.iter().enumerate() {
            assert!(cutscene_hold(&mut gone, *open, CUTSCENE_EXIT_POLLS), "poll {i} gave up the hold mid-dialogue");
        }
        // Any blip strictly shorter than the threshold is absorbed, from any point.
        for blip in 1..CUTSCENE_EXIT_POLLS {
            let mut gone = 0;
            assert!(cutscene_hold(&mut gone, true, CUTSCENE_EXIT_POLLS));
            for _ in 0..blip {
                assert!(cutscene_hold(&mut gone, false, CUTSCENE_EXIT_POLLS), "a {blip}-poll blip dropped the hold");
            }
            assert!(cutscene_hold(&mut gone, true, CUTSCENE_EXIT_POLLS));
        }
    }

    /// …and the other direction: a scene that ended must not keep the cutscene set for
    /// long, or the player is left without the controls they need.
    #[test] fn a_scene_that_ended_gives_the_hold_up(){
        let mut gone = 0;
        assert!(cutscene_hold(&mut gone, true, CUTSCENE_EXIT_POLLS));
        let mut polls = 0;
        while cutscene_hold(&mut gone, false, CUTSCENE_EXIT_POLLS) {
            polls += 1;
            assert!(polls < 100, "the hold never expires");
        }
        assert_eq!(polls, CUTSCENE_EXIT_POLLS - 1, "the hold must cost exactly the threshold");
        // …and stay given up.
        for _ in 0..10 { assert!(!cutscene_hold(&mut gone, false, CUTSCENE_EXIT_POLLS)); }
    }

    /// The choice flag is the OTHER way the arrows can appear, so it needs the same
    /// filter — a single poll landing inside a page turn must not raise them, and a
    /// real choice (the flag holds until the player answers) must.
    #[test] fn one_poll_of_the_choice_flag_is_not_a_choice(){
        let mut seen = 0;
        assert!(!choice_hold(&mut seen, true, CHOICE_CONFIRM_POLLS), "a single sample raised the arrows");
        assert!(!choice_hold(&mut seen, false, CHOICE_CONFIRM_POLLS));
        // A real choice box: the flag stays up.
        let mut seen = 0;
        assert!(!choice_hold(&mut seen, true, CHOICE_CONFIRM_POLLS));
        for _ in 0..5 { assert!(choice_hold(&mut seen, true, CHOICE_CONFIRM_POLLS), "a held flag must raise the arrows"); }
        // Answering it takes them away at once — no hysteresis on the way down, or the
        // arrows would outlive the box they belong to.
        assert!(!choice_hold(&mut seen, false, CHOICE_CONFIRM_POLLS));
        assert_eq!(seen, 0);
    }

    /// ★★ DEVICE-REPORTED 2026-08-21: "창고열쇠 수량 변경해서 샀는데도 1개만 사지는데?
    /// 내가 아까 말한 리스트들 제외하고는 수량 입력창 나오면 안돼".
    ///
    /// The picker used to be offered when the PAGE held anything eligible, and the cash
    /// shop's page 0 always does — so it appeared over 창고열쇠 (cat 0x12 idx 0x13,
    /// "네트워크창고 확장;(최대 8가방)") and 여행가방 (0x0e), took a number, and the
    /// purchase then correctly handed over one. A picker that cannot affect the purchase
    /// it is drawn on is worse than no picker.
    #[test] fn the_picker_is_offered_per_item_and_fails_closed(){
        // The seven the player named.
        for idx in [0x07, 0x0a, 0x0b, 0x0c, 0x15, 0x19, 0x1a] {
            assert!(cash_qty_offerable(Some((0x12, idx))), "idx {idx:#x} is one of the seven");
        }
        // …and nothing else on that page, however eligible its neighbours are.
        for idx in [0x00u8, 0x08, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x16, 0x17, 0x18] {
            assert!(!cash_qty_offerable(Some((0x12, idx))), "cat 0x12 idx {idx:#x} must not offer a quantity");
        }
        // An unreadable cursor shows nothing rather than something that might lie.
        assert!(!cash_qty_offerable(None), "an unknown selection must not offer a quantity");
    }

    /// The count reaches the client's row for five of the seven, and the ENGINE for the
    /// other two: 오브상자/소켓확장 are delivered as warehouse orb rows by the host, so
    /// poking their cash row as well would have the client grant N inert bag copies on
    /// top of the N orbs the engine already delivered.
    ///
    /// ★ 2026-08-22 — and now the client grants **소켓확장** nothing at all: the engine
    /// answers its `05 42` with a message instead of an ack (`offline::notice`), which is
    /// what stops the `05 40` commit the grant hangs off. ⛔ **오브상자 `0x19` was left
    /// alone on purpose** — its bag copy is `buffs::box_reward`'s premium random box and the
    /// cash row is its only source. ⛔ And row+10 is the granted STACK SIZE, so poking a 0
    /// there would leave a zero-quantity phantom row instead of nothing: do not "fix" the
    /// count for either of them, in either direction.
    #[test] fn the_two_host_delivered_items_never_get_a_row_write(){
        for idx in [0x07, 0x0a, 0x0b, 0x0c, 0x15] {
            assert!(cash_qty_row_write(0x12, idx), "idx {idx:#x} is granted client-side from the row");
        }
        for idx in [0x19, 0x1a] {
            assert!(cash_qty_allowed(0x12, idx), "the picker is still offered for idx {idx:#x}");
            assert!(!cash_qty_row_write(0x12, idx), "idx {idx:#x} is delivered by the engine — the row must not be poked");
        }
        // A row write is never possible for something the picker would not be offered for.
        for cat in 0..=0x13u8 {
            for idx in 0..=0x30u8 {
                if cash_qty_row_write(cat, idx) {
                    assert!(cash_qty_allowed(cat, idx), "cat {cat:#x} idx {idx:#x} writes a row it may not offer");
                }
            }
        }
    }
    /// ★★★ 80차-E — **the network warehouse's record is live, and two gates said it was
    /// not.** `CHAR+0xec` (max HP) is DERIVED: `ComputeMaxHP` is its only writer and it
    /// runs at world init, level-up and equip. The online slot list deserialises the
    /// record and goes straight to app state 3 without world init (71차-L), so max HP is
    /// still 0 while the game is happily drawing the bag out of that same record.
    ///
    /// Device measurement (vc250, S9, 싱글가방 with items on screen):
    /// `level=Some(3) hp_max=Some(0) sp=Some(1000)`.
    ///
    /// Player: *"네트워크 창고 접속했을때 가방칸 내부에서 터치로 템 포커스 이동이
    /// 안되고있어"* (2026-09-05) — `resolve_char` refusing that triple took the whole
    /// memory-resolving half of `menu_tap_decide` with it.
    #[test] fn the_warehouses_derived_max_hp_is_not_evidence_of_a_dead_record(){
        assert!(char_live(3, 0, 1000), "the measured warehouse record IS a character");
        assert!(char_live(70, 0, 0), "…at any level, with any SP");
        assert!(char_live(99, 0, 0), "…including the new cap (111차: doLevelUp clamps at 99 now)");
        assert!(char_live(71, 0, 1000), "71..98 are real levels since the patch, not garbage");
        // The unequip case 71차-L paid for, from the other side: hp > hp_max is fine.
        let worn_off = Stats { hp: 900, hp_max: 400, sp: 500, sp_max: SP_MAX, exp: 0, exp_next: 100, level: 20 };
        assert!(plausible_stats(&worn_off), "unequipping drops hp_max below hp — still a character");
        let warehouse = Stats { hp: 120, hp_max: 0, sp: 1000, sp_max: SP_MAX, exp: 0, exp_next: 100, level: 3 };
        assert!(plausible_stats(&warehouse), "and the warehouse record publishes its gauges");
    }

    /// …and `level` is still the witness: a record nobody has created is refused.
    #[test] fn a_zeroed_record_is_still_not_a_character(){
        assert!(!char_live(0, 0, 0), "the title screen reads exactly this");
        assert!(!char_live(100, 500, 0), "past the game's own level clamp (99 since 111차)");
        // The ceilings still reject garbage in the two derived fields.
        assert!(!char_live(3, u32::from(HP_CAP) + 1, 0));
        assert!(!char_live(3, 0, u32::from(SP_MAX) + 1));
        let zeroed = Stats::default();
        assert!(!plausible_stats(&zeroed));
    }
}
fn main(){}
