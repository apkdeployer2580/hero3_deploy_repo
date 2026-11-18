// IN-GAME 오브결합 / 소켓확장 — the pure half.
//
// ── WHY THIS EXISTS (player request, 2026-08-25) ─────────────────────────────
// "네트워크 창고 말고 인게임에서 인벤토리 들어가서 오브결합 누르면 바로 아이템에 결합이
//  가능한거지. 장착중인 아이템에도 오브결합이나 소켓확장이 가능한거고."
//
// Until now socketing an orb meant a round trip through the network warehouse: the
// game only offers its 오브결합 row when the highlighted WAREHOUSE row's category byte
// is 0x10 (sole gate `0x7674 cmp r3,#0x10`), the target picker walks the same warehouse
// row array, and the mutation itself is done by the SERVER — the client just re-reads
// the result out of the following `05 3d` list. None of that can be re-pointed at the
// bag: the picker indexes `CHAR+0xa584 + i*20` directly and the bag tab's OK arm never
// even loads the pending-orb global. (Verified; do not try again.)
//
// What makes an in-game combine possible anyway is that the BAG INSTANCE CARRIES THE
// COMPLETE SOCKET STATE. The 60-byte inventory record holds the refine level, the
// socket count, the socketed orb id and the filled count, the save serialises exactly
// those for every equipment item, and the save LOADER re-derives the two cached numbers
// from them by calling the game's own `0x528f0`. So the host can do the whole operation
// with three record bytes plus the game's own two refresh helpers — no server, no wire
// record, no warehouse.
//
// ── DEPENDENCY-FREE ──────────────────────────────────────────────────────────
// `orbcombine_selftest.rs` at the repository root `include!`s THIS FILE, so it may not
// name anything outside `core`/`alloc`/`std`. ⛔ No `use crate::…`, no `//!` docs.
// Drawing and hit-testing live in `overlay.rs`; the guest reads/writes and the two
// guest calls live in `emulator.rs`.

// ── the inventory grid ───────────────────────────────────────────────────────
//
// `grid = *(CHAR + 0x9e28)` (ONE hop on LGT — `guest_build::container_data_off()` is
// `None` there), and `items.rs` already owns those two facts. ⛔ The three constants
// below are DELIBERATELY not re-declared here for that reason; the caller passes the
// resolved record address in.

/// Offset of the record block inside the grid container, for tab 0 / slot 0.
///
/// Proven by the address arithmetic every accessor repeats: `rec = grid + tab*0x3c4 +
/// slot*0x3c + 0xe5*4` (guest `0x131ec`-`0x132cc`, `0x52924`-`0x5293e`, `0x13a1c`-
/// `0x13a30`, `0x1a338`-`0x1a346`). `0x394 + 0x3c == 0x3d0`, so the record is exactly
/// one stride long and the first field of slot 1 begins where slot 0 ends.
pub const REC_BASE: u32 = 0x394;

/// Bytes per slot record, and per tab (bag). Mirrors `items.rs`'s `SLOT_STRIDE` /
/// `TAB_STRIDE`; kept here because this file may not reference that module.
pub const SLOT_STRIDE: u32 = 0x3c;
pub const TAB_STRIDE: u32 = 0x3c4;
/// Slots in one bag — a hard-coded loop bound in four places (`cmp #0xf; ble` at guest
/// `0x52a06`, `0x1a42e`, `0x3c202`, `0x55916`).
pub const SLOTS_PER_TAB: u8 = 16;
/// Bags the grid can hold. `0x394 + 5*0x3c4 == 0x1668`, which is exactly where the live
/// bag COUNT byte sits (`grid + 0x1668`, guest `0x529c2`) — i.e. the count byte is
/// parked immediately past the fifth tab block.
pub const MAX_TABS: u8 = 5;
/// The live bag count, as a signed byte at this offset from the grid base.
pub const GRID_BAG_COUNT: u32 = 0x1668;

// ── record fields, relative to the record base ───────────────────────────────
//
// Every one of these is proven by at least three independent guest sites: the deposit
// serialiser (bag -> 20-byte wire record), the save serialiser/deserialiser, and either
// the wire grant or the stat recompute. The citation trio is on each line.

/// `char[0x1e]`, EUC-KR, NUL-terminated — the name the game itself baked in. Rebuilt by
/// guest `0x5283c(tab, slot, cat, idx)`. (dest of the sprintf at `0x137a0` / `0x528cc`)
pub const REC_NAME: u32 = 0x000;
/// `u8` — 1 = the slot holds an item. (write `0x13572`; reads `0x529f8`, `0x4ce28`, `0x1a51a`)
pub const REC_OCCUPIED: u32 = 0x01f;
/// `s16` category. (`0x1359e` grant, `0x13a40` deposit, `0x1a52e` save)
pub const REC_CAT: u32 = 0x022;
/// `s16` item index within its category. (`0x135aa`, `0x13a4c`, `0x1a53c`)
pub const REC_IDX: u32 = 0x024;
/// `s16` stack count. ⚠ The wire copy of this byte is read SIGNED by the client
/// (`0x135da`), and the bag stack cap is 99 (`0x135e2 cmp #0x63`).
pub const REC_COUNT: u32 = 0x026;
/// `u64` — the item instance's UNIQUE KEY, the identity the warehouse itself is keyed on
/// (`0x13a32` copies it to wire[0..8]). ⚠ Two words, `+0x3bc` and `+0x3c0`.
///
/// ★★ THIS IS WHY IT IS READ AT ALL: `(tab, slot)` IS NOT A STABLE IDENTITY. The game
/// compacts the bag — `0x5380c` bubbles empty slots to the end by SWAPPING adjacent
/// 60-byte records, and its wrapper `0x55048` has seventeen call sites including the item
/// popup's own close path. So between the moment the player chooses 오브결합 and the
/// moment they pick a target, the records at both ends can have MOVED. A host that
/// re-validated only "is there still an orb here" would happily socket a different one.
/// The key travels with the record, so comparing it is what makes the commit safe.
pub const REC_KEY: u32 = 0x028;
/// `s8` — `-1` = not equipped, else the item is worn. The deposit path refuses a worn
/// item on exactly this test (`0x6f28`: `ldrsb`, `adds #1`, `beq ok`).
pub const REC_EQUIPPED: u32 = 0x030;
/// `s8` refine level (제련). (`0x1337c` grant, `0x13a68` deposit, `0x1a56c` save, and the
/// stat formula reads it at `0x52974`)
pub const REC_REFINE: u32 = 0x031;
/// `u8` TOTAL sockets (홈). (`0x13398` grant, `0x13a62` deposit, `0x1a574` save, dot
/// painter `0x125b2`)
pub const REC_SOCKETS: u32 = 0x032;
/// `s8` the socketed orb's index, `-1` = empty. (`0x133b6` grant, `0x13a74` deposit,
/// `0x1a582` save, stat formula `0x52942`)
pub const REC_ORB: u32 = 0x034;
/// `s8` FILLED socket count — a COUNT, not a mask. It is the `f` in the game's own
/// bonus formula `orbPower * (f*f + f) / 2` (`0x52950`-`0x52960`).
pub const REC_FILLED: u32 = 0x035;

