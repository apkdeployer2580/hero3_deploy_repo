// Bag boxes — 오브가방 · 재료가방 · 소비가방: three host-owned containers that open from
// one bag cell each. The RULES, dependency-free so `bagbox_selftest.rs` can `include!`
// this file on the Windows host (`cargo test -p wie_android` cannot run there).
//
// ── THE MODEL (user decision 2026-09-14, `BAG_TYPES_FEASIBILITY.md`) ─────────
// The guest's bag keeps equipment, quest items and the THREE BAG-ITEM CELLS. Everything
// of a box category lives in the host's box instead, as `(cat, idx, count)` stacks with
// the guest's own stacking rule (a stack takes more only while it is <= 98, 99 is the
// cap — `findSlot 0x529a0`, `0x52a60 cmp #0x62`). Each box is the size of the WHOLE bag
// at its largest (`CAP` = 80 stacks = 5 tabs x 16 — 120차, user decision 2026-09-20; it
// was 48 = three tabs from 92차 to 119차). The boxes are counted APART from the bag: a
// box category never costs a bag cell while its box has room.
//
// Items reach a box through:
//   * the two guest ENTRY hooks (`wie_core_arm::binary_patches::bagbox`) — every route
//     that places an item passes `giveItem 0x52b14` or `addItemToInventory 0x131d0`, so
//     a box category never touches a bag cell when the box has room;
//   * the SWEEP — whatever the hooks let through (feature off at the time, box full, a
//     save made before the boxes existed) is moved out of the bag by the worker while no
//     window is open. `sweep_plan` decides; the guest writes are the worker's.
// Items leave a box through host verbs (오브결합 spends an orb; 사용 / 올리기 / 버리기 /
// 판매) and, 125차-C, through the smith's own countOf/spend (`0x55534`/`0x555e0`): the
// game counts bag+box and spends from the box first, so a craft never materialises.
//
// ★★ 96차 (2026-09-15): the online screens have a SECOND set — the network warehouse's
// own 오브가방·재료가방·소비가방. Those are not stored here at all: they are a VIEW of
// `offline::Store::items` filtered by category (`Screen::OnlineWarehouse`), and the
// grids hide those rows so they only ever show inside a box. Moving between the two
// sets (올리기 / 내리기) is host to host and never touches the guest.
//
// ⛔ Invariant the worker keeps: BOX WRITE FIRST, bag cell cleared SECOND. A crash between
// the two duplicates an item; the other order loses one.
//
// ⛔ Do not add a `use crate::…` here; that is the selftest's alarm going off.

/// Stacks per box — the whole bag at its largest (5 tabs x 16). ★ 120차: 48 → 80, user
/// decision 2026-09-20 (*"인게임 48 도 80 으로 올려야해"*). The warehouse's boxes have their
/// own `offline::WH_BOX_CAP` (128 = 8 bags x 16) by the same rule.
pub const CAP: usize = 80;
/// The guest's stack cap and its merge ceiling (`0x52a60`: merge only while `<= 98`).
pub const STACK_MAX: u16 = 99;
pub const MERGE_MAX: u16 = 98;
/// One text line per stack: `<box> <cat> <idx> <count>`, decimal.
pub const FILE_HEADER: &str = "# bagbox v1";
/// The line that records a bag cell the box has ALREADY ABSORBED — see [`Boxes::absorbed`].
/// (`told <box>` lines from 92차..98차 files are still read and ignored: the announcement
/// they tracked is gone — 100차, the migration happens at load time, before the bag can
/// be seen.)
pub const ABSORBED_TAG: &str = "absorbed";

/// The three boxes, in the order `wie_util::stats::BAGBOX_ROOM` is indexed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Orb = 0,
    Material = 1,
    Consumable = 2,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Orb, Kind::Material, Kind::Consumable];

    pub fn index(self) -> usize {
        self as usize
    }

    /// The window title, as the player named the boxes.
    pub fn title(self) -> &'static str {
        match self {
            Kind::Orb => "오브가방",
            Kind::Material => "재료가방",
            Kind::Consumable => "소비가방",
        }
    }

    /// The text key in the box file.
    pub fn tag(self) -> &'static str {
        match self {
            Kind::Orb => "orb",
            Kind::Material => "mat",
            Kind::Consumable => "use",
        }
    }

    fn from_tag(s: &str) -> Option<Kind> {
        match s {
            "orb" => Some(Kind::Orb),
            "mat" => Some(Kind::Material),
            "use" => Some(Kind::Consumable),
            _ => None,
        }
    }

    /// The bag cell that opens this box: a cat-0x12 row the cash shop had DELISTED as a
    /// no-op (배신의문장 · 명예의문장 · 오브상자), renamed by `patch_asset.py`. Zero guest
    /// bytes, and a `price` of 0 makes the game's own gates refuse 버리기 / 올리기 / 판매
    /// on the cell (72차: the sell-price cache `price >> 2` gates all three).
    pub fn bag_item(self) -> (u16, u16) {
        match self {
            Kind::Orb => (0x12, 0x16),
            Kind::Material => (0x12, 0x17),
            Kind::Consumable => (0x12, 0x19),
        }
    }

    pub fn from_bag_item(cat: u16, idx: u16) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.bag_item() == (cat, idx))
    }

    /// The categories this box holds. The assignment the user confirmed: 오브 `0x10` ·
    /// 재료 `0x0e` · 소비 `0x12` + 전투카드 `0x0d`. Equipment (`0x00..=0x0c`), quest
    /// items (`0x11`) and — since 125차-B — 조합서 (`0x0f`) stay in the bag.
    ///
    /// ★★★ 125차-B (2026-09-22, user decision: *"레시피는 재료가방에 있으면 안되지 …
    /// 밖으로 빼줘"* · *"대장장이가 … 특수 조합 시에 가방으로 잠시 옮기는 로직이 아니라"*): the
    /// smith's 특수조합 screen lists BAG cells of cat 0x0f through the game's own index
    /// (`grid+0x17ac`, see [`RECIPE_CAT`]), and the only way a boxed recipe could reach it
    /// was the window-edge push-back — a round trip the user rejected outright. So a
    /// recipe is not a box item at all: the intake hooks leave it to the game, the sweep
    /// does not take it, no plan names it, and a stack an old file still holds goes back
    /// to the bag once ([`Boxes::stranded`], `BagBoxHost::evict_stranded`).
    pub fn cats(self) -> &'static [u16] {
        match self {
            Kind::Orb => &[0x10],
            Kind::Material => &[0x0e],
            Kind::Consumable => &[0x12, 0x0d],
        }
    }

    /// Categories this box USED to hold. A stack line under this box's tag with one of
    /// these is still read from an old file (⛔ never dropped — that is a loss), reported by
    /// [`Boxes::stranded`] and moved back to the bag by the host; nothing else looks at
    /// it. 조합서 `0x0f` left the 재료가방 in 125차-B.
    pub fn retired_cats(self) -> &'static [u16] {
        match self {
            Kind::Material => &[0x0f],
            _ => &[],
        }
    }
}

/// Which box a category belongs to. `None` = it stays in the bag.
pub fn kind_of(cat: u16) -> Option<Kind> {
    Kind::ALL.into_iter().find(|k| k.cats().contains(&cat))
}

/// Is `(cat, idx)` one of the three bag-item cells? Those are never box contents.
pub fn is_bag_item(cat: u16, idx: u16) -> bool {
    Kind::from_bag_item(cat, idx).is_some()
}

/// `BAGBOX_ACCEPT`: bit per category, for the boxes that are switched on.
pub fn accept_mask(enabled: &[Kind]) -> u32 {
    enabled.iter().flat_map(|k| k.cats()).fold(0, |m, c| m | (1 << c))
}

/// `BAGBOX_KEEP_IDX`: the cat-0x12 indices the intake must leave in the bag — the three
/// bag-item cells. (118차 put 부활의서 here for a day; 118차-B took it out again — see
/// [`REVIVE_SCROLL`].)
pub fn keep_idx_mask() -> u32 {
    Kind::ALL.into_iter().map(|k| k.bag_item().1).fold(0, |m, i| m | (1 << i))
}

/// 부활의서 (cat 0x12 idx 0x0b) — the one consumable the GAME reaches into the BAG for on
/// its own. It lives in the 소비가방 like every other consumable; the host lends the game
/// ONE unit at the moment it needs it.
///
/// ★★★ HOW THE GAME REVIVES (118차-B, disassembled): the GAME OVER window's tick, once its
/// "key accepted" flag `base+0xa1d2` is up, calls **`0x3bdec(0x12, 0x0b, &base+0xa264,
/// &base+0xa265)`** — a BAG search for the first cell holding 부활의서, which writes that
/// cell's (tab, slot) — and if it found one opens window 17 (「부활하시겠습니까?」, text 169).
/// Window 17's 예 arm (`0x3b510`) restores HP/SP, `setState(0)`, and **`0x5340c(tab, slot,
/// 1)`** takes ONE unit off that very cell. Nothing in that path consults the quest-VM
/// counter (`0x3bcf0`, whose mirror 94차 hooked), so a scroll sitting in the box was simply
/// not there: *"소비가방에 부활의서가 있는데도 … 부활하시겠습니까 창이 안뜨고있어"*.
///
/// 118차 answered by keeping 부활의서 OUT of the box, which the player refused the same
/// day: *"소비가방에 2개가있는데 … 1개가 사용되고 1개가 일반가방으로 나오고 소비가방엔
/// 0개가 돼. 소비가방에서 그냥 1개 사용되고 일반가방으로 안나오게 해야해"*. So 118차-B
/// keeps it boxed and has the host put exactly one unit into the bag when the player is
/// DYING (state 16) or on the GAME OVER screen (state 7) — the check runs a second or so
/// after the game-over screen appears, well after the death animation begins — and the
/// game's own 예 consumes that unit; the sweep pauses while the player is dying so it is
/// not taken back first (`BagBoxHost::revive_supply` / `revive_frame`).
pub const REVIVE_SCROLL: (u16, u16) = (0x12, 0x0b);

