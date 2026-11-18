// Build-specific guest anchors — the DATA half of `guest_build.rs`.
//
// ── WHY THIS FILE EXISTS SEPARATELY ──────────────────────────────────────────
// It carries no `//!` docs, no `use`, and no dependencies, so it can be `include!`d
// into more than one crate:
//   * `wie_android/src/guest_build.rs` splices it in and adds the "which build
//     booted" atomic on top, so the app keeps calling `guest_build::char_anchor()`
//     exactly as before;
//   * `wie_probe/src/main.rs` splices it into a module of the same name.
// The probe used to carry a hard-coded COPY of the KTF numbers (`0x1b3084` /
// `0x1b7b6c` / `+0x7c`), which is why `--map-check`, `--char-dump`, `--find-fb` and
// `--trace-state` all read unrelated memory on an LGT boot (MIGRATION_LGT.md §3.4).
// A shared table is the fix: a copy drifts, and `items_selftest.rs` is this repo's
// standing proof of that.
//
// One thing the probe deliberately does NOT share: the "is a character alive yet"
// plausibility test for `Static` anchors. That test is made of CHAR-record offsets
// (level / hp / sp) which live in `items.rs`, and a diagnostic tool wants the raw
// base even when the record reads dead — that is precisely the state one debugs.

/// Which carrier build of the game is running. The two that matter here are
/// different COMPILES of the same game (MIGRATION_LGT.md §1): no byte-level overlap,
/// so a guest address lifted from one is meaningless in the other.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GuestBuild {
    /// KTF `client.bin` md5 `3792a476…`, Ver 01.00.03 — every guest CODE address in
    /// `wie_android` was reverse engineered from this one.
    Ktf,
    /// LGT `binary.mod` md5 `6a352944…` (Ver 01.00.07) or `e76fafdd…` (01.00.04).
    Lgt,
    /// SKT. Boots, but nothing here has been measured on it.
    Skt,
}

/// How to reach the character record on a given build.
///
/// The two carriers disagree about something more basic than an address: KTF
/// allocates the record and keeps a pointer to it in a global, LGT compiles the
/// record itself into `.bss` and never needs a pointer at all.
#[derive(Clone, Copy)]
pub enum CharAnchor {
    /// KTF `client.bin`: `*(sl+0x444)` = `0x1b3084` holds the record's address, with
    /// `0x1b7b6c` as the observed static fallback when that global reads wrong.
    Pointer { global: u32, fallback: u32 },
    /// LGT `binary.mod`: the record itself lives at this `.bss` address. Nothing to
    /// dereference, so "is a character loaded" has to be answered by the CONTENTS
    /// (see `items::resolve_char`) instead of by a pointer being non-null.
    ///
    /// Measured 2026-08-10 on Ver 01.00.07 — the STATUS screen's own numbers
    /// (HP 167/167, SP 100/100, LV 1, EXP 1/814) read back at the KTF offsets from
    /// this base, and `+0xa0c0` carried KTF's window ids (3 = none, 4 = script).
    Static(u32),
}

/// How to reach `popup_handle[8]` — the array holding one allocation per row of the
/// game's context popup, whose row 0 text is what tells the host a `사용하기` is about
/// to run `useItem` on a cash item (`items::cash_use_pending`).
///
/// Same disagreement as [`CharAnchor`], for the same reason: KTF keeps the array
/// behind a data slot, LGT compiles the array itself into `.bss`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PopupHandles {
    /// KTF `client.bin`: the `sl`-relative data slot at `global` HOLDS the array's
    /// address. Read at runtime rather than hardcoding `0x1b7a2c` so a relocated
    /// build still resolves. [CONFIRMED `sl+0x1b4`.]
    Pointer { global: u32 },
    /// LGT `binary.mod`: the eight-word array IS at this `.bss` address.
    Static(u32),
}

/// How the host makes the GAME take the popup row it has just highlighted — the commit
/// half of the 퀵슬롯등록 rewrite.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PopupCommit {
    /// KTF: write the highlighted-row byte, then call the game's own popup key handler
    /// with its select key. `fn_addr` carries the Thumb bit.
    GuestCall { fn_addr: u32, select_key: u32 },
    /// LGT: write the highlighted-row byte and let an ORDINARY `OK` keypress reach the
    /// game — the identical path a finger on the real handset takes. No guest address is
    /// involved, so nothing here can be stale.
    ///
    /// ── MEASURED, NOT ASSUMED (2026-08-11, 8차) ──────────────────────────────────
    /// Two deterministic probe runs, identical except that one poked the row byte
    /// (`CHAR+0x9c86`) to 1 ten frames before the `OK`:
    ///   * untouched (row 0 = `사용하기`): the quickbar stayed all zeros and the potion
    ///     was consumed, i.e. the game ran row 0;
    ///   * poked to row 1 (`1키등록`): `CHAR+0x252` became **2** (kind = item) and
    ///     `CHAR+0x257` became **6** — inventory slot 6, exactly the potion the cursor
    ///     was on. The popup closed itself either way.
    /// So the game's own popup dispatcher reads that byte when the key arrives, and the
    /// whole bind (kind, ref, the existing-binding cleanup, the teardown) is its code.
    /// The same run also verified the quickbar offsets on LGT, which MIGRATION_LGT.md
    /// §2.11 had listed as a silent-bug watch item.
    ///
    /// ★ The caller must restore the NATIVE row count before the key arrives. Our
    /// relabel shrinks `CHAR+0x9c85` to 4, and a row byte above that could be clamped by
    /// the guest's own cursor logic in the tick between the write and the keypress —
    /// which is a hazard the KTF path does not have, because a direct call leaves no
    /// tick in between. See `items::popup_count_restore`.
    SyntheticOk,
}

/// Where the two record base-name strings ("GameSave…" / "SlotSave…") live on a build.
/// The save chain passes these to the record writer, which appends `"_<slot>"` itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SaveNames {
    /// KTF `client.bin`: the RWPI `sl` table holds POINTERS to the strings
    /// (`"./Hero3GameSave"` / `"./Hero3SlotSave"`) — read them at runtime so a
    /// relocated build still resolves.
    SlPtrs { game: u32, slot: u32 },
    /// LGT `binary.mod`: absolute address model, the strings themselves are compiled
    /// into `.text` — pass the addresses straight through, nothing to read.
    Direct { game: u32, slot: u32 },
}

/// Everything per-build the host-driven save (quick-save / autosave) needs: the three
/// callable primitives, the out-parameter scratch word, the CHAR-relative slot-number
/// offset, and where the record names come from. `None` = the chain is not ported to
/// this build and the save UI must not offer it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SaveChain {
    /// `serialiseSave(u32* outLen) -> buf`. Thumb bit set.
    pub fn_serialise: u32,
    /// `slotSummary() -> buf16`. Thumb bit set.
    pub fn_slot_summary: u32,
    /// `writeSaveRecord(baseName, buf, len)` — appends `"_<slot>"` itself, then the
    /// 0x10-header + encrypt + WIPI DB write. Thumb bit set.
    pub fn_write_record: u32,
    /// Four bytes of guest memory PROVEN out of the game's reach, used as the
    /// serialise step's out-parameter. See each arm for that build's proof.
    pub scratch: u32,
    /// CHAR-relative offset of the save slot number (`"%s_%d"`'s `%d`).
    pub slot_off: u32,
    pub names: SaveNames,
}

/// Shape of the equip-level-relief rewrite — see [`GuestBuild::equip_relief_site`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EquipReliefSite {
    /// KTF: rewrite the one-halfword register move (`adds r1,r3,#0` → `subs r1,r3,#N`)
    /// at `site`. Encoded by `buffs::equip_relief_halfword`.
    MoveHalfword { site: u32 },
    /// LGT: rewrite the two-halfword PLAYER-LEVEL load (`movs r3,#0;
    /// ldrsh r1,[r4,r3]` → `ldrh r1,[r4,#0]; adds r1,#N`) at `site`, so the check
    /// compares `playerLevel + N` against the shipped requirement. Encoded by
    /// `buffs::equip_relief_level_bias`.
    ///
    /// ── WHY THIS SIDE OF THE COMPARISON AND NOT THE OTHER ────────────────────────
    /// §2.8 proposed rewriting the REQUIREMENT load instead (`movs r3,#2;
    /// ldrsb r3,[r2,r3]` → `ldrb r3,[r2,#2]; subs r3,#N`, four bytes at `0x477d2`).
    /// That works arithmetically and it was fully plumbed and tested in 6차, but it
    /// silently changes the SIGNEDNESS of the field it loads: the shipped instruction
    /// is `ldrsb` and the replacement `ldrb`, so any item whose requirement byte is
    /// ≥ 0x80 flips from "allowed" (`ldrsb` → negative → the signed `bge` passes) to
    /// "blocked" (`ldrb` → 128..255, minus N, still far above any level). A `-1`
    /// sentinel for "no requirement" is exactly the encoding a C `signed char` field
    /// invites, and nothing in the item data had been checked for one. That is the
    /// silent-bug class this migration keeps finding, so the patch moved to the other
    /// operand instead of arguing the hazard away.
    ///
    /// Biasing the player level is provably equivalent, not merely plausible:
    /// * The base is the CHAR record's level. `0x477cc adds r4,#0xba` on top of the
    ///   accessor that §2.8 showed returns `CHAR+0x28` puts `r4` at `CHAR+0xe2`, which
    ///   is `items::LEVEL_OFF` — the same halfword the HUD reads as `lv` and the same
    ///   one `--verify-offsets` printed as `lv=Some(1)`.
    /// * Level is a small positive: the game's own level-up routine caps it at 70 — 99
    ///   since the 111차 patch (`items::LEVEL_CAP`) — and `items::resolve_char` refuses a
    ///   record whose level is outside `1..=LEVEL_CAP`. For every value the field can hold, `ldrh` and `ldrsh`
    ///   return the same thing — so this rewrite changes the comparison and nothing
    ///   else, for any item in the game.
    ///
    /// Both halfwords were read back out of RUNNING guest memory before this shipped
    /// (`wie_probe --poke 7850:0x477ce:21880531` logged `0023e15e`, exactly what the
    /// static disassembly predicted), and the same run's frame hashes matched the
    /// unpatched run's.
    LevelBias { site: u32 },
}

/// The words the game's own `showModal` writes to raise its "연결이 끊어졌습니다" box —
/// how the host both TELLS the player the link is gone and gets them back out of the
/// network screens. `None` on a build where they have not been located.
///
/// ── WHY THE HOST HAS TO WRITE THESE AT ALL (2026-08-15, 48차) ────────────────────
/// 47차 established, on the device, three things that between them kill every simpler
/// idea (PLAY_FEEDBACK_FIXES.md §47차-D):
///   1. closing the socket does not change the screen — the guest ran its own
///      `MC_netSocketClose` + `MC_netClose` and still sat on its loading modal;
///   2. a synthetic reply frame only opens a box on SOME screens (`05 42` status-0
///      works in the cash shop and not in the network warehouse);
///   3. closing that box leaves the player exactly where they were.
/// So the message and the way OUT are two different problems, and the game already
/// solves both — in code that Android backgrounding never reaches.
///
/// ── WHAT WAS FOUND, AND RE-VERIFIED HERE BEFORE IT WAS USED ─────────────────────
/// The online module (title / slot / single-warehouse / network-warehouse / cash shop,
/// i.e. everything `items::shell_state` calls state 3) draws and keys through two
/// tables indexed by ONE word:
/// ```text
///   0x60ac  moduleDraw(base): screenDraw[*(base+0xa3d4)](base)
///                             modalDraw [*(base+0xa438)](base)   <- painted on top
///   0x6164  moduleKey (base,key): if (modalKey[*(base+0xa438)](base,key)) return
///                                 screenKey[*(base+0xa3d4)](base,key)
/// ```
/// `getBase()` (`0x1068`, `ldr r0,[pc,#0]; bx lr`) returns **`0x1503ab0`**, the same
/// `CHAR+0x28` the EXP and auto-loot work already stand on. Both tables were rebuilt
/// from their initializer at `0x59f0` rather than trusted second-hand, and entry **5**
/// is the pair that matters:
/// ```text
///   modalDraw[5] = 0xb608   panel if *(base+0xa7fc) != 0, then the text at base+0xa43c
///   modalKey [5] = 0xbe30   key == -5 (OK):  closeModal(0)          (0x4604)
///                                            netTeardown(0)         (0x18608)
///                                            if (screen != 9) screen = 1
/// ```
/// **That is the whole missing half**: the game's own OK handler tears the connection
/// down and puts the player back on the online module's main screen. The game uses the
/// same modal itself in exactly this situation — `0x1417c` (the WIPI *pause* notifier,
/// which on a real handset fired on an incoming call) and the 5-minute idle watchdog
/// `0x18a0c` both do `netTeardown(0); showModal("서버와의 접속이 끊어졌습니다.", 5, 1, 1)`.
/// Android never calls `pauseClet`, and by the time anything else notices, the guest's
/// own teardown has cleared `sessionActive` (`0x150ea94`) — which is the precondition
/// of `0x1417c`, so the shipped notifier can no longer fire. Hence: write what it writes.
///
/// The five fields below are the words `showModal(text, 5, 1, 1)` (`0x4684`) writes.
/// (The host no longer writes them — 99차 — but `state_word` is the modal-id anchor
/// the cash-shop code reads, so the measurement stays.)
///
/// ★ It STAYS up: of the 16 code sites in the image that load the `0xa438` offset,
/// only `showModal`/`closeModal`, the `cat 0x64` kick handler (`0x4a10`), the connect
/// paths and modal-key 2 write it — nothing re-asserts a modal per frame, so the box
/// cannot be overwritten by a link that is already dead. And `netTeardown` (`0x18608`)
/// touches neither this word nor the screen word, so the OK path is not self-erasing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NetDisconnectModal {
    /// u32. The modal id, index into BOTH modal tables. `0` = no modal, so writing it
    /// is what makes the box exist.
    pub state_word: u32,
    /// The id whose OK key runs the game's own teardown and leaves the network screens.
    pub disconnect_state: u32,
    /// The EUC-KR text buffer the drawer prints (NUL-terminated, `text_cap` bytes).
    pub text_buf: u32,
    pub text_cap: usize,
    /// u8, non-zero = draw the translucent panel behind the text. The drawer skips the
    /// panel entirely when this is 0, which would leave the words floating on the map.
    pub panel_flag: u32,
    /// u8, the per-modal sub-flag. Modal 5's drawer does not read it, but `showModal`
    /// always writes it and a stale value belongs to whatever box was up before.
    pub sub_flag: u32,
    /// u32 the game's own `showModal` zeroes on every open (`ctx+0x40`).
    pub clear_word: u32,
}

// ⛔ `NET_DISCONNECT_TEXT_EUCKR` and `NetDisconnectModal::writes` (48차: the host raising
// modal 5 after Android killed a backgrounded socket) WERE HERE and are gone (99차,
// 2026-09-16 — 100 % offline, no socket to kill). The struct stays: its `state_word` is
// the modal-id anchor `items.rs` reads for the cash-shop rows and `modal_id()`, and
// `anchors_selftest.rs` still pins every field against the CHAR anchor.

