// Host-driven save: the quick-save button (and the cash-item commit retry).
//
// ★★★ 2026-09-16 — THE 5-MINUTE AUTOSAVE IS GONE (user decision, 99차). Every save the
// host drives now happens because the player asked for it (SAVE) or because a cash item
// was just consumed (`Reason::Item`, so the consumption cannot be rolled back by a crash
// while the host-side buff survives). Nothing writes a character record on a timer any
// more, so any record write can be attributed to an action — see PLAY_FEEDBACK_FIXES.md
// §99차 for the audit that led here.
//
// The game's own save is five call sites, and NONE of them is a function the host can
// call: each is a jump-table arm that shares its parent's stack frame and exits through
// the parent's epilogue, so entering one directly pops garbage into `pc`. What IS
// callable is the four primitives they are built from (all ordinary functions with
// their own prologue/epilogue, each computing its own `sl`):
//
//   serialiseSave(u32* outLen) -> buf      guest 0x1960bc   (payload 1392 B as shipped)
//   slotSummary()             -> buf16     guest 0x196b90
//   writeDbRecord(name, ptr, len)          guest 0x1951b0   (appends "_<slot>" itself,
//                                                            then encrypts: the record
//                                                            on disk is len + 0x10)
//   canSave()                 -> (s8)*(CHAR+0xa280)   guest 0x192bd0 — read directly
//                                                     instead of called; it is a
//                                                     one-byte load and nothing else.
//
// The save is therefore a four-step chain, and step N+1's arguments come from step N's
// RETURN — which is why `Emulator::call_guest_function_out` had to exist. Each step is
// issued, then picked up on a later poll (guest calls run as cooperative tasks on the
// next tick, not inline).
//
// ★ The out-parameter scratch is FOUR BYTES INSIDE THE GAME'S OWN DEAD CODE
// (`SCRATCH`), not a guest allocation. `items::hud_bottom_patch` branches over the
// bottom-HUD block, and a reachability sweep of that function — following local `bl`
// targets, honouring every literal pool, and cross-checked against all direct branches,
// thumb function-pointer words and 179 jump-table bases in the image — proved
// [0x14a32e, 0x14a6aa) unreachable. Four bytes of DATA there execute nothing and are
// read by nothing. (Writing CODE there was the other option; it needed hand-assembled
// Thumb, so it lost.)
//
// ⚠ Known cost, inherent to using the game's own primitives: each save allocates
// (serialise allocates twice, the summary once, and the record writer allocates
// `len + 0x10` for the ciphertext) and NOTHING in the game's path frees any of it.
// The original only saves when the player asks, so the leak never mattered there.
// With the autosave gone (2026-09-16) the leak is one ~2.9 KB allocation per SAVE
// press, against a 256 MB guest heap — not worth a free routine.
//
// Dependency-free on purpose: `save_selftest.rs` `include!`s this file, so **do not add
// imports here**.

/// The per-build half of the chain: which guest functions to call and where the
/// serialise step's out-parameter lives. The VALUES live in `guest_anchors.rs`
/// (`GuestBuild::save_chain`, guarded by `anchors_selftest.rs`) — this mirror exists
/// because this file is `include!`d dependency-free by `save_selftest.rs`, so it
/// cannot name that type. `guest_build::save_chain()` converts between the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chain {
    /// `serialiseSave(u32 *outLen) -> buf`. Thumb bit set.
    pub fn_serialise: u32,
    /// `slotSummary() -> buf16`. Thumb bit set.
    pub fn_slot_summary: u32,
    /// `writeSaveRecord(name, ptr, len)` — appends `"_<slot>"` itself. Thumb bit set.
    pub fn_write_record: u32,
    /// Four bytes of guest memory proven out of the game's reach (per-build proof in
    /// `guest_anchors.rs`), used as the serialise out-parameter.
    pub scratch: u32,
}