/// `u16` DERIVED socket bonus, in percent. ⛔ A CACHE — never written by this module.
/// The game rebuilds it from the six carried bytes in `0x528f0` (`strh` at `0x52964`,
/// address computed as `rec + (0xf3 << 2)`). Absolute `rec+0x3cc`.
pub const REC_BONUS_PCT: u32 = 0x038;
/// `u16` DERIVED effective stat, the number the equip screen and the item detail print.
/// Same cache, same rebuild; absolute `rec+0x3ce`. Readers found by the `0xf3` immediate
/// scan: `0x4572c` / `0x45982` (overlay screen 17 = 장비, live `ldrsh [rec+0x3cc, #2]`),
/// `0x458ce` (the bonus line), plus the `0x4ce32`/`0x4cee2` stat getters.
pub const REC_EFFECTIVE: u32 = 0x03a;

/// Category of an orb. The only category the warehouse combine gate accepts, and the
/// one this feature keys on too.
pub const CAT_ORB: u16 = 0x10;
/// Equipment occupies categories **0x00..=0x0c**, and the lower end is not a guess:
/// `dump_item_db.py` shows `i0 cat=0x00 equip 투구` — helmets are the FIRST equipment
/// file, ahead of 갑옷 0x01 / 장갑 0x02 / 신발 0x03.
///
/// ⚠⚠ 71차-C fixed a real bug here: this said `1..=0x0c`, so **투구 never appeared in the
/// in-game picker at all** (`items::orb_targets` filters on [`Cell::is_equipment`], and a
/// filtered-out cell is not even listed with a reason). Player report 2026-08-26:
/// *"소켓확장오브 사용하려고하는데 목록에 투구는 안떠"*.
///
/// The two guest sites the old comment cited are **upper bounds only** — re-read them:
/// * target validator `0x7602 ldrb / lsls #0x18 / asrs #0x18` (sign-extend to s8) then
///   `0x7608 cmp r3,#0xc` / `0x760a bgt` → refuse. Nothing tests for zero.
/// * save serialiser `0x1a560 ldrsh r3,[r0,#2]` then `0x1a562 cmp r3,#0xc` / `bgt` →
///   skip the six socket bytes. Again no lower test, so a cat 0x00 record's sockets are
///   serialised like every other piece of gear's.
///
/// And the recompute we call handles it: `0x528f0` does `lsls r3,r4,#2` / `ldr r2,[r3,r2]`
/// — `dbTable[cat]`, indexed **directly, with no `cat - 1`** — and its only category split
/// is `subs r4,#4 / cmp r4,#6 / bhi` (i.e. weapons = 0x04..=0x0a), which puts 투구 on the
/// armour side along with 갑옷/장갑/신발/방패/혼.
///
/// ★ The offline engine and the python server never had this bug: both test
/// `cat > 0x0c` alone (`offline::stacks`, `hero3_protocol.EQUIP_CAT_MAX`), which is why
/// helmets always worked through the warehouse path and only the 69차 in-game path lost
/// them. Keep all three spellings meaning the same set.
pub const CAT_EQUIP_MIN: u16 = 0x00;
pub const CAT_EQUIP_MAX: u16 = 0x0c;
/// The pseudo-orb that ADDS a socket instead of filling one — the 소켓확장 cash item,
/// delivered as `(cat 0x10, idx 0x0f)` so the game's own picker opens on it (44차).
pub const IDX_SOCKET_EXPAND: u16 = 0x0f;
/// Highest index that is a REAL orb, i.e. one that may be written into `rec+0x3c8`.
///
/// ⛔⛔ THE HOST HAS TO BOUND THIS ITSELF. The game's own bonus formula indexes the orb
/// table with NO range check at all — `0x52942 ldrsb r2,[r0] / movs r3,#0x18 / muls /
/// adds r3,r3,r6 / 0x5294c ldrsb r4,[r3,#0x15]` — and the SAVE LOADER re-runs that same
/// unchecked function on every launch, so an out-of-range id is not a transient glitch,
/// it is reproduced forever. The table has 16 rows (`dump_item_db.py --cat 16` -> rec=16)
/// and row `0x0f` is 소켓확장, which has no stat of its own — so a socketable orb is
/// `0x00..=0x0e`.
pub const IDX_ORB_MAX: u16 = 0x0e;
/// Sockets one item may have. The item's own description says 최대 5소켓, and the
/// offline/server engines cap there too (`offline::MAX_SOCKETS`).
pub const MAX_SOCKETS: u8 = 5;

/// `오브결합`, EUC-KR, NUL-terminated — what the host writes into the game's own popup
/// label buffer so the row can be read in the game's font, in the game's window.
///
/// 9 bytes into a 10-byte allocation (`movs r0,#0xa` at guest `0x3fa46`). The existing
/// `퀵슬롯등록` rewrite deliberately spends 11; this one fits.
pub const LABEL_ORB_COMBINE: [u8; 9] = [0xbf, 0xc0, 0xba, 0xea, 0xb0, 0xe1, 0xc7, 0xd5, 0x00];

/// Popup tag of the plain-item context menu — the TWO-row 상세보기 / 버리기 one that a
/// bag orb opens. ⚠ NOT the 0x66 tag the 51차 퀵슬롯등록 rewrite targets: the inventory
/// popup builder sends every category except 0x12 down this branch (`0x3fac4 cmp
/// r3,#0x11; ble`), and `popup_choose` dispatches tag 0x64 for rows 0 and 1 ONLY
/// (`0x47590`-`0x4759e`), falling through to the epilogue for anything else.
///
/// That fall-through is what makes a host-owned third row safe: on OK the game runs
/// nothing, then closes and frees the popup unconditionally (`0x47b3a`-`0x47b54`).
pub const POPUP_TAG_PLAIN: u8 = 0x64;
/// Rows the game puts on that popup, and the row count we grow it to.
pub const POPUP_ROWS_PLAIN: i8 = 2;
pub const POPUP_ROWS_OURS: i8 = 3;
/// The display row our label occupies — one past the game's own last row.
pub const POPUP_ROW_COMBINE: i8 = 2;