/// 메뉴 터치화 (66차): the cursor byte of each overlay screen whose grid is measured.
/// CHAR-relative, because that is the part that ports across carriers. See
/// [`GuestBuild::menu_cursors`] for how each one was measured and for the ⛔.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MenuCursors {
    /// The six-row MENU list (`CLR` on the field).
    pub list: u32,
    /// 스킬 3x2 grid.
    pub skill: u32,
    /// 퀘스트 6-row list.
    pub quest: u32,
    /// SYSTEM 4-row list.
    pub system: u32,
    /// 상태보기's stat grid cursor — `row * 2 + col` over a 2x2 grid (79차-C).
    ///
    /// Found by diffing the WHOLE character record between two otherwise identical
    /// probe runs, one of which pressed `DOWN` once on that screen: exactly one byte
    /// moved, `CHAR+0x9c70`, and it moved 0 -> **2**, which is what said the grid has
    /// two columns. Poking it 0/1/2/3 with no keys then walked the game's own cyan
    /// marker around all four cells (`menutouch::STATUS`); 4 and up drew nothing at all.
    pub status: u32,
    /// 장비's slot cursor — the INDEX byte of a three-byte cursor (index / column /
    /// row at `CHAR+0x9c77` / `+0x9c78` / `+0x9c79`), device-measured 79차-D.
    ///
    /// ⛔ Read-only for the host. `idx = row*3 + col` was confirmed by walking the grid
    /// with the game's own keys on a character wearing all nine pieces, but that
    /// character has nothing for the cursor to SKIP, so the repo's old warning cannot
    /// be disproved with it. The tap router therefore moves this cursor by pressing the
    /// game's own RIGHT, never by writing (`MenuTapAction::Walk`).
    pub equip: u32,
    /// The SHARED list widget the game's own WINDOWS use (상점 · 제련 · …): cursor.
    pub widget: u32,
    /// …its row count. The game keeps this live, so the host never has to know how many
    /// rows a screen has — which is what makes one geometry rule cover them all.
    pub widget_rows: u32,
    /// …its axis pick: 1 = a row list, 0 = a `예/아니오` confirm (both measured).
    pub widget_axis: u32,
    /// …and the byte that says the widget is actually PAINTING right now. The cursor and
    /// the count go stale — they keep the last list's values while the same window shows
    /// something else — but this one is cleared. Measured 66차-B: entering 상점's 판매 bag
    /// grid dropped it 8 -> 0 together with the widget's rect descriptors, and leaving the
    /// grid put it back. It is what tells "this window is showing a LIST" from "…a BAG".
    pub widget_paint: u32,
}

/// 인게임 오브결합 (2026-08-25): the game's own two "this bag record changed, fix its
/// derived fields" helpers.
///
/// The host writes the three socket bytes itself — they are plain record fields — but the
/// two HALFWORDS the game caches next to them (`rec+0x3cc` socket bonus %, `rec+0x3ce`
/// effective power) are derived, and an exhaustive scan for a `strh` through a `+0x3cc`
/// base finds only four producers in the whole image: the wire grant, new-item creation,
/// and these. **Nothing recomputes them on the fly**, so writing the orb byte alone is
/// inert until the next save/load.
///
/// Rather than re-derive them (the formula needs the orb-power table at `*(grid+0x28)`
/// stride 0x18 and a per-category item DB whose stride is 0x20 only for `cat <= 0x0c`),
/// the host calls the game's own functions with the same four arguments its SAVE LOADER
/// passes right after restoring the very same bytes — `0x1a3f4` then `0x1a40a`.
///
/// ⛔ Per-item, like every other code anchor here: `guest_code_addresses_valid()` stays
/// false on LGT. `None` turns the whole in-game combine off (the popup row is never even
/// offered), which is the fail-closed direction — a build that cannot refresh the cache
/// must not be allowed to write the bytes either.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ItemRefresh {
    /// `recalcItemStats(tab, slot, cat, dbIdx)` — rebuilds `rec+0x3cc` / `rec+0x3ce`.
    /// Thumb bit set. Body at `0x52940`-`0x52990`:
    /// `bonus = (s8)orbDB[orbId].b[0x15] * f*(f+1)/2`, then
    /// `power = (itemDB[cat][idx].s16@0x16 + mult*refine) * (100 + bonus) / 100`
    /// with `mult = 2` when `(u8)(cat-4) <= 6`.
    pub fn_recalc: u32,
    /// `rebuildEquipSnapshots()` — **no arguments**. Thumb bit set.
    ///
    /// ★★★ 71차-L. `fn_recalc` above rebuilds the two DERIVED halfwords and nothing
    /// else (its only stores are `0x52964 strh -> rec+0x3cc` and
    /// `0x5298e strh -> rec+0x3ce`). The 장비 screen's socket pips for the item under
    /// the cursor are read live out of the bag, but the CANDIDATE LIST and the EQUIPPED
    /// array it draws are BYTE COPIES of whole records, and only this function bakes
    /// them: `CAND[row][j]` at `base+0x580` (20 bytes from `rec+0x3bc`) and
    /// `EQUIPPED[slot]` at `base+0x364` (a full `0x3c`-byte memcpy). Both carry
    /// `+0x32/+0x34/+0x35` — total holes, orb id, filled holes — at the same offsets,
    /// and the character's own orb bonus is computed from `EQUIPPED[]`, not from the bag.
    ///
    /// So a combine that only ran `fn_recalc` left every one of those copies stale, and
    /// they stayed stale until the player actually equipped something (the equip routine
    /// `0x4dc58` ends by calling `0x55048`, which is `compact 0x5380c` **then this**).
    /// That is the standing report "오브결합 후에 장비장착할때 … 최신화가 안되".
    ///
    /// ⛔ Call THIS, not `0x55048`: the wrapper compacts the bag first, which moves
    /// records and would invalidate the host's `(tab, slot)` + u64-key re-check.
    /// ⛔ And never `0x4f014` — six arguments, two on the stack, and it zeroes the
    /// derived stat block on entry.
    ///
    /// Convention proven at the only call site (`0x55048`): `ldr r3, =0x4d859` then
    /// `bl 0x5a6f4` (= `bx r3`) with r0/r1/r2 untouched, and the body fetches the base
    /// and the grid itself (`0x4d85c bl getBase`, `0x4d868` -> `*(base+0x9e00)`).
    pub fn_equip_snapshot: u32,
}

/// How to move the player to another PLACE, and what decides who lives there.
///
/// The unit the game moves in is a SCENE, not a map: `res/event/e<scene>_scn` is a
/// nine-byte header naming a map id plus the three monster species that fill that
/// map's spawn points, and 244 scenes share 135 maps. `tools_re/dump_map_catalog.py`
/// prints the whole join and verifies it (all 135 maps parse to exactly EOF).
///
/// ★★ Everything here is DATA except one function, and that function takes one
/// halfword. That is what makes an extra dungeon cheap: the host picks a scene,
/// calls one guest function, and optionally overwrites three words to change the
/// monsters — no asset edit, no code patch.
///
/// ⛔ The offsets are CHAR-relative, and the literals in the image are NOT. The game
/// reaches these through `getBase()` (`0x1068`), which returns **CHAR + 0x28**, so the
/// literal `0x9e1c` in `parseScnHeader` is `CHAR + 0x9e44` here. 78차-C lost a release
/// to exactly this arithmetic (the "hard flag at CHAR+0xa318" that was really
/// `base+0xa318`). The independent check that the +0x28 is right: `creatures_off`
/// below comes out at **CHAR+0x9cbc**, which is the address KTF was already known to
/// use for the same array (`GUARDIAN_CASTLE_FEASIBILITY.md §1`), and the two builds
/// agree on CHAR offsets while sharing no code addresses at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SceneWorld {
    /// `loadScene(u16 scene)` — Thumb bit set. Body at `0x15f28`:
    /// builds `res/event/e%02d%02d_scn` from `scene/100` and `scene%100`
    /// (`0x1964c` with the template at rodata `0x5c6e0`), loads it (`0x1c080`),
    /// stores the scene id at `scene_id_off`, calls `parseScnHeader` (`0x14284`)
    /// which returns the map id, builds `res/map/map%d_mp`, calls `loadMap`
    /// (`0x15edc` -> the parser `0x151bc`, whose own tail re-reads the scene id and
    /// runs the scenario's event script through `0x14328` -> `0x3bac0`), then frees
    /// the scenario buffer.
    ///
    /// Its ONLY caller in the whole image is the warp handler at `0x3f280`, which
    /// passes `*(u8*)(warpObject + 0xd)` — a warp tile carries its destination scene
    /// in one byte.
    ///
    /// ⛔⛔ **Prefer [`SceneWorld::warp_request_off`] over calling this.** Two byte
    /// writes make the game warp itself, correctly and completely; this call loads the
    /// data but leaves the world undrawn (below). This entry stays because it is the
    /// fastest way to ask "what is in scene N" from a probe run, and because the byte
    /// pair only works from the field — but shipping code should not use it.
    ///
    /// ⚠ VERIFIED, WITH A LIMIT (`wie_probe --host-scene 1750:205`, frames 1749-1753).
    /// Calling this alone really does load the place: the map id, the tile dimensions,
    /// the spawn count, the species triple and the creature-array pointer all change
    /// in one frame and match what `dump_map_catalog.py` predicts for that scene from
    /// the assets alone (map 122, 60x20, 17 spawns, species 118/137/152). The player
    /// lands on a sensible tile of the new map and the camera tracks it.
    /// **But the world is not drawn** — the framebuffer goes black except for the HUD,
    /// and it stays black. So the tile graphics are re-initialised by something the
    /// warp handler does AROUND this call and not by this call:
    ///   * before: `memset(*(base+0x9e00) + 0x356, 0, 0x3c)`, `+0x2b9 = 0`,
    ///     `+0x354 = 0`, `*(base+0xa30e) = -1`, `*(base+0xa2b2) = -1`, `0x3cf00()`
    ///   * after: `0x491e0(...)` (six arguments — the player placement),
    ///     `*(base+0x9f96) = 0`, `0x3ebf4()`, `0x16280()`
    /// ⛔ Calling `0x16280` and `0x3ebf4` after it changes nothing (probe A/B: the
    /// frame hash is identical), so do not try that again — the missing piece is
    /// either the player placement or one of the pre-call invalidations.
    /// ★ The cheaper route, and the one `GUARDIAN_CASTLE_FEASIBILITY.md §2-1(a)`
    /// already preferred, is to not call this at all: overwrite a live warp object's
    /// destination byte (`+0xd`) and the event tile the player is standing next to,
    /// and let the game run its own handler — which does all of the above by
    /// construction.
    pub fn_load_scene: u32,
    /// `u16` holding the id of the scene currently loaded. `loadScene` writes it
    /// before parsing anything, and the MAP parser's tail reads it back to re-load
    /// the same scenario and run its event script, so it is authoritative rather
    /// than a leftover.
    ///
    /// ⚠ Do not read this as a length. The literal `0x9e08` sits next to the
    /// resource-length out-parameter on `loadScene`'s stack (`sp+4` is the scene id,
    /// `sp+8` the length) and reading the wrong one of the two is an easy mistake to
    /// make from the disassembly; the parser tail settles it by dividing this value
    /// by 100 to rebuild `e%02d%02d_scn`.
    ///
    /// It is also the head of the small SCENE CONTROL BLOCK that
    /// [`SceneWorld::warp_request_off`] writes into — `+0`/`+1` the id, `+2` the
    /// request, `+0xd` the destination.
    pub scene_id_off: u32,
    /// ★★★ THE WARP PRIMITIVE. Write the destination scene to `warp_dest_off`, then
    /// `1` to this byte, and the game warps itself on its next tick — correctly and
    /// completely, because it is the game's own handler that runs.
    ///
    /// The handler is `0x3f1d4`, and it is a per-tick poll, not a callback:
    /// ```text
    ///     r7 = base + 0x9e08                      ; the scene control block
    ///     kind = (i8)r7[2] ; if kind == 0: return
    ///     r7[2] = 0                               ; consumed before dispatch
    ///     if kind != 1: return                    ; 1 = scene warp
    ///     <teardown: Event_freeID, 0x146e8, 0x26320, 0x3767c, 0x4c388, flag resets>
    ///     loadScene(r7[0xd])
    ///     <placement 0x56134(.., r7[0xc], ..), 0x491e0(), 0x3ebf4(), 0x16280()>
    /// ```
    /// ⛔ It was a natural misreading that `r7` is a warp OBJECT and that `+0xd` is a
    /// field of one — the handler's only caller-visible input looks like a tile you
    /// stepped on. It is not: `r7` is a fixed address, so a warp is a REQUEST anyone
    /// can post, and there is no object to find.
    ///
    /// ★★ VERIFIED end to end (`wie_probe --poke 1750:150d8c5:cd,1750:150d8ba:01`,
    /// i.e. these two bytes and nothing else, from a new character standing in the
    /// starting town): scene 0 -> scene 205, map 122 `GUARDIAN_CAVE_5` 60x20, 17
    /// creatures spawned, species 118/137/152 — and **the world renders**
    /// (111,687 non-black pixels at frame 1780 against 2,569 for the direct
    /// `loadScene` call, screenshot in `GUARDIAN_CASTLE_FEASIBILITY.md §A.1`).
    ///
    /// Byte `+0xc` (`CHAR+0x9e3c`) is the arrival FACING, not a location.
    /// Leaving it at zero renders the map but hides the player.
    pub warp_request_off: u32,
    /// Destination scene for the request above — ONE byte, so scenes `>= 256` could
    /// not be reached this way even if they existed (the game ships 244).
    pub warp_dest_off: u32,
    /// Arrival facing (legacy name): `+0x0c` of the scene control block.
    /// `0x56134 -> 0x560a4 -> 0x5609c` stores it at player+0 (CHAR+0x9bb4).
    /// Valid directions are 1..=4; the tower uses 2 (down), including death returns.
    pub warp_entry_off: u32,
    /// `u32` holding the map id of the loaded scene — `parseScnHeader` writes SCN[0]
    /// here and `loadMap`'s prologue copies it to `CHAR+0x9e40` first, so the pair is
    /// "previous map, current map". Reading it is how the host confirms a warp landed.
    pub map_id_off: u32,
    /// First of **three** `u32`s holding the scene's monster species (SCN[3..5]).
    /// A map's spawn point stores a type of 0/1/2 indexing this triple, so these
    /// three words ARE the spawn table: overwrite them and the same map spawns
    /// different monsters. `0xff` = slot unused.
    ///
    /// Species `0..160` index `res/dat/enemy_dat` (161 rows, byte 0 of the record
    /// tail = level); species `>= 161` index `res/boss/boss_dat` at `species - 161`
    /// (15 rows). The catalogue tool joins both.
    pub spawn_species_off: u32,
    /// `u32` holding SCN[6], the region byte (1 for the town scenes, 4..18 for the
    /// field chapters, 255 for the boss arenas). Only used to pick chrome.
    pub region_off: u32,
    /// `u32` holding the pointer to the live creature array. Element `i` is at
    /// `ptr + i * creature_stride`; `+0x1c` is that creature's object pointer.
    /// The map loader fills it right after parsing (`0x15908`..`0x1597a`), placing
    /// each spawn at the tile CENTRE (`x*16 + 8`).
    pub creatures_off: u32,
    /// Stride of that array — the loader computes it as `0xac << 1`.
    pub creature_stride: u32,
    /// Offset **inside the MAP object** (`*(CHAR + 0x9c90)`, see
    /// `minimap_render::MAP_HANDLE_OFF`) of the `i32` count of creatures the loader
    /// actually spawned. Written at `0x158de`; a floor is cleared when the live ones
    /// among these are gone.
    pub map_spawn_count_off: u32,
    /// Where the stats live INSIDE one of those records. See [`CreatureRecord`].
    pub rec: CreatureRecord,
}

