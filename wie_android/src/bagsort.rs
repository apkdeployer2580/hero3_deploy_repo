// 136차 (2026-09-25) — 정렬: the pure rules behind the 「정렬」 button on the three
// bag screens (INVENTORY · 네트워크 SINGLE · 네트워크 창고).
//
// ⛔ NO `use crate::` here. `bagsort_selftest.rs` `include!`s this file and runs on the
// Windows host where `cargo test -p wie_android` cannot; the level table it needs comes in
// as a closure (`item_levels::item_level` in the app, a stub in the test).
//
// ── THE ORDER (user decision 2026-09-24) ─────────────────────────────────────────
//
//   부위 → 요구레벨 ↓ → 등급 ↓ → idx → 제련도 ↓ → 인스턴스 키
//
// Worn gear is NOT pulled forward (the user gave the key without it). Items the level
// table does not know (quest items, 조합서, a stray consumable) come after all equipment,
// grouped by category. The three bag-item cells (오브가방·재료가방·소비가방) never move:
// they are the pin (`bagbox::pin_plan`), bag 1 cells 1·2·3.
//
// ── WHY A DENSE-PER-TAB LAYOUT IS STABLE ─────────────────────────────────────────
//
// The game's own compaction `0x5380c` (23 call sites, the popup close among them) walks
// ONE TAB at a time: it bubbles every empty cell to the tail of that tab and never moves
// an item across tabs (disassembled 2026-09-24). So a layout that fills each tab from
// slot 0 with no gaps is a fixed point of it — nothing we write gets reshuffled by the
// game a frame later. The quickbar's item refs (`(tab<<5)|slot`) travel in the same
// pass, exactly as the compaction's own `0x466b4` fix-up does.

/// Which screen the 정렬 button is offered on. The WORKER decides (`items::sort_screen`),
/// the UI only draws what it was told.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortScreen {
    /// In-game INVENTORY page (menu overlay, item page, no popup, no window).
    Inventory,
    /// 네트워크 · 싱글가방 (shell screen 3, online panel 0) — the same bag records.
    OnlineBag,
    /// 네트워크 · 창고 (shell screen 3, online panel 1) — the host's `offline::Store`.
    OnlineWarehouse,
}

/// One OCCUPIED bag cell as the sort sees it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub tab: u8,
    pub slot: u8,
    pub cat: u16,
    pub idx: u16,
    pub count: u16,
    /// `rec+0x31`, s8.
    pub refine: i8,
    /// `rec+0x28`, the u64 instance key — the record's identity across a compaction.
    pub key: u64,
}

/// Cells per bag tab (the game's grid is 5 x 16).
pub const SLOTS_PER_TAB: u8 = 16;

/// The 부위 order for equipment categories — weapons first (스피어 0x04 … 광휘석 0x0a),
/// then 방패, 투구, 갑옷, 장갑, 신발, 악세. `None` = not equipment.
pub fn slot_rank(cat: u16) -> Option<u8> {
    Some(match cat {
        0x04..=0x0a => (cat - 0x04) as u8, // 0..=6 weapons
        0x0b => 7,                         // 방패
        0x00 => 8,                         // 투구
        0x01 => 9,                         // 갑옷
        0x02 => 10,                        // 장갑
        0x03 => 11,                        // 신발
        0x0c => 12,                        // 악세 / 혼
        _ => return None,
    })
}

/// The coarse group a category sorts into: equipment, then quest items, then 조합서, then
/// whatever else is in the bag (a consumable the box hooks let through, a staged unit).
pub fn group(cat: u16) -> u8 {
    match cat {
        0x00..=0x0c => 0,
        0x11 => 1,
        0x0f => 2,
        _ => 3,
    }
}

/// The comparison key. `Ord` field by field, in this order:
/// group → 부위 (or category, outside equipment) → level DESC → grade DESC → idx →
/// refine DESC → count DESC → instance key → original position (stability).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Key {
    pub group: u8,
    pub rank: u8,
    pub level_desc: u8,
    pub grade_desc: u8,
    pub idx: u16,
    pub refine_desc: u8,
    pub count_desc: u16,
    pub key: u64,
    pub pos: u16,
}