/// KTF `client.bin` chain — the addresses this file carried as bare consts since 19차
/// (RE_SAVE_LEVELCAP_EXP_POINTS.md). Kept here as a named constant so the selftest can
/// keep asserting the RE-note values and exercising the step machine with them.
/// Scratch = 4 bytes inside the reachability-proven dead range `[0x14a32e, 0x14a6aa)`.
pub const KTF_CHAIN: Chain = Chain {
    fn_serialise: 0x0019_60bc | 1,
    fn_slot_summary: 0x0019_6b90 | 1,
    fn_write_record: 0x0019_51b0 | 1,
    scratch: 0x0014_a400,
};

/// LGT `binary.mod` (01.00.07) chain — 9차 (MIGRATION_LGT.md §2.13/§2.18-(5)): the
/// game's own save routine at `0x2515e..0x25186` calls exactly these, in this order,
/// with these argument shapes. Scratch = the page-rounding tail past the declared
/// `.bss` end (`0x1512958..0x1513000`): statically unreferenced (max literal
/// `0x1512954`, max literal+imm5 reach `0x15129d0`) and live-proven (a poked sentinel
/// survived boot + play + the game's own save untouched).
pub const LGT_CHAIN: Chain = Chain {
    fn_serialise: 0x0001_ad88 | 1,
    fn_slot_summary: 0x0001_a6d0 | 1,
    fn_write_record: 0x0001_a264 | 1,
    scratch: 0x0151_2f00,
};

/// The slot record is a fixed 16 bytes (`movs r2,#0x10` at guest 0x191520 / 0x1243fe;
/// the file on disk is 32 = 16 + the writer's 0x10 header). Same on LGT — the probe's
/// game-driven save drops a 32-byte `SlotSave_11_0`, and the save routine passes
/// `movs r2,#0x10` at `0x2517c`.
pub const SLOT_RECORD_LEN: u32 = 0x10;

/// Bounds of the KTF dead range the scratch must stay inside — asserted so the address
/// cannot drift out of the region the reachability proof covers.
pub const DEAD_LO: u32 = 0x0014_a32e;
pub const DEAD_HI: u32 = 0x0014_a6aa;

/// Compile-time proof that each build's scratch word stays inside the range its proof
/// covers, and stays 4-aligned. A const assert rather than a test because nudging a
/// scratch out of its proven range means the host scribbles on live guest memory —
/// that must not be something a passing build can do.
const _: () = assert!(KTF_CHAIN.scratch >= DEAD_LO && KTF_CHAIN.scratch + 4 <= DEAD_HI && KTF_CHAIN.scratch % 4 == 0);
/// LGT: past the maximum literal+imm5 reach (`0x15129d0`), inside the mapped page
/// (`< 0x1513000`).
const _: () = assert!(LGT_CHAIN.scratch >= 0x0151_29d0 && LGT_CHAIN.scratch + 4 <= 0x0151_3000 && LGT_CHAIN.scratch % 4 == 0);