/// The fields of one live monster record - the array is
/// `*(CHAR + SceneWorld::creatures_off)` with `SceneWorld::creature_stride` between
/// elements, and these are offsets into an element.
///
/// -- HOW THESE WERE ESTABLISHED (2026-09-08) ------------------------------------
/// Two independent readings that agree, which is why they are here rather than in a
/// feasibility document:
///
/// 1. **Static.** The target-box painter (the same function
///    [`GuestBuild::target_box_x_sites`] patches) computes the HP bar as
///    `43 * (i16)rec[0x7e] / (i16)rec[0x7c]` at `0x27e0`..`0x2800`, and prints
///    `sprintf(.., "%d", (i8)rec[0x70])` for the level at `0x2796`..`0x27a2`. The
///    combat code re-derives the same two independently at `0x4e168` (level) and
///    `0x4e1a6` (`(i16)rec[0x7e] <= 0` = skip this target).
/// 2. **Runtime.** `wie_probe --host-warp 1700:205 --creatures 1710` dumps all 17
///    records of `GUARDIAN_CAVE_5`, and every decoded field equals what
///    `res/dat/enemy_dat` holds for that species - species 118 (아일기버로크) level
///    42, HP 2564, attack 398, defences 69 / 65; species 137 (커브이라) level 43,
///    HP 3427, attack 446, defences 73 / 69. A wrong offset cannot match four
///    numbers of three different species at once.
///
/// * The record is a copy of an `enemy_dat` row, so the DB row order names the
///   fields: tail byte 0 -> level, `[5..6]` -> HP, `[7..8]` -> attack, `[9..10]` and
///   `[11..12]` -> the two defences. The row parser is `0x376c0`..`0x37830`, which
///   builds a `0x24`-stride species table out of those same bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CreatureRecord {
    /// `i8` - the monster's level, and the tower's first fork. The painter prints it
    /// and the combat code reads it as one side of the level-difference evade roll
    /// (`0x4e168`, `0x38670`), so forking it changes both the number on screen and
    /// the fight.
    pub level: u32,
    /// `i16` - MAXIMUM HP. Denominator of the HP bar, so a fork shows immediately.
    pub hp_max: u32,
    /// `i16` - CURRENT HP, and the whole of the liveness test.
    ///
    /// ** `(i16)rec[hp_cur] > 0` IS "alive". ** The only writer that lowers it is
    /// `0x38620(rec, delta)`, which does `*(u16*)(rec + 0x7e) += delta` and clamps a
    /// negative result to 0 - so a dead monster stays in the array with exactly 0
    /// here rather than vanishing, and the AI tick (`0x39c36`) and the combat
    /// targeting (`0x4e1aa`) both branch on `<= 0`.
    /// => a floor is cleared when none of the `MAP + map_spawn_count_off` records has
    /// a positive value here. Do NOT look for a separate "valid" or "dead" flag; the
    /// game does not keep one.
    pub hp_cur: u32,
    /// `i16` - HP again, written alongside `hp_max` at spawn.
    ///
    /// ** THIS IS THE IMMUTABLE BASE FOR AN HP FORK, SO DO NOT WRITE IT. ** No reader
    /// of it has been found, and measurement says the game does not move it either:
    /// with `--crea-set … 7e:0000` on scene 205 it kept the spawn value while current
    /// HP went to 0. That makes it the HP equivalent of `def_phys_base` - the field a
    /// host fork can scale FROM every time and land on the same number.
    ///
    /// ⛔ An earlier version of this comment said the opposite ("a fork that moves
    /// `hp_max` and leaves this behind is a silent inconsistency"). That was wrong and
    /// acting on it cost a real bug: `guardian.rs`'s first draft scaled max HP from
    /// the LIVE max HP and a 150% floor walked one species 2564 -> 3846 -> 5769 on
    /// every re-assert, which `guardian_selftest.rs` caught before it shipped. If a
    /// reader of this field ever turns up, the answer is per-record bookkeeping, not
    /// writing it.
    pub hp_spawn: u32,
    /// `u16` - THE RESPAWN TIMER, and the reason a tower floor could never be cleared.
    ///
    /// **`0xFFFF` = disarmed.** It reads `0xFFFF` on every live monster; the moment one
    /// is dead the game starts counting this up (~0.5 per frame), and at roughly 300 it
    /// re-spawns that record IN PLACE: `hp_cur` goes back to full, two fresh sprite
    /// pointers appear at `+0xc0`, and the position changes. `MAP + map_spawn_count_off`
    /// and the array pointer never move, so a respawn is a record being reused - which
    /// is why the live count climbs again and [`crate::guardian::Run`] would wait for a
    /// clear that never comes.
    ///
    /// ★ MEASURED HEADLESSLY, and reproducibly (`wie_probe`, scene 134, 19 spawns):
    /// `--crea-set <f>:all:7e:0000` to kill them all, then dumps every 15 frames show
    /// this field walking `ffff -> 0004 -> ... -> 0129` and records #3 and #8 alive
    /// again at frame 2420. Writing `0xFFFF` back every 40 frames instead keeps the
    /// count at ZERO for 1150 frames (~23 s) - the game re-arms it and the host keeps
    /// cancelling it, and the counter never gets near 300.
    /// ⛔ ONE write is not enough (the game re-arms within ~50 frames); it has to be
    /// re-asserted, which is exactly what the tower's poll already does for the forks.
    /// ⛔ Do not write it on a LIVE monster - it is already `0xFFFF` there, so a
    /// read-compare-write costs nothing and a blind write would be a guess.
    pub respawn_timer: u32,
    /// `i16` - regeneration per tick in PER MILLE of `hp_max`, applied by the AI tick
    /// (`0x39bde`: `hp_max * this / 1000`; a negative value is damage, so this is the
    /// poison channel too). Zero on every shipped species measured so far.
    pub hp_regen_permille: u32,
    /// `i16` - BASE attack (`enemy_dat` tail `[7..8]`).
    pub atk_base: u32,
    /// `i16` - EFFECTIVE attack, which the game's own setters at `0x38888` and
    /// `0x388c4` recompute from `atk_base` as `atk_base * (100 + pct) / 100`.
    pub atk_eff: u32,
    /// `i16` x 2 - a BOSS record's second and third attack values (160차 T4). `bossh_dat`'s
    /// tail carries three attacks (`t[4..9]`; 홀리가디언 2410 · 2410 · 1205) and the boss
    /// loader lays them at `+0x8e` · `+0x90` · `+0x92`; the boss attack (`0x26d52..0x26d7a`)
    /// picks one by its state byte (2 -> the first, 0xb -> the second, 0xc/0xd -> the third)
    /// and passes it to the same damage function as `atk_eff`. Probe dump, scene 231:
    /// `+0x8c 00 00 | 41 04 | 41 04 | 20 02`. ⛔ BOSS RECORDS ONLY (species >= 161) — on an
    /// ordinary record these two halfwords are something else and nothing here writes them.
    pub atk_skill1: u32,
    pub atk_skill2: u32,
    /// `i16` - BASE physical defence (`enemy_dat` tail `[9..10]`).
    pub def_phys_base: u32,
    /// `i16` - EFFECTIVE physical defence. **Subtracted from the player's damage** by
    /// the hit resolver at `0x4bdd4` / `0x4be3a`.
    ///
    /// * Physical vs special is chosen by the equipped weapon's category byte
    /// (`0x4bd00`: `> 6` takes the other arm), which is the same 리츠-melee /
    /// 케이-gun split `PLAY_FEEDBACK_FIXES.md` 75차-B measured for item options -
    /// categories `0x04..=0x06` (스피어 / 스워드 / 나이프) subtract this one.
    pub def_phys_eff: u32,
    /// `i16` - BASE special defence (`enemy_dat` tail `[11..12]`).
    pub def_spec_base: u32,
    /// `i16` - EFFECTIVE special defence, subtracted instead of
    /// [`Self::def_phys_eff`] for the gun categories (`0x07` / `0x08`), at `0x4be4c`.
    pub def_spec_eff: u32,
    /// `i16` percentage knobs that the game's own modifier API adds into, one per
    /// effective stat above: attack twice (`0x38888` -> `+0x40` and `0x388c4` ->
    /// `+0x42`, both recomputing `atk_eff`), then physical defence (`0x38900` ->
    /// `+0x44`) and special defence (`0x3893c` -> `+0x46`).
    ///
    /// ! Those setters are `+=` on the percentage rather than assignments, so calling
    /// one twice compounds. A host fork that wants to be idempotent should write the
    /// effective field itself, recomputing it from the base (which never changes) -
    /// the same shape as the buff pattern's read-compare-write-reassert.
    pub pct_atk_a: u32,
    /// See [`Self::pct_atk_a`].
    pub pct_atk_b: u32,
    /// See [`Self::pct_atk_a`].
    pub pct_def_phys: u32,
    /// See [`Self::pct_atk_a`].
    pub pct_def_spec: u32,
    /// `u32` - the VIRTUAL HP word (2026-09-18): the host seeds it with a floor's total
    /// when that total will not fit the `i16` `hp_cur`, and the `dmg_apply_lgt` hook
    /// (`wie_core_arm::binary_patches::dmgwide`) takes every hit off it and writes
    /// `hp_cur = 32767 × remaining / total` itself — PROPORTIONAL, never below 1 while any
    /// remains (102차-B; ⛔ not `min(remaining, 32767)`, the vc341 shape that kept the target
    /// bar full until the pool was under 32767). 0 = no pool, the record is all there is.
    ///
    /// ★ SPARE, measured: sentinels in `+0xf8..+0x107` survived 1000 frames of combat,
    /// kills and a respawn re-initialisation on 80/80 records with every frame
    /// pixel-identical, while a control sentinel at `+0x40` was wiped by the re-init.
    /// ⛔ The host writes it ONCE (read-compare-write on 0) and never re-asserts it:
    /// after the seed the hook owns the word, and a poll that wrote it back would undo
    /// the player's hits.
    /// ★ And nothing CLEARS it, because nothing has to (104차-B code review): the array is
    /// `MC_knlCalloc`'d on every scene load (map loader `0x373c2` → `.data` stub
    /// `0x1400264` = svc `0x76` Calloc, zero-filled), so a fresh record always reads 0 —
    /// a floor left mid-fight cannot hand its half-spent word to the next scene's
    /// monsters. The respawn RE-INIT is the one path that keeps it, and it only ever
    /// reuses a KILLED record, whose spending hit wrote 0. Measured headlessly: sentinels
    /// in both words on scene 243 read 0 on all 20 records after a warp to 244, the array
    /// having been re-allocated at the SAME address (§104차-B).
    pub vhp: u32,
    /// `u32` - the total [`Self::vhp`] was seeded with, the bar's denominator: the hook
    /// keeps `hp_cur = 32767 * vhp / vhp_max` so the target box's bar drains in
    /// proportion to the real HP (user 2026-09-18: the box and its bar must not change).
    /// Seeded together with `vhp` (this one first), never re-asserted, same spare block.
    pub vhp_max: u32,
    /// `u32` - the 32-bit DISPLAY copy of the last hit (the patched `hitMonster` stores
    /// it here instead of the halfword at `+0xaa`; the drawer `0x1ff8` reads it with
    /// `ldr`). Same spare block as [`Self::vhp`]. The host never touches it.
    /// ★ Both monster callers of the drawer land on THIS array's records: `0x39b2e` through
    /// `CHAR+0x9cbc` and `0x2a574` through `CHAR+0x9cb8`, which is an alias of the same
    /// pointer (the map loader stores its one calloc into both words, and every other
    /// writer of `+0x9cb8` copies `+0x9cbc` — 104차-B, `binary_patches.toml`). So the
    /// sentinel measurement covers the second caller as well.
    pub dmg32: u32,
    /// Offset of the pointer at the head of the record. `*(u32*)(rec + actor)` is a
    /// second struct whose `+0x1c` is the drawable object, and THAT object's `+0x0e`
    /// is the SPECIES id - the same numbering [`SceneWorld::spawn_species_off`] uses,
    /// so `> 160` means the boss table (`0x3865a` tests exactly that).
    /// * The AI state id is `(i8)*(u32*)(rec + actor) + 2`, a 19-way switch dispatched
    /// at `0x39c1a`.
    pub actor: u32,
    /// `i16` — the monster's 명중 (156차). The spawn loader (`0x36ec4`) writes
    /// `speciesRow[0x16] + rec[0x48]`, and the species loader (`0x377f2`) filled that row
    /// halfword with `ldrsb` of the dat byte — so every hard-table species of Lv 49+ (dat
    /// byte 128..212) carries −127..−44 here, and the monster→player roll
    /// (`0x4bfca`: `clamp(80 + hit − 회피 + 5·(mLv − pLv), 15, 95)`, the sum truncated to 16
    /// bits and sign-extended) sits at its 15 % floor. The hell fork overwrites it.
    pub hit: u32,
    /// `u16` — the monster's 회피 (156차), `speciesRow[0x18] + rec[0x4a]` (hard table 10..45;
    /// the loader also keeps a copy at `+0x94` that no roll reads). The player→monster roll
    /// (`0x4bf60`: `clamp(80 + 명중 + d24 − 회피 + 5·(pLv − mLv), 15, 95)`) reads it with `ldrh`.
    pub dodge: u32,
    /// `u16` — the monster's 기절 저항 (156차), `speciesRow[0x1c] + rec[0x4e]` (hard table
    /// 37..87). The crit roll (`0x4bed8`: `clamp(d25 + 기절 − 저항 + (pLv − mLv), 2, 60)`) reads it
    /// with `ldrh`; a two's-complement negative acts as a bonus because the roll truncates the
    /// sum to 16 bits and sign-extends it.
    pub crit_res: u32,
}

/// Where the auto-loot flag lives, and what has to be in it. See
/// [`GuestBuild::autoloot_flag`] for the disassembly of both the writer and the reader.
///
/// It is a THREE-part answer and every part has drawn blood: the address is an array,
/// not a constant; the index is the save slot; and the value has to have bit 7 set
/// because the reader tests the byte signed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AutolootFlag {
    /// CHAR-relative address of element 0.
    pub base_off: u32,
    /// `Some(off)` = the flag is a byte ARRAY and the u32 at `CHAR+off` picks the
    /// element. `None` = a single byte at `base_off` (KTF, where only slot 0 was ever
    /// observed).
    pub slot_off: Option<u32>,
    /// What the game's own `useItem` arm stores. NOT a boolean: the LGT reader
    /// sign-extends the byte and branches on `< 0`, so a value without bit 7 is ignored
    /// no matter how right the address is.
    pub value: u8,
}