/// How many 부활의서 the host lends the bag while the player is dying: ONE, and only when
/// the bag has none and the box has some — the game takes exactly one on 예, and a unit
/// already in the bag (a leftover, a fresh purchase the sweep has not collected) serves.
pub fn revive_supply_units(box_units: u32, bag_units: u32) -> u32 {
    if bag_units == 0 && box_units > 0 { 1 } else { 0 }
}

// ── STAGING — a boxed unit is used THROUGH the box's own bag cell (127차, 2026-09-22) ──
//
// User: *"소비창고에서 아이템을 쓰려면 가방에 반드시 빈칸이 한개있어야하는데 … 가방이
// 꽉차있어도 쓸 수 있어야해. 다른 모든 3종 가방도 마찬가지야 … 3종가방 내에서 바로사용되는
// 로직으로 변경해야해"*.
//
// The game's `useItem` (27 arms), its hotkey path and its GAME OVER revive all act on a
// BAG CELL `(tab, slot)` — a record in the grid — and nothing else. 92차-B put a boxed
// unit into such a cell with the game's own `giveItem`, which needs a FREE cell (and a
// full bag has none: *"가방에 빈 칸이 하나 필요합니다"*). The cell was the wrong one to
// borrow: the box already OWNS a bag cell — the 오브가방/재료가방/소비가방 item pinned at
// bag 1, cell 1–3 — a record the game never spends (price 0 refuses every verb) and the
// host itself puts back whenever it goes missing (the grant). So the unit is written
// OVER that record: the name, the eight bytes `giveItem`'s own new-cell block writes
// (`0x52f6e`..`0x52fae`: `+0x1e` -1, `+0x1f` occupied, `+0x20` 0, cat, idx, count) and
// nothing else — the instance key and the equipment bytes stay what the bag-item record
// had, exactly as a merged stack keeps its cell's. Then the game's own 사용하기 / hotkey /
// revive runs on that cell as before, and when its removal primitive `0x5340c` reaches
// the cell the `bagbox_restage_lgt` guest hook puts the bag-item record back instead of
// clearing it (`wie_util::stats::bagbox_restage`). No free cell, no compaction, no
// second-tab detour; the 소비가방 cell reads as the potion for the few frames the use
// takes, under the box window.
//
// ⛔ ONE unit, always (`STAGE_COUNT`): the tails that reach `0x5340c` at count 0 are the
// ones the hook answers; staging two and letting one survive would leave a unit no
// debit accounts for. ⛔ The cell is found by CONTENT (the bag-item `(cat, idx)`), never
// assumed at its seat — an unpinned character has it wherever `findSlot` put it.

/// Bytes a staged record needs: the name buffer plus the eight through the count.
pub const STAGE_LEN: usize = 0x28;
/// The record's name buffer (`REC_NAME`, `char[0x1e]`, NUL-terminated).
pub const STAGE_NAME_LEN: usize = 0x1e;
/// Units a staging puts in the cell — see the header.
pub const STAGE_COUNT: u16 = 1;

/// The first `STAGE_LEN` bytes of a bag record holding one unit of `(cat, idx)` named
/// `name` (EUC-KR, as the item DB has it) — what `giveItem`'s non-equipment block writes,
/// byte for byte: the name copied into a zeroed 0x1e buffer, `+0x1e` = -1, `+0x1f` = 1,
/// `+0x20` = 0, `+0x21` = -1 (the cleared form's, which the block leaves), category,
/// index, count.
pub fn stage_record(name: &[u8], cat: u16, idx: u16) -> [u8; STAGE_LEN] {
    let mut out = [0u8; STAGE_LEN];
    let n = name.iter().position(|b| *b == 0).unwrap_or(name.len()).min(STAGE_NAME_LEN - 1);
    out[..n].copy_from_slice(&name[..n]);
    out[0x1e] = 0xff;
    out[0x1f] = 1;
    out[0x20] = 0;
    out[0x21] = 0xff;
    out[0x22..0x24].copy_from_slice(&cat.to_le_bytes());
    out[0x24..0x26].copy_from_slice(&idx.to_le_bytes());
    out[0x26..0x28].copy_from_slice(&STAGE_COUNT.to_le_bytes());
    out
}