/// Address of the record for `(tab, slot)` in a grid whose base is `grid`.
///
/// `None` for an out-of-range cell rather than a clamped one: the caller writes through
/// this, and a clamp would silently retarget the write at a real item. (66차 learned the
/// same lesson about the cell cursor — see `items::item_grid_action`.)
pub fn rec_addr(grid: u32, tab: u8, slot: u8) -> Option<u32> {
    if tab >= MAX_TABS || slot >= SLOTS_PER_TAB {
        return None;
    }
    Some(grid + REC_BASE + u32::from(tab) * TAB_STRIDE + u32::from(slot) * SLOT_STRIDE)
}

/// What a `(cat 0x10, idx)` orb does when it is combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A real orb: fills one empty socket and becomes the item's orb.
    Socket,
    /// 소켓확장: adds one socket and touches nothing else.
    Expand,
}

pub fn kind_of(orb_idx: u16) -> Kind {
    if orb_idx == IDX_SOCKET_EXPAND { Kind::Expand } else { Kind::Socket }
}

/// One inventory cell, as the worker read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub tab: u8,
    pub slot: u8,
    /// The instance's own identity — see [`REC_KEY`]. Compared at commit time, because
    /// the cell it was read from can move in between.
    pub key: u64,
    pub cat: u16,
    pub idx: u16,
    pub count: u16,
    pub refine: i8,
    pub sockets: u8,
    pub orb: i8,
    pub filled: i8,
    /// `REC_EQUIPPED != -1` — the item is being worn right now.
    pub equipped: bool,
    /// The game's own baked name, EUC-KR bytes without the terminator.
    pub name: Vec<u8>,
}

impl Cell {
    pub fn is_equipment(&self) -> bool {
        (CAT_EQUIP_MIN..=CAT_EQUIP_MAX).contains(&self.cat)
    }
    pub fn is_orb(&self) -> bool {
        self.cat == CAT_ORB
    }
    /// Is this the same ITEM INSTANCE as `other`, wherever either has moved to?
    ///
    /// Identity is the key alone. A key of 0 is treated as no identity at all and never
    /// matches: an empty or half-initialised record reads as zeros, and "two things that
    /// are both nothing" must not be a match — that is the shape a fail-open bug takes.
    pub fn same_item(&self, key: u64) -> bool {
        self.key != 0 && self.key == key
    }

    /// Filled sockets, clamped into a sane range. The two bytes are independent on the
    /// bag side — nothing in the game cross-checks `filled <= total` — so every reader
    /// here treats a nonsense pair as "no room" rather than trusting it.
    pub fn filled_u8(&self) -> u8 {
        if self.filled < 0 { 0 } else { self.filled as u8 }
    }
}

/// Why a target cannot take this orb. Each maps to one line of player-facing text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Not equipment at all (a potion, another orb, a quest item).
    NotEquipment,
    /// Equipment with no sockets — the game gives sockets at drop time (`0x52cf0`
    /// rolls 1..5) and plenty of items roll zero.
    NoSockets,
    /// Every socket is already filled.
    NoRoom,
    /// Already at [`MAX_SOCKETS`]; 소켓확장 cannot add a sixth.
    SocketCap,
    /// The record's own socket bytes disagree with each other. Nothing in the game
    /// cross-checks them, so this is either a drifted offset or a save that is already
    /// damaged — and both are reasons to leave the item alone rather than write to it.
    Corrupt,
    /// The item is being WORN. See [`eligible`] for why that is refused rather than
    /// allowed with a caveat.
    Worn,
}

impl Refusal {
    /// The message the host shows. Plain Rust text — `gamefont` renders it in the game's
    /// own font, so these must stay inside KS X 1001 (see `gamefont_selftest.rs`).
    pub fn text(self) -> &'static str {
        match self {
            Refusal::NotEquipment => "장비가 아닙니다.",
            Refusal::NoSockets => "홈이 없는 장비입니다.",
            Refusal::NoRoom => "빈 홈이 없습니다.",
            Refusal::SocketCap => "소켓이 최대입니다.",
            Refusal::Corrupt => "장비 정보가 올바르지 않습니다.",
            Refusal::Worn => "장착을 해제해야 합니다.",
        }
    }
}

/// Do this record's own socket bytes agree with each other?
///
/// The same three clauses [`eligible`] refuses on, without the worn/room questions —
/// 71차-F needs exactly this and nothing else: it re-runs the game's derived-field
/// rebuild over every equipment record, and ⛔ **69차's rule is that a self-contradicting
/// record is the one thing not to touch** (its §3, "모순된 레코드는 손대지 말 것"). The
/// rebuild indexes the orb table with the id UNCHECKED (`0x52942`), so feeding it a
/// record whose `filled >= 1` sits next to `orbId == -1` reads a byte from before the
/// table. The save loader would do the same on the next load — but that is the game's
/// own doing, and there is no reason for us to reach it more often.
pub fn consistent(c: &Cell) -> bool {
    c.sockets <= MAX_SOCKETS && c.filled_u8() <= c.sockets && !(c.orb < 0 && c.filled_u8() > 0)
}