impl AutolootFlag {
    /// Highest slot the array is allowed to be indexed by, matching the save-slot bound
    /// the rest of the crate enforces (`save::SAVE_SLOT_MAX`). A word outside this is a
    /// record that is not loaded yet, and indexing on it would write a byte somewhere
    /// arbitrary in the record — the exact hazard this table exists to prevent.
    pub const MAX_SLOT: u32 = 15;

    /// The flag's CHAR-relative offset for a given save-slot word, or `None` when that
    /// word is not a slot number this may be indexed by.
    pub fn offset_for(&self, slot: Option<u32>) -> Option<u32> {
        match (self.slot_off, slot) {
            (None, _) => Some(self.base_off),
            (Some(_), Some(s)) if s <= Self::MAX_SLOT => Some(self.base_off + s),
            _ => None,
        }
    }
}

impl GuestBuild {
    pub fn char_anchor(self) -> Option<CharAnchor> {
        match self {
            Self::Ktf => Some(CharAnchor::Pointer {
                global: 0x1b_3084,
                fallback: 0x1b_7b6c,
            }),
            Self::Lgt => Some(CharAnchor::Static(0x150_3a88)),
            Self::Skt => None,
        }
    }

    /// Offset of the coarse game-state byte inside the character record.
    ///
    /// The one struct offset that did NOT survive the KTF → LGT revision gap:
    /// everything else this crate reads (level/exp/hp/sp, the inventory grid handle
    /// at `+0x9e28`, the window id at `+0xa0c0`) sits at the same offset in both,
    /// but the state byte moved back one word. Measured the same way — 4 while
    /// free-roaming, 5 with the menu open, matching KTF's `STATE_FIELD` /
    /// `STATE_SELECTOR`.
    pub fn state_off(self) -> u32 {
        match self {
            Self::Lgt => 0x78,
            _ => 0x7c,
        }
    }

    /// CHAR offset of the ONLINE MODULE's screen word — `*(base + 0xa3d4)` with
    /// `base = getBase() = CHAR + 0x28`, i.e. `CHAR + 0xa3fc` (u32). `None` = not located
    /// on this build, which turns the creation-screen save pin (76차) off.
    ///
    /// ★★★ 76차 — WHY THE HOST NEEDS IT. The save-slot pin's sources were two observed
    /// LOADS (`04 3d` · a `GameSave_11_<n>` record read). A character CREATED in an empty
    /// slot is neither: the local 게임시작 arm (`0xaee8`, key −5) reads `GameSave_11_<cursor>`,
    /// gets 0, and instead of loading anything sets this word to **10** (`0xaf3c`). So when a
    /// character of the same class was loaded earlier in the process, the pin stayed on THAT
    /// slot, `pin_holds` kept it (the class name did not change), and `redirect_slot` landed
    /// the new character's first save on the old slot — «슬롯 3 에 새로 만들었는데 슬롯 1 이
    /// 덮였다» (2026-09-03). The screen word is the one guest fact that says "this character
    /// was CREATED, in the slot the cursor names".
    ///
    /// The values, from the shell key table `0x150e838` (initializer `0x59f0`) and the
    /// handlers' own stores to `[base+0xa39c, #0x38]`:
    ///   1 main menu · 2 online slot list · 3 warehouse/cash shop · 9 게임시작 slot list ·
    ///   **10 class select (`0xafb4`) · 11 portrait (`0xb044`) · 12 confirm (`0xb174`)** ·
    ///   13 login notice. `0x59f0` (run by `setAppState(3)`) writes 0 first.
    /// Screen 12's OK with 예 is the ONLY path from a creation screen into the world
    /// (`0xb1c8..0xb1cc` → `setAppState(4)`), and the word is not touched in states 4/5:
    /// the headless probe read **12** for 2,400 frames after entry, through the intro
    /// cutscene and a host save (2026-09-03, `scratchpad/probe6.log`).
    pub fn shell_screen_off(self) -> Option<u32> {
        match self {
            Self::Lgt => Some(0xa3fc),
            Self::Ktf | Self::Skt => None,
        }
    }

    /// 세이브 슬롯 창 (130차): the first of the slot list's three `"SLOT n"` label cells in
    /// the guest image — `0x5c2c4 "SLOT 1  "`, `0x5c2cc "SLOT 2  "`, `0x5c2d4 "SLOT 3  "`,
    /// 8 bytes each, drawn by the two list painters (`0xdc72..0xdcb8`, `0xf0da..0xf120`)
    /// through `0x53c0`. `None` = the window feature is off on this build: the record
    /// store's remap then never leaves the identity, because `first` never moves.
    ///
    /// The slot ARRAYS the window refills are CHAR offsets and live in `slotwindow.rs`
    /// (`OCCUPIED_OFF` …), with the cursor being `SaveChain::slot_off` (`0xa410`).
    pub fn slot_label_base(self) -> Option<u32> {
        match self {
            Self::Lgt => Some(0x5c2c4),
            Self::Ktf | Self::Skt => None,
        }
    }

    /// 메뉴 터치화 (66차): the cursor byte of each of the game's own overlay screens, as
    /// CHAR offsets. `None` = not measured on this build, which gates menu taps off.
    /// (Stage 1 — the taps on the `«1» «3» «7» «9»` badges the GAME itself prints —
    /// needs no anchor at all: it sends the very key the game advertises.)
    ///
    /// ★ These four are the ONLY addresses this feature adds. The other bytes it reads —
    /// the overlay flag, the overlay page, the popup-open flag and the per-bag INVENTORY
    /// cursor — are ones `items.rs` has owned since 17차/44차, and they stay there: two
    /// copies of a guest offset in one crate is how the watcher and the writer end up
    /// disagreeing about which item is under the cursor. See the block comment above
    /// `items::menu_state`.
    ///
    /// All four are LGT, measured **on the device** (66차-B, S9): with the screen open,
    /// one arrow press and `items::flag_window_changes` (the whole-record watcher) named
    /// exactly one byte each time.
    ///
    /// ```text
    ///   list   CHAR+0x9c6d  the six-row MENU list (probe-proven 66차: poke 4 -> the
    ///                       highlight moved to QUEST with NO key sent)
    ///   skill  CHAR+0x9c7d  스킬 3x2 grid. RIGHT +1, DOWN +3 -> index = row*3 + col
    ///   quest  CHAR+0x9c81  퀘스트 6-row list. DOWN +1
    ///   system CHAR+0x9c82  SYSTEM 4-row list. DOWN +1
    ///
    ///   paint  CHAR+0xa249  != 0 while the widget PAINTS (7 for a 2-row list, 8 for a
    ///                       3-row one, 0 in a bag grid). ⛔ The cursor and count bytes do
    ///                       NOT clear, so this is the only honest "is it a list" test.
    ///   widget CHAR+0xa284  the SHARED list widget of `TOUCH_MENU_FEASIBILITY.md §2.1`,
    ///          +0xa285      confirmed live on 상점 (win 12) and 제련 (win 13): DOWN moved
    ///          +0xa276      `0xa284` 0->1 while `0xa285` read the real row count (2 for
    ///                       구매/판매, 3 for the category and the smith's list) and
    ///                       `0xa276` read 1. ★ On those windows the OVERLAY flag is 0 —
    ///                       the gate there is the window id, not `open`.
    /// ```
    ///
    /// ★ The widget's row count being LIVE is what makes 4단계 cheap: the geometry rule
    /// `menutouch::choice_grid` takes the count and centres the block, so one measurement
    /// covers every screen that uses the widget instead of one table per screen.
    ///
    /// ⛔ Each is valid only inside ITS OWN grid (`menutouch::MENU` / `SKILL` / `QUEST` /
    /// `SYSTEM`). The game clamps these bytes only inside its KEY arms
    /// (`0x461b8`/`0x461dc` for the list); its OK arm re-reads them with no range check,
    /// so an out-of-range value dispatches a screen the menu itself cannot reach
    /// (measured, 63차-B §7-1). `menutouch::Grid::hit` cannot answer out of range and
    /// `Grid::holds` is the second gate.
    ///
    /// ★ 상태보기 (`status`, 79차-C) and 장비 (`equip`, 79차-D) are measured now. 장비 is
    /// three bytes — index `0x9c77`, column `0x9c78`, row `0x9c79` — and the cursor SKIPS
    /// unusable slots, so it is never written directly: `MenuTapAction::Walk` presses the
    /// game's own RIGHT until it arrives.
    ///
    /// ⚠ `equip`'s three bytes are INSIDE the inventory cursor array
    /// (`items::INV_CURSOR_BASE` 0x9c72 + bag, up to `menutouch::PAGES`): bags 5·6·7 are
    /// 0x9c77·78·79. That is the GAME's own layout — the two screens initialise their own
    /// bytes on entry — and both paths write only the byte the game itself writes there,
    /// each cross-checked against the live value before the write. `anchors_selftest`
    /// pins the arithmetic so it is a recorded fact rather than a surprise.
    ///
    /// 🔶 NOT measured yet, so not here and not tappable: 상점 / 제련 / 창고 / 캐시샵.
    pub fn menu_cursors(self) -> Option<MenuCursors> {
        match self {
            Self::Lgt => Some(MenuCursors {
                list: 0x9c6d,
                skill: 0x9c7d,
                quest: 0x9c81,
                system: 0x9c82,
                status: 0x9c70,
                equip: 0x9c77,
                widget: 0xa284,
                widget_rows: 0xa285,
                widget_axis: 0xa276,
                widget_paint: 0xa249,
            }),
            _ => None,
        }
    }

    /// The two derived-field refresh helpers, when they are located on this build.
    ///
    /// LGT `binary.mod` 01.00.07. Both addresses come from ONE site and it is the best
    /// possible one: the save loader restores `rec+0x3c5/0x3c6/0x3c8/0x3c9/0x3c4/0x3c7`
    /// for every `cat <= 0x0c` bag record (`0x1a3a4`-`0x1a3dc`) — the exact bytes an
    /// in-game combine writes — and then calls these two with `(tab, slot, cat, idx)`
    /// (`0x1a3f4` via literal `0x1a458` = `0x528f1`). So the argument tuple and the
    /// precondition are the game's own, not an inference, and `0x528f0` has exactly ONE
    /// caller in the whole image — that one.
    ///
    /// ⛔ Its sibling at `0x5283c` (the name rebuild, called next at `0x1a40a`) is NOT
    /// here. Its tail is `sprintf(rec+0x394, "%s", name)` — one argument, no orb prefix —
    /// so it cannot change anything a combine did, and `0x52872` reaches that sprintf with
    /// the source register possibly unset for `cat > 0x12`. Nothing to gain, something to
    /// lose.
    ///
    /// ⛔ KTF is `None`: the equivalents were never located there, and the APK ships LGT.
    /// Guessing them would put the write on a build whose refresh does not exist.
    /// The game's own give-item routine (`0x52b14(cat, idx, -1, 0)`), thumb bit set.
    ///
    /// Same function the 오브원석 arm and the quest/NPC grants call. Equipment writes
    /// the stack count itself (`0x52c5c movs r1,#1`). `None` on a build whose address
    /// has not been located — do not guess; a stale call jumps into unrelated code.
    /// Used by the `wie_grant.txt` maintenance grant (`config::read_grant`).
    pub fn give_item_fn(self) -> Option<u32> {
        match self {
            Self::Lgt => Some(0x0005_2b14 | 1),
            Self::Ktf | Self::Skt => None,
        }
    }

    /// `ComputeMaxHP 0x49000(block)` — `(14*Lv + 18*체력) * (100 + HP%) / 100`, stored at
    /// `block+0x38` (142차-B hooks the store). Pure: it only computes and stores, so the host
    /// can re-run it. 142차-C 붉은마녀 「마녀의 생명력」.
    pub fn compute_max_hp_fn(self) -> Option<u32> {
        match self {
            Self::Lgt => Some(0x0004_9000 | 1),
            Self::Ktf | Self::Skt => None,
        }
    }

    /// `addHP 0x4b9c0(block, delta)` — the ONE writer every HP change goes through (hit,
    /// heal, drain, potion, level-up), thumb bit set. 150차 레아의목걸이 「5초당 HP회복 +N%」:
    /// the host's 5-second heal calls it with `(STAT_BLOCK, n)`.
    ///
    /// ★ On LGT the whole function is the `hp_add_lgt` hook (142차-B), which scales a delta
    /// only when its caller is one of the three max-proportional heals (`RELATIVE_LR`). A
    /// host call's LR is none of them, so `n` lands as TRUE HP — the same shape the probe's
    /// `--regen` lever drives. `None` elsewhere: an unlocated address is not guessed.
    pub fn add_hp_fn(self) -> Option<u32> {
        match self {
            Self::Lgt => Some(0x0004_b9c0 | 1),
            Self::Ktf | Self::Skt => None,
        }
    }

    /// `useItem 0x5505c(tab, slot)` — what the field hotkey's item arm calls (`0x50130`,
    /// literal `0x50214`) and the inventory popup's 사용하기 (literal `0x47840`), thumb bit
    /// set. 164차: a potion pressed during a skill cast — the field key handler `0x502ec`
    /// drops every key while the cast flag is up (`items::field_key_gate`), so the host
    /// runs this arm itself. `None` elsewhere: an unlocated address is not guessed.
    pub fn use_item_fn(self) -> Option<u32> {
        match self {
            Self::Lgt => Some(0x0005_505c | 1),
            Self::Ktf | Self::Skt => None,
        }
    }

    /// The paper-doll layer setter `0x517bc(slot, cat, idx, remove)` — what 장착 (`0x478e8`) and
    /// 해제 (`0x484b8`) call, thumb bit set. 167차 아바타 옷장: the host calls it with the slot's
    /// own EQUIPPED cell to RE-DRESS one layer after a try-on or a wear — the
    /// `avatar_layer_set_lgt` hook on its join then picks the costume sheet. Loads a sprite
    /// sheet (the game's own resource read + allocation), so only from a menu, like the game.
    pub fn layer_set_fn(self) -> Option<u32> {
        match self {
            Self::Lgt => Some(0x0005_17bc | 1),
            Self::Ktf | Self::Skt => None,
        }
    }

    /// An EQUIPMENT page `.bss` byte (s8) its OK handler switches on (`0x48390`: 0 / 1 / other).
    /// ⛔ 172차: NOT "0 = worn grid, 1 = candidate list" as 167차 read it — every one of its five
    /// writers follows a `0x46b7c(id)` widget install (`0x47804` · `0x479fc` · `0x48780` · `0x486cc` ·
    /// `0x4867c`), and nothing clears it when the widget closes, so it stays non-zero after a
    /// 장비해제 confirm while the worn grid is back on screen. The candidate list's own
    /// flag is `CHAR+0x9c7a` (`items::EQUIP_CANDIDATES_OFF`). Kept as a build-capability anchor
    /// and for the wardrobe's give-up diagnostic; no gate reads it.
    pub fn equip_mode_addr(self) -> Option<u32> {
        match self {
            Self::Lgt => Some(0x0151_2904),
            Self::Ktf | Self::Skt => None,
        }
    }