/// Does a 0x3c-byte record snapshot read as the bag-item cell of `kind` — occupied, that
/// `(cat, idx)`? What the host checks before it overwrites, and what the restore writes
/// back; a snapshot that fails this is never written anywhere.
pub fn snapshot_is_bag_item(snapshot: &[u8], kind: Kind) -> bool {
    if snapshot.len() < STAGE_LEN || snapshot[0x1f] == 0 {
        return false;
    }
    let cat = u16::from_le_bytes([snapshot[0x22], snapshot[0x23]]);
    let idx = u16::from_le_bytes([snapshot[0x24], snapshot[0x25]]);
    (cat, idx) == kind.bag_item()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stack {
    pub cat: u16,
    pub idx: u16,
    pub count: u16,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Boxes {
    stacks: [Vec<Stack>; 3],
    /// ★★★ THE STALE-SAVE LEDGER (100차, 2026-09-16). The bag cells the last sweep cleared
    /// — position AND content — as they were in the bag at that moment.
    ///
    /// The sweep moves a cell into the box FILE first and clears the bag record in guest
    /// MEMORY second; the bag reaches disk only when the game saves. Until then a reload
    /// of the same save (a crash, a kill, 타이틀 → 게임시작 without saving, and — with the
    /// 5-minute autosave gone in 99차 — any session the player forgets to SAVE) brings the
    /// very same cells back into the bag, and a sweep that did not remember them would put
    /// them into the box a SECOND time. So a cell whose `(tab, slot, cat, idx, count)` is
    /// on this ledger is cleared WITHOUT being added: the box already holds it.
    ///
    /// ★★★ 100차-B (code review): the ledger is the UNION of every cell swept since the
    /// last GameSave COMMIT for this slot, and it empties on that commit — not on every
    /// sweep. The first 100차 rewrote it to "the cells this sweep cleared", so any later
    /// sweep in a session that never saved (a 재료가방 reclaim, a hook-fallback cell)
    /// erased the migration's entries, and a kill + reload of the same record then added
    /// the whole bag a second time. The commit is the one moment the disk is known to
    /// hold the swept bag ([`crate::gamesave_commit`], the hook the loan already uses).
    ///
    /// ★★★ And it is CONSULTED ONLY AT THE LOAD EDGE (`sweep_plan(.., use_ledger)`). Inside
    /// a running session no cell ever comes back from disk; a cell that reappears at a
    /// ledger position with a ledger count is a genuinely new stack — the 재료가방 pushing
    /// 제련석 x20 back into the very cell the migration emptied is the concrete case, and a
    /// sweep that called that "already in the box" cleared it without `add`/`loan.sub`,
    /// so the stones the box had spent were never restored and `loan.clear()` at the next
    /// SAVE dropped them from the file for good (a LOSS — the direction 97차 forbids).
    /// ⛔ Per SAVE SLOT, because this rides in the slot's own file.
    absorbed: Vec<BagCell>,
}

impl Boxes {
    /// Stacks that sit in a box their category no longer belongs to — an old file's 조합서
    /// in the 재료가방 (125차-B, [`Kind::retired_cats`]). The host moves them back to the
    /// bag one stack at a time (`BagBoxHost::evict_stranded`, a materialise with no window
    /// and the same loan); until then the file keeps them and nothing else — the sweep,
    /// the hooks, the plans, all keyed on [`kind_of`] — touches them.
    pub fn stranded(&self) -> Vec<(Kind, Stack)> {
        let mut out = Vec::new();
        for k in Kind::ALL {
            out.extend(self.stacks(k).iter().filter(|s| kind_of(s.cat) != Some(k)).map(|s| (k, *s)));
        }
        out
    }

    pub fn stacks(&self, kind: Kind) -> &[Stack] {
        &self.stacks[kind.index()]
    }

    /// Free stacks in `kind` — what the intake hook is told.
    pub fn room(&self, kind: Kind) -> u32 {
        (CAP - self.stacks(kind).len().min(CAP)) as u32
    }

    /// Was this exact cell (position and content) cleared by the previous sweep, i.e. is it
    /// a stale save bringing back a stack the box already holds? See [`Boxes::absorbed`].
    pub fn was_absorbed(&self, cell: &BagCell) -> bool {
        self.absorbed.contains(cell)
    }

    /// The ledger as it stands — for the selftest and the log.
    pub fn absorbed(&self) -> &[BagCell] {
        &self.absorbed
    }

    /// Add the cells a sweep just cleared to the ledger (a union — a cell already on it
    /// stays once). ⛔ Not a replacement: see [`Boxes::absorbed`].
    pub fn extend_absorbed(&mut self, cells: impl IntoIterator<Item = BagCell>) {
        for c in cells {
            if !self.absorbed.contains(&c) {
                self.absorbed.push(c);
            }
        }
    }

    /// A GameSave for this slot committed with the swept bag: nothing on the ledger is on
    /// disk any more.
    pub fn clear_absorbed(&mut self) {
        self.absorbed.clear();
    }

    /// Selftest-only since 125차-C removed the smith materialise that summed a box
    /// (`bagbox_selftest.rs` still asserts box totals); the lib build sees no caller.
    #[allow(dead_code)]
    pub fn total(&self, kind: Kind) -> u32 {
        self.stacks(kind).iter().map(|s| u32::from(s.count)).sum()
    }

    /// Units of `(cat, idx)` the box's EXISTING stacks can still take — what [`Boxes::add`]
    /// fills before it opens a stack (each while `<= MERGE_MAX`, up to `STACK_MAX`).
    ///
    /// ★ 165차: published per item beside the counts (`wie_util::stats::BAGBOX_MERGE`). The
    /// intake hook spends it before it asks for a free stack, so a 99-unit purchase inside
    /// one guest call costs one stack — it used to cost one per unit, and the shop's 99
    /// `giveItem` calls ran out after min(64, free stacks).
    pub fn merge_room(&self, kind: Kind, cat: u16, idx: u16) -> u32 {
        self.stacks(kind)
            .iter()
            .filter(|s| s.cat == cat && s.idx == idx && s.count <= MERGE_MAX)
            .map(|s| u32::from(STACK_MAX.saturating_sub(s.count)))
            .sum()
    }

    /// How many units of `(cat, idx)` the box could take right now: the merge room of every
    /// stack at or under `MERGE_MAX`, plus a full stack per free cell.
    pub fn fits(&self, kind: Kind, cat: u16, idx: u16) -> u32 {
        self.merge_room(kind, cat, idx) + self.room(kind) * u32::from(STACK_MAX)
    }

    /// Put `n` units of `(cat, idx)` in. Returns what did NOT fit. The guest's order:
    /// merge into existing stacks first (each while `<= 98`), then open new stacks.
    pub fn add(&mut self, kind: Kind, cat: u16, idx: u16, n: u32) -> u32 {
        let mut left = n;
        let list = &mut self.stacks[kind.index()];
        for s in list.iter_mut().filter(|s| s.cat == cat && s.idx == idx && s.count <= MERGE_MAX) {
            if left == 0 {
                break;
            }
            let take = left.min(u32::from(STACK_MAX - s.count));
            s.count += take as u16;
            left -= take;
        }
        while left > 0 && list.len() < CAP {
            let take = left.min(u32::from(STACK_MAX));
            list.push(Stack {
                cat,
                idx,
                count: take as u16,
            });
            left -= take;
        }
        left
    }

    /// Units of `(cat, idx)` this box holds, summed across its stacks — what the guest
    /// hooks are told (`wie_util::stats::set_bagbox_count`) so the quest scripts' own
    /// item helper can count the box as well as the bag.
    pub fn units_of(&self, kind: Kind, cat: u16, idx: u16) -> u32 {
        self.stacks(kind)
            .iter()
            .filter(|s| s.cat == cat && s.idx == idx)
            .map(|s| u32::from(s.count))
            .sum()
    }

    /// Every `(cat, idx)` this box holds with its total — the whole mirror in one pass.
    pub fn totals(&self, kind: Kind) -> Vec<((u16, u16), u32)> {
        let mut out: Vec<((u16, u16), u32)> = Vec::new();
        for s in self.stacks(kind) {
            match out.iter_mut().find(|(id, _)| *id == (s.cat, s.idx)) {
                Some((_, n)) => *n += u32::from(s.count),
                None => out.push(((s.cat, s.idx), u32::from(s.count))),
            }
        }
        out
    }

    /// Take `n` units of `(cat, idx)` out, across stacks, smallest stack first (so the
    /// box ends up with fewer, fuller stacks). Returns what was actually removed — which
    /// is less than `n` only when the box does not hold that much.
    ///
    /// ⛔ The caller decides what a short answer means. A quest hand-in the guest hook
    /// already committed to must never come up short, which is why
    /// [`wie_util::stats::bagbox_take`] checks the MIRROR before it commits; this is the
    /// host applying what the hook already decided.
    pub fn spend_units(&mut self, kind: Kind, cat: u16, idx: u16, n: u32) -> u32 {
        let list = &mut self.stacks[kind.index()];
        let mut left = n;
        let mut order: Vec<usize> = (0..list.len()).filter(|i| list[*i].cat == cat && list[*i].idx == idx).collect();
        order.sort_by_key(|i| list[*i].count);
        for i in order {
            if left == 0 {
                break;
            }
            let take = left.min(u32::from(list[i].count));
            list[i].count -= take as u16;
            left -= take;
        }
        list.retain(|s| s.count > 0);
        n - left
    }

    /// Take `n` units out of stack `at`. `false` when there is no such stack or it holds
    /// fewer than `n`; an emptied stack is removed.
    pub fn spend(&mut self, kind: Kind, at: usize, n: u16) -> bool {
        let list = &mut self.stacks[kind.index()];
        let Some(s) = list.get_mut(at) else { return false };
        if n == 0 || s.count < n {
            return false;
        }
        s.count -= n;
        if s.count == 0 {
            list.remove(at);
        }
        true
    }

    /// The file: one header line, one line per stack, then the absorbed ledger.
    pub fn to_text(&self) -> String {
        let mut out = String::from(FILE_HEADER);
        out.push('\n');
        for k in Kind::ALL {
            for s in self.stacks(k) {
                out.push_str(&format!("{} {} {} {}\n", k.tag(), s.cat, s.idx, s.count));
            }
        }
        for c in &self.absorbed {
            out.push_str(&format!("{} {} {} {} {} {}\n", ABSORBED_TAG, c.tab, c.slot, c.cat, c.idx, c.count));
        }
        out
    }

    /// Tolerant parse: unknown lines are skipped, a stack line with a category that does
    /// not belong to its box is DROPPED (never trusted into the wrong window) — unless it
    /// is one the box USED to hold ([`Kind::retired_cats`]), which is kept as a stranded
    /// stack for the host to move back to the bag (⛔ dropping it would be a loss) —
    /// counts are clamped to the guest's cap, and a box past `CAP` keeps its first `CAP`
    /// stacks.
    pub fn parse(text: &str) -> Boxes {
        let mut b = Boxes::default();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let (Some(tag), rest) = (it.next(), (it.next(), it.next(), it.next())) else {
                continue;
            };
            if tag == ABSORBED_TAG {
                // `absorbed <tab> <slot> <cat> <idx> <count>` — five numbers, all required.
                let (Some(tab), Some(slot), Some(cat)) = rest else { continue };
                let (Some(idx), Some(count)) = (it.next(), it.next()) else { continue };
                let (Ok(tab), Ok(slot), Ok(cat), Ok(idx), Ok(count)) = (
                    tab.parse::<u8>(),
                    slot.parse::<u8>(),
                    cat.parse::<u16>(),
                    idx.parse::<u16>(),
                    count.parse::<u16>(),
                ) else {
                    continue;
                };
                if kind_of(cat).is_none() || is_bag_item(cat, idx) {
                    continue;
                }
                b.absorbed.push(BagCell { tab, slot, cat, idx, count });
                continue;
            }
            let (Some(cat), Some(idx), Some(count)) = rest else {
                continue;
            };
            let Some(kind) = Kind::from_tag(tag) else { continue };
            let (Ok(cat), Ok(idx), Ok(count)) = (cat.parse::<u16>(), idx.parse::<u16>(), count.parse::<u32>()) else {
                continue;
            };
            let belongs = kind_of(cat) == Some(kind) || kind.retired_cats().contains(&cat);
            if !belongs || is_bag_item(cat, idx) || count == 0 {
                continue;
            }
            let list = &mut b.stacks[kind.index()];
            if list.len() < CAP {
                list.push(Stack {
                    cat,
                    idx,
                    count: count.min(u32::from(STACK_MAX)) as u16,
                });
            }
        }
        b
    }

    /// The file as [`Self::to_text`] writes it: `None` when the header line is missing.
    ///
    /// Every file this format has ever produced begins with [`FILE_HEADER`] (92차,
    /// `d10cd0e`), so a text without it is not an old file and not an empty box — it is a
    /// file that did not make it to the disk whole: zero bytes after a publish whose data
    /// never reached the flash (the `flush_fast` window, 104차 ③), or a truncation. The
    /// host fails CLOSED on it (boxes off, one loud line) instead of arming an empty box
    /// over it and then writing that emptiness back durably. Everything after the header
    /// is still read by the tolerant [`Self::parse`]; a later header revision keeps the
    /// `# bagbox` prefix so this test stays true for it.
    pub fn parse_file(text: &str) -> Option<Boxes> {
        let first = text.lines().next()?;
        first.trim_end().starts_with("# bagbox").then(|| Self::parse(text))
    }
}

/// The 재료가방 crash-durability ledger (2026-09-15).
///
/// ★★★ WHY. A box debit that the guest has not yet confirmed in a GameSave must still
/// appear on disk until that save (94차 TAKE, 107차, 124차-B, 125차-C spend). The smith
/// used to materialise into the bag (`materialise_for_window`); 125차-C spends from the
/// box through `0x555e0` instead, and that path still `loan.add`s. Without the loan the
/// box would be short on disk while the matching bag/craft result lives only in unsaved
/// guest memory — a hard kill mid-window would then lose them.
///
/// This ledger closes that: the box FILE is written as `boxes + loan`, so the debited
/// materials still appear IN THE BOX on disk until a real GameSave confirms the bag holds
/// them. A crash then restores them to the box, exactly the way a normal 제련 restores its
/// stones to the bag from the last save. When a GameSave for this slot commits, the loan
/// is cleared and the file promotes to the true debited state.
///
/// The direction is always LOSS → (at worst rare) DUP: the file never shows fewer
/// materials than are durably accounted for. ⛔ Not persisted — the box file already holds
/// the pre-debit state, so a restart is safe with an empty loan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Loan {
    /// Per-kind `(cat, idx, units)` transferred box->bag and not yet GameSave-confirmed.
    out: [Vec<(u16, u16, u32)>; 3],
}