// ── may the game save HERE? ──────────────────────────────────────────────────
//
// ★★ CORRECTED 2026-08-09, and 27차's version was wrong in BOTH the address and the
// polarity. It used `(s8)*(CHAR+0xa280)`, "0 = allowed", on the strength of guest
// `0x192bd0` loading that byte and returning it. That function is `getMenuCursor()`:
// `CHAR+0xa280` is the SELECTED ROW of the game's shared list widget, with the row
// count in `CHAR+0xa281`. Proof (three independent RE passes, then re-run by an
// adjudicator): the whole image contains exactly five writes to it, and they are the
// widget's constructor (`guest 0x19298c`, cursor = 0) and its `moveCursor`
// (`0x192c34 / 0x192c66 / 0x192c82 / 0x192cb0`, ±1 wrapping against `0xa281`). No map
// load, no field update and no save path touches it, and NOTHING resets it when a
// window closes — so it simply holds whatever row the player last stood on, forever.
//
// That is exactly what the device showed on 2026-08-09: a new LV.2 character on an
// ordinary outdoor field, the byte non-zero for 13 minutes straight, the SAVE button
// and the minimap gone the whole time, and the AUTOSAVE — which runs through the same
// gate — landing twice in fifty minutes instead of every five.
//
// The game's real test is a pointer chain through the current AREA object, and it is
// the OPPOSITE way round:
//
//     area    = *( *(u32*)(CHAR + 0x9c90) ) + 8      // same shape as the item grid
//     canSave = (s8)*(area + 0x11d) != 0             // ZERO = refuse
//
// Readers — the only two ways the game reaches its save window, and they agree:
//   guest 0x12c828  the `openWindow(14)` arm (id 14 decoded from the window jump
//                   table at guest 0x1a6f60); `beq` -> message 0x20, "cannot save"
//   guest 0x13c572  the field menu's save item; `beq` -> the same refusal
// Writers — and there are only two:
//   guest 0x124186  area init, sets it to 1 (= allowed)
//   guest 0x127f10  MAP LOAD, deserialises it out of the map's own data file
//
// ⚠ Because the value comes from the map DATA, `client.bin` alone cannot say which
// maps (if any) ship a 0. That is the one thing still unmeasured here — see
// `emulator.rs`'s `HIDE_UI_ON_NO_SAVE`, which is why this build no longer hides any
// UI on it.

/// CHAR-relative: handle of the current AREA object. `area = *(*(CHAR+0x9c90)) + 8`,
/// the same double indirection the inventory grid uses.
///
/// ★ NOT `CHAR+0x9c8c`. That is a CACHE of the resolved pointer which the guest only
/// refreshes while a save UI is opening (`guest 0x12c812` / `0x13c560`), so at any
/// other moment it is stale.
pub const AREA_HANDLE_OFF: u32 = 0x9c90;

/// AREA-relative: `(s8)` save permission. **NON-ZERO = allowed, ZERO = refuse**
/// ("저장할 수 없는 지역입니다").
pub const AREA_CAN_SAVE_OFF: u32 = 0x11d;
/// CHAR-relative: `u32` current save slot, the `_%d` the record writer appends.
/// ⚠ Per-build — the third documented offset that did NOT port (`0x7c→0x78`,
/// `0xa384→0xa370`, then this one, §2.13): KTF `0xa3f8`, LGT `0xa410`. The
/// authoritative per-build value is `guest_anchors::SaveChain::slot_off`; this const
/// stays as the KTF value for the selftest's arithmetic.
pub const SAVE_SLOT_OFF: u32 = 0xa3f8;
/// The game only ever shows a handful of slots; a wild value here means we are not
/// looking at a live character and must not write a record named after it.
pub const SAVE_SLOT_MAX: u32 = 15;

/// Plausible band for the serialised payload length.
///
/// ★ The length is CONTENT-DEPENDENT, not a per-build constant. Device-measured
/// 2026-08-04: a live LV.12 save serialises to **1808 B** (record on disk 1824 = 1808 +
/// the writer's 0x10), while the archived backup this was first reverse-engineered from
/// held 1392 (record 1408). An earlier version of this file asserted 1392 as "what this
/// build returns" and warned on anything else — that was wrong, and the warning fired on
/// the very first real save.
///
/// So the only honest check is a BAND: reject 0 and reject something that would blow the
/// writer's `alloc(len + 0x10)`, and accept whatever the game actually produced.
pub const PAYLOAD_MIN: u32 = 64;
pub const PAYLOAD_MAX: u32 = 8192;

/// How many polls a step may take before the job is abandoned. Guest calls land on the
/// next `System::tick`; the worker polls once per frame, so this is generous by ~100x
/// and exists only so a call that never completes cannot wedge the machine forever.
pub const STEP_TIMEOUT_POLLS: u32 = 240;