    /// The `.bss` word that holds the game's string table (`InGame_txt`) base while a menu
    /// is open — `getString 0x1bdcc(base, id) = base + u16_le(base + 4 + id * 2)`. 0 on the
    /// field (host-ui report: `0x401447bc` with the menu up). 150차: the 상세보기 overlay reads
    /// an orb's label (`id = orb + 0xab`, 「@투신의」) out of it to compose the name box the
    /// way the painter does (`0x45474`, `"%s %s"`). The same word `orbsum.rs` reads.
    pub fn string_table_ptr(self) -> Option<u32> {
        match self {
            Self::Lgt => Some(0x0151_2928),
            Self::Ktf | Self::Skt => None,
        }
    }

    pub fn item_refresh(self) -> Option<ItemRefresh> {
        match self {
            Self::Lgt => Some(ItemRefresh { fn_recalc: 0x0005_28f0 | 1, fn_equip_snapshot: 0x0004_d858 | 1 }),
            _ => None,
        }
    }

    /// How a field that holds a WIPI allocation references its payload.
    ///
    /// `Some(off)` — the field is a HANDLE and the payload is `*(handle) + off`.
    /// `None` — the field is already the payload address.
    ///
    /// ★ This is NOT a property of the game's compile but of the carrier runtime the
    /// game is linked against, which is why one flag settles it for every container
    /// at once (inventory grid, area object, item DB, map arrays, framebuffers):
    ///   * `wie_ktf`'s `WIPICContext::data_ptr` is `*(handle) + 8`
    ///     (`vendor/wie/wie_ktf/src/runtime/wipi_c/context.rs`), so everything the
    ///     game gets back from `MC_knlCalloc` / `MC_grpCreateOffScreenFrameBuffer`
    ///     is a handle;
    ///   * `wie_lgt`'s is the IDENTITY (`Ok(memory.0)`), so the same allocation call
    ///     hands the game a direct pointer.
    /// Independently corroborated by the sprite-blitter port, where the descriptor's
    /// pixel/palette words needed exactly this split
    /// (`blit565::SpriteLayout::{KtfHandles, LgtDirect}`, MIGRATION_LGT.md §2.4).
    pub fn container_data_off(self) -> Option<u32> {
        match self {
            Self::Lgt => None,
            // SKT is untested; the KTF shape is the safer default because every
            // caller validates each hop lands in the guest heap and gives up if not.
            _ => Some(8),
        }
    }

    /// Where `popup_handle[8]` lives on this build, or `None` where it has not been
    /// located. See [`PopupHandles`] for the two shapes.
    ///
    /// ── WHY THIS MATTERS MORE THAN IT LOOKS (2026-08-11, 7차) ────────────────────
    /// It is the ONLY thing standing between LGT and three of §2.11's matrix rows.
    /// Every cash buff this app supplies — EXP x2/x4 (#9), the equip-level relief
    /// (#10), the random boxes' full restore (#16) — is granted from exactly one
    /// place, `items::cash_use_pending`, and that reads the highlighted popup row's
    /// label through this array. On LGT the KTF slot address is not even mapped
    /// (device-measured: `handles=None`), so no buff can ever start, which makes the
    /// EXP-divisor and relief patch sites unreachable however correct they are.
    ///
    /// LGT value found by LIFETIME, not by address (MIGRATION_LGT.md §3.3(b) widened
    /// from a word to a run): `free_popup` NULLs every handle, so with a popup open
    /// the array is N consecutive plausible heap pointers and after the close the same
    /// words are zero. Diffing `.bss` between an open-popup frame and a closed-popup
    /// frame gave **exactly one** run in the whole section — eight words at
    /// `0x1512934` — and dereferencing them produced the two rows that were on screen,
    /// `상세보기` (`bb f3 bc bc ba b8 b1 e2 00`) and `버리기`, in EUC-KR, with the
    /// remaining six pointing at allocated-but-zeroed 10-byte buffers.
    ///
    /// ★ The label buffers are reached with ONE hop fewer, and that falls out of
    /// [`Self::container_data_off`] rather than being a second discovery: the dumped
    /// pointers land directly on the text, with no 8-byte allocator header in front.
    pub fn popup_handles(self) -> Option<PopupHandles> {
        match self {
            Self::Ktf => Some(PopupHandles::Pointer { global: 0x1b_2df4 }),
            Self::Lgt => Some(PopupHandles::Static(0x151_2934)),
            Self::Skt => None,
        }
    }