impl Loan {
    /// Record `n` units of `(cat, idx)` pushed box->bag for `kind`.
    pub fn add(&mut self, kind: Kind, cat: u16, idx: u16, n: u32) {
        if n == 0 {
            return;
        }
        let v = &mut self.out[kind.index()];
        match v.iter_mut().find(|(c, i, _)| *c == cat && *i == idx) {
            Some((_, _, u)) => *u += n,
            None => v.push((cat, idx, n)),
        }
    }

    /// Materials came BACK (the sweep reclaimed leftovers): drop up to `n` from the loan.
    /// Extra beyond what was loaned is ignored — those units were never this loan's.
    pub fn sub(&mut self, kind: Kind, cat: u16, idx: u16, n: u32) {
        let v = &mut self.out[kind.index()];
        if let Some(e) = v.iter_mut().find(|(c, i, _)| *c == cat && *i == idx) {
            e.2 = e.2.saturating_sub(n);
        }
        v.retain(|(_, _, u)| *u > 0);
    }

    /// A GameSave for this slot landed (or the character changed): the bag is durable, so
    /// the box file may now show the real debited state.
    pub fn clear(&mut self) {
        for v in &mut self.out {
            v.clear();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.out.iter().all(|v| v.is_empty())
    }

    /// `boxes` with the loaned units added back — what the file must hold so a crash
    /// leaves the materials recoverable in the box.
    pub fn overlay(&self, boxes: &Boxes) -> Boxes {
        let mut b = boxes.clone();
        for k in Kind::ALL {
            for &(cat, idx, n) in &self.out[k.index()] {
                // The units were debited FROM this box, so they fit going back; a `left`
                // here would only be the CAP edge (the box filled with other items since),
                // and dropping the overshoot is the safe (LOSS-not-corruption) direction.
                let _ = b.add(k, cat, idx, n);
            }
        }
        b
    }
}

/// ★★ 165차 — the units ONE drain of the intake ring put into the boxes, per `(cat, idx)`:
/// what a TAKE further down the same drain cancels instead of loaning.
///
/// A scene script can GIVE a box item and TAKE it back in one run — 리파이너의 유적 7's boss
/// door gives 그리폰의피리 (op 0x14) and op 0x12 takes it a few opcodes later, all inside one
/// guest call, so both land in the ring before the host drains. An intake is filed for good
/// at once (a pickup is never rolled back by a reload) and a TAKE is a [`Loan`] until a
/// GameSave (107차) — so that pair used to leave the file one flute UP: reload without
/// saving (and a boss fight cannot be saved) and the flute came back, every time. Units the
/// guest gave and took inside the same drain were never anyone's on disk; the pair cancels.
/// ⛔ Only units from THIS drain: a TAKE of anything filed earlier (a pickup an hour ago, a
/// hand-in now) stays a loan, exactly as 107차 made it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FreshIntake(Vec<((u16, u16), u32)>);

impl FreshIntake {
    /// `n` units of `(cat, idx)` were just filed into a box from the ring.
    pub fn add(&mut self, cat: u16, idx: u16, n: u32) {
        if n == 0 {
            return;
        }
        match self.0.iter_mut().find(|(k, _)| *k == (cat, idx)) {
            Some((_, v)) => *v += n,
            None => self.0.push(((cat, idx), n)),
        }
    }

    /// Of `took` units a TAKE just spent, how many were given in this drain — those cancel
    /// (and are used up here); the rest is the loan.
    pub fn cancel(&mut self, cat: u16, idx: u16, took: u32) -> u32 {
        let Some((_, v)) = self.0.iter_mut().find(|(k, _)| *k == (cat, idx)) else { return 0 };
        let c = (*v).min(took);
        *v -= c;
        c
    }
}

// ── NEW GAME PLUS (2026-09-21) ──────────────────────────────────────
//
// ★★★ WHAT THE GAME'S OWN 2회차 TRANSITION TAKES AWAY — AND IT IS NOT A BAG WIPE.
//
// The ending's window 18 has exactly one KEY handler (window KEY table `0x5d790`, id-3
// → `0x3b420`) and its FIRST instruction is `bl 0x3af2c`: any key there runs the
// transition, there is no 예/아니오. `0x3af2c` then walks all 5 tabs x 16 slots and
// calls `0x5340c(tab, slot, [rec+0x3ba])` — remove the WHOLE stack — for exactly three
// shapes, keeping everything else:
//
//     0x3afcc   cat == 0x11                    every quest item
//     0x3afe6   cat == 0x12 && idx == 0x09     그리폰의피리
//     0x3b01a   cat == 0x0f && idx == 0x12     그리폰의피리 레시피
//
// (`cat` is `(i16)[rec+0x3b6]`, `idx` `(i16)[rec+0x3b8]`, the count `[rec+0x3ba]`; the
// record is `grid + tab*0x3c4 + slot*0x3c`. It then sets the difficulty byte to 1 at
// `0x3b0bc` and warps to scene 83.)
//
// ⛔⛔ THE OLD NOTE "그 전환이 가방을 전소거하고" WAS WRONG. It matters: a wipe would
// have meant emptying the boxes too, which would delete a player's whole orb and
// material collection on new game plus. The game keeps all of that.
//
// Two of the three live in OUR boxes (소비가방 holds cat 0x12, 재료가방 held cat 0x0f until
// 125차-B — the recipe row now only ever meets a stack an old file still keeps there;
// the bag's copy is the game's own business),
// so the game's loop cannot reach them and the host has to repeat the removal — the
// report this answers: *"그리폰의 피리 … 난이도를 넘어가게 되면 소비가방에 여전히
// 남아있어 … 레시피도 마찬가지야"*. Quest items are in the table for completeness;
// they are never boxed (92차 배정 — cat 0x11 stays in the bag).
//
// ⛔ THIS IS A ONE-SHOT AT THE TRANSITION, NOT A STANDING RULE. *"사라진 뒤 게임
// 중간에 퀴스트로 얻을땐 또 문제없이 잘 작동해야해"* — a re-earned 그리폰의피리
// must sweep into the 소비가방 and be usable exactly as before, so nothing here may
// become a filter on what the boxes are allowed to hold.
//
// ★★★ 124차-B — AND THE DEBIT IS A LOAN (`Loan`), NOT A DURABLE WRITE. The game's
// removal is unsaved memory until a GameSave, and a reload of the pre-transition record
// brings the bag back; the box has to come back with it (*"다시 이전 저장데이터를
// 불러오면 피리가 있어야해"*). The host applies the plan with `spend_units` +
// `loan.add(took)` — the same shape as the 107차 quest TAKE — so the file keeps the
// rows until the save that makes the 2회차 bag real. See `BagBoxHost::new_game_plus`.

/// Quest items: the game's transition takes every one of them, whatever the index.
pub const NEW_GAME_PLUS_CAT: u16 = 0x11;

/// The two named rows it takes as well — `(cat, idx)`, in the order the guest tests them.
pub const NEW_GAME_PLUS_ROWS: &[(u16, u16)] = &[(0x12, 0x09), (0x0f, 0x12)];

/// Does the game's new-game-plus transition take this row out of the bag?
pub fn new_game_plus_removes(cat: u16, idx: u16) -> bool {
    cat == NEW_GAME_PLUS_CAT || NEW_GAME_PLUS_ROWS.contains(&(cat, idx))
}

/// The same removal, applied to the BOXES: `(kind, cat, idx, units)` for every stack the
/// transition would have taken if it could see them. Empty when there is nothing to do,
/// which is the ordinary case.
pub fn new_game_plus_plan(boxes: &Boxes) -> Vec<(Kind, u16, u16, u32)> {
    let mut out = Vec::new();
    for k in Kind::ALL {
        for ((cat, idx), n) in boxes.totals(k) {
            if n > 0 && new_game_plus_removes(cat, idx) {
                out.push((k, cat, idx, n));
            }
        }
    }
    out
}

/// One occupied bag cell, as the worker read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BagCell {
    pub tab: u8,
    pub slot: u8,
    pub cat: u16,
    pub idx: u16,
    pub count: u16,
}

/// A cell the sweep will clear, whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sweep {
    pub cell: BagCell,
    pub kind: Kind,
    /// `true` = the box ALREADY holds this stack ([`Boxes::absorbed`]): clear the cell,
    /// add nothing. `false` = a new stack: add it to the box, then clear the cell.
    pub already_in_box: bool,
}