/// Why a save is running (log + whether the player gets told).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The player tapped SAVE. Worth a toast.
    Manual,
    // ⛔ `Auto` WAS HERE (the 5-minute timer and the quit-time save) and is gone
    // (2026-09-16, 99차). Do not add a timer-driven reason back: the whole point of
    // removing it is that every record write is now something the player did.
    /// A cash item was just used (경험의 보주 etc., 2026-08-07): the host granted the
    /// effect, so the consumption must stick — without a save a crash rolls the item
    /// count back while the host-side buff file keeps the effect, a free duplicate.
    /// Silent (no toast). Armed as a RETRY (the use happens with the inventory
    /// window open, where the gate refuses) and fires once the player is back on a
    /// field the game itself would save in. Kept on the 2026-09-16 audit on purpose —
    /// it is the one non-manual save that still exists, and it is action-driven.
    Item,
    /// ★★★ 72차-D (TODO `BB` ⓑ) — REBUILD A SLOT SUMMARY, AND NOTHING ELSE.
    ///
    /// The slot row is decided by `SlotSave_11_<slot>` alone (guest scan `0x5c06`), and
    /// the four highest-frequency saves go through `saveGame 0x4748`, which never calls
    /// `slotSummary`. So an intact character can be invisible on the slot list, which is
    /// the 「빈슬롯」 half of the 제련 report.
    ///
    /// This job runs the last TWO steps of the chain only: `slotSummary` and the write.
    /// It does NOT serialise, so it neither saves progress nor commits anything — see
    /// the `Reason::Summary` arms in `emulator.rs::save_poll`, which is where that
    /// distinction has to be respected (no toast, no backup upload, no cash-baseline
    /// clear, and a queued cash-item retry stays queued).
    Summary,
    /// ★ 136차 (2026-09-25) — the player sorted the bag on the 네트워크 싱글가방 screen.
    ///
    /// The game saves the bag there only in its own transaction tails (deposit, withdraw,
    /// cash delivery — each ends in `saveGame 0x4748`) and never on the way out (probe:
    /// both CLRs write nothing). A host reorder has no such tail, so without this the sort
    /// would silently undo itself unless a deposit happened to follow. Player action, one
    /// full chain, no toast of its own (the sort already toasted); a failure logs.
    /// Gated by `items::sort_save_gate` — the online screen's twin of `save_gate`.
    Sort,
}

/// Which primitive the job is waiting on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Serialise,
    WriteGame,
    Summary,
    WriteSlot,
}

/// What the worker should do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Keep waiting.
    Wait,
    /// Issue this guest call and expect its result.
    Call { addr: u32, args: [u32; 3], argc: usize, step: Step },
    /// Finished successfully.
    Done,
    /// Give up. The string is a reason for the log.
    Failed(&'static str),
}

/// One save in flight. Only ever one at a time: a second serialise while the first
/// buffer is still unwritten would leak it and could interleave two records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Job {
    pub reason: Reason,
    pub step: Step,
    /// The build's chain — resolved once when the job started.
    pub chain: Chain,
    /// Serialised payload, from step 1.
    pub buf: u32,
    /// Payload length, read out of the chain's scratch after step 1.
    pub len: u32,
    /// 16-byte slot summary, from step 3.
    pub sum: u32,
    /// Polls spent on the current step.
    pub waited: u32,
    /// The two record base names ("GameSave…"/"SlotSave…"), resolved once when the job
    /// started (KTF: read out of the `sl` table; LGT: `.text` string addresses).
    pub name_game: u32,
    pub name_slot: u32,
    /// ★★★ 72차 (F-05) — DID THE HOST'S RECORD STORE ACTUALLY TAKE THE LAST WRITE?
    ///
    /// `writeSaveRecord` cannot tell us: `0x1a1d4` calls the DB writer and then two
    /// `free`s, so the value in `r0` on return is the last free's, not the write's. The
    /// guest's own `0x1c020` does keep `stream_write`'s -1 — and throws it away at
    /// `0x254b2`/`0x4790` too. So the only party that knows a write was refused is the
    /// host, and it used to keep that to itself: the chain wrote the SUMMARY over a
    /// payload that never landed, leaving a slot whose row says one character and whose
    /// bytes are another.
    ///
    /// The caller sets this from `vault`'s refusal counter around each write step. It is
    /// a plain field rather than an argument so this file stays dependency-free for
    /// `save_selftest.rs`'s `include!`, and it defaults to `true` so a caller that does
    /// not track refusals behaves exactly as before.
    pub last_write_ok: bool,
}