    /// CHAR-relative bytes that PRE-EMPT the inventory page's key routing: with any of
    /// them non-zero the page never routes a key into its popup handler, so the popup
    /// on screen is not really live. `items::popup_view` refuses the ITEM popup while
    /// one is set. Empty = no pre-emptor is checked on this build.
    ///
    /// ── WHY THIS IS PER-BUILD, AND WHY LGT IS EMPTY (2026-08-15, 51차) ────────────
    /// The check exists for KTF's commit shape: [`PopupCommit::GuestCall`] BYPASSES the
    /// game's own key router (guest `0x13aa24`/`0x13aa56` is where these bytes are
    /// tested), so the host must apply the same refusal itself or it commits a row into
    /// a popup the game considers dead. LGT's [`PopupCommit::SyntheticOk`] is the
    /// opposite by construction: the key travels the game's OWN router, pre-emptors
    /// included, so the game applies whatever its real gate is — the host checking a
    /// guessed address adds nothing it does not already get.
    ///
    /// And the addresses WERE only guesses on LGT. Both offsets are KTF measurements;
    /// `0xa220` in particular sits outside the popup cluster the 647-callsite analysis
    /// verified on LGT (`+0x9c6c..+0x9c87`) and inside the region where the LGT record
    /// is DOCUMENTED to shift (`0xa384`→`0xa370`, `0xa3f8`→`0xa410`), so reading it on
    /// LGT reads some other live field. That is the device report this row fixes
    /// (2026-08-15): skills relabel to 퀵슬롯등록 but consumables never do — the skill
    /// popup skips this gate entirely, while a stray non-zero byte at the KTF offset
    /// failed the item popup every time. §2.16's probe passed all ten gates only
    /// because a fresh-boot character happens to have zeros there.
    pub fn popup_preempt_offsets(self) -> &'static [u32] {
        match self {
            Self::Ktf => &[0x9c89, 0xa220],
            // The real LGT pre-emptors (if that compile has any) are unlocated. Locate
            // them the §3.3(b) way — diff the record between a routed and a pre-empted
            // key state — before ever putting an address here.
            Self::Lgt => &[],
            Self::Skt => &[],
        }
    }

    /// How to commit a popup row on this build, or `None` where neither mechanism has
    /// been established. See [`PopupCommit`].
    ///
    /// ── WHY LGT DOES NOT NEED A GUEST ADDRESS (2026-08-11, 8차) ───────────────────
    /// The 퀵슬롯등록 rewrite looked like it was blocked on finding LGT's `popup_key`,
    /// because that is what KTF calls. It was not: the host already synthesises `OK` for
    /// the popup's row 0 (`emulator::InputEvent::PopupConfirm`), so the same synthesis
    /// commits ANY row once the row byte says which one. A probe A/B proved it (see
    /// [`PopupCommit::SyntheticOk`]), which removes a whole reverse-engineering step and
    /// a whole class of risk — a stale call address jumps into unrelated code, a key
    /// press cannot.
    ///
    /// KTF keeps its measured call: it is proven in the field and the two mechanisms are
    /// not interchangeable in one detail (a direct call leaves no tick in which the guest
    /// could re-clamp the row byte).
    pub fn popup_commit(self) -> Option<PopupCommit> {
        match self {
            // `popup_key(kind, key)`; the jump table at guest 0x1a745c is indexed by
            // `key + 0x10` and -5 is its select arm (0x13a8be).
            Self::Ktf => Some(PopupCommit::GuestCall {
                fn_addr: 0x0013_a86c | 1,
                select_key: (-5i32) as u32,
            }),
            Self::Lgt => Some(PopupCommit::SyntheticOk),
            Self::Skt => None,
        }
    }

    /// The global whose value SUPPRESSES the minimap's corner blend, when it is known
    /// for this build (guest `0x17b97e` on KTF reads `sl - 0x8084 + 8`). `None` = not
    /// located on this build, so the blend is never suppressed — which is also what
    /// KTF does in practice, since the word there holds none of the three tile
    /// colours. `wie_probe --map-check` is what proves the choice right.
    pub fn minimap_noblend_global(self) -> Option<u32> {
        match self {
            Self::Ktf => Some(0x001a_4bc4),
            _ => None,
        }
    }

    /// The one-instruction patch that stops the game drawing its own BOTTOM field HUD
    /// (panel + HP/SP/EXP bars + the 5 quickbar cells), all of which the overlay
    /// replaces: `(site, original_halfword, branch_halfword)`.
    ///
    /// The caller writes `branch` over `site` only after reading back `original`, so a
    /// revision this was never measured on is left alone. `None` = not located on this
    /// build, and then the game keeps drawing its own HUD (which is a cosmetic
    /// duplicate of the overlay's, not a malfunction).
    ///
    /// ── WHY THE SAME PATCH EXISTS TWICE ─────────────────────────────────────────
    /// The KTF and LGT `.text` share no bytes (MIGRATION_LGT.md §1) but they are
    /// compiled from the same C, so the block is recognisable by its ARITHMETIC. Both
    /// builds compute the panel's origin as `((s16)screen_w - 0xaa)/2 + 2` and
    /// `screen_h - 0x18`, and in both the instruction that loads `screen_w` for that
    /// sum is the first instruction of the block — everything above it (buff band,
    /// target-monster box) has already been drawn, and everything below it is the
    /// bottom panel. Branching from there to the function's own epilogue drops exactly
    /// that block.
    ///
    /// KTF `client.bin` (Ver 01.00.03), renderer `0x149b30`:
    ///   `0x149f7a  ldr r3,[sp,#0x100]` → `b 0x14a324`  (imm11 467 = 0xe1d3)
    ///
    /// LGT `binary.mod` (Ver 01.00.07), renderer **`0x22a4`..`0x3050`**
    /// (`push {r4-r7,lr}` / `sub sp,#0xd4` … `add sp,#0xd4; pop {r4-r7}; pop {r0};
    /// bx r0`), located 2026-08-10 by searching the ELF for `subs rX,#0xaa` — 14 hits,
    /// exactly one of them followed by `asrs #1` + `adds #2`:
    ///   ```
    ///   0x2860  bl  0x5a6fc          ; last draw of the target box (bx r5 veneer)
    ///   0x2864  ldr r5,[sp,#0x94]    ; <= PATCH SITE (screen_w)
    ///   0x2868  lsls r5,r5,#0x10
    ///   0x286a  asrs r3,r5,#0x10
    ///   0x286c  subs r3,#0xaa
    ///   0x286e  asrs r3,r3,#1
    ///   0x2870  adds r3,#2
    ///   ...
    ///   0x287c  subs r3,#0x18        ; screen_h - 0x18
    ///   ```
    ///   `b 0x304a` = imm11 `(0x304a - 0x2868)/2` = 0x3f1 → `0xe3f1`.
    ///
    /// ── MEASURED, NOT ARGUED (wie_probe, 2026-08-10) ────────────────────────────
    /// Two deterministic LGT runs differing only by `--poke 7860:0x2864:f1e3`:
    ///   * the halfword at the site read back `0x9d25`, as the static disassembly said;
    ///   * the bottom panel disappears and the world shows through it (the pond and its
    ///     lily pads continue into the rows the panel used to own);
    ///   * the 45 KB character record is **byte-identical** in both runs at every
    ///     sampled frame — the skipped block writes no game state;
    ///   * the only other bytes that differ are `.bss 0x1500000`/`0x1500004`, whose
    ///     ONLY pc-relative loaders (`0x2f64`, `0x2f78`, `0x2fa8`, `0x2ffc`) are inside
    ///     the skipped range, i.e. HUD-private blink counters — the same finding KTF's
    ///     adoption note records for `0x14a2a0`/`0x14a2bc`/`0x14a2d0` — plus one word
    ///     at `.bss 0x150ec58` that the block caches and nothing else writes (its
    ///     neighbourhood holds the screen geometry at `0x150ec88`/`0x150ec8c`).
    pub fn hud_bottom_patch(self) -> Option<(u32, u16, u16)> {
        match self {
            Self::Ktf => Some((0x14_9f7a, 0x9b40, 0xe1d3)),
            Self::Lgt => Some((0x00_2864, 0x9d25, 0xe3f1)),
            Self::Skt => None,
        }
    }

    /// ★★★ 72차 — THE «오브 0개» BUG, AS `(site, orig_halfword, patched_halfword)`.
    ///
    /// Player, 2026-08-29: *"오브원석으로 오브를 얻을 시 오브 0개로 표기되어 나오는 버그도
    /// 있네요."* The zero is real, it is a bag-record byte, and the game writes it — or
    /// rather, fails to.
    ///
    /// ── THE PATH (all static, LGT 01.00.07) ─────────────────────────────────────
    /// 오브원석 is cash item `cat 0x12 idx 0x07`. Using it enters the `useItem` dispatcher
    /// `0x550c8` (table `0x5e538`, literal @`0x553f0`), whose entry `[7]` = `0x55194`
    /// rolls `rand(0, 14)` and calls the give-item routine
    /// `0x52b14(cat = 0x10, idx = roll, -1, 0)` at `0x551da`.
    ///
    /// `0x52b14` first asks the bag scanner `0x529a0` for a cell, and the scanner reports
    /// back whether it MERGED into an existing stack (`0x52a1c` stores 1) or claimed an
    /// EMPTY cell (`0x52a26` stores 0). It then splits on category at `0x52bc4`, and the
    /// non-equipment arm at `0x52f48` writes the whole record — occupied `+0x3b3`, worn
    /// `+0x3b4`, cat `+0x3b6`, idx `+0x3b8`, refine `+0x3b2` — and then:
    ///
    /// ```text
    ///   0x52fb0  ldrb r3,[r4]      ; r4 -> the scanner's "merged" byte
    ///   0x52fb2  cmp  r3,#0
    ///   0x52fb4  beq  0x52fce      ; ← NOT merged: skip the count entirely
    ///   0x52fb6..0x52fcc           ; rec[+0x3ba] += 1
    ///   0x52fce  ...               ; and here it assumes the count IS 1 (0x52fe6 cmp #1)
    /// ```
    ///
    /// So the count is only ever INCREMENTED, never initialised: a brand-new cell keeps
    /// the 0 the cell-clear path left there (`0x535b6 memset(rec+0x3bc, -1, 0x14)`, and
    /// `orbcombine::spend_one` on the host writes 0 too). The item cell painter then prints
    /// it verbatim — `0x1214a` loads `+0x3ba` and `sprintf("%d")` with no `> 1` guard —
    /// which is the «0» the player is looking at.
    ///
    /// Three independent cross-checks that 1 is the value the game itself intends:
    ///   * `addItemToInventory 0x131d0`, the OTHER give path, writes the count on a new
    ///     cell at the symmetric spot (`0x1360e strh`);
    ///   * this routine's EQUIPMENT arm writes it outright (`0x52c5c movs r1,#1; strh`);
    ///   * `0x52f48` itself branches on `count == 1` fourteen instructions later
    ///     (`0x52fe6`) to stamp the u64 instance key.
    ///
    /// ── THE FIX, AND THE ONE THAT WAS WRONG ─────────────────────────────────────
    /// ⛔⛔ THE FIRST ATTEMPT — nop out the `beq` so the increment always runs — SHIPPED IN
    /// vc199 AND WAS WRONG. Player, same day: *"오브가 하나도없는 상태에서 까면 맨처음
    /// A,B,C,D 오브가 나오면서 2개2개2개2개 나오고 그뒤로 A,B,C,D 가나오면 1개씩 추가가 돼"*.
    ///
    /// The premise it rested on — "an empty cell holds a count of 0" — is false. **Nothing
    /// clears `+0x3ba` when a cell is emptied**: the new-cell block writes occupied `+0x3b3`,
    /// worn `+0x3b4`, cat `+0x3b6`, idx `+0x3b8` and `+0x3b2`, and the removal path
    /// (`0x53648` -> `0x5340c`) only decrements the count or clears the occupancy byte. So a
    /// newly claimed cell carries its PREVIOUS occupant's count — for a stack the player used
    /// up that is 1, and `stale 1 + 1 = 2`. It is also why the original bug read as «0»
    /// rather than as garbage: a cell that had never been used is still zero from the
    /// allocation, and those are the cells a fresh bag hands out first.
    ///
    /// So the increment was never the whole operation. What the game means is
    /// **`count = merged ? old + 1 : 1`**, and that is what this writes, in place, over the
    /// 14 bytes at `0x52fb0`:
    ///
    /// ```text
    ///   0x52fb0  ldrb r2,[r4]        ; the scanner's merged flag, 0 or 1
    ///   0x52fb2  ldrh r1,[r3,#0xa]   ; old count — r3 is rec+0x3b0 here (set at 0x52faa),
    ///                                ;   so +0xa is rec+0x3ba
    ///   0x52fb4  rsbs r2,r2,#0       ; 0 -> 0 · 1 -> 0xffffffff
    ///   0x52fb6  ands r1,r2          ; r1 = merged ? old : 0
    ///   0x52fb8  adds r1,#1
    ///   0x52fba  strh r1,[r3,#0xa]
    ///   0x52fbc  b    0x52fce        ; over the original block, now dead
    /// ```
    ///
    /// Register safety, checked instruction by instruction: only r1/r2 are written and both
    /// are reassigned at `0x52fe2` / `0x52fea` before any read; r3 is reloaded at `0x52fd4`;
    /// r4..r7 are untouched (r4 is only read). Verified by disassembling the patched image.
    ///
    /// ⚠ It also covers the routine's other four callers (`0x1e754`, `0x3d592`, `0x469e4`,
    /// `0x493fe` — quest/NPC grants), which have the same defect for the same reason.
    ///
    /// ⛔ Item-by-item, like every other gate here: this must NOT ride on
    /// [`Self::guest_code_addresses_valid`].
    /// ⛔ Do not shorten this back to a single halfword. The correct operation is a SELECT,
    /// and there is no two-byte way to say it at this site — that is exactly the trap vc199
    /// fell into.
    pub fn give_item_count_patch(self) -> Option<(u32, &'static [u8], &'static [u8])> {
        match self {
            // KTF's give-item routine has not been located; the bug may not even be there.
            Self::Ktf | Self::Skt => None,
            Self::Lgt => Some((
                0x00_052f_b0,
                // ldrb r3,[r4] · cmp r3,#0 · beq 0x52fce · movs r1,#0x52 · add r1,sp,r1 ·
                // movs r3,#0 · ldrsb r3,[r1,r3]
                &[0x23, 0x78, 0x00, 0x2b, 0x0b, 0xd0, 0x52, 0x21, 0x69, 0x44, 0x00, 0x23, 0xcb, 0x56],
                &[0x22, 0x78, 0x59, 0x89, 0x52, 0x42, 0x11, 0x40, 0x01, 0x31, 0x59, 0x81, 0x07, 0xe0],
            )),
        }
    }

    /// The network-warehouse deposit (`올리기` / `05 41`) GRADE gate, as
    /// `(site, orig_halfword, patched_halfword)`, or `None` when not located.
    ///
    /// ── WHAT IT IS (LGT `binary.mod` 01.00.07, RE 2026-08-13 from the player-supplied
    /// game zip, `binary.mod` md5-identical to the shipped one) ────────────────────
    /// The deposit handler validates the item by category. For EQUIPMENT (cat ≤ 0x0c)
    /// it loads the item's grade byte from the in-memory item DB —
    /// `grade = *(perCatBase[cat] + idx*0x20 + 0x10)` at `0x6f8a` — and at **`0x6f9a`**
    /// does `cmp grade, #3; ble 0x6fe6`. `0x6fe6` is the branch that BUILDS AND SENDS
    /// the `05 41` upload (it writes the bytes `0x05`,`0x41` then the 20-byte record
    /// through the frame serializer). Grade `> 3` falls through to `0x6f9e`, which
    /// shows **"아이템 등급이 영웅 이상인 아이템은 올릴 수 없습니다"** (rodata `0x5bd08`,
    /// the string that NAMES this gate) and returns without sending.
    ///
    /// The colour ladder is the grade: 0 무색 · 1 노랑 · 2 초록 · 3 하늘 · 4 금색/빨강
    /// (영웅) · 5 흰색. So the shipped client refuses every 영웅-이상 item — which is
    /// exactly the gold `}` and white `@` CASH equipment the shop sells (green `'`
    /// tier-2 gear deposits fine), the player's "네트워크 창고로 못 들어가고" report.
    ///
    /// ── THE PATCH ────────────────────────────────────────────────────────────────
    /// One size-neutral halfword: `cmp r3,#3` (`0x2b03`) → `cmp r3,#7` (`0x2b07`).
    /// Grade maxes at 5, so `ble` is then taken for every real item and all grades
    /// reach the same `05 41` upload the game already uses — nothing else changes, the
    /// record is the game's own, and the server stores it unchanged. Applied the
    /// `hud_bottom_patch` way (write only when the site still reads the original), so a
    /// build where `0x6f9a` is not this instruction is left untouched.
    ///
    /// KTF: not located (the shipped APK carries the LGT asset).
    pub fn warehouse_grade_gate(self) -> Option<(u32, u16, u16)> {
        match self {
            Self::Ktf => None,
            Self::Lgt => Some((0x00_6f9a, 0x2b03, 0x2b07)),
            Self::Skt => None,
        }
    }

    /// Address of the `movs r1,#100` that divides the kill-EXP formula, which the EXP
    /// x2 / x4 cash item halves and quarters. `None` = not located on this build.
    ///
    /// The instruction is the SAME halfword on both builds (`0x2164`), so only the
    /// address is per-build; `items::exp_multiplier_patch` still validates that what is
    /// there is a `movs r1,#imm8` whose immediate divides 100.
    ///
    /// KTF `client.bin`: `0x14654e`, inside `addExpFromKill` (`0x14650c`).
    ///
    /// LGT `binary.mod`: **`0x4d3cc`**, inside `addExpFromKill` = **`0x4d388`**, found
    /// 2026-08-10 by scanning the ELF for a window holding all four of the formula's
    /// constants (`#0xd`, `#0x82`, `#0xa`, `#0x64`) plus two `muls` — **one** hit in the
    /// whole binary. The twenty instructions then match KTF's one for one:
    /// `movs #0xd; muls; adds #0xb4; ldrsh; adds #0x82; muls; <<1; adds #0xa; muls;
    /// movs r1,#0x64; bl divide; if (gain<=0) gain=1; [r4,#0x30] += gain;
    /// [r4,#0x30] += enemy[0xb4]`.
    ///
    /// ★ Corroborated ARITHMETICALLY, which is stronger than the pattern match: the
    /// routine has exactly one call site in the image (it is reached through a function
    /// pointer, so the only reference is the literal `0x4d389` at `0x3a6b0`), and that
    /// caller does `r0 = getCHAR-ish(); r0 += 0x8c; addExpFromKill(r0, enemy);` then
    /// calls `checkLevelUp` (`0x4d714`). The accessor (`0x1068`) returns `0x1503ab0`,
    /// i.e. `CHAR + 0x28`, so `+0x8c` is `CHAR + 0xb4` — the stats sub-struct `H` this
    /// crate already documents — and the routine's `[r4,#0x30]` is therefore
    /// `CHAR + 0xe4`, which is exactly `items::EXP_OFF` as measured on LGT in §2.7.
    /// A wrong site could not produce that agreement.
    pub fn exp_divisor_site(self) -> Option<u32> {
        match self {
            Self::Ktf => Some(0x14_654e),
            Self::Lgt => Some(0x04_d3cc),
            Self::Skt => None,
        }
    }

    /// The game's own `checkLevelUp`, THUMB-bit included, when located on this build —
    /// the whole of what the 천상의 영약 cash item calls. `None` = the item consumes
    /// with no effect on this build, so the host must not arm a grant.
    ///
    /// LGT `binary.mod` (01.00.07): **`0x4d715`** (`0x4d714|1`), named by the
    /// `exp_divisor_site` RE above — the kill path's caller invokes `addExpFromKill`
    /// and then this. Disassembled in full (2026-08-13, from the player-supplied
    /// game zip whose `binary.mod` is md5-identical to the shipped one):
    ///
    /// * `checkLevelUp()` takes NO arguments — it fetches `CHAR+0x28` through the
    ///   game's own accessor (`0x1068`) and loops (≤15 times): while
    ///   `exp (CHAR+0xe4) > expToNext (CHAR+0xe8)`, call `doLevelUp` (`0x4d690`).
    /// * `doLevelUp`: level `CHAR+0xe2` +1 — at the 0x46 (70) cap it clamps, pins
    ///   `exp = expToNext` and returns (★ 111차: `binary_patches.toml` rewrites that
    ///   pair to 0x63 = 99, and the two point clamps to 392 / 58); otherwise it recomputes the derived block
    ///   (`0x48f48`, `0x49000` — expToNext among them), refills HP to `CHAR+0xec`
    ///   and SP to 1000 (the full-restore pair `full_restore_writes` mirrors),
    ///   grants +4 stat points (`0x4d610(4)`; cap 276 = 69 levels × 4 ✓) and +1
    ///   skill point (`0x4d648(1)`) for levels ≤ 20 and even levels after (total 44
    ///   = `SKILL_POINT_CAP` ✓), and fires the level-up effect through the callback
    ///   at `CHAR+0x9bb0`.
    ///
    /// So the host-side item is two steps, both of them the game's own mechanism:
    /// write `exp = expToNext + 1`, call this. Everything downstream — points, caps,
    /// refills, visuals, the multi-level loop — is game code. The call is deferred to
    /// field state (`items::in_field_state`) so it runs in exactly the context the
    /// kill path runs it in.
    ///
    /// KTF: not located (the shipped APK carries the LGT asset; port on demand).
    pub fn level_up_fn(self) -> Option<u32> {
        match self {
            Self::Ktf => None,
            Self::Lgt => Some(0x04_d715),
            Self::Skt => None,
        }
    }

    /// How to move the player to another scene on this build, and where the scene's
    /// monster table lives. See [`SceneWorld`] for the derivation of every field.
    ///
    /// KTF: not located. The two builds share the CHAR offsets (the array at
    /// `CHAR+0x9cbc` is the same on both) but share no code addresses at all, so
    /// `fn_load_scene` has to be found again there before this may return anything —
    /// and this table is all-or-nothing on purpose: the data offsets are useless
    /// without a way to make the game act on them.
    pub fn scene_world(self) -> Option<SceneWorld> {
        match self {
            Self::Ktf => None,
            Self::Lgt => Some(SceneWorld {
                fn_load_scene: 0x01_5f29,
                scene_id_off: 0x9e30,
                warp_request_off: 0x9e32,
                warp_dest_off: 0x9e3d,
                warp_entry_off: 0x9e3c,
                map_id_off: 0x9e44,
                spawn_species_off: 0x9e64,
                region_off: 0x9e70,
                creatures_off: 0x9cbc,
                creature_stride: 0x158,
                map_spawn_count_off: 0x34,
                rec: CreatureRecord {
                    level: 0x70,
                    hp_max: 0x7c,
                    hp_cur: 0x7e,
                    hp_spawn: 0x82,
                    respawn_timer: 0x74,
                    hp_regen_permille: 0x84,
                    atk_base: 0x8c,
                    atk_eff: 0x8e,
                    atk_skill1: 0x90,
                    atk_skill2: 0x92,
                    def_phys_base: 0x68,
                    def_phys_eff: 0x6c,
                    def_spec_base: 0x6a,
                    def_spec_eff: 0x6e,
                    pct_atk_a: 0x40,
                    pct_atk_b: 0x42,
                    pct_def_phys: 0x44,
                    pct_def_spec: 0x46,
                    vhp: 0x100,
                    vhp_max: 0x104,
                    dmg32: 0xfc,
                    actor: 0x00,
                    hit: 0x78,
                    dodge: 0x7a,
                    crit_res: 0x96,
                },
            }),
            Self::Skt => None,
        }
    }

    /// How to relax the equip-level check ("장비를 장착할수 없습니다") on this build,
    /// when the site is located AND verified: `None` means the buff stays host-side
    /// only and the game keeps enforcing the shipped requirement.
    ///
    /// The check is one site in the whole image on BOTH builds — the only caller of
    /// `showMessage(0x67)` — but the two compiles need different rewrites, which is why
    /// the variant carries the SHAPE and not just an address (the full disassembly of
    /// both sites lives with the patch logic, `items::equip_relief_patch`):
    ///
    /// * KTF keeps the required level in a register move (`adds r1,r3,#0` at
    ///   `0x13d0aa`), so one halfword (`subs r1,r3,#N`) does it.
    /// * LGT compares the freshly-loaded byte directly, so there is no move to rewrite.
    ///   The patch goes on the OTHER operand instead: the player-level load
    ///   `movs r3,#0; ldrsh r1,[r4,r3]` at **`0x477ce`** becomes
    ///   `ldrh r1,[r4,#0]; adds r1,#N` (check `0x477cc`..`0x477dc`, `showMessage` =
    ///   `0x46b7c`). [`EquipReliefSite::LevelBias`] carries the full argument for why
    ///   that is provably equivalent and why the requirement-side rewrite §2.8
    ///   proposed is not.
    pub fn equip_relief_site(self) -> Option<EquipReliefSite> {
        match self {
            Self::Ktf => Some(EquipReliefSite::MoveHalfword { site: 0x13_d0aa }),
            Self::Lgt => Some(EquipReliefSite::LevelBias { site: 0x4_77ce }),
            Self::Skt => None,
        }
    }

    /// The three one-byte immediates that place the game's own BUFF BAND horizontally,
    /// as `(address, pristine opcode high byte, this site's delta from the band origin)`.
    /// Empty = not located on this build, and then the band keeps the game's own `x = 1`.
    ///
    /// The band's x has no origin global (unlike its y): each of the three draws computes
    /// `cellWidth * i + K` with its own `adds rN, #imm8`, so all three have to move
    /// together or the band tears apart (measured on KTF: patching only the frame left
    /// the icons at the far left). `items::band_x_patches` re-checks the opcode byte
    /// before writing, so a build this was not measured on is left alone.
    ///
    /// ── KTF `client.bin`, renderer `0x149b30` [workflow wf_69d1cba7 RC1, HIGH] ────
    ///   `0x149c2e adds r1,#1` frame · `0x149c8c adds r4,#3` icon · `0x149d2c adds r1,#1`
    ///   tick number (its own x is `w*i + w/2 + K`).
    ///
    /// ── LGT `binary.mod` 01.00.07, renderer `0x22a4` (this table's addition, 10차) ──
    /// Disassembled `0x22a4..0x2484` — the band loop, bounded by its own guard
    /// (`0x2320 ldrsh r3,[CHAR+0x16c]; bgt` → else `b 0x2484`, the target box) and its
    /// counter (`[sp,#0xa0]`, `i` as a u8, compared against that same halfword). The
    /// three sites fall out of the arithmetic, not out of a pattern match:
    /// ```text
    ///   0x233e  w  = ldrsh [LAY,#0x30]      ; ui_bm frame-3 width (the cell pitch, 20)
    ///   0x2350  r1 = w * i                  ; i = (s8)[sp,#0xa0]
    ///   0x2372  adds r1,#1                  ; <= FRAME   x = w*i + 1
    ///   0x23ba  r5 = w * i
    ///   0x23ec  adds r5,#3                  ; <= ICON    x = w*i + 3   (+2 of the frame)
    ///   0x2444  r1 = w * i
    ///   0x2446  r3 = w >> 1 ; adds r1,r1,r3
    ///   0x2450  adds r1,#1                  ; <= NUMBER  x = w*i + w/2 + 1
    /// ```
    /// ⚠ `0x237c movs r3,#0x14` in the middle of that range is the 20-byte BUFF RECORD
    /// stride (`muls r3,r5,r3` with `r5 = i`), not a coordinate — the same trap KTF's
    /// `0x149c48` documents. The y side is a single global at `.bss 0x1503a50`
    /// (`+2` frame, `+5` icon, `+h-7` number; the target box below shares it), so it is
    /// the x that has to be patched per site.
    pub fn band_x_sites(self) -> &'static [(u32, u16, u8)] {
        match self {
            Self::Ktf => &[
                (0x14_9c2e, 0x3100, 0), // frame:  K = X
                (0x14_9c8c, 0x3400, 2), // icon:   K = X + 2
                (0x14_9d2c, 0x3100, 0), // number: K = X
            ],
            Self::Lgt => &[
                (0x00_2372, 0x3100, 0), // frame:  K = X
                (0x00_23ec, 0x3500, 2), // icon:   K = X + 2   (r5 here, r4 on KTF)
                (0x00_2450, 0x3100, 0), // number: K = X
            ],
            Self::Skt => &[],
        }
    }

    /// Every immediate that places the game's own TARGET-MONSTER box horizontally, as
    /// `(address, pristine halfword, this site's signed constant)`. Empty = not located,
    /// and then the box stays right-edge anchored — i.e. underneath our minimap panel.
    ///
    /// Every x in the block is `W - FW + K` (W = screen width, FW = the 85-px frame-4
    /// width) or `W + K`, and each ends in its own add/sub-imm8 on r1, so shifting the
    /// box left by N just makes every K become `K - N`. Both encodings are 16-bit Thumb,
    /// so the instruction never changes length, and on BOTH builds every one of these is
    /// followed by another flag-setting ALU op or a `bl` before any conditional — which
    /// is what makes rewriting ADDS as SUBS safe. `TARGET_BOX_SHIFT_MAX` (211 = 255 - 44)
    /// is bound by the `subs r1,#0x2c` site, which is the same on both builds.
    ///
    /// ── KTF `client.bin`: TWELVE sites, `0x149db2..0x149f78` + a byte-identical second
    /// copy at `0x14a6aa..0x14a88c` (the selector at `0x149d5a` reads `(s8)CHAR[0x1ea]`
    /// and reaches the second copy through a `bl` used as a long branch). Patching one
    /// copy moved the box for one target kind and not the other.
    ///
    /// ── LGT `binary.mod` 01.00.07: **TEN**, and the difference is structural, not a
    /// missed site (10차). The same two paths exist — selector `0x2484` on the same byte,
    /// path A `0x2498..0x2608`, path B `0x2660..0x27e2` — but the two HP-bar-fill draws
    /// are **shared**: path A ends in `b 0x27e4` and path B falls through into the same
    /// `0x27e4`, so the last two immediates are reached by both and exist once.
    /// ```text
    ///   path A                     path B                     both
    ///   0x252a subs r1,#2          0x2706 subs r1,#2          0x282e adds r1,#0x1f
    ///   0x2560 adds r1,#1          0x273c adds r1,#1          0x285a adds r1,#0x1f
    ///   0x259a subs r1,#0x2c       0x2776 subs r1,#0x2c
    ///   0x25f4 adds r1,#0xf        0x27d0 adds r1,#0xf
    /// ```
    /// W is `(s16)[sp,#0x94]` (`ldr r3,[ctx,#0x44]` at `0x22f2`) and FW is
    /// `ldrsh [[r7],#0x40]`, re-derived at every site exactly as on KTF — which is why
    /// there is no single shared source to patch instead of these ten.
    pub fn target_box_x_sites(self) -> &'static [(u32, u16, i32)] {
        match self {
            Self::Ktf => &[
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
            ],
            Self::Lgt => &[
                // PATH A - the same selector byte > 0
                (0x00_252a, 0x3902, -2),  // the 85x26 frame
                (0x00_2560, 0x3101, 1),   // HP-bar background rect
                (0x00_259a, 0x392c, -44), // name / level text (x = W - 44, no FW)
                (0x00_25f4, 0x310f, 15),  // second text run
                // PATH B - the else arm, inline here rather than outlined
                (0x00_2706, 0x3902, -2),
                (0x00_273c, 0x3101, 1),
                (0x00_2776, 0x392c, -44),
                (0x00_27d0, 0x310f, 15),
                // SHARED TAIL - both paths reach these two
                (0x00_282e, 0x311f, 31), // HP-bar fill, left end
                (0x00_285a, 0x311f, 31), // HP-bar fill, right end
            ],
            Self::Skt => &[],
        }
    }

    /// May the host WRITE the five-slot quickbar arrays on this build?
    ///
    /// The four arrays are `CHAR+0x252` (kind), `+0x257` (item ref), `+0x25c` (skill
    /// ref) and `+0x26c` (strip cell), all read-only until now because a wrong offset
    /// here does not fail loudly — it sets a byte in the middle of the character record.
    ///
    /// ── WHY LGT IS TRUE (MIGRATION_LGT.md §2.16, 8차) ────────────────────────────
    /// The 퀵슬롯등록 A/B measured them from the other side: poking the popup's row byte
    /// to `1키등록` and letting the GAME do the bind made `CHAR+0x252` become 2 (kind =
    /// item) and `CHAR+0x257` become 6 (inventory slot 6 — the potion the cursor was
    /// on), with the untouched run leaving both at zero. The game's own writer landing
    /// on exactly these offsets is the verification; nothing here is inferred from KTF.
    ///
    /// SKT is `false` for the reason the flag exists: nothing has been measured on it.
    pub fn quickbar_writes_ok(self) -> bool {
        match self {
            Self::Ktf | Self::Lgt => true,
            Self::Skt => false,
        }
    }

    /// May the host read the smith's 제련 session and print the live odds over it?
    ///
    /// A BOOLEAN and not an offset table on purpose: every address the readout needs is
    /// already named — and unit-tested — in `refine.rs`, and CLAUDE.md's rule about two
    /// copies of a guest offset disagreeing applies to this crate as much as to the
    /// inventory cursors. This flag says only "that layout is the one this build has".
    ///
    /// ── WHY LGT IS TRUE (71차-I) ────────────────────────────────────────────────
    /// The context base is constant-folded from two literals in the image itself —
    /// `@0x106c = 0x01503ab0` (CHAR+0x28) plus `@0x1fd1c = 0xa838` — so it is a fixed
    /// `.bss` static, not a pointer that could be stale. Every field the readout reads
    /// is loaded by the smith's own handlers at named addresses (the roll `0x25234`, the
    /// spinner keys `0x215f4`/`0x21608`/`0x2161a`/`0x21630`, the view builder `0x4db50`).
    ///
    /// ⛔ KTF and SKT are `false`, and that is the fail-closed direction: on KTF the
    /// character record is reached through a POINTER global, so `CHAR + 0xa860` is not
    /// even the same kind of address, and none of the smith's code was ever located
    /// there. `false` means the bar is simply never drawn — the readout is display-only,
    /// so there is nothing else to disable.
    ///
    /// ★ This gates READS ONLY. The feature writes no guest byte and synthesises no key.
    pub fn refine_odds_ok(self) -> bool {
        match self {
            Self::Lgt => true,
            Self::Ktf | Self::Skt => false,
        }
    }

    /// `skill_bind(cat, cell, slot)` — the game's own quickbar skill binder, Thumb bit
    /// set. `None` = not located on this build, and then a page restore writes the four
    /// arrays itself from the stored ref byte (`quickpage::Action::BindSkill` carries the
    /// argument for why that is equivalent). It is NOT a reason to disable page flips:
    /// clears and item binds never needed this function at all.
    ///
    /// KTF `0x145c48`: callable because it takes its arguments in registers and never
    /// touches the caller's frame (verified across `0x145c48..0x145cf8`).
    ///
    /// LGT: unlocated. The two static routes both came up empty — no literal anywhere in
    /// `.text`/`.data` points into the quickbar arrays (they are reached as
    /// `getter() + imm`, and `getter()` is `CHAR+0x28`), and the game's own binder runs
    /// inside the popup dispatcher, which §2.16 already showed the host can drive with a
    /// synthetic `OK` instead of a call.
    pub fn skill_bind_fn(self) -> Option<u32> {
        match self {
            Self::Ktf => Some(0x0014_5c48 | 1),
            _ => None,
        }
    }

    /// Offset of the auto-loot flag inside the character record, per build.
    /// `None` = not located, so the host must not force it (a guest WRITE at a shifted
    /// offset silently corrupts the record — the reason §2.13 deferred this).
    ///
    /// ── WHY THIS IS ITS OWN GATE NOW (2026-08-13) ────────────────────────────────
    /// [`items::autoloot_to_force`] used to ask [`guest_code_addresses_valid`], which is
    /// true only on KTF. The APK ships LGT, so the forcing loop in `emulator.rs` had
    /// **never once written the byte on a real device** — and the server had meanwhile
    /// delisted the `오토루팅` cash item on the grounds that the host did it, so the
    /// player ended up with neither. The item is listed again
    /// (`hero3_protocol.AUTOLOOT_IDX`) and this table is what will turn the host path
    /// back on for LGT without loosening the flag for everything else.
    ///
    /// ── KTF ──────────────────────────────────────────────────────────────────────
    /// `CHAR+0x2d7`, and the whole chain is verified: `useItem`'s dispatcher bounds the
    /// item-DB index with `cmp #0x10` at `0x153f02` and jumps through the 17-entry table
    /// at `0x1a923c` (entries are offsets relative to the table itself, reached as
    /// `sl + 0xffff65fc`). Reading that table out of `client.bin` reproduces every arm
    /// this repo has ever cited — `[0x07] 오브원석 -> 0x15403e`, `[0x0a] 비밀의상자` and
    /// `[0x0b] 부활의서 -> 0x154542` (the consume-only tail, i.e. no client effect),
    /// `[0x0c] 컵라면 -> 0x1540cc`, `[0x0e] 여행가방 -> 0x154106` — and `[0x0d] 오토루팅
    /// -> 0x1540f6`, whose body is the five instructions this offset comes from:
    ///
    /// ```text
    /// 0x1540f6  bl   0x14ad10          ; -> r0 = CHAR
    /// 0x1540fc  ldr  r1,[pc,#0x12c]    ; literal 0x000002d7
    /// 0x1540fe  adds r2,r3,r1
    /// 0x154100  movs r3,#1
    /// 0x154102  strb r3,[r2]           ; CHAR+0x2d7 = 1
    /// 0x154104  b    0x154542          ; shared tail (decrement quantity)
    /// ```
    ///
    /// ── The FIRST LGT attempt, which was wrong — kept because the trap is a good one ──
    /// This nearly went in as `Some(0x2d7)` on evidence that looked conclusive and was
    /// not.
    ///
    /// The 오토루팅 item was bought on an S25 (`<S25_SERIAL>`) and used while
    /// [`items::flag_window_changes`] watched `CHAR+0x280..0x300`. Exactly one non-counter
    /// byte moved, to exactly the value KTF's arm writes, and stayed there:
    ///
    /// ```text
    /// 03:23:32.401  CHAR+0x2d7 (0x1503d5f)  0x81 -> 0x01   <- and never moved again
    /// ```
    ///
    /// KTF's offset, KTF's value, and auto-loot did start working. Three independent
    /// agreements. **All three were coincidence.** Writing `1` there from the host on a
    /// fresh install (vc51, `forcing ON at 0x1503d5f (write ok = true)`, byte held at 1
    /// for minutes) produced NO auto-loot. The item working in the earlier trial was the
    /// ITEM working, not the byte.
    ///
    /// The static evidence agreed all along and was not weighed: resolving all 647 CHAR
    /// getter call sites found NOTHING that reads or writes game offset `0x2af`
    /// (= this crate's `0x2d7`). A byte no code touches cannot be a gate. Its `0x00 ->
    /// 0x81` five seconds into the process — i.e. at save deserialisation — says what it
    /// really is: collateral of a BLOCK write, and `0x81 -> 0x01` was another one.
    ///
    /// Also refuted: `CHAR+0x2dd`, the static candidate below — it never changed at all.
    ///
    /// The lesson, and the reason the window is now the whole record: a byte changing at
    /// the right moment is NOT evidence on its own, because block writes move whole
    /// neighbourhoods — and a window chosen around the answer you expect can only confirm
    /// or mislead, never find. MIGRATION_LGT.md §2.22-(5).
    ///
    /// ── The static search that did NOT get there, kept so it is not repeated ──────
    /// * The game's own base is `CHAR+0x28`: the accessor thunk `0x1068`
    ///   (`ldr r0,[pc,#0]; bx lr`) returns **`0x1503ab0`**, called indirectly as
    ///   `ldr rN,[pc]=0x1069; bl <bx rN veneer>` from **647** sites. So a game offset of
    ///   `k` is this crate's `k + 0x28` — which independently re-confirms twelve of the
    ///   offsets in §2.11 (level `0xba`, exp `0xbc`, maxHP `0xc4`, HP `0xca`, SP `0xd0`,
    ///   stat/skill points `0x162`/`0x164`, the popup cluster `0x9c44..0x9c5f`, the
    ///   inventory grid handle `0x9e00`, window id `0xa098`), including the popup TAG
    ///   byte, whose six writers store exactly 100..105 — the `0x64` §2.13 measured.
    /// * Resolving all 647 call sites by local abstract interpretation yields exactly ONE
    ///   `strb #1` anywhere in `game+0x200..0x300`: **`0x51152`, into `game+0x2b5` =
    ///   `0x1503d65` = this crate's `CHAR+0x2dd`**, from an arm shaped exactly like KTF's
    ///   (getter, literal offset, `movs #1`, `strb`, `b` to a tail that decrements a
    ///   quantity at `+0x80` with a `0x14` stride).
    /// * But that arm is entry **`[0x05]`** of the dispatcher at `0x508fc` (bound
    ///   `cmp #0x10`, table `0x5e224`, tail `0x51248`), and that dispatcher is NOT
    ///   `useItem`: its `[0x00]..[0x03]` are four distinct arms where cat-0x12's
    ///   포션/하이포션/미라클포션/엘릭서 must share one, and its records are a `0x14`-stride
    ///   array with id at `+0x7c`, not the `0x3c4`/`0x3c` inventory grid KTF's `useItem`
    ///   indexes. It is most likely skill/action use.
    /// ── LGT: `CHAR+0x2d8`, VALUE `0x81` — found by watching the WHOLE record ─────
    /// The second trial replaced the guessed window with all 43 KB of the record, and the
    /// answer was one byte away from KTF's and carried a different value — which is
    /// exactly why the first attempt failed even after it "matched":
    ///
    /// ```text
    /// 03:47:16.427  +0x9c84=1 +0x9c85=3 +0x9c87=0x69     item popup opens (tag 105)
    /// 03:47:33.490  CHAR+0x2d8 (0x1503d60)  0x00 -> 0x81  <== and still 0x81 a minute on
    ///               +0x9c84 1->0  +0x9c85 3->0            popup closes = item consumed
    /// ```
    ///
    /// In the whole record that was the ONLY persistent 0 -> non-zero at the moment of
    /// use, and the same run reports `+0x2d5=0 +0x2d7=0 +0x2dd=0` while auto-loot is
    /// live, which retires all three earlier candidates on the spot.
    ///
    /// **The value is why this needs a pair and not just an offset.** KTF's arm stores
    /// literal `1`; LGT's stores `0x81`. Writing `1` (to the wrong byte, at that) is what
    /// the failed attempt did, and a reader that tests a particular bit would reject it
    /// even at the right address. So the host writes what the game writes.
    ///
    /// ── ★★ SOLVED (2026-08-15, 50차): IT IS AN ARRAY, INDEXED BY THE SAVE SLOT ──
    /// Everything below this line supersedes the doubt recorded in 49차 and the three
    /// "refuted" candidates above. **All of them were the same array, seen from
    /// different save slots.**
    ///
    /// LGT's `useItem` was finally located — the dispatcher at `0x550c8`
    /// (`ldrsh [item+0x3b6]; cmp #0x12` category gate, then `ldrsh [item+0x3b8];
    /// cmp #0x10; bls`, jump table `0x5e538`) with the `0x3c4`/`0x3c` inventory-grid
    /// strides KTF's has. 12차 examined a DIFFERENT dispatcher (`0x508fc`, table
    /// `0x5e224`) and rightly rejected it; this is the real one, cross-checked by three
    /// arms against behaviour this repo had already documented:
    /// * `[12]` = `HP = *(base+0xc4); SP = 1000` — the full-restore 컵라면, and those
    ///   game offsets are `CHAR+0xec`/`+0xf2`/`+0xf8` exactly as `items` has them;
    /// * `[14]` = "byte <= 4 ? +1" — 여행가방, KTF's `[0x0e]`;
    /// * `[13]` = 오토루팅, and `hero3_protocol.AUTOLOOT_IDX` is `0x0D`.
    ///
    /// The 오토루팅 arm, `0x5523c`:
    /// ```text
    ///   r4 = 0x1069 (getBase) ; base = getBase()          ; = CHAR + 0x28
    ///   r3 = *(u32*)(base + 0xa3e8)                       ; = *(CHAR + 0xa410)
    ///   r3 = r3 + base + 0x2ac
    ///   movs r2,#0x7f ; rsbs r2,r2,#0                     ; 0x81, built as -0x7f
    ///   strb r2,[r3,#3]                                   ; *(base + N + 0x2af) = 0x81
    /// ```
    /// `CHAR+0xa410` is not a new discovery — it is the SAVE SLOT NUMBER this table
    /// already names as [`SaveChain::slot_off`], which is why the save chain's own code
    /// (`0x1a5f0`, `0x1a6e4`, `0x1a982`, `0x1ad94`) reads the same word. So:
    ///
    /// ```text
    ///   auto-loot flag = CHAR + 0x2d7 + saveSlot        (one byte per slot)
    /// ```
    ///
    /// and every earlier observation falls out of it:
    /// * KTF's documented `CHAR+0x2d7` is the **slot 0** element;
    /// * 12차's device measurement of `CHAR+0x2d8` was **slot 1**;
    /// * `CHAR+0x2dd`, "refuted" above, is just **slot 6**;
    /// * the vc51 test that wrote the byte and got no auto-loot was on a slot the host
    ///   was not writing — the write was real, the address was another slot's.
    ///
    /// ★ And the VALUE is confirmed from the other side. The reader at `0x3ac02`
    /// recomputes the identical address and tests it SIGNED:
    /// ```text
    ///   ldrb r3,[r3,#3] ; lsls r3,#0x18 ; asrs r3,#0x18 ; cmp r3,#0 ; bge (skip)
    /// ```
    /// i.e. the gate is **bit 7**, so `1` can never pass it however right the address is.
    /// That retires the last mystery: 6차's "KTF value, KTF offset, and it worked" was a
    /// slot-0 character using the ITEM, and the host writing `1` afterwards did nothing
    /// because `1 >= 0`.
    ///
    /// ⚠ The reader CLEARS the byte on the branch it takes, so this is one-shot per
    /// pickup — which is exactly why the host re-arms it on a timer rather than once.
    ///
    /// ── the earlier, superseded record, kept because the METHOD errors are worth ──
    /// ── remembering even though the conclusions are now explained away ───────────
    /// ── ⚠ THE LGT PAIR WAS IN DOUBT (2026-08-15, 49차) ───────────────────────────
    /// Player: "오토루팅이 안돼", with the device log showing `+0x2d8=Some(129)` — the
    /// host writing exactly what this table asks for, and no auto-loot. Three static
    /// searches over the shipped `binary.mod` (md5 `6a352944…`) then came back empty:
    ///
    /// | question | answer |
    /// |---|---|
    /// | any pc-relative literal `0x2b0` (= this offset seen from the game's `CHAR+0x28` base) | **0 sites** |
    /// | any neighbouring sub-base literal (`0x290`..`0x2b0`) reaching it by displacement | **0** — all five use displacement 0 |
    /// | any `movs rX,#0x81` followed by a `strb` | **0 sites** (10 materialise `0x81`, all `strh` or shifted offsets) |
    ///
    /// The third is the damaging one: if the game never stores this VALUE as a byte, then
    /// the `0x00 -> 0x81` transition 12차 watched cannot have been a store aimed at this
    /// field — it was collateral of a wider write, which is the exact trap this comment
    /// already documents for `0x2d7` and `0x2dd`. Widening the window from a guess to the
    /// whole record fixed the WINDOW; it did not stop "a byte that changed at the right
    /// moment" from being weak evidence on its own.
    ///
    /// ★ Left in place rather than flipped to `None`, deliberately: the searches above are
    /// literal/displacement analysis and would MISS a runtime-computed index
    /// (`ldrb rX,[base, rIdx]` walking the record), so this is strong grounds for doubt and
    /// not a proof of absence. Turning the write off is a behaviour change and wants device
    /// evidence, not a second inference. What has changed is that nothing may now DESCRIBE
    /// this as a working feature — see the boot line in `emulator.rs`.
    ///
    /// To settle it: find LGT's `useItem` dispatcher (KTF's is `0x153f02`, `cmp #0x10` into
    /// the 17-entry table at `0x1a923c`, arm `[0x0d]` = 오토루팅 -> `0x1540f6`). The LGT
    /// candidate at `0x508fc` was examined in 12차 and rejected — its records are a
    /// `0x14`-stride array with the id at `+0x7c`, not the `0x3c4`/`0x3c` inventory grid —
    /// so the real one is still unlocated. Its 오토루팅 arm names the true flag in one step.
    pub fn autoloot_flag(self) -> Option<AutolootFlag> {
        match self {
            // KTF's arm stores a plain `1` at a plain `CHAR+0x2d7`, which is the slot-0
            // element of the same array — but no KTF build has been checked on a second
            // slot, so it stays unindexed rather than being assumed symmetrical.
            Self::Ktf => Some(AutolootFlag {
                base_off: 0x2d7,
                slot_off: None,
                value: 0x01,
            }),
            Self::Lgt => Some(AutolootFlag {
                base_off: 0x2d7,
                slot_off: Some(0xa410),
                value: 0x81,
            }),
            Self::Skt => None,
        }
    }

    /// The host-driven save chain (MIGRATION_LGT.md §4-3). `None` = unported.
    ///
    /// ── KTF ──────────────────────────────────────────────────────────────────────
    /// The values `save.rs` carried since 19차 (RE_SAVE_LEVELCAP_EXP_POINTS.md):
    /// `serialiseSave 0x1960bc` / `slotSummary 0x196b90` / `writeDbRecord 0x1951b0`,
    /// scratch = 4 bytes inside the reachability-proven dead range
    /// `[0x14a32e, 0x14a6aa)`, slot at `CHAR+0xa3f8`, names read out of the `sl` table.
    ///
    /// ── LGT (this table's addition, 2026-08-11 9차) ──────────────────────────────
    /// The five sites loading BOTH name strings (§2.13) were disassembled; the save
    /// routine is the one at `0x2515e..0x25186`, and it hands the whole contract over:
    ///
    /// ```text
    /// 0x2515e  outLen = 0 (stack); r0 = &outLen
    /// 0x25166  bx [0x25200]=0x1ad89   →  serialiseSave(&outLen) -> r0 = buf
    /// 0x25172  bx [0x25204]=0x1a265   →  writeSaveRecord([0x25208]="GameSave_11"@0x5bbb4, buf, outLen)
    /// 0x25178  bx [0x2520c]=0x1a6d1   →  slotSummary() -> r0 = buf16
    /// 0x25182  bx [0x25204]           →  writeSaveRecord([0x25210]="SlotSave_11"@0x5bbc0, buf16, 0x10)
    /// ```
    ///
    /// `writeSaveRecord = 0x1a264` and the caller's argument shapes were already
    /// established independently in §2.13 (sprintf `"%s_%d"` with `*(CHAR+0xa410)`),
    /// so the two new functions cross-check: the routine that calls the §2.13 writer
    /// twice with the §2.13 strings is the game's own save, and the two producers of
    /// its 2nd argument are exactly the two functions §4-3 was missing.
    ///
    /// ★ Scratch = `.bss` TAIL, not dead code — a stronger property than KTF's:
    /// `.bss` is declared `0x1500000..0x1512958` but the loader maps sections
    /// page-rounded (`core.load` → `next_multiple_of(0x1000)`), so
    /// `0x1512958..0x1513000` is mapped RW yet outside everything the ELF declares.
    /// Proof, three independent legs (2026-08-11):
    ///   1. STATIC: no literal word anywhere in `.text`/`.data` points at or past
    ///      `0x1512958` (the maximum is `0x1512954`, the last declared word), and the
    ///      largest literal+imm5 reach (`+0x7c`, Thumb-1 word load) tops out at
    ///      `0x15129d0` — `0x1512f00` keeps an 0x530-byte margin past it.
    ///   2. MAPPED: a probe `--poke 100:0x1512f00:5aa5c33c` wrote and read back fine.
    ///   3. LIVE: that sentinel survived byte-identical through boot, 7,800 frames of
    ///      play and the game's OWN save (GameSave_11_0 1408 B dropped) to frame 9200.
    pub fn save_chain(self) -> Option<SaveChain> {
        match self {
            Self::Ktf => Some(SaveChain {
                fn_serialise: 0x0019_60bc | 1,
                fn_slot_summary: 0x0019_6b90 | 1,
                fn_write_record: 0x0019_51b0 | 1,
                scratch: 0x0014_a400,
                slot_off: 0xa3f8,
                names: SaveNames::SlPtrs {
                    game: 0x1b_2c40 + 0x138,
                    slot: 0x1b_2c40 + 0x13c,
                },
            }),
            Self::Lgt => Some(SaveChain {
                fn_serialise: 0x0001_ad88 | 1,
                fn_slot_summary: 0x0001_a6d0 | 1,
                fn_write_record: 0x0001_a264 | 1,
                scratch: 0x0151_2f00,
                slot_off: 0xa410,
                names: SaveNames::Direct {
                    game: 0x0005_bbb4,
                    slot: 0x0005_bbc0,
                },
            }),
            Self::Skt => None,
        }
    }

    /// The game's own "connection lost" modal, per build. See [`NetDisconnectModal`]
    /// for the disassembly and for why the host has to raise it by hand.
    ///
    /// LGT `binary.mod` 01.00.07 — every address below was re-derived from the shipped
    /// asset (`assets/hero3_fixed.zip` → `0002865E.jar` → `binary.mod`, md5
    /// `6a352944…`) before it shipped, because the handover that produced them was
    /// stopped before its verification pass ran:
    /// * `getBase 0x1068` literal = `0x1503ab0`, so every `base+off` below is that
    ///   plus the offset the instruction actually carries;
    /// * `showModal 0x4684`'s literal pool: `0xa39c` ctx, `0xa43c` text, `0xa7fc` type,
    ///   `0xa438` state, `0xa7fd` flag — the exact five this struct names;
    /// * both dispatch tables replayed out of the initializer at `0x59f0`:
    ///   `modalKey[5] = 0xbe31`, `modalDraw[5] = 0xb609`, and `0x6164`/`0x60ac` index
    ///   them with `*(base+0xa438)`.
    ///
    /// KTF: not located. The APK ships LGT; the equivalent box exists in that compile
    /// too (`showMessage`-class), and this stays `None` until someone finds it, which is
    /// the per-item discipline [`hud_bottom_patch`](Self::hud_bottom_patch) documents.
    pub fn net_disconnect_modal(self) -> Option<NetDisconnectModal> {
        match self {
            Self::Ktf => None,
            Self::Lgt => Some(NetDisconnectModal {
                state_word: 0x0150_dee8,      // base + 0xa438
                disconnect_state: 5,          // teardown + back to the online main screen
                text_buf: 0x0150_deec,        // base + 0xa43c
                text_cap: 0x100,              // the memset width in showModal
                panel_flag: 0x0150_e2ac,      // base + 0xa7fc
                sub_flag: 0x0150_e2ad,        // base + 0xa7fd
                clear_word: 0x0150_de8c,      // base + 0xa3dc (ctx + 0x40)
            }),
            Self::Skt => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Ktf => "KTF",
            Self::Lgt => "LGT",
            Self::Skt => "SKT",
        }
    }
}