/// May `orb` go into `target`?
///
/// Mirrors the offline engine's `orb_combine` (`offline.rs`) rule for rule, because the
/// two paths must not disagree about what a legal socket is — the same item can be
/// combined in the warehouse today and in the bag tomorrow.
pub fn eligible(target: &Cell, kind: Kind) -> Result<(), Refusal> {
    if !target.is_equipment() {
        return Err(Refusal::NotEquipment);
    }
    // ★★ UNEQUIPPED ONLY (2026-08-25, player decision: "그럼 2번에서 해제된 장비에만 오브
    // 결합이 되도록해야겠네"), and with this gate the feature has NO lag at all.
    //
    // The reason it was ever a question: `rec+0x3ce` is the ITEM's power and `0x528f0`
    // rebuilds it here and now, but the CHARACTER's attack is a SECOND cache at
    // `CHAR+0x190`/`+0x192` that only `0x4f014` writes — so socketing something already
    // worn left the character's number behind until that ran, and `0x4f014` takes six
    // arguments (two on the stack, unexplained) and zeroes the derived-stat block on
    // entry, so calling it from the host was never acceptable.
    //
    // Refusing the worn case removes the problem instead of papering over it: the player
    // unequips, sockets (the item's own numbers update at once), and EQUIPS — and the
    // equip action runs the full recompute itself. Proven, not assumed: the literal for
    // the equip routine (`0x4dc59`) and the literal for `0x4f014` (`0x4f015`) sit EIGHT
    // BYTES APART in one literal pool, at `0x47880` and `0x47888`, i.e. the item-action
    // dispatcher loads both. So equipping is exactly the event that picks a host-written
    // socket up.
    //
    // ⛔ Do not relax this to "allow it with a toast". That was the shipped-for-an-hour
    // version and the honest read of it is that the feature would sometimes appear to do
    // nothing.
    if target.equipped {
        return Err(Refusal::Worn);
    }
    // ★★ THE CONSISTENCY GATE, and every clause of it is a real failure mode. Nothing in
    // the game cross-checks these three bytes — the socket-pip painter and the bonus
    // formula both consume them unvalidated — so an item whose own numbers disagree is
    // either a drifted offset on our side or a save that is already damaged. Either way
    // it is the last thing a host write should touch.
    //
    // The third clause is the sharp one: the bonus formula multiplies the orb id by 0x18
    // into the orb table with NO range check (`0x52944`), and `orbId == -1` is survivable
    // today only because `filled` is 0 alongside it, which makes `f*(f+1)/2` zero and the
    // out-of-array byte irrelevant. Leave `filled >= 1` next to `orbId == -1` and the
    // multiplier becomes a byte from before the table — reproduced deterministically on
    // every load, because the load path re-runs the same unchecked function.
    if target.sockets > MAX_SOCKETS || target.filled_u8() > target.sockets || (target.orb < 0 && target.filled_u8() > 0) {
        return Err(Refusal::Corrupt);
    }
    match kind {
        Kind::Expand => {
            if target.sockets >= MAX_SOCKETS {
                return Err(Refusal::SocketCap);
            }
            Ok(())
        }
        Kind::Socket => {
            if target.sockets == 0 {
                return Err(Refusal::NoSockets);
            }
            if target.filled_u8() >= target.sockets {
                return Err(Refusal::NoRoom);
            }
            Ok(())
        }
    }
}

/// The record bytes the combine changes, as `(absolute address, value)`.
///
/// Returns an empty list — never a partial one — when the pair is not eligible, so a
/// caller that forgets to check cannot half-apply it.
///
/// ⛔ Note what is NOT here: the two DERIVED halfwords at `rec+0x3cc` (socket bonus %)
/// and `rec+0x3ce` (effective stat). They are a cache, and the game rebuilds them from
/// exactly these bytes by calling `0x528f0(tab, slot, cat, idx)` — which is what the
/// save loader does after restoring the same six bytes (`0x1a3f4`). Recomputing them
/// here would mean re-deriving the orb power table and the per-category item DB in the
/// host and hoping the two arithmetics never drift; calling the game's own function
/// cannot drift. Same for the name at `rec+0x394` and `0x5283c`.
pub fn writes(rec: u32, target: &Cell, kind: Kind, orb_idx: u16) -> Vec<(u32, u8)> {
    if eligible(target, kind).is_err() {
        return Vec::new();
    }
    match kind {
        Kind::Expand => vec![(rec + REC_SOCKETS, target.sockets + 1)],
        // ⛔ An orb whose index is outside the real rows writes nothing — see
        // [`IDX_ORB_MAX`]. `kind_of` already routes `0x0f` to `Expand`, so this catches a
        // bag record that is damaged or a build whose orb table is not the one we think.
        Kind::Socket if orb_idx <= IDX_ORB_MAX => vec![
            (rec + REC_ORB, orb_idx as u8),
            (rec + REC_FILLED, target.filled_u8() + 1),
        ],
        Kind::Socket => Vec::new(),
    }
}

/// The writes that spend exactly ONE unit out of the orb stack whose record starts at
/// `orb_rec`, given its current stack count.
///
/// ★★ ONE UNIT, NOT THE ROW. A bag cell holds a STACK — ten of the same orb live in one
/// cell — and the warehouse path already learned this the hard way (player, 2026-08-25:
/// "A오브 10개를 네트워크 창고에 넣고서 그중 1개를 장비에 결합하면 A오브 10개가 전부
/// 사라지는거야"; see `offline::ITEM_COUNT_OFF`). ⛔ Do not "simplify" this to clearing
/// the cell.
///
/// On the last unit the cell is freed by clearing the OCCUPANCY byte and zeroing the
/// count, which is all the game itself needs: every reader of a bag cell gates on
/// `rec+0x3b3` first — the painter (`0x4ce28`), the free-slot/merge scan (`0x529f8`), and
/// the save serialiser (`0x1a51a`). The stale `(cat, idx)` left behind is unreachable for
/// exactly that reason.
///
/// ⛔ The cell is NOT compacted and the quickbar is NOT touched. Compaction would move
/// other items under the player's cursor, and an orb cannot be quickbar-bound in the
/// first place — the game routes every category except 0x12 to the two-row popup that has
/// no 키등록 rows ([`POPUP_TAG_PLAIN`]).
pub fn spend_one(orb_rec: u32, count: u16) -> Vec<(u32, Vec<u8>)> {
    if count > 1 {
        return vec![(orb_rec + REC_COUNT, (count - 1).to_le_bytes().to_vec())];
    }
    vec![
        (orb_rec + REC_COUNT, 0u16.to_le_bytes().to_vec()),
        (orb_rec + REC_OCCUPIED, vec![0]),
    ]
}

/// The line the host shows after a combine.
///
/// A `&'static str` rather than a format, because `UserEvent::Notice` carries one — the
/// same constraint `buffs::box_reward_notice` works under, and for the same reason: a
/// notice is raised on the worker and read on the UI thread (and, since 140차, shown as
/// the game-style message box rather than a toast).
///
/// ★ There is no "…but the stats will catch up later" variant any more, and removing the
/// need for one is the whole point of the worn gate in [`eligible`]: an unequipped item's
/// own numbers are correct the moment `0x528f0` returns, and equipping it is what
/// recomputes the character's.
pub fn done_text(kind: Kind) -> &'static str {
    match kind {
        Kind::Expand => "소켓을 늘렸습니다.",
        Kind::Socket => "오브를 결합했습니다.",
    }
}

/// Shown when a write or the re-validation refused. The orb is NOT spent: the whole
/// mutation is planned before any of it is applied, and the orb is the last thing in it.
pub const FAILED: &str = "결합에 실패했습니다.";

/// Nothing in the bag can take this orb.
pub const NO_TARGET: &str = "결합할 수 있는 장비가 없습니다.";

/// Dialog title for each kind.
pub fn title(kind: Kind) -> &'static str {
    match kind {
        Kind::Expand => "소켓확장",
        Kind::Socket => "오브결합",
    }
}