impl Job {
    /// Start a job. The two record-name pointers are resolved by the caller (see
    /// `items::save_gate`) and validated here.
    pub fn new(reason: Reason, chain: Chain, name_game: u32, name_slot: u32) -> Option<Self> {
        if !plausible_ptr(name_game) || !plausible_ptr(name_slot) {
            return None;
        }
        Some(Self {
            reason,
            step: Step::Serialise,
            chain,
            buf: 0,
            len: 0,
            sum: 0,
            waited: 0,
            name_game,
            name_slot,
            last_write_ok: true,
        })
    }

    /// Start a SUMMARY-ONLY job: rebuild `SlotSave_11_<slot>` from the live character
    /// and write it, without serialising or touching the payload.
    ///
    /// ★★★ 72차-D. `name_game` is left at 0 deliberately: the step machine is linear
    /// (`Summary` -> `WriteSlot` -> `Done`) and never revisits `WriteGame`, so the
    /// payload name is unreachable from here — and a 0 there is a hard failure if a
    /// future edit ever makes it reachable, which is the direction that fails safe.
    ///
    /// The caller must have proved that the write will land on the character the summary
    /// describes (`items::summary_repair_gate` + the pin), because `writeSaveRecord`
    /// appends `_<slot>` from the guest's own cursor word and cannot be told otherwise.
    pub fn summary_only(chain: Chain, name_slot: u32) -> Option<Self> {
        if !plausible_ptr(name_slot) {
            return None;
        }
        Some(Self {
            reason: Reason::Summary,
            step: Step::Summary,
            chain,
            buf: 0,
            len: 0,
            sum: 0,
            waited: 0,
            name_game: 0,
            name_slot,
            last_write_ok: true,
        })
    }

    /// The very first call, issued the moment the job is created — the step the job was
    /// constructed at, which is the serialise for an ordinary save and the summary for a
    /// [`Job::summary_only`] repair.
    pub fn first_call(&self) -> Next {
        match self.step {
            Step::Summary => Next::Call {
                addr: self.chain.fn_slot_summary,
                args: [0, 0, 0],
                argc: 0,
                step: Step::Summary,
            },
            _ => Next::Call {
                addr: self.chain.fn_serialise,
                args: [self.chain.scratch, 0, 0],
                argc: 1,
                step: Step::Serialise,
            },
        }
    }