/// Build the key for `(cat, idx, count, refine, key)` at original position `pos`.
/// `level(cat, idx)` = `(등급, 요구레벨)` from the item table, `None` outside it.
pub fn key_of(cat: u16, idx: u16, count: u16, refine: i8, key: u64, pos: u16, level: &impl Fn(u16, u16) -> Option<(u8, u8)>) -> Key {
    let equip = slot_rank(cat);
    let (grade, lv) = level(cat, idx).unwrap_or((0, 0));
    Key {
        group: group(cat),
        // Outside equipment the "부위" column is the category itself, so quest items and
        // recipes stay grouped by kind.
        rank: equip.unwrap_or(cat.min(0xff) as u8),
        level_desc: 255 - lv,
        grade_desc: 255 - grade,
        idx,
        refine_desc: (127 - i16::from(refine)).clamp(0, 255) as u8,
        count_desc: u16::MAX - count,
        key,
        pos,
    }
}

/// Where every occupied cell goes: `(destination (tab, slot), index into `cells`)`, one
/// entry per input cell (fewer only when the bags cannot hold them all). Destinations
/// not listed are to be EMPTY.
///
/// `pinned` = the bag-item `(cat, idx)` ids in seat order (오브가방 → 재료가방 → 소비가방):
/// a present one takes bag 1 cell `k`; an absent one's seat is simply the next regular
/// cell (the game's compaction would fill it anyway, and the later grant + pin swap the
/// bag item in, as they do today). `tabs` = the live bag count (1..=5).
pub fn layout(cells: &[Cell], pinned: &[(u16, u16)], tabs: u8, level: &impl Fn(u16, u16) -> Option<(u8, u8)>) -> Vec<((u8, u8), usize)> {
    let tabs = tabs.clamp(1, 5);
    let mut out: Vec<((u8, u8), usize)> = Vec::with_capacity(cells.len());
    let mut taken = vec![false; cells.len()];
    // 1. the pins, in seat order
    for (seat, want) in pinned.iter().enumerate().take(usize::from(SLOTS_PER_TAB)) {
        if let Some(i) = cells.iter().position(|c| (c.cat, c.idx) == *want) {
            out.push(((0, seat as u8), i));
            taken[i] = true;
        }
    }
    // 2. everything else, in key order
    let mut rest: Vec<(Key, usize)> = cells
        .iter()
        .enumerate()
        .filter(|(i, _)| !taken[*i])
        .map(|(i, c)| (key_of(c.cat, c.idx, c.count, c.refine, c.key, i as u16, level), i))
        .collect();
    rest.sort();
    // 3. dense per tab, skipping the seats the pins took
    let pinned_seats: Vec<(u8, u8)> = out.iter().map(|(d, _)| *d).collect();
    let mut free = (0..tabs).flat_map(|t| (0..SLOTS_PER_TAB).map(move |s| (t, s))).filter(|d| !pinned_seats.contains(d));
    for (_, i) in rest {
        let Some(dest) = free.next() else { break };
        out.push((dest, i));
    }
    out
}

/// `(tab<<5)|slot` — the quickbar's item reference byte (`0x466b4`).
pub fn item_ref(tab: u8, slot: u8) -> u8 {
    (tab << 5) | (slot & 0x1f)
}

/// Where an item reference points after the layout: the ref of the cell its item moved to,
/// or the same byte when that cell did not hold an item (an unbound or stale ref is left
/// alone, exactly as the game's own compaction leaves it).
pub fn remap_ref(r: u8, cells: &[Cell], plan: &[((u8, u8), usize)]) -> u8 {
    let (tab, slot) = (r >> 5, r & 0x1f);
    plan.iter()
        .find(|(_, i)| cells[*i].tab == tab && cells[*i].slot == slot)
        .map(|((t, s), _)| item_ref(*t, *s))
        .unwrap_or(r)
}

/// The new `(tab, slot)` of the cell that was at `(tab, slot)`, when it held an item.
pub fn moved_to(tab: u8, slot: u8, cells: &[Cell], plan: &[((u8, u8), usize)]) -> Option<(u8, u8)> {
    plan.iter().find(|(_, i)| cells[*i].tab == tab && cells[*i].slot == slot).map(|(d, _)| *d)
}

/// The warehouse's key for one wire row (`cat`, `idx`, `count`): the same order as the
/// bag, with no refine or instance key on a row (`pos` keeps it stable).
pub fn wire_key(cat: u8, idx: u8, count: u8, pos: u16, level: &impl Fn(u16, u16) -> Option<(u8, u8)>) -> Key {
    key_of(u16::from(cat), u16::from(idx), u16::from(count), 0, 0, pos, level)
}