/// One row of the target list, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Index into the candidate list the worker built.
    pub cell: usize,
    /// The item's name as the game spells it (already decoded by the caller).
    pub name: String,
    /// `홈 2/3`, plus `+5` when refined and `[장착]` when worn.
    pub detail: String,
    /// A row the player may pick. Ineligible targets are still LISTED — with the reason
    /// in `detail` — because "my gear is not in the list" is indistinguishable from a
    /// broken feature, while "홈이 없는 장비입니다" explains itself.
    ///
    /// ★ 70차 (player request 2026-08-25): WORN gear is listed again — 69차 hid it
    /// (`items::orb_targets` filtered it out), and the player asked for the opposite:
    /// *"착용중인 장비에 사용하면 confirm팝업이 뜨도록 조치"*. So a worn row shows with
    /// its `[장착]` marker, disabled, and CONFIRMING it pops the game-style message box
    /// with [`Row::notice`]. ⛔ Display only, in both directions: `orb_apply` re-refuses
    /// through [`eligible`] whatever the list showed.
    pub enabled: bool,
    /// What the game-style message box says when the player tries to CONFIRM this row
    /// anyway. `None` for an eligible row.
    pub notice: Option<String>,
}

/// Build the display row for one candidate.
pub fn row(cell: usize, c: &Cell, name: String, kind: Kind, orb_idx: u16) -> Row {
    let mut detail = format!("홈 {}/{}", c.filled_u8(), c.sockets);
    if c.refine > 0 {
        detail.push_str(&format!("  +{}", c.refine));
    }
    if c.equipped {
        detail.push_str("  [장착]");
    }
    // ★ AN ITEM HAS ONE ORB, NOT ONE PER HOLE (`rec+0x3c8` is a single byte). Socketing a
    // DIFFERENT kind retypes every hole at once — that is the game's own behaviour, not a
    // regression (43차-B rolled back an attempt to sum them and the reason is structural:
    // the save carries one id and the loader recomputes from it). But in the warehouse it
    // took a round trip to get here, and now it is one keypress — so the row has to say so
    // before the player spends the orb.
    if kind == Kind::Socket && c.filled_u8() > 0 && c.orb >= 0 && u16::from(c.orb as u8) != orb_idx {
        detail.push_str("  기존 오브 교체");
    }
    let (enabled, notice) = match eligible(c, kind) {
        Ok(()) => (true, None),
        Err(r) => {
            detail.push_str("  ");
            detail.push_str(r.text());
            (false, Some(r.text().to_string()))
        }
    };
    Row { cell, name, detail, enabled, notice }
}

// ── the list VIEWPORT (2026-09-14) ──────────────────────────────
//
// ★★ THE SCROLL POSITION IS PIXELS NOW, NOT A ROW INDEX (user, 2026-09-14: *"우측에
// 버튼으로 스크롤 내리는 형식에서 드래그로 내리는 형식으로 수정"*). The ▲/▼ column on the
// right of the list is gone: a finger DRAGS the list, and a gamepad moves the selection
// with the left-hand direction keys while the list follows. Both need a position that can
// sit BETWEEN rows, which `scroll_top`'s "first visible index" could not express.
//
// ⛔ The old row-window helper is deliberately NOT kept beside these. Two scroll models
// on one list is how a drag and a key end up disagreeing about what is on screen.

/// How far the list may be scrolled, in pixels. `0` when everything fits.
pub fn scroll_span(len: usize, row_h: u32, view_h: u32) -> f64 {
    (len as f64 * f64::from(row_h) - f64::from(view_h)).max(0.0)
}

/// A scroll position the viewport can actually be at. ⛔ EVERY write to the position goes
/// through this — a drag can otherwise throw the list off either end and leave the window
/// showing nothing at all, with no control left on screen to bring it back.
pub fn clamp_scroll(px: f64, len: usize, row_h: u32, view_h: u32) -> f64 {
    if !px.is_finite() {
        return 0.0;
    }
    px.clamp(0.0, scroll_span(len, row_h, view_h))
}

/// The SMALLEST move that brings row `sel` fully into view — what a direction key does.
/// A row already on screen moves the list not at all, which is what stops arrowing down a
/// long list from jumping the page under the player's eyes.
pub fn scroll_to_show(px: f64, sel: usize, len: usize, row_h: u32, view_h: u32) -> f64 {
    let (top, h, view) = (sel as f64 * f64::from(row_h), f64::from(row_h), f64::from(view_h));
    let want = if px > top {
        top
    } else if px + view < top + h {
        top + h - view
    } else {
        px
    };
    clamp_scroll(want, len, row_h, view_h)
}

/// The row under a point `dy` pixels below the viewport's TOP EDGE, or `None` past the end
/// of the list — the tail of a short list is empty parchment, not row `len - 1`.
pub fn row_at(px: f64, dy: f64, len: usize, row_h: u32) -> Option<usize> {
    if row_h == 0 || dy < 0.0 {
        return None;
    }
    let i = ((px.max(0.0) + dy) / f64::from(row_h)) as usize;
    (i < len).then_some(i)
}

/// `(first row with a pixel in the viewport, how far its top sits ABOVE the viewport's top
/// edge)`. The offset is always `< row_h`, which is what lets the dialog draw from
/// `list.y - offset` and still stay inside its own panel — see `overlay::draw_orb_dialog`.
pub fn first_visible(px: f64, row_h: u32) -> (usize, u32) {
    if row_h == 0 {
        return (0, 0);
    }
    let px = px.max(0.0);
    let i = (px / f64::from(row_h)) as usize;
    (i, (px - i as f64 * f64::from(row_h)) as u32)
}