    /// Feed the completed step's result in and get the next action.
    ///
    /// `result` = `None` while the guest call is still running, `Some(Ok(r0))` on
    /// return, `Some(Err(()))` if it could not be made. `scratch` = the current 32-bit
    /// value at `self.chain.scratch`, only consulted after the serialise step.
    pub fn advance(&mut self, result: Option<Result<u32, ()>>, scratch: u32) -> Next {
        let Some(result) = result else {
            self.waited += 1;
            return if self.waited > STEP_TIMEOUT_POLLS {
                Next::Failed("guest call never completed")
            } else {
                Next::Wait
            };
        };
        let Ok(ret) = result else {
            return Next::Failed("guest call failed");
        };
        self.waited = 0;
        match self.step {
            Step::Serialise => {
                if !plausible_ptr(ret) {
                    return Next::Failed("serialise returned an implausible buffer");
                }
                if !(PAYLOAD_MIN..=PAYLOAD_MAX).contains(&scratch) {
                    return Next::Failed("serialise reported an implausible length");
                }
                self.buf = ret;
                self.len = scratch;
                self.step = Step::WriteGame;
                Next::Call {
                    addr: self.chain.fn_write_record,
                    args: [self.name_game, self.buf, self.len],
                    argc: 3,
                    step: Step::WriteGame,
                }
            }
            Step::WriteGame => {
                // ⛔ Do NOT write the summary over a payload that was refused (72차).
                if !self.last_write_ok {
                    return Next::Failed("the record store refused the payload; not writing a summary over it");
                }
                self.step = Step::Summary;
                Next::Call {
                    addr: self.chain.fn_slot_summary,
                    args: [0, 0, 0],
                    argc: 0,
                    step: Step::Summary,
                }
            }
            Step::Summary => {
                if !plausible_ptr(ret) {
                    return Next::Failed("slot summary returned an implausible buffer");
                }
                self.sum = ret;
                self.step = Step::WriteSlot;
                Next::Call {
                    addr: self.chain.fn_write_record,
                    args: [self.name_slot, self.sum, SLOT_RECORD_LEN],
                    argc: 3,
                    step: Step::WriteSlot,
                }
            }
            Step::WriteSlot => {
                if !self.last_write_ok {
                    return Next::Failed("the record store refused the summary");
                }
                Next::Done
            }
        }
    }
}