/// Which bag cells to absorb, given the boxes as they are and the boxes that are ON.
///
/// A NEW cell is taken only when its WHOLE stack fits (simulated in order), so a cell is
/// never half-moved; a cell that does not fit is simply left for a later poll. With
/// `use_ledger` (the first sweep after a record LOAD — the only time the bag can hold a
/// cell that came from disk) a cell on the absorbed ledger is always taken and consumes
/// no room — the box has it already; without it the ledger is not looked at, and every
/// cell is what it is: a new stack (see [`Boxes::absorbed`] for the 재료가방 loss the
/// other choice made). The three bag-item cells are never candidates, nor is anything of
/// a category that stays in the bag. A count of 0 (72차's zero-count orbs) is taken too —
/// the box does not keep it, and the cell was unusable where it was.
/// ★★★ 106차 — A LEDGER HIT IS A CLAIM, AND THE CLAIM IS VERIFIED AGAINST THE BOX.
///
/// `already_in_box` DELETES a bag cell and adds nothing, so a ledger entry that is wrong
/// destroys the stack. 100차-B already caught one way for it to go wrong — *"재료가방이
/// 실체화한 제련석이 이주로 빈 칸에 같은 개수로 착지하면 「이미 상자에 있다」로 지워져 재료가
/// 소실됐다"* — and answered it by consulting the ledger only on the first sweep after a
/// load. That is not enough, because **a materialise followed by a reload re-arms exactly
/// that sweep** (제보 2026-09-19: 일반조합 → 실패 → 메인메뉴 → 재진입 → *"모든 재료가방 내의
/// 아이템이 사라졌어"*):
///
/// 1. a sweep absorbs the materials and writes the ledger (`tab, slot, cat, idx, count`);
/// 2. a crafting window materialises them BACK, and `findSlot 0x529a0` takes the first
///    FREE cell — which is the very cell the sweep just emptied, with the whole stack, so
///    the new cell matches the ledger entry **byte for byte** ([`Boxes::was_absorbed`]
///    compares position AND content);
/// 3. the box is debited for them (they are in the bag now, held by the [`Loan`]);
/// 4. the player reloads, `ledger_live` goes true again, and this function deletes every
///    one of those cells while adding nothing. The box was already debited, so the
///    materials are gone from both sides.
///
/// The fix is not another liveness flag — it is to stop trusting the ledger blind. The
/// entry claims *"the box already holds this stack"*, and that is a statement this
/// function can CHECK: `boxes.units_of(..) >= n`. When the box really holds them the
/// verdict is unchanged (the stale-save case 100차 exists for); when it does not, the
/// claim is false and the cell is a NEW stack that must be ADDED. Every ledger mistake
/// therefore costs a duplicate instead of a character's materials — the direction 100차
/// fixed on (*"방향은 언제나 손실 → (최악의 경우 드문) 중복"*).
///
/// ⛔ The claim is CONSUMED (`claimed`), so two identical cells on the ledger need two
/// stacks' worth in the box; otherwise one stale duplicate entry could delete a cell the
/// box only covers once.
/// ⛔ The units are NOT spent from `sim`. They are still physically in the box, so
/// spending them would overstate the room left for the NEW cells below, and a cell that
/// then does not fit loses its remainder in `add`.
pub fn sweep_plan(boxes: &Boxes, cells: &[BagCell], enabled: &[Kind], use_ledger: bool) -> Vec<Sweep> {
    let mut sim = boxes.clone();
    let mut out = Vec::new();
    let mut claimed: Vec<((u16, u16), u32)> = Vec::new();
    for c in cells {
        let Some(kind) = kind_of(c.cat) else { continue };
        if !enabled.contains(&kind) || is_bag_item(c.cat, c.idx) {
            continue;
        }
        let n = u32::from(c.count);
        if use_ledger && boxes.was_absorbed(c) {
            let used = claimed.iter().find(|(k, _)| *k == (c.cat, c.idx)).map_or(0, |(_, v)| *v);
            if boxes.units_of(kind, c.cat, c.idx) >= used.saturating_add(n) {
                match claimed.iter_mut().find(|(k, _)| *k == (c.cat, c.idx)) {
                    Some((_, v)) => *v = v.saturating_add(n),
                    None => claimed.push(((c.cat, c.idx), n)),
                }
                out.push(Sweep {
                    cell: *c,
                    kind,
                    already_in_box: true,
                });
                continue;
            }
            // The ledger is stale — the box does NOT hold these. Fall through and treat
            // the cell as what it now is: a new stack.
        }
        if n > 0 && sim.fits(kind, c.cat, c.idx) < n {
            continue;
        }
        let left = sim.add(kind, c.cat, c.idx, n);
        debug_assert_eq!(left, 0);
        out.push(Sweep {
            cell: *c,
            kind,
            already_in_box: false,
        });
    }
    out
}

// ── the three bag-item cells are PINNED to bag 1, slots 1–3 ────────────────
//
// ★★ User, 2026-09-14: *"참고로 재료가방,소비가방,오브가방은 항상 인벤토리의 맨첫번째
// 가방 1,2,3번 칸에 있어야해. 고정이야 이건"*. Nothing in the game puts them there:
// `giveItem` asks `findSlot 0x529a0`, which takes the first FREE cell, so on an existing
// character the three land wherever the equipment is not.
//
// ★ Once pinned they STAY pinned for free. The game's compaction (`0x5380c`) only ever
// shifts items toward lower slots inside a tab, preserving order, and these three are
// then the first three occupied — so nothing can move above them. A new item goes to the
// first free cell, which is after them.

/// One record swap the pin needs: `from` and `to` are `(tab, slot)`.
///
/// ⛔ A SWAP, not a move-into-a-hole: the destination usually holds the player's gear,
/// and that gear has to end up somewhere. Swapping is also what keeps the plan a
/// permutation, so nothing can be lost however it is interleaved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinSwap {
    pub from: (u8, u8),
    pub to: (u8, u8),
}

/// Where the three bag-item cells belong: bag 1, cells 1–3, in [`Kind::ALL`] order.
pub fn pinned_seat(kind: Kind) -> (u8, u8) {
    (0, kind.index() as u8)
}

/// The swaps that put every bag-item cell the bag HOLDS into its seat.
///
/// Computed in seat order on a simulated layout, so a plan that moves one cell into a
/// seat another cell is sitting in still lands both correctly. A kind the bag does not
/// hold is skipped — the grant will add it, and `findSlot` puts it in the first free
/// cell, which a later poll then pins.
pub fn pin_plan(cells: &[BagCell]) -> Vec<PinSwap> {
    // Where each bag-item cell is, as the simulation moves them.
    let mut at: Vec<((u8, u8), (u16, u16))> = cells.iter().map(|c| ((c.tab, c.slot), (c.cat, c.idx))).collect();
    let mut out = Vec::new();
    for kind in Kind::ALL {
        let want = pinned_seat(kind);
        let item = kind.bag_item();
        let Some(i) = at.iter().position(|(_, id)| *id == item) else { continue };
        let from = at[i].0;
        if from == want {
            continue;
        }
        out.push(PinSwap { from, to: want });
        // Apply it, so the next kind sees the layout it will actually meet.
        let other = at.iter().position(|(pos, _)| *pos == want);
        at[i].0 = want;
        if let Some(j) = other {
            at[j].0 = from;
        }
    }
    out
}

/// 조합서 — the recipe category (125차). A recipe is ONE unit in ONE cell, and the smith's
/// 특수조합 pick grid (sub-screen 5 of window 13, painter `0x238dc`) lists nothing but cells
/// of this category: it reads the index `grid+0x17ac`, which `0x4d858` fills with every
/// bag cell whose `cat == 0x0f` (`0x4dbe8`) and rebuilds at the end of every successful
/// `giveItem` (`0x532a0`).
///
/// ★ 125차-B: a recipe is not a box item ([`Kind::cats`]) — it stays in the bag, where
/// the smith reads it. 125차-C counts and spends MATERIALS from the box (`countOf` /
/// `0x555e0`); this constant is still the pick-grid category, not a forge-plan rank.
#[allow(dead_code)]
pub const RECIPE_CAT: u16 = 0x0f;

/// One row of the generated names table (`bagbox_names.rs`):
/// `(cat, idx, colour marker, name, first effect line, price)`.
pub type NameRow<'a> = (u16, u16, &'a str, &'a str, &'a str, u32);

/// `(colour marker, name, first effect line)` for `(cat, idx)` out of the generated table
/// (`bagbox_names.rs`, passed in so this file stays dependency-free). Unknown rows get a
/// `cat/idx` label rather than nothing — a box must never show an unnamed stack.
pub fn name_of<'a>(table: &'a [NameRow<'a>], cat: u16, idx: u16) -> (&'a str, String, &'a str) {
    match table.iter().find(|r| r.0 == cat && r.1 == idx) {
        Some(r) => (r.2, display_name(cat, r.3), r.4),
        None => ("", format!("{cat:#04x}/{idx:#04x}"), ""),
    }
}

/// The object particle that follows `name` in a sentence — `을` after a syllable with a
/// final consonant (`파편을`), `를` without one (`오브를`). A name ending in anything
/// but a Hangul syllable (a digit, a bracket) gets the neutral `을(를)`. Used for the
/// window's own 「○○ 오브를 입수 하였습니다.」 (110차-B), which wraps the game's line.
pub fn object_particle(name: &str) -> &'static str {
    match name.chars().last() {
        Some(c) if ('가'..='힣').contains(&c) => {
            if (u32::from(c) - u32::from('가')) % 28 != 0 {
                "을"
            } else {
                "를"
            }
        }
        _ => "을(를)",
    }
}