/// Move the selection by `step`, skipping nothing — a disabled row can be highlighted so
/// the player can read WHY it is disabled. Saturates at both ends rather than wrapping:
/// the list can be long, and a wrap at the bottom of a long list is disorienting.
pub fn step_sel(sel: usize, len: usize, step: i32) -> usize {
    if len == 0 {
        return 0;
    }
    let last = (len - 1) as i32;
    (sel as i32 + step).clamp(0, last) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A consistent equipment record: an item with filled holes also has an orb in it,
    /// because [`eligible`] now refuses the pair that disagrees (and the game's own bonus
    /// formula reads past its table on exactly that pair).
    fn equip(sockets: u8, filled: i8) -> Cell {
        Cell {
            tab: 0,
            slot: 0,
            key: 0x1234_5678_9abc_def0,
            cat: 0x04,
            idx: 0x07,
            count: 1,
            refine: 0,
            sockets,
            orb: if filled > 0 { 3 } else { -1 },
            filled,
            equipped: false,
            name: b"sword".to_vec(),
        }
    }

    #[test]
    fn the_record_is_exactly_one_stride_long() {
        // Every field must land inside the 0x3c bytes the stride gives it, or a write
        // aimed at slot N lands in slot N+1.
        for off in [REC_NAME, REC_OCCUPIED, REC_CAT, REC_IDX, REC_COUNT, REC_EQUIPPED, REC_REFINE, REC_SOCKETS, REC_ORB, REC_FILLED] {
            assert!(off < SLOT_STRIDE, "{off:#x} is past the record");
        }
        // …and the name buffer must not run into the first byte after it.
        assert!(REC_NAME + 0x1e <= REC_OCCUPIED - 1);
        // The tab block holds its 16 slots and the count byte sits right after it.
        assert_eq!(REC_BASE + u32::from(MAX_TABS) * TAB_STRIDE, GRID_BAG_COUNT);
        assert!(u32::from(SLOTS_PER_TAB) * SLOT_STRIDE <= TAB_STRIDE);
    }

    #[test]
    fn addresses_match_the_guests_own_arithmetic() {
        let grid = 0x1000_0000;
        assert_eq!(rec_addr(grid, 0, 0), Some(grid + 0x394));
        assert_eq!(rec_addr(grid, 0, 1), Some(grid + 0x394 + 0x3c));
        assert_eq!(rec_addr(grid, 1, 0), Some(grid + 0x394 + 0x3c4));
        assert_eq!(rec_addr(grid, 2, 5), Some(grid + 0x394 + 2 * 0x3c4 + 5 * 0x3c));
        // ⛔ Refused, not clamped.
        assert_eq!(rec_addr(grid, MAX_TABS, 0), None);
        assert_eq!(rec_addr(grid, 0, SLOTS_PER_TAB), None);
    }

    #[test]
    fn a_worn_item_is_refused_outright() {
        let mut c = equip(3, 0);
        assert_eq!(eligible(&c, Kind::Socket), Ok(()));
        assert_eq!(eligible(&c, Kind::Expand), Ok(()));
        c.equipped = true;
        // Both kinds, and the refusal comes BEFORE anything else could accept it.
        assert_eq!(eligible(&c, Kind::Socket), Err(Refusal::Worn));
        assert_eq!(eligible(&c, Kind::Expand), Err(Refusal::Worn));
        assert!(writes(0x100, &c, Kind::Socket, 3).is_empty(), "and it writes nothing");
        assert!(writes(0x100, &c, Kind::Expand, IDX_SOCKET_EXPAND).is_empty());
        // The row still LISTS it, with the reason — "my sword is missing" is worse.
        let r = row(0, &c, "검".into(), Kind::Socket, 3);
        assert!(!r.enabled);
        assert!(r.detail.contains("[장착]") && r.detail.contains(Refusal::Worn.text()), "{}", r.detail);
        // ★ 70차: confirming a worn row pops the game-style message box — the row
        // carries the box's text, and an eligible row carries none.
        assert_eq!(r.notice.as_deref(), Some(Refusal::Worn.text()));
        c.equipped = false;
        assert_eq!(row(0, &c, "검".into(), Kind::Socket, 3).notice, None);
    }

    #[test]
    fn only_equipment_can_be_a_target() {
        let mut c = equip(3, 0);
        // ⛔ 0x00 is NOT in this list — it is 투구. See `CAT_EQUIP_MIN`.
        for cat in [0x0du16, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x99, 0xffff] {
            c.cat = cat;
            assert_eq!(eligible(&c, Kind::Socket), Err(Refusal::NotEquipment), "cat {cat:#x}");
            assert_eq!(eligible(&c, Kind::Expand), Err(Refusal::NotEquipment), "cat {cat:#x}");
        }
        for cat in CAT_EQUIP_MIN..=CAT_EQUIP_MAX {
            c.cat = cat;
            assert_eq!(eligible(&c, Kind::Socket), Ok(()), "cat {cat:#x}");
        }
    }

    /// ★ 71차-C regression: 투구 is `cat 0x00` and must be a legal target. It was excluded
    /// for two 회차 because `CAT_EQUIP_MIN` was 1, and the symptom was not an error message
    /// — helmets simply were not in the list (`items::orb_targets` drops non-equipment
    /// before the list is built, so there was no greyed row to explain itself).
    /// ★ 71차-F: `consistent` must agree with `eligible`'s corrupt clause exactly — the
    /// bag-wide refresh uses it to decide what it may hand to the game's rebuild.
    #[test]
    fn consistency_matches_the_corrupt_clause() {
        for sockets in 0..=7u8 {
            for filled in -2i8..=7 {
                for orb in [-1i8, 0, 7] {
                    let mut c = equip(sockets, filled);
                    c.orb = orb;
                    let corrupt = eligible(&c, Kind::Expand) == Err(Refusal::Corrupt);
                    assert_eq!(consistent(&c), !corrupt, "{c:?}");
                }
            }
        }
    }

    #[test]
    fn a_helmet_is_equipment() {
        let mut c = equip(3, 1);
        c.cat = 0x00;
        assert!(c.is_equipment(), "cat 0x00 is 투구 — the first equipment file (i0_dat)");
        assert_eq!(eligible(&c, Kind::Socket), Ok(()));
        assert_eq!(eligible(&c, Kind::Expand), Ok(()));
        // …and a socket write really is emitted for it, not just permitted.
        assert!(!writes(0x1234_0000, &c, Kind::Expand, IDX_SOCKET_EXPAND).is_empty());
        // The whole 0x00..=0x0c span is the game's own `cat <= 0x0c` gate.
        assert_eq!(CAT_EQUIP_MIN, 0x00);
        assert_eq!(CAT_EQUIP_MAX, 0x0c);
    }

    #[test]
    fn a_socket_needs_an_empty_hole() {
        assert_eq!(eligible(&equip(0, 0), Kind::Socket), Err(Refusal::NoSockets));
        assert_eq!(eligible(&equip(3, 3), Kind::Socket), Err(Refusal::NoRoom));
        assert_eq!(eligible(&equip(3, 2), Kind::Socket), Ok(()));
        // A nonsense record is refused, never "fixed" — a write derived from garbage is
        // how a save gets corrupted.
        assert_eq!(eligible(&equip(MAX_SOCKETS + 1, 0), Kind::Socket), Err(Refusal::Corrupt), "more holes than the cap");
        assert_eq!(eligible(&equip(2, 9), Kind::Socket), Err(Refusal::Corrupt), "more filled than holes");
        assert_eq!(eligible(&equip(2, -3), Kind::Socket), Ok(()), "a negative count reads as zero filled");
        // ★ …and the shape that makes the game's own bonus formula read past its orb
        // table: holes reported filled with no orb in them.
        let mut ghost = equip(3, 2);
        ghost.orb = -1;
        assert_eq!(eligible(&ghost, Kind::Socket), Err(Refusal::Corrupt), "filled holes with no orb");
        assert_eq!(eligible(&ghost, Kind::Expand), Err(Refusal::Corrupt), "…for expansion too");
    }

    #[test]
    fn expansion_stops_at_five() {
        for n in 0..MAX_SOCKETS {
            assert_eq!(eligible(&equip(n, 0), Kind::Expand), Ok(()), "{n} sockets");
        }
        assert_eq!(eligible(&equip(MAX_SOCKETS, 0), Kind::Expand), Err(Refusal::SocketCap));
        assert_eq!(eligible(&equip(MAX_SOCKETS + 3, 0), Kind::Expand), Err(Refusal::Corrupt), "past the cap is corrupt");
        // Expansion does not care whether the existing sockets are full.
        assert_eq!(eligible(&equip(2, 2), Kind::Expand), Ok(()));
    }

    #[test]
    fn the_writes_are_the_three_bytes_and_nothing_else() {
        let rec = 0x2000_0000;
        let c = equip(3, 1);
        let w = writes(rec, &c, Kind::Socket, 0x07);
        assert_eq!(w, vec![(rec + REC_ORB, 0x07), (rec + REC_FILLED, 2)]);
        let w = writes(rec, &c, Kind::Expand, IDX_SOCKET_EXPAND);
        assert_eq!(w, vec![(rec + REC_SOCKETS, 4)]);
        // ⛔ Never the derived cache — the game's own 0x528f0 owns those two halfwords.
        for (a, _) in writes(rec, &c, Kind::Socket, 0x07) {
            assert!(a - rec < 0x36, "{a:#x} is outside the six bytes the save carries");
        }
    }

    #[test]
    fn an_ineligible_pair_produces_no_partial_write() {
        let rec = 0x2000_0000;
        assert!(writes(rec, &equip(3, 3), Kind::Socket, 1).is_empty(), "full item");
        assert!(writes(rec, &equip(0, 0), Kind::Socket, 1).is_empty(), "no sockets");
        assert!(writes(rec, &equip(MAX_SOCKETS, 0), Kind::Expand, IDX_SOCKET_EXPAND).is_empty(), "at the cap");
        let mut potion = equip(3, 0);
        potion.cat = 0x12;
        assert!(writes(rec, &potion, Kind::Socket, 1).is_empty(), "not equipment");
    }

    #[test]
    fn socket_expansion_is_told_apart_by_its_index() {
        assert_eq!(kind_of(IDX_SOCKET_EXPAND), Kind::Expand);
        for idx in 0..IDX_SOCKET_EXPAND {
            assert_eq!(kind_of(idx), Kind::Socket, "orb {idx:#x}");
        }
        // ★ And the two must never share a write set: 소켓확장 must not become the item's
        // orb (it has no stat of its own, so it would zero the bonus).
        let c = equip(3, 1);
        let rec = 0x100;
        assert!(!writes(rec, &c, Kind::Expand, IDX_SOCKET_EXPAND).iter().any(|(a, _)| *a == rec + REC_ORB));
    }

    #[test]
    fn the_label_fits_the_guests_own_buffer() {
        assert_eq!(LABEL_ORB_COMBINE.len(), 9, "the popup allocates 10 bytes per row");
        assert_eq!(*LABEL_ORB_COMBINE.last().unwrap(), 0, "the game prints it as a C string");
        assert!(LABEL_ORB_COMBINE[..8].iter().all(|b| *b >= 0x80), "EUC-KR, two bytes per syllable");
        assert_eq!(POPUP_ROWS_OURS, POPUP_ROWS_PLAIN + 1);
        assert_eq!(POPUP_ROW_COMBINE, POPUP_ROWS_PLAIN, "our row is one past the game's last");
    }

    /// ⛔ `(tab, slot)` is not an identity — the game compacts the bag. The commit has to
    /// match the KEY or it can socket an item the player never chose.
    #[test]
    fn identity_is_the_key_and_a_zero_key_is_no_identity() {
        let c = equip(3, 0);
        assert!(c.same_item(0x1234_5678_9abc_def0));
        assert!(!c.same_item(0x1234_5678_9abc_def1), "one bit apart is a different item");
        assert!(!c.same_item(0), "an empty record must not match anything");
        let mut blank = equip(3, 0);
        blank.key = 0;
        assert!(!blank.same_item(0), "…and must not match another empty record either");
        assert!(!blank.same_item(0x1234_5678_9abc_def0));
    }

    #[test]
    fn spending_takes_one_unit_and_frees_only_the_last() {
        let rec = 0x4000;
        // A stack loses one unit and keeps its cell.
        assert_eq!(spend_one(rec, 10), vec![(rec + REC_COUNT, vec![9, 0])]);
        assert_eq!(spend_one(rec, 2), vec![(rec + REC_COUNT, vec![1, 0])]);
        // The last unit frees the cell — occupancy is the byte every reader gates on.
        let last = spend_one(rec, 1);
        assert!(last.contains(&(rec + REC_OCCUPIED, vec![0])), "{last:?}");
        assert!(last.contains(&(rec + REC_COUNT, vec![0, 0])), "{last:?}");
        // A cell that already reads zero is treated as the last unit, never as -1.
        let zero = spend_one(rec, 0);
        assert!(zero.contains(&(rec + REC_OCCUPIED, vec![0])), "{zero:?}");
        assert!(zero.iter().all(|(_, v)| v.iter().all(|b| *b != 0xff)), "no wrap to 0xffff");
        // ⛔ Nothing outside the two fields.
        for (a, _) in spend_one(rec, 5).into_iter().chain(spend_one(rec, 1)) {
            assert!([rec + REC_COUNT, rec + REC_OCCUPIED].contains(&a), "{a:#x}");
        }
    }

    #[test]
    fn a_row_explains_itself() {
        let mut c = equip(3, 1);
        c.refine = 5;
        // `equip()` puts orb 3 in a filled item, so combining ANOTHER 3 is not a swap.
        let r = row(0, &c, "검".into(), Kind::Socket, 3);
        assert!(r.enabled);
        assert!(r.detail.contains("홈 1/3") && r.detail.contains("+5"), "{}", r.detail);
        // Worn is still SHOWN — and shown as unusable, with the reason.
        let mut worn = c.clone();
        worn.equipped = true;
        let r = row(0, &worn, "검".into(), Kind::Socket, 3);
        assert!(!r.enabled);
        assert!(r.detail.contains("[장착]"), "{}", r.detail);
        assert!(!r.detail.contains("교체"), "same orb is not a replacement: {}", r.detail);
        // …but a DIFFERENT orb retypes the ones already in there, and must say so.
        let r = row(0, &c, "검".into(), Kind::Socket, 7);
        assert!(r.detail.contains("기존 오브 교체"), "{}", r.detail);
        // An empty item has nothing to replace, whatever orb is coming.
        let r = row(0, &equip(3, 0), "검".into(), Kind::Socket, 7);
        assert!(!r.detail.contains("교체"), "{}", r.detail);
        // Socket EXPANSION never retypes anything.
        let r = row(0, &c, "검".into(), Kind::Expand, IDX_SOCKET_EXPAND);
        assert!(!r.detail.contains("교체"), "{}", r.detail);
        // An ineligible target is still listed, with the reason attached.
        let r = row(1, &equip(0, 0), "방패".into(), Kind::Socket, 3);
        assert!(!r.enabled);
        assert!(r.detail.contains(Refusal::NoSockets.text()), "{}", r.detail);
    }

    /// ⛔ The orb id goes into a byte the game indexes into a 16-row table with no bounds
    /// check, and the save loader re-runs that read on every launch.
    #[test]
    fn an_orb_index_outside_the_table_writes_nothing() {
        let rec = 0x3000;
        let c = equip(3, 0);
        for idx in 0..=IDX_ORB_MAX {
            assert!(!writes(rec, &c, Kind::Socket, idx).is_empty(), "orb {idx:#x} is real");
        }
        // 0x0f is 소켓확장 and never reaches the Socket arm; everything above is nonsense.
        for idx in [IDX_SOCKET_EXPAND, 0x10, 0x30, 0xff, 0xffff] {
            assert!(writes(rec, &c, Kind::Socket, idx).is_empty(), "orb {idx:#x} must not be socketed");
        }
        assert_eq!(kind_of(IDX_SOCKET_EXPAND), Kind::Expand);
    }

    #[test]
    fn a_short_list_cannot_be_scrolled_at_all() {
        // Four rows of 100 in a 500-tall viewport: there is nowhere to go.
        assert_eq!(scroll_span(4, 100, 500), 0.0);
        assert_eq!(clamp_scroll(250.0, 4, 100, 500), 0.0);
        assert_eq!(clamp_scroll(-80.0, 4, 100, 500), 0.0);
        for sel in 0..4 {
            assert_eq!(scroll_to_show(0.0, sel, 4, 100, 500), 0.0, "sel {sel} is already on screen");
        }
        // An empty list, and a zero-height viewport, are both simply "no scroll".
        assert_eq!(scroll_span(0, 100, 500), 0.0);
        assert_eq!(clamp_scroll(10.0, 0, 100, 500), 0.0);
        assert_eq!(first_visible(0.0, 0), (0, 0));
        assert_eq!(row_at(0.0, 5.0, 10, 0), None);
    }

    #[test]
    fn a_drag_cannot_push_the_list_off_either_end() {
        let (len, row_h, view) = (20usize, 100u32, 500u32);
        assert_eq!(scroll_span(len, row_h, view), 1500.0);
        assert_eq!(clamp_scroll(-1.0, len, row_h, view), 0.0);
        assert_eq!(clamp_scroll(9_999.0, len, row_h, view), 1500.0);
        assert_eq!(clamp_scroll(f64::NAN, len, row_h, view), 0.0, "a NaN drag must not poison the position");
        // At the far end the last row is flush with the bottom, and nothing follows it.
        assert_eq!(first_visible(1500.0, row_h), (15, 0));
        assert_eq!(row_at(1500.0, 499.0, len, row_h), Some(19));
    }

    #[test]
    fn a_key_moves_the_list_by_the_least_it_can() {
        let (len, row_h, view) = (20usize, 100u32, 500u32);
        // Rows 0..4 are on screen at rest, so arrowing among them moves nothing.
        for sel in 0..5 {
            assert_eq!(scroll_to_show(0.0, sel, len, row_h, view), 0.0);
        }
        // Row 5 is one row past the bottom edge: the list moves exactly one row.
        assert_eq!(scroll_to_show(0.0, 5, len, row_h, view), 100.0);
        // Coming back up to row 0 puts its top on the edge, not in the middle.
        assert_eq!(scroll_to_show(100.0, 0, len, row_h, view), 0.0);
        // …and the last row lands the list exactly at the end of its span.
        assert_eq!(scroll_to_show(0.0, 19, len, row_h, view), 1500.0);
        // Whatever the selection, the row it names is wholly on screen afterwards.
        for sel in 0..len {
            let px = scroll_to_show(0.0, sel, len, row_h, view);
            let top = sel as f64 * f64::from(row_h);
            assert!(px <= top && top + f64::from(row_h) <= px + f64::from(view), "sel {sel} is off screen at {px}");
        }
    }

    #[test]
    fn a_tap_names_the_row_under_it_and_nothing_past_the_end() {
        let (len, row_h) = (3usize, 100u32);
        assert_eq!(row_at(0.0, 0.0, len, row_h), Some(0));
        assert_eq!(row_at(0.0, 99.9, len, row_h), Some(0));
        assert_eq!(row_at(0.0, 100.0, len, row_h), Some(1));
        // The empty tail of a short list is NOT the last row.
        assert_eq!(row_at(0.0, 299.0, len, row_h), Some(2));
        assert_eq!(row_at(0.0, 300.0, len, row_h), None);
        assert_eq!(row_at(0.0, -1.0, len, row_h), None);
        // Scrolled by half a row, the row under the top edge is the one straddling it.
        assert_eq!(row_at(50.0, 0.0, len, row_h), Some(0));
        assert_eq!(row_at(50.0, 60.0, len, row_h), Some(1));
        // A partial row's offset is always inside one row, so the caller's
        // `list.y - offset` never leaves the panel.
        for px in [0.0, 0.4, 49.9, 99.9, 100.0, 250.5] {
            let (i, off) = first_visible(px, row_h);
            assert!(off < row_h, "offset {off} at {px} is a whole row");
            assert!(i as f64 * f64::from(row_h) <= px);
        }
    }

    #[test]
    fn the_selection_saturates_rather_than_wrapping() {
        assert_eq!(step_sel(0, 5, -1), 0);
        assert_eq!(step_sel(4, 5, 1), 4);
        assert_eq!(step_sel(2, 5, 1), 3);
        assert_eq!(step_sel(2, 5, -1), 1);
        assert_eq!(step_sel(0, 5, 10), 4);
        assert_eq!(step_sel(4, 5, -10), 0);
        assert_eq!(step_sel(3, 0, 1), 0, "an empty list has no selection to move");
    }
}