/// Loose "could be a guest pointer" test. The live regions differ per build — KTF maps
/// its image at ~0x10_0000, but LGT's absolute address model starts `.text` at 0x1000
/// (the record-name strings sit at 0x5bbb4/0x5bbc0) — so the window opens at the lowest
/// mapped guest page. The WIPI heap (0x4000_0000..) is common to both. Null, tiny ints
/// and kernel-ish values still fail.
pub fn plausible_ptr(p: u32) -> bool {
    (0x1000..0x8000_0000).contains(&p)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real, device-measured payload length (LV.12 save, 2026-08-04) so the happy path
    /// is fed something the game actually produced rather than a round number.
    const PAYLOAD_MEASURED: u32 = 1808;
    const NG: u32 = 0x1a_68a0;
    const NS: u32 = 0x1a_68b0;
    const BUF: u32 = 0x4001_0000;
    const SUM: u32 = 0x4002_0000;

    fn job() -> Job {
        Job::new(Reason::Manual, KTF_CHAIN, NG, NS).unwrap()
    }

    #[test]
    fn each_scratch_word_lies_inside_its_proven_range() {
        assert!(
            KTF_CHAIN.scratch >= DEAD_LO && KTF_CHAIN.scratch + 4 <= DEAD_HI,
            "KTF scratch escaped the dead range"
        );
        // And it must not sit on the epilogue the HUD-skip patch branches TO.
        assert!(KTF_CHAIN.scratch > 0x14_a32c, "0x14a324..0x14a32d is the live epilogue");
        // LGT: past every literal+imm5 reach (max 0x15129d0), inside the mapped page.
        assert!(LGT_CHAIN.scratch >= 0x151_29d0 && LGT_CHAIN.scratch + 4 <= 0x151_3000, "LGT scratch escaped the .bss tail");
        for c in [KTF_CHAIN, LGT_CHAIN] {
            assert_eq!(c.scratch % 4, 0, "the out-param is a u32; keep it aligned");
        }
    }

    #[test]
    fn every_callable_address_carries_the_thumb_bit() {
        for c in [KTF_CHAIN, LGT_CHAIN] {
            for a in [c.fn_serialise, c.fn_slot_summary, c.fn_write_record] {
                assert_eq!(a & 1, 1, "{a:#x} would enter in ARM state");
            }
        }
        // The even addresses are the ones in the RE notes (KTF 19차 / LGT 9차).
        assert_eq!(KTF_CHAIN.fn_serialise & !1, 0x19_60bc);
        assert_eq!(KTF_CHAIN.fn_slot_summary & !1, 0x19_6b90);
        assert_eq!(KTF_CHAIN.fn_write_record & !1, 0x19_51b0);
        assert_eq!(LGT_CHAIN.fn_serialise & !1, 0x1_ad88);
        assert_eq!(LGT_CHAIN.fn_slot_summary & !1, 0x1_a6d0);
        assert_eq!(LGT_CHAIN.fn_write_record & !1, 0x1_a264);
    }

    #[test]
    fn the_happy_path_issues_exactly_the_games_own_four_calls() {
        // Both chains drive the same step machine; the ADDRESSES are the only per-build
        // part, so run the whole path once per chain.
        for chain in [KTF_CHAIN, LGT_CHAIN] {
            let mut j = Job::new(Reason::Manual, chain, NG, NS).unwrap();
            assert_eq!(
                j.first_call(),
                Next::Call {
                    addr: chain.fn_serialise,
                    args: [chain.scratch, 0, 0],
                    argc: 1,
                    step: Step::Serialise
                }
            );
            // serialise -> (buf, len)
            assert_eq!(
                j.advance(Some(Ok(BUF)), PAYLOAD_MEASURED),
                Next::Call {
                    addr: chain.fn_write_record,
                    args: [NG, BUF, PAYLOAD_MEASURED],
                    argc: 3,
                    step: Step::WriteGame
                }
            );
            // the GameSave write returns nothing useful
            assert_eq!(
                j.advance(Some(Ok(0)), 0),
                Next::Call {
                    addr: chain.fn_slot_summary,
                    args: [0, 0, 0],
                    argc: 0,
                    step: Step::Summary
                }
            );
            // summary -> 16-byte record
            assert_eq!(
                j.advance(Some(Ok(SUM)), 0),
                Next::Call {
                    addr: chain.fn_write_record,
                    args: [NS, SUM, SLOT_RECORD_LEN],
                    argc: 3,
                    step: Step::WriteSlot
                }
            );
            assert_eq!(j.advance(Some(Ok(0)), 0), Next::Done);
        }
    }

    #[test]
    fn a_bad_serialise_result_aborts_instead_of_writing_a_record() {
        // Null / tiny / kernel-ish buffer (0xfff = the last byte below the lowest
        // mapped guest page — LGT maps `.text` from 0x1000).
        for bad in [0u32, 1, 0xfff, 0xffff_ffff] {
            let mut j = job();
            assert!(matches!(j.advance(Some(Ok(bad)), PAYLOAD_MEASURED), Next::Failed(_)), "buf {bad:#x}");
        }
        // Implausible lengths: zero, too small, too big.
        for bad in [0u32, 1, PAYLOAD_MIN - 1, PAYLOAD_MAX + 1, 0xffff_ffff] {
            let mut j = job();
            assert!(matches!(j.advance(Some(Ok(BUF)), bad), Next::Failed(_)), "len {bad}");
        }
        // A bad SUMMARY must not write the slot record either.
        let mut j = job();
        j.advance(Some(Ok(BUF)), PAYLOAD_MEASURED);
        j.advance(Some(Ok(0)), 0);
        assert!(matches!(j.advance(Some(Ok(0)), 0), Next::Failed(_)));
    }

    #[test]
    fn a_call_that_never_lands_times_out_rather_than_wedging() {
        let mut j = job();
        for i in 0..STEP_TIMEOUT_POLLS {
            assert_eq!(j.advance(None, 0), Next::Wait, "poll {i}");
        }
        assert!(matches!(j.advance(None, 0), Next::Failed(_)));
    }

    #[test]
    fn a_failed_call_aborts_immediately() {
        let mut j = job();
        assert!(matches!(j.advance(Some(Err(())), 0), Next::Failed(_)));
    }

    #[test]
    fn waiting_does_not_accumulate_across_steps() {
        let mut j = job();
        for _ in 0..STEP_TIMEOUT_POLLS - 1 {
            j.advance(None, 0);
        }
        // The step completes just in time; the next step gets a full budget again.
        j.advance(Some(Ok(BUF)), PAYLOAD_MEASURED);
        assert_eq!(j.waited, 0);
        for i in 0..STEP_TIMEOUT_POLLS {
            assert_eq!(j.advance(None, 0), Next::Wait, "poll {i} of the second step");
        }
    }

    /// ★★★ 72차-D (TODO `BB` ⓑ) — the summary-only repair runs the LAST TWO steps and
    /// nothing else. What this pins is that it can never reach the payload: the job is
    /// constructed with `name_game = 0`, so a machine that wandered back to `WriteGame`
    /// would write a record named by a null pointer, and the only thing stopping that is
    /// the step sequence asserted here.
    #[test]
    fn a_summary_repair_writes_the_slot_record_and_never_the_payload() {
        for chain in [KTF_CHAIN, LGT_CHAIN] {
            let mut j = Job::summary_only(chain, NS).unwrap();
            assert_eq!(j.reason, Reason::Summary);
            assert_eq!(j.name_game, 0, "a repair has no payload name and must not acquire one");
            assert_eq!(
                j.first_call(),
                Next::Call { addr: chain.fn_slot_summary, args: [0, 0, 0], argc: 0, step: Step::Summary },
                "it starts at the summary, not at the serialise"
            );
            assert_eq!(
                j.advance(Some(Ok(SUM)), 0),
                Next::Call { addr: chain.fn_write_record, args: [NS, SUM, SLOT_RECORD_LEN], argc: 3, step: Step::WriteSlot }
            );
            assert_eq!(j.advance(Some(Ok(0)), 0), Next::Done);
            assert_eq!(j.len, 0, "nothing was serialised");
        }
        // The same guards as the full chain: a bad summary buffer writes no record, and a
        // refused write is reported rather than swallowed.
        let mut bad = Job::summary_only(KTF_CHAIN, NS).unwrap();
        assert!(matches!(bad.advance(Some(Ok(0)), 0), Next::Failed(_)));
        let mut refused = Job::summary_only(KTF_CHAIN, NS).unwrap();
        refused.advance(Some(Ok(SUM)), 0);
        refused.last_write_ok = false;
        assert!(matches!(refused.advance(Some(Ok(0)), 0), Next::Failed(_)));
        // And it still refuses to start without a real record name.
        assert!(Job::summary_only(KTF_CHAIN, 0).is_none());
        assert!(Job::summary_only(LGT_CHAIN, 0x5_bbc0).is_some());
    }

    #[test]
    fn a_job_refuses_to_start_without_real_record_names() {
        assert!(Job::new(Reason::Item, KTF_CHAIN, 0, NS).is_none());
        assert!(Job::new(Reason::Item, KTF_CHAIN, NG, 0).is_none());
        assert!(Job::new(Reason::Item, KTF_CHAIN, 0xffff_ffff, NS).is_none());
        assert!(Job::new(Reason::Item, KTF_CHAIN, NG, NS).is_some());
        // LGT's name arguments are the `.text` string ADDRESSES themselves (absolute
        // model, image mapped from 0x1000) — the plausibility window must admit them.
        assert!(Job::new(Reason::Item, LGT_CHAIN, 0x5_bbb4, 0x5_bbc0).is_some());
    }

    #[test]
    fn the_slot_bound_matches_the_record_naming() {
        // The writer formats "%s_%d" with this value; a wild slot would create records
        // the loader never looks for.
        assert!(SAVE_SLOT_MAX < 100, "the slot has to stay a short decimal");
        assert_eq!(SLOT_RECORD_LEN, 0x10);
    }
}