/// The item DB's price for `(cat, idx)`, or `None` for a row the table does not know.
pub fn price_of(table: &[NameRow<'_>], cat: u16, idx: u16) -> Option<u32> {
    table.iter().find(|r| r.0 == cat && r.1 == idx).map(|r| r.5)
}

/// The name as the game prints it in the bag: an orb's DB row is only its prefix
/// (`투신의`), and `formatItemName 0x6b90` appends the category word with `"%s %s"`
/// (`0x5bc98`) — except 소켓확장, whose row is a full name already.
pub fn display_name(cat: u16, db_name: &str) -> String {
    if cat == 0x10 && !db_name.ends_with("소켓확장") {
        format!("{db_name} 오브")
    } else {
        db_name.to_string()
    }
}

// ── the DROP BOX verbs (95차, 2026-09-15) ──────────────────────────────────
//
// ★★ User: *"기존처럼 버리기 같은것도 있어야하는데 왜 사용하기 퀵슬롯등록밖에없어?"*.
// The game's own item popup is 사용하기 / 상세보기 / 버리기, and a box row had only the
// two verbs 92차-B needed. 상세보기 is the one that does not come back: the row already
// prints the name and the effect line the detail window would show.
//
// ⛔ 버리기 is CONFIRMED before it runs ([`Verb::Drop`] → a second box of 예 / 아니오).
// It is the only verb here that destroys something, and the game asks too.

// ⛔⛔ THERE IS NO 꺼내기 (2026-09-16, user: *"인게임 가방안 선택지에서 꺼내기라는선택지가
// 있는데 이건 왜넣은거야? 이거 없애줘 꺼내기라는 기능은 없어."*). 95차 invented it — the game
// has no such verb on any item popup, and a box is not a place you take things out of by
// hand: 125차-C lets the smith count and spend the box itself (`countOf` / `0x555e0`),
// and consumables are used from the window itself. ⛔ Do not add it back.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// Hand one unit to the game's own `useItem` (consumables only).
    Use,
    /// Put the item on a quickslot plate (consumables only).
    Bind,
    /// Throw the stack away. Confirmed first.
    Drop,
    /// The game-style message box with the row's name and effect (the online screens'
    /// 상세보기 — the game's own first verb on a warehouse row).
    Detail,
    /// 올리기: bag box → the network warehouse (host to host, 96차).
    Deposit,
    /// 내리기: the network warehouse's box → the bag box (host to host, 96차).
    Withdraw,
    /// 판매: the bag box → gold, at the shop's sell grid (105차).
    ///
    /// Host-side like 올리기 / 내리기, and for the same reason: the item is in a container
    /// the guest cannot see, so there is no cell for the game's own sell arm to act on.
    /// The PRICE is the game's own (`sell_price` = the DB price >> 2, the rule the shop
    /// gate at `0x1e364` reads) and the credit goes where the game's own sell puts it —
    /// `gold += 판매가` at `base+0xa4` (`0x1e466`: `adds r4,#0xa4` · `adds r3,r3,r2`).
    Sell,
}

impl Verb {
    pub fn label(self) -> &'static str {
        match self {
            Verb::Use => "사용하기",
            Verb::Bind => "퀵슬롯등록",
            Verb::Drop => "버리기",
            Verb::Detail => "상세보기",
            Verb::Deposit => "올리기",
            Verb::Withdraw => "내리기",
            Verb::Sell => "판매",
        }
    }
}

// ── WHICH SCREEN a box window was opened from (96차, 2026-09-15) ────────────
//
// ★★ User: *"네트워크 접속의 싱글가방안에 오브가방,재료가방,소비가방있고 … 네트워크 접속
// 가방도 마찬가지야 … 그 각각 가방 창에서 … 상세보기, 버리기, 내리기(싱글에서는 상세보기
// 버리기, 올리기) 선택지가 나와야"*. There are TWO SETS of boxes on the online screens:
// the bag's own three (the same containers the field shows) and the WAREHOUSE's three,
// which are a VIEW of `offline::Store::items` filtered by category — no storage of their
// own (`BAG_TYPES_FEASIBILITY.md §9.1`). Which set, and which verbs, is the screen's call.

/// Where the window was opened from — decides the SET of boxes and the verbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// The in-game inventory page: the bag's boxes, the field verbs.
    Inventory,
    /// 네트워크 · 싱글가방 (online panel 0): the bag's boxes, 상세보기 · 버리기 · 올리기.
    OnlineBag,
    /// 네트워크 · 창고 (online panel 1): the WAREHOUSE's boxes, 상세보기 · 버리기 · 내리기.
    OnlineWarehouse,
    /// 상점의 판매 격자 (window [`SHOP_WINDOW`]): the bag's boxes, 상세보기 · 판매.
    ///
    /// ★★★ 105차 (2026-09-19, 사용자: *"아이템 판매시에 3종가방에 있는걸 열수가없는데 …
    /// 판매시에 3종가방 선택하면 열려서 판매 가능하게 해줘"*). The shop's sell list walks
    /// the bag with its own loop, so a boxed item is simply not in it — and 97차 removed
    /// 꺼내기, which had been the only way out of a box. That left the boxes with **no
    /// route to a shop at all** (the 97차 note says so in as many words: *"상자 안의 재료를
    /// 상점에 팔 수 없다"*). This screen is that route.
    ///
    /// ⛔ NOT by materialising the box into the bag. 125차-C lets the smith count and spend
    /// the box itself; a shop is entered constantly and mostly to BUY, so filling the bag
    /// with materials would leave nowhere to put what was bought. The box opens as a
    /// window instead and the host sells out of it directly, so the bag is never touched.
    Shop,
}

/// The game's own window id of 상점 (`openWindow`), whose 판매 arm draws a bag grid.
///
/// ⛔ The same window id covers 구매 as well — the shop is one window with two lists, and
/// MEMORY CANNOT TELL THEM APART (`items::menu_tap_decide` says so where it picks between
/// a row list and an item grid). That is why only the TAP path opens a box here: a tap
/// reaches [`Screen::Shop`] only after `menutouch::item_grid` has found the game's own
/// empty-cell frames AND `item_grid_action` has matched the derived index against the live
/// cursor byte, i.e. only when a bag grid is genuinely on screen. The OK KEY has no such
/// proof, so `items::bagbox_open_key` refuses this screen — on the 구매 list a stale cell
/// cursor sitting on a pinned box cell would otherwise swallow the key and the player
/// could not buy.
pub const SHOP_WINDOW: u8 = 12;

/// The online panel index (`items::ONLINE_PANEL_OFF`) whose grid IS the bag.
pub const ONLINE_PANEL_BAG: u32 = 0;
/// The online panel index whose grid is the NETWORK WAREHOUSE's own rows.
pub const ONLINE_PANEL_WAREHOUSE: u32 = 1;

impl Screen {
    /// Does this screen show the warehouse's set (a view of the store) rather than the
    /// bag's own boxes?
    pub fn is_warehouse(self) -> bool {
        matches!(self, Screen::OnlineWarehouse)
    }

    /// Which screen a FOCUSED CELL belongs to, from the three facts the worker can read:
    /// `shell` = app state 3 · `online` = the shell's screen word is the online screen's
    /// (창고 · 캐시샵 — `items::SHELL_SCREEN_ONLINE`) · `panel` = the online panel index.
    /// `None` = no box opens here, the key or tap goes to the game.
    ///
    /// ⛔⛔ `shell` alone is NOT "the online screen" (2026-09-16, 제보: *"네트워크 창고에서
    /// 오브가방 오픈 -> 뒤로가기(세이브 슬롯 선택까지) -> 세이브 슬롯 선택하면 선택하자마자
    /// 오브가방이 열려있는채로 나타나고 … 게임시작 세이브 슬롯 선택해도 게임시작이 되는게
    /// 아니라 오브가방이 열린채로"*). App state 3 is also the title, BOTH slot lists and
    /// the creation screens (66차-C), and the two bytes the caller reads next — the panel
    /// index and the shared cell cursor — keep their LAST value when the player backs out
    /// of the warehouse. A cursor left on our cell there turned every OK on a slot row
    /// (a pad OK, the OK disc, the 80차-M second tap) into 「open the box」: the key was
    /// swallowed, the slot never loaded, and CLR from the box put the player back on the
    /// same list. Same shape as the in-game regression 98차 §4 closed with the inventory
    /// page gate — this is the shell half of it.
    /// ★ `window` is the game's own window id (`items::window_id`), which only matters
    /// IN THE WORLD: [`SHOP_WINDOW`] there is the shop, and its 판매 arm is the one place
    /// outside the inventory where a bag grid is drawn (105차). Every other window — and
    /// an unreadable id — stays [`Screen::Inventory`], whose caller applies the inventory
    /// page gate on top.
    pub fn from_shell(shell: bool, online: bool, panel: Option<u32>, window: Option<u8>) -> Option<Screen> {
        if !shell {
            return Some(if window == Some(SHOP_WINDOW) { Screen::Shop } else { Screen::Inventory });
        }
        if !online {
            return None;
        }
        match panel {
            Some(ONLINE_PANEL_BAG) => Some(Screen::OnlineBag),
            Some(ONLINE_PANEL_WAREHOUSE) => Some(Screen::OnlineWarehouse),
            // The 캐시샵 (2) and an unreadable byte: fail closed.
            _ => None,
        }
    }
}

/// The verbs a row of `kind` offers on `screen`, in drop-box order.
///
/// ⛔ In the INVENTORY the 오브가방 has none: choosing an orb row opens the COMBINE TARGET
/// PICKER, which is the whole reason that window exists (69차). A drop box in front of it
/// would put a menu between the player and the one thing they came for.
/// ★ On the ONLINE screens every box has the same three — exactly the verbs the game's
/// own popup gives an ordinary cell there (*"인게임이랑 같아"*), with 올리기 / 내리기
/// swapped by which side of the screen the box belongs to. No combine there: the
/// picker writes bag records the inventory page owns; 내리기 first, then combine in-game.
pub fn verbs(kind: Kind, screen: Screen) -> &'static [Verb] {
    match screen {
        Screen::Inventory => match kind {
            Kind::Orb => &[],
            Kind::Consumable => &[Verb::Use, Verb::Bind, Verb::Drop],
            // ⛔ 버리기 only. The smith counts and spends the box itself (125차-C
            // `countOf` / `0x555e0`); there is no verb for moving them by hand. See the
            // 꺼내기 tombstone above `Verb`.
            Kind::Material => &[Verb::Drop],
        },
        Screen::OnlineBag => &[Verb::Detail, Verb::Drop, Verb::Deposit],
        Screen::OnlineWarehouse => &[Verb::Detail, Verb::Drop, Verb::Withdraw],
        // ★ 105차. 상세보기 so the player can check what a row is worth keeping, and 판매.
        // ⛔ NO 버리기 here: the shop is where things are turned into gold, and a stray
        // 버리기 next to 판매 on an 80-stack list is a destructive mis-tap waiting to
        // happen. 버리기 stays on the inventory and online screens, which already have it.
        // ⛔ The 오브가방 is NOT empty here the way it is in the inventory: an orb row at
        // the shop is a thing to sell, not a combine target — the combine picker belongs
        // to the inventory page, which owns the bag records it writes (96차).
        Screen::Shop => &[Verb::Detail, Verb::Sell],
    }
}

// ── WHICH VERBS THE GAME'S OWN POPUP GIVES ONE CONSUMABLE (115차, 2026-09-19) ────────
//
// ★★ User: *"부활의서는 사용하기가 뜨면안되는데 뜨고있어"*. The game agrees — its inventory
// popup builder (`0x3fa38`; the cat-0x12 branch is `0x3fb1c..0x3fc4e`) shapes a consumable's
// popup BY INDEX, and the box window's drop box has to give the same verbs:
//
//   idx 0x0b 부활의서                          `cmp #0xb` → tag 0x64, 2 rows: 상세보기 · 버리기
//                                                                    (no 사용하기, no N키등록)
//   idx 7 오브원석, and every idx > 0x0a       → tag 0x69, 3 rows: 사용하기 · 상세보기 · 버리기
//   (cash tier 0x0b.. except 컵라면 0x0c)                          (no N키등록)
//   idx 0x0c 컵라면 and the rest ≤ 0x0a         → tag 0x66, 8 rows: 사용하기 · 1~5키등록 · 상세보기 · 버리기
//
// (`0x3fb3a cmp #0xc; beq` lets 컵라면 past the `> 0x0a` test, `0x3fb7e cmp #7; bne` sends
// 오브원석 into the three-row shape; label ids 0x21 사용하기 · 0x20 상세보기 · 0x22 버리기 ·
// 0x23..0x27 N키등록.) Every category but 0x12 takes the two-row popup (`0x3fac4 cmp
// #0x11; ble`), so a cat-0x0d unit in the 소비가방 has no 사용하기 either — and the
// `useItem` dispatcher (`0x550b8 cmp #0x12`) would ignore it anyway.
// 부활의서 is consumed by the GAME OVER revive (「게임오버 상태에서 자동 부활」); its `useItem`
// arm (table `0x5e538` [0x0b] = the bare consume tail `0x55450`) would only destroy a unit
// for nothing. That is why the game hides the verb, and why the box does too.

/// Does the game's own popup offer 사용하기 for this item? (cat 0x12 only — every other
/// category gets the two-row popup with no use verb at all.)
pub fn game_offers_use(cat: u16, idx: u16) -> bool {
    cat == 0x12 && idx != 0x0b
}

/// Does the game's own popup offer the N키등록 rows (our 퀵슬롯등록) for this item?
pub fn game_offers_hotkey(cat: u16, idx: u16) -> bool {
    cat == 0x12 && ((idx <= 0x0a && idx != 0x07) || idx == 0x0c)
}

/// Does this item's `useItem` arm RAISE THE GAME'S OWN DIALOG instead of consuming the unit?
///
/// ★★ 귀환서 (idx 8) and 그리폰의피리 (idx 9): table `0x5e538` [8] = [9] = `0x551f0`. The arm
/// does not consume: it puts the game's own question up and consumes ON 예, together with
/// the warp (아니오 / CLR leave the unit in the bag). So there is no result for the use job
/// to watch — `WaitResult` saw the question as "a box, nothing consumed", called it a
/// refusal, and the OK it sent to dismiss that box pressed 예 for the player (115차 report
/// *"거절하였습니다 라고 뜨고는 귀환서 사용이 되고있어"*).
///
/// ⛔⛔ 117차-C CORRECTS 116차: from the inventory popup that dialog is NOT `openWindow(15)`.
/// The device `focus probe` right after 사용하기 read `win=3` (no window) with
/// `list rows=2 axis=0(horizontal)` — the SHARED CHOICE WIDGET (the same 예/아니오 pair 80차-G
/// measured for 저장하시겠습니까?), raised over the inventory page. 116차's job waited for
/// `window_id == 15` and never saw it, so it gave up after 1.5 s with the box window still
/// covering the game's question — the 2026-09-20 report *"사용하기 -> 아무일 일어나지 않고
/// 갯수만 1개 감소 -> 취소로 나가면 confirm 떠있음"*. `openWindow(idx + 7)` may well be what the
/// FIELD hotkey path does; the popup path is the widget, measured.
pub fn use_opens_window(cat: u16, idx: u16) -> bool {
    cat == 0x12 && (idx == 0x08 || idx == 0x09)
}

/// May the host run this item's `useItem` arm itself while a skill is being cast?
///
/// ★★★ 164차 (user: *"스킬 선딜과 후딜 사이에 포션을 사용하면 아이템 사용중 이라는 창이 뜨는데 …
/// 스킬 사용중에도 포션 아무런 문구없이 사용되도록"*). The game drops every field key while a
/// skill is cast (`items::field_key_gate`), so a potion plate's hotkey did nothing and the
/// use job waited out its guard — the next press met 「아이템을 사용하는 중입니다」.
/// Only the POTION arms: `useItem`'s table `0x5e538` sends idx 0..3 to `0x550d6` (`addHP`)
/// and 4..6 to `0x5510a` (`0x48f90`), and both just add, then reach the consume tail —
/// neither touches the player's state or pose (their one state reset is for the
/// `base+0x2b5` stun, `0x55144`). ⛔ Not 귀환서 / 그리폰의피리 (`openWindow` over a cast) or
/// anything else: those keep the game's own refusal.
pub fn use_runs_through_a_cast(cat: u16, idx: u16) -> bool {
    cat == 0x12 && idx <= 0x06
}

/// Is this item's game dialog a MESSAGE BOX the box window can show THROUGH itself?
///
/// ★★★ 117차-D (user: *"최근 귀환장소로 이동하시겠습니까? 팝업이 기존 팝업 그대로 소비가방 위에
/// 떠야하는데"*). 귀환서 (idx 8) raises the shared widget's 예/아니오 message box — a framed
/// box inside the game panel (`menutouch::game_dialog_frame`, measured dx 10..166 /
/// dy 25..125). The UI re-blits exactly that rectangle over the box window, so the player
/// sees the game's own question over the 소비가방 and answers it with the game's own keys;
/// the bag page around it stays covered. 그리폰의피리 (idx 9) is a TOWN LIST whose frame has
/// not been measured, so for it the box window is hidden instead (116차's shape) and comes
/// back on cancel. ⛔ 117차-C's host-drawn drop-box question is gone: the user wants the
/// game's popup, not a copy.
pub fn use_dialog_over_window(cat: u16, idx: u16) -> bool {
    cat == 0x12 && idx == 0x08
}

/// [`verbs`] narrowed to ONE stack: the game's own per-item rule applied to 사용하기 and
/// 퀵슬롯등록, everything else untouched. The drop box shows this list and maps the
/// player's pick through the same list, so a hidden verb cannot be reached by position.
pub fn verbs_for(kind: Kind, screen: Screen, cat: u16, idx: u16) -> Vec<Verb> {
    verbs(kind, screen)
        .iter()
        .copied()
        .filter(|v| match v {
            Verb::Use => game_offers_use(cat, idx),
            Verb::Bind => game_offers_hotkey(cat, idx),
            _ => true,
        })
        .collect()
}

/// What the shop pays for ONE unit of `(cat, idx)` — the game's own rule.
///
/// ★★★ 판매가 = `price >> 2`, and the shop refuses a row whose sell price is ZERO
/// (`0x1e364`: `ldr r3,[r0]` of the sell-price cache, `cmp r3,#0`, `bne` past the refusal).
/// That is the same byte-level rule 버리기 and 올리기 already lean on — the three bag-item
/// cells themselves are priced 0 precisely so the game refuses all three verbs on them
/// (72차) — so a boxed row the table prices at 0 must be refused here too, or the host
/// would pay gold for something the game itself would not buy.
///
/// ⛔ FAIL CLOSED: a row the names table does not know has no price, and `None` from
/// [`price_of`] becomes 0 here, i.e. "not sellable".
pub fn sell_price(table: &[NameRow<'_>], cat: u16, idx: u16) -> u32 {
    price_of(table, cat, idx).unwrap_or(0) >> 2
}

/// The shop's refusal for a row it will not buy, or `None` when it will.
pub fn sell_refusal(table: &[NameRow<'_>], cat: u16, idx: u16) -> Option<&'static str> {
    (sell_price(table, cat, idx) == 0).then_some(REFUSE_SELL)
}

/// Shown when [`sell_price`] is 0. The game's own sell arm just refuses silently at
/// `0x1e364`, so this is the host's wording — a silent no-op on a tapped verb is the one
/// outcome 66차-C and 80차-B both wrote down as unacceptable.
pub const REFUSE_SELL: &str = "판매할 수 없는 아이템입니다.";

// ── the client's OWN deposit gates, applied by the host (96차) ──────────────
//
// A host 올리기 never passes through the game's popup arm (`0x6ea8`), so the refusals
// that arm applies have to be applied here or things the game never let into the
// warehouse would get in. Disassembled 2026-09-15 (LGT `0x6f4c`..`0x6fe6`), in order:
//
//   1. `canDeposit(cat, idx)` (`0x6d70`) == 0  →  「해당 아이템은 올릴 수 없습니다.」
//      A 19-entry jump table on the category (`0x5b7a0`, dumped 2026-09-15). Every arm
//      but two ends at `ldrb r0,[r3, #8]` — the item's PRICE LOW BYTE, out of that
//      category's own table at its own stride. The two exceptions answer 1 without
//      reading anything: cat 0x11 outright (`0x6dce: movs r0,#1`), and cat 0x12 for
//      `idx - 0xb <= 5` (`0x6dc6`).
//   2. equipment grade (cat <= 0x0c only — never a box category)
//   3. cat 0x0e && idx > 0x1d   →  「해당 조합 재료는 올릴 수 없습니다.」
//   4. cat 0x0f                 →  「조합 래시피 아이템은 올릴 수 없습니다.」
//   5. cat 0x11                 →  「퀘스트 아이템은 올릴 수 없습니다.」 (never boxed)
//
// The strings are the game's own (`0x5bce8` / `0x5bd40` / `0x5bd64` / `0x5bd8c`),
// misspelling included.

/// cat-0x12 rows `canDeposit` accepts without looking at the price.
pub const DEPOSIT_FREE_CONSUMABLES: core::ops::RangeInclusive<u16> = 0x0b..=0x10;
/// The one category whose `canDeposit` arm is a bare `movs r0,#1` — it is refused a step
/// later, by name (`퀘스트 아이템은 …`), which is the message the player should see.
pub const DEPOSIT_FREE_CAT: u16 = 0x11;
/// The last material (cat 0x0e) index the game lets into the warehouse.
pub const MATERIAL_DEPOSIT_IDX_MAX: u16 = 0x1d;

pub const REFUSE_DEPOSIT_PRICE: &str = "해당 아이템은 올릴 수 없습니다.";
pub const REFUSE_DEPOSIT_MATERIAL: &str = "해당 조합 재료는 올릴 수 없습니다.";
pub const REFUSE_DEPOSIT_RECIPE: &str = "조합 래시피 아이템은 올릴 수 없습니다.";
pub const REFUSE_DEPOSIT_QUEST: &str = "퀘스트 아이템은 올릴 수 없습니다.";

/// Why the game would refuse to deposit `(cat, idx)`, in the game's own words — or
/// `None` when it would let it through. ⛔ FAIL CLOSED: an item the table does not know
/// has no price to read, and the game reads one.
pub fn deposit_refusal(table: &[NameRow<'_>], cat: u16, idx: u16) -> Option<&'static str> {
    let free = cat == DEPOSIT_FREE_CAT || (cat == 0x12 && DEPOSIT_FREE_CONSUMABLES.contains(&idx));
    if !free && price_of(table, cat, idx).is_none_or(|p| p & 0xff == 0) {
        return Some(REFUSE_DEPOSIT_PRICE);
    }
    match cat {
        0x0e if idx > MATERIAL_DEPOSIT_IDX_MAX => Some(REFUSE_DEPOSIT_MATERIAL),
        0x0f => Some(REFUSE_DEPOSIT_RECIPE),
        0x11 => Some(REFUSE_DEPOSIT_QUEST),
        _ => None,
    }
}

/// The two rows of the 버리기 confirmation, in the game's own words.
pub const CONFIRM_ROWS: [&str; 2] = ["예", "아니오"];
pub const CONFIRM_YES: usize = 0;

/// `열기` in EUC-KR + NUL — the one popup row a bag-item cell offers (fits the game's
/// 10-byte label allocation with room to spare).
pub const LABEL_OPEN: [u8; 5] = [0xbf, 0xad, 0xb1, 0xe2, 0x00];

/// The bag-item popup shows exactly this many rows.
pub const POPUP_ROWS_OPEN: u8 = 1;

// ── THE GAME'S OWN ACQUISITION NOTICE (★★★ 103차, 2026-09-18) ───────────────
//
// The floating item name that rises over the player when a drop is picked up is a queue
// inside the inventory grid, and `giveItem` / `addItemToInventory` append to it right
// AFTER placing the item — the half our entry hooks replace. So an absorbed material,
// consumable or orb went into its box silently while equipment, which the boxes never
// take, still announced itself. That asymmetry is exactly what the player reported.
//
// The queue, all offsets from `grid` (`*(CHAR+0x9e28)`, the same base the bag records
// use). Read off the renderer at `0x54d28` and the reset at `0x53760`:
//
//   grid + 0x2b9  i8    how many entries are queued (0..5)
//   grid + 0x2ba  i8    the entry the quest tracker owns, -1 = none
//   grid + 0x2bc  u16   frame timer; the renderer pops the front entry every 0x1f frames
//   grid + 0x2be  5 x 0x1e   the texts, EUC-KR and NUL-terminated
//
// `0x53760` memsets exactly `0x96 = 5 * 0x1e` bytes at `0x2be`, which is where the five
// comes from. Appending is what the game does: write the name at `queue[count]`, then
// `count++`. The renderer draws entry `i` at `y - i*16` for `i < count`, and on each pop
// it clears the front slot, shifts the rest down and resets the timer to 0 — so the timer
// is always 0 while the queue is empty and an appended entry gets its full 0x1f frames.
//
// ⛔ The game writes at `queue[count]` and only THEN clamps (`0x53226`: increment while
// `count <= 4`), so at count 5 it writes 0x1e bytes over `grid+0x354`. Refuse instead:
// the sixth notice is not drawn either way, and scribbling past the queue is not a
// behaviour worth reproducing.
// ⛔ This is the guest's text buffer, so the name goes in as EUC-KR. A name that encodes
// to nothing is refused rather than queued empty — a blank line rising over the player
// would read as a glitch, and a missing one reads as "nothing dropped".

/// Where the queued notice texts start, relative to `grid`.
pub const NOTICE_TEXT_OFF: u32 = 0x2be;
/// Bytes per queued notice.
pub const NOTICE_STRIDE: u32 = 0x1e;
/// How many notices the queue holds (`0x53760` memsets `NOTICE_SLOTS * NOTICE_STRIDE`).
pub const NOTICE_SLOTS: i8 = 5;
/// The queued-notice count, relative to `grid`. Read `ldrsb` by the renderer.
pub const NOTICE_COUNT_OFF: u32 = 0x2b9;

/// Where an appended notice goes and what the count becomes — pure, `grid`-relative.
///
/// `count` is the byte at [`NOTICE_COUNT_OFF`] as the renderer reads it (signed), `text`
/// the already-encoded EUC-KR name. `None` = do not queue: the queue is full, the count
/// byte is not one the game could have written, or there is no text to show.
pub fn notice_plan(count: i8, text: &[u8]) -> Option<(u32, Vec<u8>, i8)> {
    if count < 0 || count >= NOTICE_SLOTS || text.is_empty() {
        return None;
    }
    let mut bytes = text.to_vec();
    bytes.truncate(truncate_euckr(text, NOTICE_STRIDE as usize - 1));
    if bytes.is_empty() {
        return None;
    }
    bytes.push(0);
    Some((NOTICE_TEXT_OFF + (count as u32) * NOTICE_STRIDE, bytes, count + 1))
}

/// The longest prefix of `text` that is at most `max` bytes and does not split a two-byte
/// EUC-KR pair — half a pair would print as the wrong syllable, not as a shorter name.
fn truncate_euckr(text: &[u8], max: usize) -> usize {
    let mut i = 0;
    while i < text.len() {
        let w = if text[i] >= 0x80 { 2 } else { 1 };
        if i + w > max {
            break;
        }
        i += w;
    }
    i
}
