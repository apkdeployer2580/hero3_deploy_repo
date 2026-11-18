#![no_std]
extern crate alloc;

use alloc::{format, string::String, vec::Vec};
use core::{
    any::Any,
    error::Error,
    fmt::{self, Display, Formatter},
    mem::{MaybeUninit, size_of},
    result,
    slice::from_raw_parts_mut,
};

use bytemuck::{AnyBitPattern, NoUninit, bytes_of};

#[derive(Debug)]
pub enum WieError {
    InvalidMemoryAccess(u32),
    AllocationFailure,
    JavaException(u32), // to pass java exception down to rust
    JavaExceptionUnwind { context_base: u32, target: u32, next_pc: u32 },
    Unimplemented(String),
    FatalError(String),
}

impl Display for WieError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            WieError::InvalidMemoryAccess(address) => write!(f, "Invalid memory access; address: {address}"),
            WieError::AllocationFailure => write!(f, "Allocation failure"),
            WieError::JavaException(exception) => write!(f, "Java exception: {exception:#x}"),
            WieError::JavaExceptionUnwind {
                context_base,
                target,
                next_pc,
            } => write!(
                f,
                "Java exception unwind: context_base={context_base:#x}, target={target:#x}, next_pc={next_pc:#x}"
            ),
            WieError::Unimplemented(message) => write!(f, "Unimplemented: {message}"),
            WieError::FatalError(message) => write!(f, "Fatal error: {message}"),
        }
    }
}

impl Error for WieError {}

pub type Result<T> = result::Result<T, WieError>;

/// ★ 150차 — the reworked accessories' third option (generated, `tools_re/gen_acc_options.py`).
pub mod acc_options;
/// ★ 151차: the 하늘성 조합서's 효과 line (GENERATED — `tools_re/gen_recipe_effects.py`).
pub mod recipe_effects;
/// ★ 167차: 아바타 옷장 — the costume numbers the paper-doll hooks and the host share.
pub mod avatar;

/// TEMPORARY measurement (2026-08-06): where does a frame actually go?
/// The game renders its world by writing the framebuffer from guest code, so the
/// interpreter — not the WIPI graphics layer — is the suspect. Counted here (a
/// no_std-safe atomic, bumped once per `ArmEngine::run` batch, not per
/// instruction) so `wie_probe` can print instructions/frame.
pub mod stats {
    use alloc::vec::Vec;
    use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};

    pub static GUEST_INSTRUCTIONS: AtomicU64 = AtomicU64::new(0);
    /// Frames the GUEST produced (one per `Screen::paint`). Not the compositor's rate:
    /// the host presents at the display's 60 Hz whether or not the emulator managed to
    /// draw anything new, so this is the only number that says how fast the game is
    /// actually running. The game targets 25.
    pub static GUEST_FRAMES: AtomicU64 = AtomicU64::new(0);
    pub static SVC_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static RUN_BATCHES: AtomicU64 = AtomicU64::new(0);
    /// ★ SVC MIX (2026-08-13). `SVC_CALLS` alone says the guest left the interpreter N
    /// times; it cannot say whether those were cheap ones (`memcpy` — a plain Rust
    /// helper) or expensive ones (`MC_netSocketRecv` — a context object, a boxed body
    /// and a socket syscall). On LGT the whole C stdlib is an SVC too, so the mix is
    /// the only way to attribute a stall. Keyed by `category` (1=init, 3=wipic,
    /// 5=stdlib, 8+=binary-patch hooks) and by the id the stub leaves in `r12`.
    ///
    /// Ids alias mod 2048 — deliberate: every id Hero3 actually uses (net `0x258..`,
    /// sockets `0x7d0..`, 901/904, and the small stdlib/graphics enums) is below that,
    /// so the aliasing only affects ids nothing calls.
    pub static SVC_BY_CATEGORY: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
    pub static SVC_BY_ID: [AtomicU64; 2048] = [const { AtomicU64::new(0) }; 2048];
    /// Guest-heap allocator traffic. `ALLOCS - FREES` is the live object count;
    /// if it climbs during steady play, that is the exhaustion the device crash
    /// ends in. `LIST_WALK` is the first-fit cursor cost, and `BUCKET_HEADER_BYTES`
    /// the per-allocation bookkeeping read out of guest memory — both were the
    /// measurements that justified the 2026-08-06 allocator rewrite (lazy
    /// coalescing in `ListAllocator::find_address`, and an indexed rather than
    /// scanned in-use map in `BucketAllocator`), so both should now stay flat
    /// where they used to climb.
    pub static ALLOCS: AtomicU64 = AtomicU64::new(0);
    pub static FREES: AtomicU64 = AtomicU64::new(0);
    pub static LIST_WALK: AtomicU64 = AtomicU64::new(0);
    pub static BUCKET_HEADER_BYTES: AtomicU64 = AtomicU64::new(0);

    pub fn add_alloc() {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_free() {
        FREES.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_list_walk(n: u64) {
        LIST_WALK.fetch_add(n, Ordering::Relaxed);
    }

    pub fn add_bucket_header(n: u64) {
        BUCKET_HEADER_BYTES.fetch_add(n, Ordering::Relaxed);
    }

    /// 64 KB guest pages that have actually been committed (see the lazy-`map` note in
    /// `wie_core_arm::engine::arm32_cpu`). Before that change this was pinned at 4096 pages
    /// (256 MB) for the heap alone the moment the allocator initialised; it is the number that
    /// says whether the low-memory-kill fix is doing its job on a device, where `dumpsys
    /// meminfo` is not available to the app itself.
    pub static GUEST_PAGES: AtomicU64 = AtomicU64::new(0);

    /// A LEVEL, not a counter: the engine republishes its own total once per `run` batch,
    /// because the page table is deliberately dependency-free and cannot reach this module.
    pub fn set_guest_pages(pages: u64) {
        GUEST_PAGES.store(pages, Ordering::Relaxed);
    }

    pub fn guest_page_bytes() -> u64 {
        GUEST_PAGES.load(Ordering::Relaxed) * 0x10000
    }

    /// Guest sprite blitter (`0x110fe4`) telemetry. The game rasterises its whole
    /// world through that one function — 26% of all guest instructions — so before
    /// reimplementing it natively we measure what it is actually asked to do:
    /// which pixel formats and transforms turn up, how big the blits are (an SVC
    /// round trip has to be cheaper than the guest loop it replaces), and how often
    /// the safety gate would refuse.
    pub static BLIT_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static BLIT_HLE: AtomicU64 = AtomicU64::new(0);
    pub static BLIT_PIXELS: AtomicU64 = AtomicU64::new(0);
    pub static BLIT_PIXELS_HLE: AtomicU64 = AtomicU64::new(0);
    /// Indexed by pixel format: [0] = 4bpp (0x0b), [1] = 8bpp (0x0c), [2] = neither.
    pub static BLIT_FMT: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
    /// Indexed by transform code 0..=8, with [9] collecting everything out of range.
    pub static BLIT_TRANSFORM: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];
    /// Why the gate refused: [0] format, [1] transform, [2] geometry (w/h/stride/
    /// negative origin/row wrap), [3] guest memory not readable, [4] null argument.
    pub static BLIT_REFUSED: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

    /// Master switch for the host sprite blitter. Lives here, beside the counters, so
    /// the host binaries can flip it without an export chain through `wie_core_arm`.
    /// ★ It is what makes the A/B frame-hash check possible: the SAME binary and the
    /// SAME deterministic input, run twice, differing only in this bool. Any pixel
    /// difference is then unambiguously the host blitter's fault.
    pub static BLIT_HLE_ENABLED: AtomicBool = AtomicBool::new(true);

    pub fn set_blit_hle(enabled: bool) {
        BLIT_HLE_ENABLED.store(enabled, Ordering::Relaxed);
    }

    // ── the PORTRAIT capture (141차, 2026-09-30) ──────────────────────────────
    //
    // The status HUD's ring shows the player's CURRENT look — helmet, hair, palette —
    // and the one place that look exists, already composed exactly as the game draws
    // it, is the field frame. So the host blitter (`blit565`) copies every pixel it
    // draws inside a small WINDOW around the player's draw corner into this buffer, and
    // the worker takes the head off the top of it. No sprite tables, no layer offsets,
    // no palette code: the game's own paper doll, as blitted.
    //
    // ★ Same thread as the worker: the hooks run inside `emulator.tick()`, so the
    // worker clears and re-arms the window between ticks without a race; the atomics
    // are there so a `static` can hold them.
    // ★ The buffer ACCUMULATES until the window moves (the worker clears it when it
    // re-arms a different origin), so a frame that a tick cuts in half completes on the
    // next one. The worker only publishes while the player stands still, facing the
    // viewer, which is also when accumulation cannot smear.
    // ★★ WHICH blits are the player's is decided by ORDER, not by geometry: the field
    // painter draws every actor as its shadow blit followed by its pieces (head last),
    // so the worker arms the window together with the ANCHOR where the player's own
    // shadow must land, and `portrait_takes` opens a run of `CAPTURE_RUN` blits when a
    // shadow-sized blit lands there. Tiles (drawn before any actor), NPCs (their shadow
    // is elsewhere), balloons and HUD (drawn after the run has counted out) never enter.
    // Only blits wholly inside the window are copied even during the run.
    // ★★ 153차: the run ENDS at the next actor's shadow (a blended shadow-sized blit
    // anywhere) — an NPC or monster drawn right after the player (one standing just in
    // front) used to have its first pieces copied into the window. And a blended blit
    // copied during the run marks the picture BLENDED (`portrait_blended`): the player's
    // own pieces go through the opaque blitter (`blit565`, `--blit-log`), so a blended
    // one in the window is the player drawn see-through (stealth, a flash) or something
    // else's effect — a picture the worker must not keep.

    /// The window, in FRAMEBUFFER pixels: `PORTRAIT_W` wide and `PORTRAIT_H` tall.
    /// ★ 153차: 64 → 72 tall, the extra rows ABOVE the head (`portrait.rs` `PORTRAIT_UP`
    /// 36 → 44) — the helmet is drawn from the draw corner −39 and was never copied.
    pub const PORTRAIT_W: u32 = 48;
    pub const PORTRAIT_H: u32 = 72;
    /// The player's shadow blit: `SHADOW_W × SHADOW_H`, within `SHADOW_SLOP` px of the
    /// anchor; a run is `CAPTURE_RUN` blits long (`portrait.rs` has the measurements).
    pub const SHADOW_W: i32 = 26;
    pub const SHADOW_H: i32 = 17;
    pub const SHADOW_SLOP: i32 = 2;
    pub const CAPTURE_RUN: u32 = 8;
    /// The window's origin, packed `(x as u32) << 32 | (y as u32)` with the halves
    /// as `i32` bit patterns; `u64::MAX` = disarmed.
    pub static PORTRAIT_ORIGIN: AtomicU64 = AtomicU64::new(u64::MAX);
    /// Where the player's shadow blit must land, packed the same way.
    pub static PORTRAIT_ANCHOR: AtomicU64 = AtomicU64::new(u64::MAX);
    /// Blits left in the current capture run (0 = not in a run).
    pub static PORTRAIT_RUN: AtomicU32 = AtomicU32::new(0);
    /// The captured pixels, `0xAARRGGBB`, alpha 0 = nothing drawn there yet.
    pub static PORTRAIT_PX: [AtomicU32; (PORTRAIT_W * PORTRAIT_H) as usize] =
        [const { AtomicU32::new(0) }; (PORTRAIT_W * PORTRAIT_H) as usize];
    /// Blits taken into the window so far (diagnostic).
    pub static PORTRAIT_BLITS: AtomicU64 = AtomicU64::new(0);
    /// Runs opened so far — one per time the player was drawn into the window. The
    /// worker compares it around a tick: unchanged = the player was not drawn this tick
    /// (a blink, a frame the game skipped), and the window still holds an older frame.
    pub static PORTRAIT_SEQ: AtomicU32 = AtomicU32::new(0);
    /// A blended blit was copied into the window during the current run.
    pub static PORTRAIT_BLENDED: AtomicBool = AtomicBool::new(false);

    fn pack_xy(p: Option<(i32, i32)>) -> u64 {
        match p {
            None => u64::MAX,
            Some((x, y)) => (u64::from(x as u32) << 32) | u64::from(y as u32),
        }
    }

    fn unpack_xy(v: u64) -> Option<(i32, i32)> {
        match v {
            u64::MAX => None,
            v => Some(((v >> 32) as u32 as i32, v as u32 as i32)),
        }
    }

    /// Arm the window at `origin` with the player's shadow expected at `anchor`
    /// (framebuffer px, may be negative near the map edge), or disarm it with `None`.
    /// Moving the window CLEARS the buffer; re-arming the same origin keeps what has
    /// accumulated (a frame a tick cut in half completes on the next one).
    pub fn set_portrait_window(window: Option<((i32, i32), (i32, i32))>) {
        let (origin, anchor) = match window {
            None => (None, None),
            Some((o, a)) => (Some(o), Some(a)),
        };
        PORTRAIT_ANCHOR.store(pack_xy(anchor), Ordering::Relaxed);
        let packed = pack_xy(origin);
        if PORTRAIT_ORIGIN.swap(packed, Ordering::Relaxed) != packed {
            PORTRAIT_RUN.store(0, Ordering::Relaxed);
            for p in PORTRAIT_PX.iter() {
                p.store(0, Ordering::Relaxed);
            }
        }
    }

    /// Arm the window alone (no shadow trigger — every blit inside is taken). Kept for
    /// the probe's raw window dumps; the phone uses [`set_portrait_window`].
    pub fn set_portrait_origin(origin: Option<(i32, i32)>) {
        set_portrait_window(origin.map(|o| (o, (i32::MIN / 2, i32::MIN / 2))));
        if origin.is_some() {
            PORTRAIT_RUN.store(u32::MAX, Ordering::Relaxed);
        }
    }

    /// The armed window's origin, if any.
    pub fn portrait_origin() -> Option<(i32, i32)> {
        unpack_xy(PORTRAIT_ORIGIN.load(Ordering::Relaxed))
    }

    /// Called by the blitter for EVERY blit: is this one to be copied into the window?
    /// Returns the window origin when it is — the blitter then offsets by it. `blended`
    /// = the blit goes through the blended (LUT) blitter.
    ///
    /// The player's shadow (shadow-sized, on the anchor) is not copied itself: it clears
    /// the window and opens a run of `CAPTURE_RUN` blits. During a run every blit counts
    /// down and the ones wholly inside the window are copied — until the NEXT actor's
    /// shadow (blended and shadow-sized, anywhere), which ends the run. Outside a run
    /// nothing is.
    pub fn portrait_takes(x: i32, y: i32, w: i32, h: i32, blended: bool) -> Option<(i32, i32)> {
        let (ox, oy) = portrait_origin()?;
        let shadow_sized = w == SHADOW_W && h == SHADOW_H;
        if let Some((ax, ay)) = unpack_xy(PORTRAIT_ANCHOR.load(Ordering::Relaxed)) {
            if shadow_sized && (x - ax).abs() <= SHADOW_SLOP && (y - ay).abs() <= SHADOW_SLOP {
                for p in PORTRAIT_PX.iter() {
                    p.store(0, Ordering::Relaxed);
                }
                PORTRAIT_BLENDED.store(false, Ordering::Relaxed);
                PORTRAIT_SEQ.fetch_add(1, Ordering::Relaxed);
                PORTRAIT_RUN.store(CAPTURE_RUN, Ordering::Relaxed);
                return None;
            }
        }
        let run = PORTRAIT_RUN.load(Ordering::Relaxed);
        if run == 0 {
            return None;
        }
        if run != u32::MAX {
            // ★ 153차: another actor's shadow — the player's pieces are over.
            if blended && shadow_sized {
                PORTRAIT_RUN.store(0, Ordering::Relaxed);
                return None;
            }
            PORTRAIT_RUN.store(run - 1, Ordering::Relaxed);
        }
        let inside = x >= ox && y >= oy && x + w <= ox + PORTRAIT_W as i32 && y + h <= oy + PORTRAIT_H as i32;
        if inside && blended {
            PORTRAIT_BLENDED.store(true, Ordering::Relaxed);
        }
        inside.then_some((ox, oy))
    }

    /// The capture's state for the worker, read after a tick: `(runs opened so far,
    /// blits left in the open run (0 = the last run counted out or ended), whether a
    /// blended blit was copied during the last run)`.
    pub fn portrait_run_state() -> (u32, u32, bool) {
        (
            PORTRAIT_SEQ.load(Ordering::Relaxed),
            PORTRAIT_RUN.load(Ordering::Relaxed),
            PORTRAIT_BLENDED.load(Ordering::Relaxed),
        )
    }

    /// One drawn pixel at framebuffer `(x, y)`, colour `rgb565`.
    pub fn portrait_put(x: i32, y: i32, ox: i32, oy: i32, rgb565: u16) {
        let (lx, ly) = (x - ox, y - oy);
        if lx < 0 || ly < 0 || lx >= PORTRAIT_W as i32 || ly >= PORTRAIT_H as i32 {
            return;
        }
        let r = u32::from((rgb565 >> 11) & 31) * 255 / 31;
        let g = u32::from((rgb565 >> 5) & 63) * 255 / 63;
        let b = u32::from(rgb565 & 31) * 255 / 31;
        PORTRAIT_PX[(ly as u32 * PORTRAIT_W + lx as u32) as usize].store(0xff00_0000 | (r << 16) | (g << 8) | b, Ordering::Relaxed);
    }

    /// The buffer as it stands, row-major `PORTRAIT_W × PORTRAIT_H`.
    pub fn portrait_snapshot() -> Vec<u32> {
        PORTRAIT_PX.iter().map(|p| p.load(Ordering::Relaxed)).collect()
    }

    // ── the BLIT LOG (141차 diagnostic) ──────────────────────────────────────────
    //
    // `wie_probe --blit-log <frame>` arms this for one tick and prints every sprite
    // blit the host blitter saw: where it landed, how big, which descriptor. It is how
    // the portrait window was sized — by reading which blits make up the player.

    pub const BLIT_LOG_LEN: usize = 1024;
    pub static BLIT_LOG_ARMED: AtomicBool = AtomicBool::new(false);
    pub static BLIT_LOG_N: AtomicU32 = AtomicU32::new(0);
    /// Entry `i`: `[0]` = `dst_x << 48 | dst_y << 32 | w << 16 | h` (u16 halves),
    /// `[1]` = `spr << 32 | src_x << 16 | src_y` (u16 halves, `spr` the descriptor),
    /// `[2]` = the guest return address the blitter was entered with (153차: which
    /// painter drew it — the blitters are reached through one function-pointer table,
    /// so this is the caller's own call site).
    pub static BLIT_LOG: [[AtomicU64; 3]; BLIT_LOG_LEN] = [const { [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)] }; BLIT_LOG_LEN];

    pub fn blit_log_arm(on: bool) {
        BLIT_LOG_N.store(0, Ordering::Relaxed);
        BLIT_LOG_ARMED.store(on, Ordering::Relaxed);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn blit_log_push(dst_x: i32, dst_y: i32, w: i32, h: i32, spr: u32, src_x: i32, src_y: i32, lr: u32) {
        if !BLIT_LOG_ARMED.load(Ordering::Relaxed) {
            return;
        }
        let i = BLIT_LOG_N.fetch_add(1, Ordering::Relaxed) as usize;
        if i >= BLIT_LOG_LEN {
            return;
        }
        let pack = |a: i32, b: i32, c: i32, d: i32| -> u64 {
            (u64::from(a as u16) << 48) | (u64::from(b as u16) << 32) | (u64::from(c as u16) << 16) | u64::from(d as u16)
        };
        BLIT_LOG[i][0].store(pack(dst_x, dst_y, w, h), Ordering::Relaxed);
        BLIT_LOG[i][1].store((u64::from(spr) << 32) | (u64::from(src_x as u16) << 16) | u64::from(src_y as u16), Ordering::Relaxed);
        BLIT_LOG[i][2].store(u64::from(lr), Ordering::Relaxed);
    }

    /// The entries recorded since the last arm: `(dst_x, dst_y, w, h, spr, src_x, src_y, lr)`.
    pub fn blit_log_take() -> Vec<(i32, i32, i32, i32, u32, i32, i32, u32)> {
        let n = (BLIT_LOG_N.load(Ordering::Relaxed) as usize).min(BLIT_LOG_LEN);
        (0..n)
            .map(|i| {
                let a = BLIT_LOG[i][0].load(Ordering::Relaxed);
                let b = BLIT_LOG[i][1].load(Ordering::Relaxed);
                let lr = BLIT_LOG[i][2].load(Ordering::Relaxed) as u32;
                let s16 = |v: u64| (v as u16) as i16 as i32;
                (s16(a >> 48), s16(a >> 32), s16(a >> 16), s16(a), (b >> 32) as u32, s16(b >> 16), s16(b), lr)
            })
            .collect()
    }

    // ── the reaver-drop TEST knobs (78차-B) ──────────────────────────────────
    //
    // ★★ WHY THEY LIVE HERE. `wie_core_arm` is `#![no_std]` and `wie_android` does not
    // depend on it at all (only transitively, through `wie_lgt`), so a `pub static` in
    // `binary_patches::reaverdrop` would cost a Cargo edge plus two visibility changes.
    // `BLIT_HLE_ENABLED` above already answers exactly this question — its comment says
    // it sits beside the counters "so the host binaries can flip it without an export
    // chain through `wie_core_arm`" — and both dependency edges already exist.
    //
    // ★★ WHY A RUNTIME KNOB AT ALL. The drop is gated on eight HARD-mode boss species at
    // Lv 65-67. A tester whose only save is mid normal mode cannot reach one, and a
    // non-boss kill produces NO log line at all, so «the hook works» and «the hook is
    // dead again» look identical from the outside — which is precisely how the wrong-
    // register bug survived vc211..vc219 unnoticed. These two words let a tester prove
    // the whole path (hook fires -> enemy resolves -> species reads -> item appends ->
    // loot spawns) on any monster they can actually kill.
    //
    // ⛔ NEITHER MAY EVER BE SET BY THE SHIPPED BUILD. Both default to `u32::MAX` =
    // unset, which leaves `RATE_PCT` and `BOSS_SPECIES` byte-identical, so there is
    // nothing to revert. The only writer is `wie_android`'s `wie_reaver_test.txt`
    // reader, which lives on one phone's external files dir and is deleted as it is
    // read. And the `vanilla` flavour cannot be re-armed at runtime by anything: its
    // hook is never INSTALLED (`hook.rs`, `cfg!(hero3_vanilla)`).

    /// Percent chance per kill, overriding `reaverdrop::RATE_PCT`. `u32::MAX` = unset.
    pub static REAVER_RATE_PCT: AtomicU32 = AtomicU32::new(u32::MAX);
    /// Treat EVERY killed enemy as this species. `u32::MAX` = unset.
    ///
    /// A species, not a set, and deliberately so: pretending the kill is 141 makes the
    /// reaver roll AND the 타이탄의 파편 roll fire exactly as a real 크레이지타이탄 kill
    /// would, so one knob exercises both dice. Widening the boss LIST instead would test
    /// the reaver only — `reaverdrop::accessory_for` is an independent species -> item map.
    pub static REAVER_AS_SPECIES: AtomicU32 = AtomicU32::new(u32::MAX);

    /// Pretend the guest's difficulty byte reads HARD, whatever it actually says.
    /// `0` = off (read the guest). Lets a tester on a normal-mode save exercise the drop.
    pub static REAVER_FORCE_HARD: AtomicBool = AtomicBool::new(false);

    /// ★★★ NOT A TEST KNOB — the 하늘성 floor rule (user decision 2026-09-11:
    /// *"몹 리젠과 아이템도 나오면 안되고"*). While a tower run is live the game's own
    /// loot routine must not hand anything out; the tower's rewards are granted per
    /// floor instead (`GUARDIAN_CASTLE_FEASIBILITY.md §3.4`).
    ///
    /// The reaver hook already owns the decision "spawn loot or not" (it sits on the
    /// `cmp r7,#0` that guards the spawn), so this is one branch there rather than a
    /// second hook. ⛔ It suppresses the STOCK drops and the reaver alike: inside the
    /// tower nothing drops at all.
    /// `vanilla` never installs that hook (`hook.rs`, `cfg!(hero3_vanilla)`) — and since
    /// 114차 (2026-09-19) it has no tower either: `wie_android::flavor::TOWER` is false
    /// there, the floor table is never read and the run stays `Idle`, so this knob is
    /// never raised on that flavour and the question "does vanilla's tower drop" is moot.
    pub static TOWER_NO_DROPS: AtomicBool = AtomicBool::new(false);

    /// Turn the tower's loot suppression on or off. Called every poll by the worker, so
    /// it is a plain store rather than a latch.
    pub fn set_tower_no_drops(on: bool) {
        TOWER_NO_DROPS.store(on, Ordering::Relaxed);
    }

    /// Is a tower run live, i.e. must the loot routine hand out nothing?
    pub fn tower_no_drops() -> bool {
        TOWER_NO_DROPS.load(Ordering::Relaxed)
    }

    /// ★★★ NOT A TEST KNOB — the 하늘성 SPECIES OVERRIDE (88차, 2026-09-12). Packed
    /// `scene << 24 | s0 << 16 | s1 << 8 | s2`; `u32::MAX` = unset.
    ///
    /// A floor's monsters used to be a property of its SCENE (the species triple lives
    /// in the scene header), so with three halls rotating, floors 1/4/7 were the same
    /// line-up — the player's "1,2,3 층 몬스터가 4,5,6 에도 똑같이". The floor table now
    /// carries a triple per floor, the worker arms this word before it posts the warp,
    /// and the `tower_species_lgt` hook (on the tail of the game's own `parseScnHeader`)
    /// writes the triple over the header's as THAT scene loads — the spawn loop that
    /// runs a few hundred instructions later reads the three words and never knows.
    ///
    /// ⛔ Keyed on the SCENE, not "while a run is live": the hook fires for every scene
    /// the game loads, and a run's `Leave` warp loads the origin town. The worker clears
    /// it once the floor has landed and on every idle poll, so a stale value cannot
    /// outlive the run it was armed for.
    pub static TOWER_SPECIES: AtomicU32 = AtomicU32::new(u32::MAX);

    /// Arm the species override for one scene. `None` disarms.
    pub fn set_tower_species(want: Option<(u8, [u8; 3])>) {
        let v = match want {
            None => u32::MAX,
            Some((scene, s)) => (scene as u32) << 24 | (s[0] as u32) << 16 | (s[1] as u32) << 8 | s[2] as u32,
        };
        TOWER_SPECIES.store(v, Ordering::Relaxed);
    }

    /// `(scene, [s0, s1, s2])` if an override is armed.
    pub fn tower_species() -> Option<(u8, [u8; 3])> {
        match TOWER_SPECIES.load(Ordering::Relaxed) {
            u32::MAX => None,
            v => Some(((v >> 24) as u8, [(v >> 16) as u8, (v >> 8) as u8, v as u8])),
        }
    }

    /// ★★★ NOT A TEST KNOB — the 하늘성 SPAWN CAP (145차, 2026-09-30). Packed
    /// `scene << 8 | cap`; `u32::MAX` = unset.
    ///
    /// A hall has twenty spawn points and a floor's `count` asks for fewer. The host used
    /// to trim the difference AFTER the load by writing 0 HP into the extras, which the
    /// game then ran its own death for — monsters falling over on arrival, untouched
    /// (user: *"입장하자마자 내가 때리지도 않았는데 죽는 버그"*). Now the worker arms this word
    /// next to [`TOWER_SPECIES`] before it posts the warp, and the `tower_spawn_cap_lgt`
    /// hook (on the spawn loop's empty-slot compare, `0x158b0`) sends every point past
    /// `cap` down the game's own "no monster here" path as THAT scene loads — the extras
    /// are never created.
    ///
    /// ⛔ Keyed on the SCENE for the same reason as the species word: the hook fires for
    /// every scene the game loads. The worker takes it down with the species word.
    pub static TOWER_SPAWN_CAP: AtomicU32 = AtomicU32::new(u32::MAX);

    /// Arm the spawn cap for one scene. `None` disarms.
    pub fn set_tower_spawn_cap(want: Option<(u8, u8)>) {
        let v = match want {
            None => u32::MAX,
            Some((scene, cap)) => (scene as u32) << 8 | cap as u32,
        };
        TOWER_SPAWN_CAP.store(v, Ordering::Relaxed);
    }

    /// `(scene, cap)` if a cap is armed.
    pub fn tower_spawn_cap() -> Option<(u8, u8)> {
        match TOWER_SPAWN_CAP.load(Ordering::Relaxed) {
            u32::MAX => None,
            v => Some(((v >> 8) as u8, v as u8)),
        }
    }

    /// ★★★ THE 2회차 TRANSITION RAN (135차-B, 2026-09-24) — a counter the
    /// `ending_transition_lgt` hook bumps on entry to `0x3af2c`, the game's new-game-plus
    /// transition (the one function ending window 18's KEY handler calls, for any key).
    ///
    /// Two host features hang off that moment: the boxes give up what the transition takes
    /// out of the bag (124차) and a hardcore clear opens 헬 (133차). They used to watch window
    /// 18 rise and fall on a poll, and the window lives only until the next key — a mashed
    /// ending skipped both, with no log line (135차 §2 ②). A counter does not expire: the
    /// worker compares it against the last value it handled, on whatever poll it likes.
    pub static ENDING_TRANSITIONS: AtomicU32 = AtomicU32::new(0);

    /// The difficulty byte (`0x150ddc8`) as the transition found it — BEFORE it writes 1 —
    /// or `u32::MAX` when the hook could not read it. Stored before the counter moves.
    pub static ENDING_DIFFICULTY_BEFORE: AtomicU32 = AtomicU32::new(u32::MAX);

    /// Hook side: a transition is running; `difficulty` is the byte it found.
    pub fn note_ending_transition(difficulty: Option<u8>) {
        ENDING_DIFFICULTY_BEFORE.store(difficulty.map_or(u32::MAX, u32::from), Ordering::Relaxed);
        ENDING_TRANSITIONS.fetch_add(1, Ordering::Release);
    }

    /// Host side: `(how many transitions have run, the difficulty the LAST one found)`.
    pub fn ending_transition() -> (u32, Option<u8>) {
        let n = ENDING_TRANSITIONS.load(Ordering::Acquire);
        let d = match ENDING_DIFFICULTY_BEFORE.load(Ordering::Relaxed) {
            v if v <= 0xff => Some(v as u8),
            _ => None,
        };
        (n, d)
    }

    /// ★★★ 아바타 옷장 (167차) — WHICH costume sheets the jar carries, one bit per set
    /// (`set - 1`) per sheet group ([`crate::avatar::group_of`]). The host fills it once at boot
    /// from the jar's own directory ([`crate::avatar::parse_sheet_member`]); the two paper-doll
    /// hooks (`wie_core_arm/binary_patches/avatar.rs`) consult it before they hand the game's
    /// file-name builder a costume sprite.
    ///
    /// ⛔ All zero = the hooks never change a layer, which is the state of a build whose asset
    /// has no costumes — and of any process that has not published yet. A missing sheet is
    /// the one load the hooks must never cause.
    pub static AVATAR_SHEETS: [AtomicU64; crate::avatar::GROUPS] = [const { AtomicU64::new(0) }; crate::avatar::GROUPS];

    /// Publish the availability masks (bit `set - 1` of `masks[group]`).
    pub fn set_avatar_sheets(masks: [u64; crate::avatar::GROUPS]) {
        for (slot, m) in AVATAR_SHEETS.iter().zip(masks) {
            slot.store(m, Ordering::Relaxed);
        }
    }

    /// Does the jar carry set `set`'s sheet for `group`?
    pub fn avatar_sheet(group: usize, set: u8) -> bool {
        if !(1..=crate::avatar::SET_MAX).contains(&set) {
            return false;
        }
        AVATAR_SHEETS.get(group).is_some_and(|m| m.load(Ordering::Relaxed) & (1u64 << (set - 1)) != 0)
    }

    /// ★ 171차 — which sets' 투구 costume carries the WHOLE head (costume.txt `head = own`: the
    /// hero's face is drawn into the costume, smaller than the hero's own head), one bit per set
    /// (`set - 1`). The host publishes it once at boot from its catalog; the painter hook
    /// (`binary_patches/avatar.rs` `handle_head_paint_lgt`) skips the hero's own head piece only
    /// for these sets. All zero = the hero's head is always drawn (the shipped game).
    pub static AVATAR_OWN_HEAD: AtomicU64 = AtomicU64::new(0);

    /// Publish [`AVATAR_OWN_HEAD`].
    pub fn set_avatar_own_head(mask: u64) {
        AVATAR_OWN_HEAD.store(mask, Ordering::Relaxed);
    }

    /// Does set `set`'s 투구 costume carry the whole head?
    pub fn avatar_own_head(set: u8) -> bool {
        (1..=crate::avatar::SET_MAX).contains(&set) && AVATAR_OWN_HEAD.load(Ordering::Relaxed) & (1u64 << (set - 1)) != 0
    }

    /// ★ 167차 — the AVATAR page's TRY-ON: per avatar slot, the set the cursor rests on
    /// (`0` = the equipment's own look) while the page is open, `u32::MAX` = no preview. The
    /// hooks read this BEFORE the worn byte in the save's flag array, so moving the cursor
    /// changes the figure without touching anything the save carries; the host clears it when
    /// the page closes and re-dresses the layer.
    pub static AVATAR_PREVIEW: [AtomicU32; crate::avatar::SLOTS] = [const { AtomicU32::new(u32::MAX) }; crate::avatar::SLOTS];

    /// Arm (`Some(set)`, `Some(0)` = no costume) or drop (`None`) one slot's try-on.
    pub fn set_avatar_preview(slot: usize, set: Option<u8>) {
        if let Some(p) = AVATAR_PREVIEW.get(slot) {
            p.store(set.map_or(u32::MAX, u32::from), Ordering::Relaxed);
        }
    }

    /// The try-on for one slot, if armed.
    pub fn avatar_preview(slot: usize) -> Option<u8> {
        match AVATAR_PREVIEW.get(slot)?.load(Ordering::Relaxed) {
            v if v <= 0xff => Some(v as u8),
            _ => None,
        }
    }

    /// Drop every try-on.
    pub fn clear_avatar_preview() {
        for p in &AVATAR_PREVIEW {
            p.store(u32::MAX, Ordering::Relaxed);
        }
    }

    /// How many times a hook has dressed a layer (diagnostics — the host logs it).
    pub static AVATAR_DRESSED: AtomicU32 = AtomicU32::new(0);

    /// Arm (or disarm) the test knobs. `None` / `false` restores the shipped behaviour.
    pub fn set_reaver_test(rate_pct: Option<u32>, as_species: Option<u8>, force_hard: bool) {
        REAVER_RATE_PCT.store(rate_pct.unwrap_or(u32::MAX), Ordering::Relaxed);
        REAVER_AS_SPECIES.store(as_species.map_or(u32::MAX, u32::from), Ordering::Relaxed);
        REAVER_FORCE_HARD.store(force_hard, Ordering::Relaxed);
    }

    /// Whether the difficulty gate should be forced open. Off unless a tester armed it.
    pub fn reaver_force_hard() -> bool {
        REAVER_FORCE_HARD.load(Ordering::Relaxed)
    }

    /// The rate the hook should actually use, given the shipped default.
    pub fn reaver_rate_pct(default: u32) -> u32 {
        match REAVER_RATE_PCT.load(Ordering::Relaxed) {
            u32::MAX => default,
            n => n,
        }
    }

    /// The species the hook should pretend it killed, if the knob is armed.
    pub fn reaver_as_species() -> Option<u8> {
        match REAVER_AS_SPECIES.load(Ordering::Relaxed) {
            n if n <= 0xff => Some(n as u8),
            _ => None,
        }
    }

    // ── BAG BOXES — 오브가방·재료가방·소비가방 (2026-09-14) ────────────────────
    //
    // The host keeps three typed containers OUTSIDE the guest's bag (see
    // `BAG_TYPES_FEASIBILITY.md`). Every acquisition route in the game passes through
    // `giveItem 0x52b14` or `addItemToInventory 0x131d0`, and the `bagbox_*_lgt` hooks on
    // those two entries hand a container-category item to the host INSTEAD of the bag
    // — which is what makes "it goes into the orb box even when the bag is full" true.
    //
    // The hook runs inside a guest call and owns no host state, so the contract is
    // three knobs the host publishes every frame and a ring the hook pushes into:
    //   * `BAGBOX_ACCEPT` — bit per item category the host is taking right now. 0 = the
    //     feature is off (no character, no save pin, a build with no container) and both
    //     hooks fall through to the game byte-for-byte.
    //   * `BAGBOX_KEEP_IDX` — cat-0x12 indices the hook must LEAVE in the bag even when
    //     0x12 is accepted: the three bag-item cells themselves.
    //   * `BAGBOX_ROOM[box]` — free stacks per box. The hook takes one only when an offer
    //     cannot MERGE into the stacks the box already holds of that item (`BAGBOX_MERGE`,
    //     per `(cat, idx)`) and refuses at zero, so offers inside one guest call cannot
    //     overshoot; the host republishes the exact numbers when it drains.
    //   * the ring — `cat<<24 | idx<<16 | count<<8 | source`, 64 deep. The source byte is
    //     a route (`BAGBOX_SRC_MASK`) plus flags, of which `BAGBOX_ANNOUNCE` says the game
    //     would have shown its floating "you got it" name for this one. An offer of the
    //     item the NEWEST waiting entry already carries (same source byte) is folded into
    //     it; a full ring refuses anything else, and a refused offer means the item takes
    //     the game's own path into the bag (the host sweeps it into the box later). Nothing
    //     is lost either way; the box is only ever a place an item goes INSTEAD of the bag.
    //   * the MIRROR below — an accepted offer adds its units there AT ONCE.
    //
    // ★★★ 165차 (2026-10-06) — ONE GUEST CALL CAN GIVE MANY UNITS, AND ASK RIGHT AFTER.
    // The hook runs inside a guest call and the host drains only BETWEEN calls, so whatever
    // a single call does has to be settled by the knobs alone:
    //   * the NPC shop gives a 99-unit purchase as 99 `giveItem(cat, idx, -1, 0)` calls in
    //     ONE key handler (`0x1e732..0x1e768`). One ring entry and one free stack per UNIT
    //     let a purchase in min(64, free stacks) and spilled the rest into the bag (제보:
    //     *"포션을 99개씩 연달아서 계속 사다보면 3~4묶음 이후 소비가방으로 안들어가지는"*,
    //     probe: 99 given → 48 taken, 51 in bag cell 0,6). Now 99 units of one item are one
    //     entry and at most one new stack;
    //   * a scene script that GIVES a box item and then TAKES it in the same run asked the
    //     box through the mirror before the host had filed the give. 리파이너의 유적 7's boss
    //     door (scene 188 `B[12]`, 케이 + 일레느) is op 0x14 `getitem_dat[0x59]` = 그리폰의피리,
    //     then op 0x12 takes one back — and the first try answered 「이벤트 실행에 필요한 아이템이
    //     부족합니다」 (InGame_txt 137) while the flute sat in the ring; the second try found the
    //     FIRST try's flute and went in. The mirror now carries an accepted unit at once.
    // ⛔ The atomics are for visibility, not for a race: the hook and the drain run on the one
    // emulator thread (a guest call is inside a tick, the drain is between ticks).

    /// Bit `cat` set = the host takes items of that category.
    pub static BAGBOX_ACCEPT: AtomicU32 = AtomicU32::new(0);
    /// Bit `idx` set = a cat-0x12 index that stays in the bag (the bag-item cells).
    pub static BAGBOX_KEEP_IDX: AtomicU32 = AtomicU32::new(0);
    /// Free stacks per box (0 orb · 1 material · 2 consumable).
    pub static BAGBOX_ROOM: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];
    pub const BAGBOX_RING_LEN: usize = 64;
    pub static BAGBOX_RING: [AtomicU32; BAGBOX_RING_LEN] = [const { AtomicU32::new(0) }; BAGBOX_RING_LEN];
    /// Producer index (the hook) and consumer index (the host); both only grow.
    pub static BAGBOX_HEAD: AtomicU32 = AtomicU32::new(0);
    pub static BAGBOX_TAIL: AtomicU32 = AtomicU32::new(0);
    /// Offers the ring could not take (diagnostic).
    pub static BAGBOX_REFUSED: AtomicU32 = AtomicU32::new(0);
    /// MATERIALISE bypass: `cat<<16 | idx<<8 | n`. While `n > 0`, offers of exactly
    /// `(cat, idx)` are LET THROUGH to the game's own placement, one unit per offer —
    /// this is how the host puts one unit of a boxed item back into a bag cell (the
    /// game's `giveItem` does the writing) so the game's own 사용하기 / hotkey can
    /// consume it. 0 = no bypass.
    pub static BAGBOX_BYPASS: AtomicU32 = AtomicU32::new(0);

    /// The offer came through `giveItem` (drops, shops, scripts, crafting).
    pub const BAGBOX_SRC_GIVE: u8 = 1;
    /// The offer came through `addItemToInventory` (warehouse `05 04`, cash `05 40`).
    pub const BAGBOX_SRC_WIRE: u8 = 2;
    /// Not an intake: the quest VM TOOK `count` units OUT of the box (`bagbox_take`) —
    /// the host must spend them from its box and persist.
    pub const BAGBOX_SRC_TAKE: u8 = 3;
    /// The low bits of the ring's source byte: which route the entry came from.
    pub const BAGBOX_SRC_MASK: u8 = 0x0f;
    /// ★★★ 103차 — set when THE GAME WOULD HAVE ANNOUNCED this acquisition.
    ///
    /// `giveItem`'s third argument is an announce flag (`0xff` = silent, anything else =
    /// queue the floating item name); `addItemToInventory` reads the same flag out of the
    /// wire record's signed count. Both blocks live AFTER the placement the hook replaces,
    /// so an absorbed item lost the notice the bag would have got — the bug the player
    /// reported as "3종 가방에 들어가는 재료는 주워도 획득 안내가 안 나온다".
    /// The hook decides (it is the only thing that can see the argument) and the host
    /// queues the name; see `bagbox::notice_plan`.
    pub const BAGBOX_ANNOUNCE: u8 = 0x10;

    // ── the box MIRROR: what the quest VM's item helper asks (3단계, 2026-09-15) ──
    //
    // The script VM's one item helper, `0x3bcf0(cat, idx, u16* out, mode)`, walks the
    // BAG to answer `fe <cat><idx><n>` ("has at least n") and op `0x12 <cat><idx><n>`
    // ("take n") — 19 quests hand in cat-0x0e materials that way, two take cat-0x12
    // consumables. Those items live in the host's boxes now, so the two hooks on that
    // helper (`bagbox_count_lgt` on its exit, `bagbox_take_lgt` on its entry) need the
    // box contents INSIDE a guest call: this mirror, published by the host whenever a
    // box changes. Per `(cat, idx)` unit totals for the five box categories.
    //   * intake (★ 165차): `bagbox_offer` ADDS what it accepts here at once, so the units a
    //     script just gave are countable and takeable before the host has filed them — the
    //     ring keeps the give ahead of the take, and the host's republish after the drain
    //     lands on the same totals.
    //   * count: the exit hook ADDS the mirror to what the guest counted in the bag.
    //   * take:  the entry hook, when the mirror holds `n`, decrements it and pushes a
    //     `BAGBOX_SRC_TAKE` ring entry; the host spends the same from its box. The guest
    //     bag walk is skipped. Fewer than `n` in the box = the game's own path (the bag).
    pub const BAGBOX_MIRROR_CATS: [u32; 5] = [0x10, 0x0e, 0x0f, 0x12, 0x0d];
    pub static BAGBOX_COUNT: [AtomicU32; 5 * 256] = [const { AtomicU32::new(0) }; 5 * 256];
    /// ★ 165차: per `(cat, idx)`, the units the box's EXISTING stacks of that item can still
    /// take (Σ `BAGBOX_STACK_MAX - count` — `Boxes::merge_room` on the host), published with
    /// the counts. The intake spends this before it asks `BAGBOX_ROOM` for a stack, which is
    /// what lets a 99-unit purchase cost one stack instead of 99.
    pub static BAGBOX_MERGE: [AtomicU32; 5 * 256] = [const { AtomicU32::new(0) }; 5 * 256];
    /// The guest's stack cap (`findSlot 0x529a0` merges while `<= 98`) — the size of every
    /// stack a box opens. The host's `bagbox::STACK_MAX` is held equal to it at compile time.
    pub const BAGBOX_STACK_MAX: u32 = 99;

    /// The mirror slot for `(cat, idx)`, or `None` for a category the boxes never hold.
    pub fn bagbox_mirror_slot(cat: u32, idx: u32) -> Option<usize> {
        if idx > 0xff {
            return None;
        }
        let row = BAGBOX_MIRROR_CATS.iter().position(|c| *c == cat)?;
        Some(row * 256 + idx as usize)
    }

    pub fn set_bagbox_count(cat: u32, idx: u32, n: u32) {
        if let Some(s) = bagbox_mirror_slot(cat, idx) {
            BAGBOX_COUNT[s].store(n, Ordering::Relaxed);
        }
    }

    /// ★ 165차: the merge room of `(cat, idx)` — see [`BAGBOX_MERGE`].
    pub fn set_bagbox_merge(cat: u32, idx: u32, n: u32) {
        if let Some(s) = bagbox_mirror_slot(cat, idx) {
            BAGBOX_MERGE[s].store(n, Ordering::Relaxed);
        }
    }

    /// The whole mirror — counts AND merge room — back to zero (the host's republish).
    pub fn bagbox_clear_counts() {
        for c in BAGBOX_COUNT.iter().chain(BAGBOX_MERGE.iter()) {
            c.store(0, Ordering::Relaxed);
        }
    }

    /// What taking `n` units costs a box whose stacks of the item can still take `merge`:
    /// `(new stacks, merge room after)`. The guest's order, which `Boxes::add` keeps: fill
    /// the stacks it has, then open new ones of [`BAGBOX_STACK_MAX`]. Pure.
    ///
    /// `const` so the offline engine's copy (`offline::box_intake_cost` — that module takes no
    /// dependencies) can be held equal to it at compile time.
    pub const fn bagbox_intake_cost(merge: u32, n: u32) -> (u32, u32) {
        let from_merge = if n < merge { n } else { merge };
        let rest = n - from_merge;
        let stacks = rest.div_ceil(BAGBOX_STACK_MAX);
        (stacks, merge - from_merge + stacks * BAGBOX_STACK_MAX - rest)
    }

    /// Append one ring entry — or FOLD it into the newest one when that entry is still
    /// waiting, names the same item from the same source byte (route and announce flag) and
    /// the two counts still fit the byte. `false` = the ring is full (`BAGBOX_REFUSED`).
    ///
    /// ★ 165차: one guest call can make 99 offers of one item (the shop), and nothing drains
    /// inside a guest call — one entry per offer ran the 64-deep ring dry in the middle of a
    /// purchase. ⛔ Only the NEWEST entry folds: the host applies entries in order, and
    /// folding into an older one would move units across a TAKE pushed after it. The host
    /// zeroes a slot as it takes it ([`bagbox_pop`]) and the fold is a compare-exchange on
    /// the word it read, so a fold never lands in an entry that is already gone.
    fn bagbox_ring_put(word: u32) -> bool {
        let head = BAGBOX_HEAD.load(Ordering::Relaxed);
        let tail = BAGBOX_TAIL.load(Ordering::Acquire);
        let n = (word >> 8) & 0xff;
        if head != tail {
            let last = &BAGBOX_RING[head.wrapping_sub(1) as usize % BAGBOX_RING_LEN];
            let w = last.load(Ordering::Relaxed);
            if w != 0
                && (w & !0xff00) == (word & !0xff00)
                && ((w >> 8) & 0xff) + n <= 0xff
                && last.compare_exchange(w, w + (n << 8), Ordering::AcqRel, Ordering::Relaxed).is_ok()
            {
                return true;
            }
        }
        if head.wrapping_sub(tail) as usize >= BAGBOX_RING_LEN {
            BAGBOX_REFUSED.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        BAGBOX_RING[head as usize % BAGBOX_RING_LEN].store(word, Ordering::Relaxed);
        BAGBOX_HEAD.store(head.wrapping_add(1), Ordering::Release);
        true
    }

    /// Units of `(cat, idx)` the host's box holds, as last published. 0 when the
    /// category is not accepted right now — a box that is off must read as empty.
    pub fn bagbox_count(cat: u32, idx: u32) -> u32 {
        if cat > 0x1f || BAGBOX_ACCEPT.load(Ordering::Relaxed) & (1 << cat) == 0 {
            return 0;
        }
        bagbox_mirror_slot(cat, idx).map_or(0, |s| BAGBOX_COUNT[s].load(Ordering::Relaxed))
    }

    /// Take `n` units of `(cat, idx)` out of the box for the quest VM. `true` = taken:
    /// the mirror is decremented and the host will spend the same; the caller must NOT
    /// let the game touch the bag. `false` = box off, fewer than `n` there, or the ring
    /// is full — the game's own bag path applies.
    pub fn bagbox_take(cat: u32, idx: u32, n: u8) -> bool {
        if n == 0 || cat > 0x1f || idx > 0xff {
            return false;
        }
        if BAGBOX_ACCEPT.load(Ordering::Relaxed) & (1 << cat) == 0 {
            return false;
        }
        let Some(slot) = bagbox_mirror_slot(cat, idx) else { return false };
        if BAGBOX_COUNT[slot]
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(u32::from(n)))
            .is_err()
        {
            return false;
        }
        let word = cat << 24 | idx << 16 | (n as u32) << 8 | BAGBOX_SRC_TAKE as u32;
        if !bagbox_ring_put(word) {
            // A full ring: the units stay the box's, and the game walks its bag.
            BAGBOX_COUNT[slot].fetch_add(u32::from(n), Ordering::Relaxed);
            return false;
        }
        true
    }

    /// Which box a category belongs to, or `None` for the categories that stay in the
    /// guest's bag (equipment `0x00..=0x0c`, quest `0x11`). The single source of the
    /// assignment the user confirmed (2026-09-14): 오브 `0x10` · 재료 `0x0e` ·
    /// 소비 `0x12`+`0x0d`. ★ 125차-B (2026-09-22): 조합서 `0x0f` left the 재료가방 —
    /// recipes stay in the plain bag, where the smith's 특수조합 screen reads them.
    pub fn bagbox_box_of(cat: u32) -> Option<usize> {
        match cat {
            0x10 => Some(0),
            0x0e => Some(1),
            0x12 | 0x0d => Some(2),
            _ => None,
        }
    }

    pub fn set_bagbox_accept(mask: u32, keep_idx: u32) {
        BAGBOX_ACCEPT.store(mask, Ordering::Relaxed);
        BAGBOX_KEEP_IDX.store(keep_idx, Ordering::Relaxed);
    }

    pub fn set_bagbox_room(bx: usize, free: u32) {
        if let Some(r) = BAGBOX_ROOM.get(bx) {
            r.store(free, Ordering::Relaxed);
        }
    }

    /// Try to take `count` of `(cat, idx)` into the host's box. `true` = taken: the caller
    /// must NOT let the game place it. `false` = the game keeps its own path.
    ///
    /// ★ 165차: a stack is spent only when the units do not fit the item's merge room
    /// ([`bagbox_intake_cost`]), the entry folds into the newest one of the same item
    /// ([`bagbox_ring_put`]), and the units are in the mirror the moment this returns — so a
    /// script that gives and then counts or takes inside one guest call sees them.
    pub fn bagbox_offer(cat: u32, idx: u32, count: u8, source: u8) -> bool {
        if cat > 0x1f || idx > 0xff || count == 0 {
            return false;
        }
        if BAGBOX_ACCEPT.load(Ordering::Relaxed) & (1 << cat) == 0 {
            return false;
        }
        if cat == 0x12 && idx < 32 && BAGBOX_KEEP_IDX.load(Ordering::Relaxed) & (1 << idx) != 0 {
            return false;
        }
        let Some(bx) = bagbox_box_of(cat) else { return false };
        let Some(slot) = bagbox_mirror_slot(cat, idx) else { return false };
        // The materialise bypass: this exact item, this many units, go to the bag.
        let bp = BAGBOX_BYPASS.load(Ordering::Relaxed);
        if bp & 0xff != 0 && (bp >> 16) == cat && ((bp >> 8) & 0xff) == idx {
            let _ = BAGBOX_BYPASS.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| if v & 0xff == 0 { None } else { Some(v - 1) });
            return false;
        }
        let n = u32::from(count);
        let (stacks, merge_after) = bagbox_intake_cost(BAGBOX_MERGE[slot].load(Ordering::Relaxed), n);
        // The stacks the units need, refused at too few. `fetch_update` so a concurrent read
        // can never go below zero.
        let room = &BAGBOX_ROOM[bx];
        if stacks > 0 && room.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |r| r.checked_sub(stacks)).is_err() {
            return false;
        }
        let word = cat << 24 | idx << 16 | n << 8 | source as u32;
        if !bagbox_ring_put(word) {
            // ⛔ Give the stacks back: a refused offer reserves nothing.
            if stacks > 0 {
                room.fetch_add(stacks, Ordering::Relaxed);
            }
            return false;
        }
        BAGBOX_MERGE[slot].store(merge_after, Ordering::Relaxed);
        BAGBOX_COUNT[slot].fetch_add(n, Ordering::Relaxed);
        true
    }

    /// ★ 166차: how many of `n` units of `(cat, idx)` the intake would take RIGHT NOW if they
    /// were given one `giveItem` call at a time — the shape every caller of the game's
    /// bag-room check (`0x3c0e4`: the NPC shop, script op 0x14) uses next. 0 whenever the
    /// hook would decline the first one: the category is off, the item is a bag-item cell,
    /// a materialise bypass is armed for it, or the ring could not take the entries.
    ///
    /// ⛔ It must never promise more than `bagbox_offer` then delivers: a promised unit the
    /// hook refuses takes the game's path into a bag that was just judged too full, and the
    /// shop has already taken the gold. So the ring is asked for one entry more than the
    /// units need (a quest-completion give can land between two of them), and the room is
    /// the same arithmetic the offers will run ([`bagbox_intake_cost`] — units fill the
    /// item's open stacks first, then whole stacks of [`BAGBOX_STACK_MAX`]).
    pub fn bagbox_room_for(cat: u32, idx: u32, n: u32) -> u32 {
        if n == 0 || cat > 0x1f || idx > 0xff {
            return 0;
        }
        if BAGBOX_ACCEPT.load(Ordering::Relaxed) & (1 << cat) == 0 || bagbox_item_declined(cat, idx) {
            return 0;
        }
        let Some(bx) = bagbox_box_of(cat) else { return 0 };
        let Some(slot) = bagbox_mirror_slot(cat, idx) else { return 0 };
        if !bagbox_ring_takes(bagbox_ring_used(), n) {
            return 0;
        }
        n.min(bagbox_box_units(BAGBOX_MERGE[slot].load(Ordering::Relaxed), BAGBOX_ROOM[bx].load(Ordering::Relaxed)))
    }

    /// The hook turns `(cat, idx)` away whatever the box's room: a bag-item cell (`0x12` with
    /// its [`BAGBOX_KEEP_IDX`] bit) or the materialise bypass armed for exactly this item. The
    /// read-only half of [`bagbox_offer`]'s first gates (the offer also spends the bypass).
    fn bagbox_item_declined(cat: u32, idx: u32) -> bool {
        if cat == 0x12 && idx < 32 && BAGBOX_KEEP_IDX.load(Ordering::Relaxed) & (1 << idx) != 0 {
            return true;
        }
        let bp = BAGBOX_BYPASS.load(Ordering::Relaxed);
        bp & 0xff != 0 && (bp >> 16) == cat && ((bp >> 8) & 0xff) == idx
    }

    /// Ring entries still waiting for the host.
    fn bagbox_ring_used() -> usize {
        BAGBOX_HEAD.load(Ordering::Relaxed).wrapping_sub(BAGBOX_TAIL.load(Ordering::Acquire)) as usize
    }

    /// ★ 166차: the intake's view of one category's box — `(free stacks, merge room of idx
    /// 0..indices)` — for a host that has to answer for SEVERAL deliveries before it sends the
    /// first: the offline engine's 소켓확장, each `05 04` of which is one whole-record offer on
    /// `addItemToInventory`. An item the hook turns away by itself reads `None`. The whole view
    /// is `None` when the category is off, or the ring lacks `entries + 1` free entries — the
    /// caller then counts on the bag alone, which is what shipped before. Pure read.
    pub fn bagbox_view(cat: u32, indices: u32, entries: usize) -> Option<(u32, Vec<Option<u32>>)> {
        if cat > 0x1f || BAGBOX_ACCEPT.load(Ordering::Relaxed) & (1 << cat) == 0 {
            return None;
        }
        let bx = bagbox_box_of(cat)?;
        if bagbox_ring_used().saturating_add(entries.saturating_add(1)) > BAGBOX_RING_LEN {
            return None;
        }
        let merge = (0..indices.min(0x100))
            .map(|idx| {
                if bagbox_item_declined(cat, idx) {
                    return None;
                }
                bagbox_mirror_slot(cat, idx).map(|s| BAGBOX_MERGE[s].load(Ordering::Relaxed))
            })
            .collect();
        Some((BAGBOX_ROOM[bx].load(Ordering::Relaxed), merge))
    }

    /// Units of one item a box can take: its open stacks' merge room plus a whole stack per
    /// free one. Pure.
    pub fn bagbox_box_units(merge: u32, free_stacks: u32) -> u32 {
        merge.saturating_add(free_stacks.saturating_mul(BAGBOX_STACK_MAX))
    }

    /// Can a ring with `used` entries waiting take `n` single-unit offers of one item? They
    /// fold into one entry per 255 units ([`bagbox_ring_put`]); one entry more is kept free
    /// for anything the caller gives in between. Pure.
    pub fn bagbox_ring_takes(used: usize, n: u32) -> bool {
        used.saturating_add(n.div_ceil(0xff) as usize + 1) <= BAGBOX_RING_LEN
    }

    /// Arm (`n > 0`) or clear (`n == 0`) the materialise bypass for `(cat, idx)`.
    pub fn set_bagbox_bypass(cat: u16, idx: u16, n: u8) {
        let v = if n == 0 { 0 } else { (cat as u32) << 16 | (idx as u32) << 8 | n as u32 };
        BAGBOX_BYPASS.store(v, Ordering::Relaxed);
    }

    /// Units the bypass still lets through.
    pub fn bagbox_bypass_left() -> u8 {
        BAGBOX_BYPASS.load(Ordering::Relaxed) as u8
    }

    // ── RESTAGE — a unit used THROUGH the box's own bag cell (127차, 2026-09-22) ──
    //
    // A boxed consumable is no longer materialised into a FREE bag cell (which a full
    // bag has none of). The host writes the unit's record OVER the box's own bag-item
    // cell (오브가방/재료가방/소비가방 — bag 1, cell 1–3), drives the game's own 사용하기 /
    // hotkey / revive on that cell, and arms THIS knob with the cell and the bag-item
    // record it replaced. `removeItem 0x5340c(tab, slot, n)` is the one primitive every
    // consumption ends in (the use tail `0x53648`, the hotkey/dialog tail `0x46784`, the
    // revive 예 `0x3b58e`, quest hand-ins): when it is called for the armed cell and the
    // record there still reads as the staged item, the `bagbox_restage_lgt` hook writes
    // the bag-item record back INSTEAD of clearing the cell and returns. So the cell is
    // never empty, the compactor that follows every one of those tails finds no hole,
    // and a save inside the tail (오브원석) captures a whole bag. Disarmed by the hook
    // when it fires, and by the host when the staged unit is no longer in that cell.
    //   * `BAGBOX_RESTAGE_CELL` — `tab<<24 | slot<<16 | cat<<8 | idx`; `u32::MAX` = off.
    //   * `BAGBOX_RESTAGE_REC` — the absolute guest address of that record.
    //   * `BAGBOX_RESTAGE_BYTES` — the 60-byte bag-item record, little-endian words.
    //   * `BAGBOX_RESTAGED` — how many times the hook has fired (diagnostic).
    pub const BAGBOX_RESTAGE_OFF: u32 = u32::MAX;
    pub const BAGBOX_REC_LEN: usize = 0x3c;
    pub static BAGBOX_RESTAGE_CELL: AtomicU32 = AtomicU32::new(BAGBOX_RESTAGE_OFF);
    pub static BAGBOX_RESTAGE_REC: AtomicU32 = AtomicU32::new(0);
    pub static BAGBOX_RESTAGE_BYTES: [AtomicU32; BAGBOX_REC_LEN / 4] = [const { AtomicU32::new(0) }; BAGBOX_REC_LEN / 4];
    pub static BAGBOX_RESTAGED: AtomicU32 = AtomicU32::new(0);

    /// What the restage knob names: the staged cell, the item staged there, the record
    /// address and the bytes that go back.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct BagboxRestage {
        pub tab: u8,
        pub slot: u8,
        pub cat: u8,
        pub idx: u8,
        pub rec: u32,
        pub bytes: [u8; BAGBOX_REC_LEN],
    }

    pub fn bagbox_restage_pack(tab: u8, slot: u8, cat: u8, idx: u8) -> u32 {
        (tab as u32) << 24 | (slot as u32) << 16 | (cat as u32) << 8 | idx as u32
    }

    /// Arm the knob. ⛔ The bytes go in BEFORE the cell word, and the cell word is what
    /// the hook tests first — so a hook that races the host can only see a fully written
    /// snapshot or nothing (the two run on the one emulator thread today; the order
    /// costs nothing and keeps that true).
    pub fn set_bagbox_restage(r: &BagboxRestage) {
        for (i, w) in BAGBOX_RESTAGE_BYTES.iter().enumerate() {
            let b = &r.bytes[i * 4..i * 4 + 4];
            w.store(u32::from_le_bytes([b[0], b[1], b[2], b[3]]), Ordering::Relaxed);
        }
        BAGBOX_RESTAGE_REC.store(r.rec, Ordering::Relaxed);
        BAGBOX_RESTAGE_CELL.store(bagbox_restage_pack(r.tab, r.slot, r.cat, r.idx), Ordering::Release);
    }

    pub fn clear_bagbox_restage() {
        BAGBOX_RESTAGE_CELL.store(BAGBOX_RESTAGE_OFF, Ordering::Relaxed);
        BAGBOX_RESTAGE_REC.store(0, Ordering::Relaxed);
    }

    /// The armed restage, or `None`.
    pub fn bagbox_restage() -> Option<BagboxRestage> {
        let cell = BAGBOX_RESTAGE_CELL.load(Ordering::Acquire);
        if cell == BAGBOX_RESTAGE_OFF {
            return None;
        }
        let rec = BAGBOX_RESTAGE_REC.load(Ordering::Relaxed);
        if rec == 0 {
            return None;
        }
        let mut bytes = [0u8; BAGBOX_REC_LEN];
        for (i, w) in BAGBOX_RESTAGE_BYTES.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&w.load(Ordering::Relaxed).to_le_bytes());
        }
        Some(BagboxRestage {
            tab: (cell >> 24) as u8,
            slot: (cell >> 16) as u8,
            cat: (cell >> 8) as u8,
            idx: cell as u8,
            rec,
            bytes,
        })
    }

    /// How many times the restage hook has put a bag-item cell back.
    pub fn bagbox_restaged() -> u32 {
        BAGBOX_RESTAGED.load(Ordering::Relaxed)
    }

    /// The next taken item, `(cat, idx, count, source)`, or `None` when the ring is empty.
    pub fn bagbox_pop() -> Option<(u16, u16, u8, u8)> {
        let tail = BAGBOX_TAIL.load(Ordering::Relaxed);
        if BAGBOX_HEAD.load(Ordering::Acquire) == tail {
            return None;
        }
        // ★ 165차: ZERO the slot as it is taken — a fold (`bagbox_ring_put`) compare-exchanges
        //   on the word it read, so it can never add units to an entry that is already filed.
        let w = BAGBOX_RING[tail as usize % BAGBOX_RING_LEN].swap(0, Ordering::AcqRel);
        BAGBOX_TAIL.store(tail.wrapping_add(1), Ordering::Release);
        Some(((w >> 24) as u16, ((w >> 16) & 0xff) as u16, (w >> 8) as u8, w as u8))
    }

    pub fn blit_snapshot() -> ([u64; 4], [u64; 3], [u64; 10], [u64; 5]) {
        let load = |a: &AtomicU64| a.load(Ordering::Relaxed);
        (
            [load(&BLIT_CALLS), load(&BLIT_HLE), load(&BLIT_PIXELS), load(&BLIT_PIXELS_HLE)],
            core::array::from_fn(|i| load(&BLIT_FMT[i])),
            core::array::from_fn(|i| load(&BLIT_TRANSFORM[i])),
            core::array::from_fn(|i| load(&BLIT_REFUSED[i])),
        )
    }

    /// `tintRect` — the per-pixel LUT colour transform a CRITICAL HIT runs over the
    /// screen (LGT `0x2b8f8`, see `wie_core_arm::binary_patches::tint565`).
    ///
    /// Kept apart from the `BLIT_*` counters on purpose: those mean "a SPRITE the host
    /// drew instead of interpreting", and this function draws no sprite. Folding it in
    /// would quietly change what the `blit N/M` ratio in the device log means.
    ///
    /// Reading these: `calls` climbing only while a critical flash is on screen is the
    /// expected shape, and `hle` short of `calls` is the gate declining — which is the
    /// first thing to check if the crit hitch ever comes back.
    pub static TINT_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static TINT_HLE: AtomicU64 = AtomicU64::new(0);
    pub static TINT_PIXELS: AtomicU64 = AtomicU64::new(0);

    pub fn add_tint(pixels: u64, hosted: bool) {
        TINT_CALLS.fetch_add(1, Ordering::Relaxed);
        if hosted {
            TINT_HLE.fetch_add(1, Ordering::Relaxed);
            TINT_PIXELS.fetch_add(pixels, Ordering::Relaxed);
        }
    }

    /// `(calls, hosted, hosted pixels)`.
    pub fn tint_snapshot() -> (u64, u64, u64) {
        let load = |a: &AtomicU64| a.load(Ordering::Relaxed);
        (load(&TINT_CALLS), load(&TINT_HLE), load(&TINT_PIXELS))
    }

    /// Guest SOUND-PATH telemetry (2026-08-09). The player reports that BGM and the
    /// critical-hit SFX cost frames, and the sound path runs **on the emulator thread**:
    /// `MC_mdaClipPutData` copies the file out of guest memory and `MC_mdaPlay` used to
    /// re-run the whole SMAF parse (chunk walk + event vector + sort) every single time a
    /// clip was played. These say how often that happens and how much of it the parse
    /// cache now absorbs.
    ///
    /// `PARSES` is what actually costs: a session where `PARSES` stays at the number of
    /// distinct sound files (33 in Hero3) while `PLAYS` climbs into the thousands is the
    /// cache working. `PARSES` tracking `PLAYS` means it is not.
    pub static SMAF_LOADS: AtomicU64 = AtomicU64::new(0);
    pub static SMAF_PLAYS: AtomicU64 = AtomicU64::new(0);
    pub static SMAF_PARSES: AtomicU64 = AtomicU64::new(0);
    pub static SMAF_PARSE_BYTES: AtomicU64 = AtomicU64::new(0);
    /// LEVELS (republished, not accumulated): parse-cache entries and live audio handles.
    /// The handle count is the one that used to grow without bound — every load minted a
    /// handle that nothing ever removed, and each one held its own copy of the file.
    pub static SMAF_CACHE_ENTRIES: AtomicU64 = AtomicU64::new(0);
    pub static AUDIO_HANDLES: AtomicU64 = AtomicU64::new(0);
    /// MIDI/PCM events a `SmafPlayer` skipped because the clock ran away from it — the
    /// app pauses the emulator worker while backgrounded, so on every foreground return
    /// a playing song is behind by however long the screen was off. Without the guard
    /// that whole backlog is dispatched in ONE poll (see `SmafPlayer::play`).
    pub static SMAF_CATCHUP: AtomicU64 = AtomicU64::new(0);

    pub fn add_smaf_load() {
        SMAF_LOADS.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_smaf_play() {
        SMAF_PLAYS.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_smaf_parse(bytes: u64) {
        SMAF_PARSES.fetch_add(1, Ordering::Relaxed);
        SMAF_PARSE_BYTES.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn add_smaf_catchup() {
        SMAF_CATCHUP.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_smaf_cache_entries(n: u64) {
        SMAF_CACHE_ENTRIES.store(n, Ordering::Relaxed);
    }

    pub fn set_audio_handles(n: u64) {
        AUDIO_HANDLES.store(n, Ordering::Relaxed);
    }

    /// `(loads, plays, parses, parse_bytes, cache_entries, live_handles, catchups)`
    pub fn audio_snapshot() -> [u64; 7] {
        let load = |a: &AtomicU64| a.load(Ordering::Relaxed);
        [
            load(&SMAF_LOADS),
            load(&SMAF_PLAYS),
            load(&SMAF_PARSES),
            load(&SMAF_PARSE_BYTES),
            load(&SMAF_CACHE_ENTRIES),
            load(&AUDIO_HANDLES),
            load(&SMAF_CATCHUP),
        ]
    }

    pub fn heap_snapshot() -> (u64, u64, u64, u64) {
        (
            ALLOCS.load(Ordering::Relaxed),
            FREES.load(Ordering::Relaxed),
            LIST_WALK.load(Ordering::Relaxed),
            BUCKET_HEADER_BYTES.load(Ordering::Relaxed),
        )
    }

    pub fn add_instructions(n: u64) {
        GUEST_INSTRUCTIONS.fetch_add(n, Ordering::Relaxed);
        RUN_BATCHES.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_frame() {
        GUEST_FRAMES.fetch_add(1, Ordering::Relaxed);
    }

    pub fn frames() -> u64 {
        GUEST_FRAMES.load(Ordering::Relaxed)
    }

    pub fn add_svc() {
        SVC_CALLS.fetch_add(1, Ordering::Relaxed);
    }

    /// ★ FRAME-PERIOD ACCOUNTING (2026-08-13, `MIGRATION_LGT.md` §2.24). The panel
    /// screens run at ~7 fps with the host thread 78 % IDLE, so the limiter is not work
    /// — it is how long the guest is left alone between frames. Two things decide that,
    /// and only measurement can say which: what the game ASKS for
    /// (`MC_knlSetTimer`'s timeout) and how long the event pump actually SLEEPS before
    /// looking again. `TIMER_MS_MIN` is the interesting one: the game's real frame
    /// interval. If min is ~20 and the pump's average nap is ~16, the loss is ours.
    pub static TIMER_SETS: AtomicU64 = AtomicU64::new(0);
    pub static TIMER_MS_MIN: AtomicU64 = AtomicU64::new(u64::MAX);
    pub static TIMER_MS_MAX: AtomicU64 = AtomicU64::new(0);
    pub static TIMER_MS_SUM: AtomicU64 = AtomicU64::new(0);
    /// Times the event pump found nothing due and slept, and the milliseconds it asked
    /// for in total.
    pub static PUMP_NAPS: AtomicU64 = AtomicU64::new(0);
    pub static PUMP_NAP_MS: AtomicU64 = AtomicU64::new(0);

    pub fn note_timer_set(timeout_ms: u64) {
        TIMER_SETS.fetch_add(1, Ordering::Relaxed);
        TIMER_MS_SUM.fetch_add(timeout_ms, Ordering::Relaxed);
        TIMER_MS_MIN.fetch_min(timeout_ms, Ordering::Relaxed);
        TIMER_MS_MAX.fetch_max(timeout_ms, Ordering::Relaxed);
    }

    /// ★ TIMER CANCELLATION (2026-08-14, PLAY_FEEDBACK_FIXES.md §41 item 7). Until this
    /// round `MC_knlUnsetTimer` was a do-nothing stub and `MC_knlSetTimer` PUSHED rather
    /// than replaced, so every screen the guest ever visited left one more live frame
    /// timer behind — the game sped up by one whole multiple per network round trip.
    /// `UNSETS` counts the guest's cancel calls; `STALE_DROPS` counts pending shots the
    /// event pump actually discarded because a re-arm or cancel outdated them. On a
    /// healthy build both track each other and `guest set N/s` stays flat across a
    /// warehouse visit; before this fix the second number did not exist and N/s grew.
    pub static TIMER_UNSETS: AtomicU64 = AtomicU64::new(0);
    pub static TIMER_STALE_DROPS: AtomicU64 = AtomicU64::new(0);

    pub fn note_timer_unset() {
        TIMER_UNSETS.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_timer_stale_drop() {
        TIMER_STALE_DROPS.fetch_add(1, Ordering::Relaxed);
    }

    /// `(unsets, stale_drops)` since the last call; resets.
    pub fn timer_cancel_take() -> (u64, u64) {
        (TIMER_UNSETS.swap(0, Ordering::Relaxed), TIMER_STALE_DROPS.swap(0, Ordering::Relaxed))
    }

    pub fn note_pump_nap(ms: u64) {
        PUMP_NAPS.fetch_add(1, Ordering::Relaxed);
        PUMP_NAP_MS.fetch_add(ms, Ordering::Relaxed);
    }

    /// ★ GUEST FRAME-TIMER CEILING (2026-08-14). §2.24 fixed the host's own two
    /// roundings and left one term standing: in its modal/SLOT states Hero3 asks to be
    /// woken every **244 ms** (the field asks 50), so those screens run at ~7 fps by the
    /// game's own design and no amount of host work changes it. Device measurement the
    /// same day pinned everything else — warehouse page turns are a single round trip at
    /// 24~49 ms and `recv polls/s` is 0 — so this constant is the whole remaining
    /// complaint.
    ///
    /// `0` disables the clamp (ship the guest's number unchanged). Any other value is
    /// the longest wake-up interval the host will honour — but ONLY for requests at or
    /// above [`TIMER_CLAMP_FLOOR_MS`], which is what keeps the player's own speed
    /// setting out of it. See that constant; it is not an optional refinement.
    ///
    /// ⚠ This is NOT free: a guest that counts frame ticks rather than wall-clock will
    /// count them faster. That is why it is a runtime switch rather than a constant —
    /// see `wie_android`'s `frame_timer_cap_ms` config key and the A/B on device.
    pub static TIMER_CLAMP_MS: AtomicU64 = AtomicU64::new(0);
    /// ★ THE FLOOR IS THE WHOLE SAFETY ARGUMENT (2026-08-14).
    ///
    /// The first version of this clamp had no floor and shipped as vc60. The player
    /// reported "네트워크에 접속한 이후로 게임이 너무 빨라졌다", and device measurement
    /// found the reason immediately: **Hero3's 게임속도 option IS this timer.**
    ///
    /// | 게임속도 | guest asks | (formula `110 - 10*speed`) |
    /// |---|---|---|
    /// | 1  | **100 ms** | slowest |
    /// | 6  | 50 ms | the shipped default |
    /// | 10 | **10 ms** | fastest |
    ///
    /// So a flat 50 ms cap silently rewrote every setting from 1 to 5 into 6 — at speed
    /// 1 the game ran at exactly double the chosen speed (`capped 100% to 50 ms`,
    /// measured in the field). 34차's note that "게임속도 설정은 게스트 로직 진행량만
    /// 바꾼다 (호스트에 speed 개념 없음)" is refuted by that measurement.
    ///
    /// The modal/network screens ask for **244 ms**, which is far outside the 10..100
    /// band the option can produce. Clamping only at or above this floor therefore
    /// keeps the entire speed setting untouched while still fixing the warehouse.
    pub const TIMER_CLAMP_FLOOR_MS: u64 = 200;
    /// How many `set_timer` calls the clamp actually shortened, since the last take.
    pub static TIMER_CLAMPED: AtomicU64 = AtomicU64::new(0);

    pub fn set_timer_clamp_ms(ms: u64) {
        TIMER_CLAMP_MS.store(ms, Ordering::Relaxed);
    }

    /// The interval the host will actually wait for, given what the guest asked for.
    /// Counts a clamp when it shortens one.
    ///
    /// Requests below [`TIMER_CLAMP_FLOOR_MS`] pass through untouched even when they are
    /// longer than the cap — those are the player's 게임속도 setting and are not ours to
    /// override.
    pub fn clamp_timer_ms(requested: u64) -> u64 {
        let cap = TIMER_CLAMP_MS.load(Ordering::Relaxed);
        if cap == 0 || requested < TIMER_CLAMP_FLOOR_MS || requested <= cap {
            return requested;
        }
        TIMER_CLAMPED.fetch_add(1, Ordering::Relaxed);
        cap
    }

    /// `(clamped_calls, cap_ms)` — clamped count resets, the cap does not.
    pub fn timer_clamp_take() -> (u64, u64) {
        (TIMER_CLAMPED.swap(0, Ordering::Relaxed), TIMER_CLAMP_MS.load(Ordering::Relaxed))
    }

    /// `(sets, min, max, mean, naps, nap_ms)` since the last call; resets.
    pub fn frame_timing_take() -> (u64, u64, u64, u64, u64, u64) {
        let sets = TIMER_SETS.swap(0, Ordering::Relaxed);
        let sum = TIMER_MS_SUM.swap(0, Ordering::Relaxed);
        let min = TIMER_MS_MIN.swap(u64::MAX, Ordering::Relaxed);
        (
            sets,
            if min == u64::MAX { 0 } else { min },
            TIMER_MS_MAX.swap(0, Ordering::Relaxed),
            sum / sets.max(1),
            PUMP_NAPS.swap(0, Ordering::Relaxed),
            PUMP_NAP_MS.swap(0, Ordering::Relaxed),
        )
    }

    /// Two more relaxed increments per SVC, on top of [`add_svc`]. See [`SVC_BY_ID`].
    pub fn add_svc_detail(category: u32, id: u32) {
        SVC_BY_CATEGORY[category as usize & 7].fetch_add(1, Ordering::Relaxed);
        SVC_BY_ID[id as usize & 2047].fetch_add(1, Ordering::Relaxed);
    }

    /// The hottest SVC ids since the last call, and the per-category totals, and
    /// RESET both — so the caller reports an interval rate rather than a lifetime
    /// average (the gauge that prints this makes the same choice for the same
    /// reason: a running average mixes the title screen with real play).
    pub fn svc_mix_take(top: usize) -> (Vec<u64>, Vec<(u32, u64)>) {
        let cats = SVC_BY_CATEGORY.iter().map(|c| c.swap(0, Ordering::Relaxed)).collect::<Vec<_>>();
        let mut hot = SVC_BY_ID
            .iter()
            .enumerate()
            .filter_map(|(id, c)| {
                let n = c.swap(0, Ordering::Relaxed);
                if n > 0 { Some((id as u32, n)) } else { None }
            })
            .collect::<Vec<_>>();
        hot.sort_unstable_by_key(|(_, n)| core::cmp::Reverse(*n));
        hot.truncate(top);
        (cats, hot)
    }

    // ── 공격속도 / 스킬사용속도 (attack + cast speed), the new stat ─────────────
    //
    // The player's every field action is a SCRIPT OF POSES and each pose step lasts until
    // the animation clock `base+0x9ba0` (bumped once per tick by the field state machine
    // `0x5089c` at `0x508bc`) equals that pose's frame count, which the guest reads via
    // `0x4aee0` (-> `0x1b780`, `sprite->frames[dir][pose] - 1`). A basic attack is state 2
    // (the script step, 3 ticks) then state 4 (recovery, 4 ticks) — eight ticks including
    // entry, measured on the probe.
    //
    // ⛔⛔ What `0x4aee0` RETURNS is never changed. 131차 first shortened it and called that
    // "the one place that cannot skip the hit, because damage resolves when a step
    // completes" — refuted twice: a shorter answer dealt ZERO damage (131차-B/D), and a hit
    // is a frame EVENT that `0x4b92c`'s switch (`0x57c3c`) fires on fixed frame numbers
    // (132차-C). The hook drags the CLOCK instead, clamped to `frames - 1` of the pose
    // running this very tick (132차), and `atk_event_lgt` replays the frame events the drag
    // stepped over. See `wie_core_arm::binary_patches::atkspeed`.
    //
    // The percent the host summed off the player's equipment (and/or the test file).
    // `0` = off, and off is byte-identical to the original: the clock is not touched.
    pub static ATTACK_SPEED_PCT: AtomicU32 = AtomicU32::new(0);

    /// Publish the player's attack/cast speed bonus, in percent. `0` disarms.
    pub fn set_attack_speed_pct(pct: u32) {
        ATTACK_SPEED_PCT.store(pct, Ordering::Relaxed);
    }

    /// The armed attack/cast speed bonus in percent (`0` = the original timing).
    pub fn attack_speed_pct() -> u32 {
        ATTACK_SPEED_PCT.load(Ordering::Relaxed)
    }

    // ── 플레이어 HP 상한 해제 (142차, 2026-09-30) ────────────────────────────────
    //
    // The player's max HP (`CHAR+0xec`) is an s32 the game computes in 32 bits
    // (`ComputeMaxHP 0x49000`), but the CURRENT HP (`CHAR+0xf2`) is an i16, and every
    // `hp = hp_max` restore stores the low half — a max above 32767 wraps the current HP
    // negative (dead). The user asked for the cap to go the way the damage cap went (102차).
    //
    // So the game keeps living in a <= 32767 world: `hp_max_lgt` hands it `min(T, 32767)`
    // (the GAUGE max) and publishes the TRUE max `T` here; the game's i16 is a proportional
    // GAUGE of the true current HP, which `hp_add_lgt` (the one HP writer every hit, heal,
    // regen and lifesteal goes through, `0x4b9c0`) keeps here. With `T <= 32767` the gauge IS
    // the HP and nothing differs from the original. Anything that writes the gauge behind
    // the hooks' back (the `hp = hp_max` restores, a save load) is noticed because the gauge
    // no longer matches [`PLAYER_HP_SEEN`], and the true HP is re-derived from it.
    pub const HP_GAUGE_CEILING: u32 = 32767;
    /// The true max HP (`0` = never published — the gauge max is then the max).
    pub static PLAYER_HP_MAX: AtomicU32 = AtomicU32::new(0);
    /// The true current HP as of the last hook write.
    pub static PLAYER_HP: AtomicU32 = AtomicU32::new(0);
    /// The gauge value the hook wrote alongside [`PLAYER_HP`]; `u32::MAX` = none.
    pub static PLAYER_HP_SEEN: AtomicU32 = AtomicU32::new(u32::MAX);

    /// The gauge (the game's i16) that shows `hp` of `max`: identity up to the ceiling,
    /// proportional above it, never 0 while anything is left (0 is "dead" to the game).
    pub fn hp_gauge(hp: u32, max: u32) -> u32 {
        if max <= HP_GAUGE_CEILING {
            return hp.min(HP_GAUGE_CEILING);
        }
        if hp == 0 {
            return 0;
        }
        let g = (u64::from(hp.min(max)) * u64::from(HP_GAUGE_CEILING) / u64::from(max)) as u32;
        g.clamp(1, HP_GAUGE_CEILING)
    }

    /// The true HP a gauge value stands for when nothing better is known.
    pub fn hp_from_gauge(gauge: u32, max: u32) -> u32 {
        if max <= HP_GAUGE_CEILING {
            return gauge;
        }
        if gauge == 0 {
            return 0;
        }
        if gauge >= HP_GAUGE_CEILING {
            return max;
        }
        ((u64::from(gauge) * u64::from(max) / u64::from(HP_GAUGE_CEILING)) as u32).max(1)
    }

    /// The true max for a gauge max: the published one when it agrees (`min(T, ceiling) ==
    /// gmax`), the gauge max otherwise (never published, another character's, a probe).
    pub fn hp_true_max(gmax: u32, published: u32) -> u32 {
        if published > 0 && published.min(HP_GAUGE_CEILING) == gmax {
            published
        } else {
            gmax
        }
    }

    /// The true current HP behind `gauge`. Up to the ceiling the gauge IS the HP (the
    /// original game, untouched). Above it: the stored value while the gauge is still the
    /// one the hook wrote with it (clamped to the max), re-derived from the gauge otherwise
    /// (a restore or a load wrote it behind the hooks' back).
    pub fn hp_true_current(gauge: u32, max: u32, stored: u32, seen: u32) -> u32 {
        if max <= HP_GAUGE_CEILING {
            return gauge;
        }
        if seen == gauge {
            return stored.min(max);
        }
        hp_from_gauge(gauge, max)
    }

    /// Publish the true max (the `hp_max_lgt` hook).
    pub fn set_player_hp_max(max: u32) {
        PLAYER_HP_MAX.store(max, Ordering::Relaxed);
    }

    /// ★ 150차 review — a character record was LOADED: everything above that belongs to the
    /// character who just left is forgotten, synchronously, before the new world's init runs
    /// its first `ComputeMaxHP` (`vault::note_slot_loaded` is called from the record read,
    /// and the same guest call goes on into `setAppState(4)` → world init `0x3ab68` → the stat
    /// init's `0x49000` · `0x4f014`). Without it the loaded character's first max was computed
    /// with the old one's kill/hit clocks and published set / accessory words, and with both
    /// characters above the ceiling at a full gauge (32767 = 32767) it ADOPTED the old one's
    /// tracked true HP. The worker republishes the mask, the accessory word and the attack
    /// speed for the new character within 30 frames. Returns the two timed clocks it cleared
    /// (for the caller's log).
    pub fn forget_character() -> (u64, u64) {
        let had = (WITCH_UNTIL_MS.swap(0, Ordering::Relaxed), HIT_UNTIL_MS.swap(0, Ordering::Relaxed));
        GUARD_UNTIL_MS.store(0, Ordering::Relaxed);
        GUARD_READY_MS.store(0, Ordering::Relaxed);
        HP_MAX_CLAMP.store(false, Ordering::Relaxed);
        PLAYER_HP_MAX.store(0, Ordering::Relaxed);
        PLAYER_HP.store(0, Ordering::Relaxed);
        // ⛔ `u32::MAX`, not 0 — a gauge of 0 is a real value; this one matches none, so the
        // first read re-derives the HP from the loaded gauge (`hp_true_current`).
        PLAYER_HP_SEEN.store(u32::MAX, Ordering::Relaxed);
        SET_BONUS.store(0, Ordering::Relaxed);
        set_acc_effects(AccEffects::NONE);
        // 160차: the four wide stats are the leaving character's too.
        for w in &PLAYER_WIDE {
            w.store(WIDE_NONE, Ordering::Relaxed);
        }
        had
    }

    /// Record the true current HP the hook just wrote `gauge` for.
    pub fn set_player_hp(hp: u32, gauge: u32) {
        PLAYER_HP.store(hp, Ordering::Relaxed);
        PLAYER_HP_SEEN.store(gauge, Ordering::Relaxed);
    }

    /// `(true current, true max)` for a gauge / gauge max read off the character — what the
    /// status screen and the host HUD show.
    pub fn player_hp_view(gauge: u32, gmax: u32) -> (u32, u32) {
        let max = hp_true_max(gmax, PLAYER_HP_MAX.load(Ordering::Relaxed));
        let cur = hp_true_current(
            gauge,
            max,
            PLAYER_HP.load(Ordering::Relaxed),
            PLAYER_HP_SEEN.load(Ordering::Relaxed),
        );
        (cur, max)
    }

    // ── 85제 세트 효과 (142차) ──────────────────────────────────────────────────
    //
    // The host counts the worn pieces (`setbonus.rs`) and publishes which sets are
    // complete; the hooks read the mask on every hit. `0` = no set, byte-identical to the
    // original.
    /// 템플러 「수호의 맹세」 — a hit that leaves HP <= [`GUARD_TRIGGER_HP`] leaves 1 and
    /// [`GUARD_INVULN_MS`] of invulnerability, then [`GUARD_COOLDOWN_MS`].
    pub const SET_GUARD: u32 = 1 << 0;
    /// 크루세이더 「기습」 — the first hit on a monster at full HP deals [`AMBUSH_MULT`]x.
    pub const SET_AMBUSH: u32 = 1 << 1;
    /// 레갈리스근위 「교단의 가호」 — damage taken -[`BLESSING_REDUCE_PCT`] %.
    pub const SET_BLESSING: u32 = 1 << 2;
    /// ★ 142차-C — the 90제 sets (user: "90제 세트 3종에도 세트효과를 줘야해"; effects re-set the
    /// same day: "「처형」 HP 50% 이하 … 「마녀의 생명력」 적 처치 시 20초간 최대 HP 20% 증가 …
    /// 「빙해의 일격」 모든 공격이 데미지 1.5배").
    /// 코르버스 「처형」 — a hit on a monster at <= [`EXECUTE_HP_PCT`] % HP deals [`EXECUTE_MULT`]x.
    pub const SET_EXECUTE: u32 = 1 << 3;
    /// 붉은마녀 「마녀의 생명력」 — a kill gives [`WITCH_BUFF_MS`] of max HP +[`WITCH_MAX_HP_PCT`] %
    /// (`dmgwide` stamps the kill, `playerhp`'s `ComputeMaxHP` hook scales the max, the host
    /// re-runs `ComputeMaxHP` when the bonus turns on or off).
    pub const SET_WITCH: u32 = 1 << 4;
    /// 빙해 「빙해의 일격」 — every hit deals [`ICE_MULT_NUM`]/[`ICE_MULT_DEN`] (1.5x).
    pub const SET_ICE: u32 = 1 << 5;
    pub const EXECUTE_HP_PCT: u32 = 50;
    pub const EXECUTE_MULT: u32 = 2;
    pub const WITCH_BUFF_MS: u64 = 20_000;
    pub const WITCH_MAX_HP_PCT: u32 = 20;
    pub const ICE_MULT_NUM: u64 = 3;
    pub const ICE_MULT_DEN: u64 = 2;
    /// 마녀의 생명력: the buff runs until (play clock ms; 0 = never). Each kill re-stamps it.
    pub static WITCH_UNTIL_MS: AtomicU64 = AtomicU64::new(0);
    /// Kills that stamped the buff (host log only).
    pub static WITCH_KILLS: AtomicU32 = AtomicU32::new(0);
    /// Set by the host right before it re-runs `ComputeMaxHP` for the buff's edge: the hook
    /// then clamps the current HP to the new max (the original game leaves an HP above a
    /// lowered max until the next `addHP`, which a 20-second buff would make visible).
    pub static HP_MAX_CLAMP: AtomicBool = AtomicBool::new(false);
    /// Hits a damage set effect changed (host log only).
    pub static SET_HITS: AtomicU32 = AtomicU32::new(0);

    /// The damage-dealt set effects on one hit, pure: `raw` the resolver's hit, `cur`/`max`
    /// the target's remaining and full HP (the pool's words when it has one), `dealt_pct` the
    /// worn accessories' 주는 피해 +N % (150차 검은 눈물, [`AccEffects::dealt_pct`]). Returns
    /// the new hit when something changed it. The multipliers are a PRODUCT — each item's
    /// words stay true whatever else is worn — taken in 64 bits and rounded down once per
    /// step (빙해 ×3/2, then the accessory ×(100 + p)/100).
    pub fn set_hit(raw: u32, cur: u64, max: u64, mask: u32, dealt_pct: u32) -> Option<u32> {
        if raw == 0 || cur == 0 {
            return None;
        }
        let mut hit = u64::from(raw);
        if mask & SET_AMBUSH != 0 && cur >= max {
            hit *= u64::from(AMBUSH_MULT);
        }
        if mask & SET_EXECUTE != 0 && cur * 100 <= max * u64::from(EXECUTE_HP_PCT) {
            hit *= u64::from(EXECUTE_MULT);
        }
        if mask & SET_ICE != 0 {
            hit = hit * ICE_MULT_NUM / ICE_MULT_DEN;
        }
        let dealt = dealt_pct.min(ACC_DEALT_CAP_PCT);
        if dealt > 0 {
            hit = hit * u64::from(100 + dealt) / 100;
        }
        (hit != u64::from(raw)).then_some(hit.min(u64::from(u32::MAX)) as u32)
    }

    /// The timed max-HP bonus in percent, pure — read by the `ComputeMaxHP` hook and by the
    /// host (which re-runs that function when the answer changes). Two clocks, one per
    /// trigger: `kill_until` ([`WITCH_UNTIL_MS`], stamped by a kill — the 붉은마녀 set's
    /// 마녀의 생명력 and 150차 레아의목걸이 share it: same trigger, same 20 s) and
    /// `hit_until` ([`HIT_UNTIL_MS`], stamped by a hit taken — 150차 굴베이그의탈리스만).
    /// The running sources ADD (set + 레아 = +40 %, all three = +60 %), capped at
    /// [`MAX_HP_BONUS_CAP_PCT`]; the result multiplies the game's own max (HP% options, the
    /// native set's +20) in [`boosted_max`].
    pub fn max_hp_bonus_pct(mask: u32, acc: AccEffects, now: u64, kill_until: u64, hit_until: u64) -> u32 {
        let mut p = 0;
        if now < kill_until {
            if mask & SET_WITCH != 0 {
                p += WITCH_MAX_HP_PCT;
            }
            p += u32::from(acc.kill_max_hp_pct);
        }
        if now < hit_until {
            p += u32::from(acc.hit_max_hp_pct);
        }
        p.min(MAX_HP_BONUS_CAP_PCT)
    }

    /// The max after the bonus, pure (saturating at `i32::MAX`, the field's type).
    pub fn boosted_max(t: i32, bonus_pct: u32) -> i32 {
        if t <= 0 || bonus_pct == 0 {
            return t;
        }
        (i64::from(t) * i64::from(100 + bonus_pct) / 100).min(i64::from(i32::MAX)) as i32
    }

    /// A kill by the player: with 마녀의 생명력 (the set) or a kill-triggered accessory
    /// (150차 레아의목걸이) worn, (re)start the kill clock. Returns the new `until` when it
    /// was stamped. One clock for both — same trigger, same [`WITCH_BUFF_MS`].
    pub fn on_kill(mask: u32, acc: AccEffects, now: u64) -> Option<u64> {
        if mask & SET_WITCH == 0 && acc.kill_max_hp_pct == 0 {
            return None;
        }
        let until = now + WITCH_BUFF_MS;
        WITCH_UNTIL_MS.store(until, Ordering::Relaxed);
        WITCH_KILLS.fetch_add(1, Ordering::Relaxed);
        Some(until)
    }

    /// A hit the player TOOK (150차 굴베이그의탈리스만 「피격 시 20초간 최대 HP +20%」), pure:
    /// `before`/`after` the true HP around the hit. `Some(until)` when the hit clock should
    /// be (re)stamped — only with the accessory worn, only for a hit that actually cost HP
    /// (an invulnerable or 0-damage hit is not one) and left the player alive (the death
    /// path owns a hit to 0).
    pub fn hit_stamp(acc: AccEffects, before: u32, after: u32, now: u64) -> Option<u64> {
        (acc.hit_max_hp_pct > 0 && after > 0 && after < before).then_some(now + HIT_BUFF_MS)
    }

    // ── 150차 레거시 장신구의 3번째 옵션 (2026-10-01) ─────────────────────────────
    //
    // `acc_options` (generated) says what each reworked accessory's third option is. The
    // `Stat` ones never reach here — the wear-recompute hook feeds them to the game's own
    // `addStat`. The rest are host/hook effects: the host sums them off the nine worn slots
    // (every copy counts, like the game sums option pairs — the two accessory slots may hold
    // the same item) and publishes ONE word; the hooks read it on every hit.

    /// The worn accessories' effects. `NONE` = the original game.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct AccEffects {
        /// 받는 피해 -N % (이안의맹세 10), capped at [`ACC_TAKEN_CAP_PCT`].
        pub taken_pct: u8,
        /// 주는 피해 +N % (검은 눈물 25), capped at [`ACC_DEALT_CAP_PCT`].
        pub dealt_pct: u8,
        /// 적 처치 시 [`WITCH_BUFF_MS`] 동안 최대 HP +N % (레아의목걸이 20).
        pub kill_max_hp_pct: u8,
        /// 피격 시 [`HIT_BUFF_MS`] 동안 최대 HP +N % (굴베이그의탈리스만 20).
        pub hit_max_hp_pct: u8,
    }

    impl AccEffects {
        pub const NONE: Self = Self { taken_pct: 0, dealt_pct: 0, kill_max_hp_pct: 0, hit_max_hp_pct: 0 };

        pub const fn pack(self) -> u32 {
            self.taken_pct as u32
                | (self.dealt_pct as u32) << 8
                | (self.kill_max_hp_pct as u32) << 16
                | (self.hit_max_hp_pct as u32) << 24
        }

        pub const fn unpack(v: u32) -> Self {
            Self {
                taken_pct: v as u8,
                dealt_pct: (v >> 8) as u8,
                kill_max_hp_pct: (v >> 16) as u8,
                hit_max_hp_pct: (v >> 24) as u8,
            }
        }

        pub const fn is_none(self) -> bool {
            self.pack() == 0
        }
    }

    /// 받는 피해 감소의 상한 — two 이안의맹세 are -20; the cap keeps any future table sane.
    pub const ACC_TAKEN_CAP_PCT: u32 = 50;
    /// 주는 피해 증가의 상한.
    pub const ACC_DEALT_CAP_PCT: u32 = 200;
    /// The timed max-HP bonus's cap (all three sources at once = 60).
    pub const MAX_HP_BONUS_CAP_PCT: u32 = 100;

    // ── 158차 (2026-10-02) — two stat-sum caps the game lacks (user: «전부 권장대로») ──
    /// P1 — the 명중 sum the wear recompute stores as a BYTE (`stat+0x2c`, `0x4f7ca strb`)
    /// is clamped here instead of wrapping at 256 (`wie_core_arm::binary_patches::statcap`).
    /// 255 is the byte's own ceiling: every sum of 0..=255 stays byte-identical.
    pub const HIT_SUM_CAP: i32 = 255;
    /// P2 — the worn 쿨타임 감소 % the cast path reads (`0x4bbe0 ldrsh`, the `addStat(0x11)`
    /// word at CHAR+0x124) is capped here: a skill's cooldown becomes `desc × (100 −
    /// min(sum, this)) / 100`. 70 = 30 % of the base cooldown (user decision 2026-10-10;
    /// without a cap a sum of 100 % or more was a one-tick cooldown). The host's own cooldown
    /// overlay (`items::read_cd_reduce_pct`) applies the same cap so the ring agrees.
    pub const COOLDOWN_REDUCE_CAP_PCT: i32 = 70;

    // ── 160차 (2026-10-03) — 방어% 상한 · 공격력/방어력의 i16 해제 (`TOWER_100_CLEAR_AUDIT.md`) ──
    /// T2 (user: «방어 % 상한을 둘꺼야» → 500) — the ceiling on the summed 물리방어% / 특수방어%
    /// the two defence getters read (`0x4cf72` / `0x4cfb2 ldrsh`): defence becomes `(armour +
    /// 10) × (100 + min(sum, this)) / 100 + 체력`. 500 = two full 금강/정령 orbs (2 × 225) plus
    /// 50 of options; without a cap a third orb made the wearer take 1 from every monster in
    /// the tower (and a fourth wrapped the i16). Below the cap the getters are byte-identical.
    pub const DEF_PCT_CAP: i32 = 500;

    /// T1 (user: «MAX치 상한자체를 해제 … 32767 넘을수있어야해») — the four derived stats the
    /// game keeps in an `i16`, by slot. The getters' tails truncated them (`lsls/asrs #16`, a
    /// value past 32767 went NEGATIVE); the `statwide` hooks saturate the game's halfword at
    /// 32767 instead and keep the TRUE value here, and the two combat readers (the player's
    /// hit `0x4bdac`/`0x4be12`, the monster's hit `0x38556`) take it back from here.
    pub const WIDE_ATK1: usize = 0;
    pub const WIDE_ATK2: usize = 1;
    pub const WIDE_PDEF: usize = 2;
    pub const WIDE_MDEF: usize = 3;
    /// "Never published" — a value no getter can return for a real character.
    const WIDE_NONE: i32 = i32::MIN;
    pub static PLAYER_WIDE: [AtomicI32; 4] =
        [AtomicI32::new(WIDE_NONE), AtomicI32::new(WIDE_NONE), AtomicI32::new(WIDE_NONE), AtomicI32::new(WIDE_NONE)];

    /// What the game's halfword holds for a true value: the value itself inside the `i16`,
    /// its nearest end outside (so an in-range stat is byte-identical to the original).
    pub fn sat_i16(v: i32) -> i32 {
        v.clamp(i32::from(i16::MIN), i32::from(i16::MAX))
    }

    /// Publish the true value of `slot` (the wear recompute's own call of the getter).
    pub fn set_player_wide(slot: usize, v: i32) -> i32 {
        PLAYER_WIDE[slot].swap(v, Ordering::Relaxed)
    }

    /// The value a combat reader should use for `slot`, given the halfword it just loaded:
    /// the published true value WHEN the halfword is what that value saturates to, otherwise
    /// the halfword itself. So anything that rewrote the halfword behind the hooks' back (a
    /// loaded record before its first recompute, a buff that pokes the field) is believed
    /// over a stale publication — the same rule as [`PLAYER_HP_SEEN`].
    pub fn player_wide(slot: usize, guest: i16) -> i32 {
        wide_or_guest(PLAYER_WIDE[slot].load(Ordering::Relaxed), guest)
    }

    /// [`player_wide`], pure.
    pub fn wide_or_guest(wide: i32, guest: i16) -> i32 {
        if wide != WIDE_NONE && sat_i16(wide) == i32::from(guest) { wide } else { i32::from(guest) }
    }

    /// The defence % the getters apply, pure: the game's signed sum capped at [`DEF_PCT_CAP`].
    pub fn capped_def_pct(sum: i16) -> i32 {
        i32::from(sum).min(DEF_PCT_CAP)
    }
    /// 굴베이그의탈리스만: a hit's buff length.
    pub const HIT_BUFF_MS: u64 = 20_000;
    /// The published [`AccEffects`] (packed).
    pub static ACC_EFFECTS: AtomicU32 = AtomicU32::new(0);
    /// The hit clock (play clock ms; 0 = never), stamped by `playerhp`'s damage branch.
    pub static HIT_UNTIL_MS: AtomicU64 = AtomicU64::new(0);
    /// Hits that stamped the hit clock (host log only).
    pub static HIT_STAMPS: AtomicU32 = AtomicU32::new(0);

    pub fn set_acc_effects(e: AccEffects) {
        ACC_EFFECTS.store(e.pack(), Ordering::Relaxed);
    }

    pub fn acc_effects() -> AccEffects {
        AccEffects::unpack(ACC_EFFECTS.load(Ordering::Relaxed))
    }

    /// Sum the effects of the worn `(cat, idx)` — all nine slots, duplicates COUNT (pure).
    pub fn acc_effects_of(worn: impl IntoIterator<Item = (u16, u16)>) -> AccEffects {
        use crate::acc_options::{Third, third_of};
        let (mut taken, mut dealt, mut kill, mut hit) = (0u32, 0u32, 0u32, 0u32);
        for (cat, idx) in worn {
            let Some(t) = third_of(cat, idx) else { continue };
            let v = t.value.max(0) as u32;
            match t.third {
                Third::Taken => taken += v,
                Third::Dealt => dealt += v,
                Third::KillMaxHp => kill += v,
                Third::HitMaxHp => hit += v,
                Third::Stat(_) | Third::AtkSpeed => {}
            }
        }
        AccEffects {
            taken_pct: taken.min(ACC_TAKEN_CAP_PCT) as u8,
            dealt_pct: dealt.min(ACC_DEALT_CAP_PCT) as u8,
            kill_max_hp_pct: kill.min(MAX_HP_BONUS_CAP_PCT) as u8,
            hit_max_hp_pct: hit.min(MAX_HP_BONUS_CAP_PCT) as u8,
        }
    }

    /// The host-summed extra `(statId, value)` pairs a worn `(cat, idx)` adds on top of its
    /// asset row — today only the 공격속도 third (타이탄의 파편), which the host sums with
    /// the option pairs (`Stat` thirds go through the game's `addStat` instead).
    pub fn host_third_pair(cat: u16, idx: u16) -> Option<(u8, i8)> {
        use crate::acc_options::{Third, third_of};
        match third_of(cat, idx) {
            Some(t) if t.third == Third::AtkSpeed => Some((ATTACK_SPEED_STAT, t.value)),
            _ => None,
        }
    }

    /// A/B switch for the `Stat` thirds (`acc_third_stat_lgt`): `true` = the hook adds
    /// nothing (the probe's `--third-opts 0`). Never set by the app.
    pub static THIRD_OPTS_OFF: AtomicBool = AtomicBool::new(false);

    pub fn third_opts_on() -> bool {
        !THIRD_OPTS_OFF.load(Ordering::Relaxed)
    }

    /// The 공격속도 option id (131차-D; `InGame_txt` row 198 − 80). Mirrors
    /// `atkspeed::ATTACK_SPEED_STAT` in `wie_core_arm`/`wie_android` (asserted there).
    pub const ATTACK_SPEED_STAT: u8 = 0x76;

    /// 150차 레아의목걸이 「5초당 HP회복 +N%」, pure: how much one 5-second tick heals, in
    /// true HP — `ceil(max × pct / 100)`, never more than the deficit, never above an i16
    /// `addHP` delta (32767). `0` = nothing to do (no option, full, dead, no max).
    pub fn regen_amount(hp: u32, max: u32, pct: u32) -> u32 {
        if pct == 0 || max == 0 || hp == 0 || hp >= max {
            return 0;
        }
        let want = (u64::from(max) * u64::from(pct.min(100))).div_ceil(100) as u32;
        want.min(max - hp).min(HP_GAUGE_CEILING).max(1)
    }

    /// The regen clock, pure: `(heal now?, next due)`. `due == 0` arms it one period out, a
    /// clock that went back re-arms it. A due time that has passed pays ONE heal: slightly
    /// late keeps the rhythm (`due + period`), a period or more behind (a menu, a scene
    /// change — the caller only ticks while healing is allowed) is re-armed from now.
    /// ⛔ Never `due + period` when that is still in the past (150차 review): the old rule
    /// paid a 5-20 s gap back as 2-4 heals in consecutive polls.
    pub fn regen_tick(now: u64, due: u64, period: u64) -> (bool, u64) {
        if due == 0 || now + period < due {
            return (false, now + period);
        }
        if now < due {
            return (false, due);
        }
        let next = due + period;
        (true, if next > now { next } else { now + period })
    }

    pub const GUARD_TRIGGER_HP: u32 = 10;
    pub const GUARD_INVULN_MS: u64 = 5_000;
    pub const GUARD_COOLDOWN_MS: u64 = 100_000;
    /// ★ 158차 P4 (2026-10-02, user «전부 권장대로»): 3 → 2. With ×3 the 85제 크루세이더 set
    /// out-damaged every 90제 set in hell (+298 % vs 빙해 +257 %, `ITEM_BALANCE_AUDIT_156.md
    /// §0 3`) — the first hit on a full monster that dies in 3~6 hits was +84 % on its own.
    pub const AMBUSH_MULT: u32 = 2;
    pub const BLESSING_REDUCE_PCT: u32 = 20;
    pub static SET_BONUS: AtomicU32 = AtomicU32::new(0);
    /// Game-time clock in ms, published by the host each worker pass; stops while paused.
    pub static PLAY_CLOCK_MS: AtomicU64 = AtomicU64::new(0);
    /// 수호의 맹세: invulnerable until / ready again at (play clock ms; 0 = never).
    pub static GUARD_UNTIL_MS: AtomicU64 = AtomicU64::new(0);
    pub static GUARD_READY_MS: AtomicU64 = AtomicU64::new(0);
    /// How many times 수호의 맹세 has fired (the host turns a change into a message).
    pub static GUARD_TRIGGERS: AtomicU32 = AtomicU32::new(0);

    pub fn set_set_bonus(mask: u32) {
        SET_BONUS.store(mask, Ordering::Relaxed);
    }

    pub fn set_bonus() -> u32 {
        SET_BONUS.load(Ordering::Relaxed)
    }

    // ── 156차 방패무시 (2026-10-02) ──────────────────────────────────────────────
    //
    // The game sums option id 0x12 into `stat+0x72` and never reads it (no load of that
    // halfword anywhere in the image), so 「방패무시 +N%」 did nothing. The host publishes the
    // worn total and whether the main weapon resolves against the SPECIAL defence (categories
    // above 6), and the `dmg_apply_lgt` hook gives a hit back `def × N / 100` of the defence
    // the resolver subtracted — one subtraction per hit component; a basic attack has one.

    /// `pct | (special as u32) << 16`, see [`set_pierce`].
    pub static PIERCE: AtomicU32 = AtomicU32::new(0);
    /// Hits the pierce bonus changed (the first few are logged).
    pub static PIERCE_HITS: AtomicU32 = AtomicU32::new(0);

    pub fn set_pierce(pct: u32, special: bool) {
        PIERCE.store((pct.min(100)) | (u32::from(special) << 16), Ordering::Relaxed);
    }

    /// `(pct, special weapon)` as last published.
    pub fn pierce() -> (u32, bool) {
        let v = PIERCE.load(Ordering::Relaxed);
        (v & 0xffff, v >> 16 != 0)
    }

    /// The pierced hit, pure: `raw + def × pct / 100`, or `None` when nothing changes —
    /// no 방패무시 worn, no defence to pierce, or a hit the resolver floored to 1 (an attack
    /// that did not exceed the defence; the true attack is unknown there, so the floor
    /// stays). Capped at the resolver's own 9,999,999.
    pub fn pierce_bonus(raw: u32, def: i16, pct: u32) -> Option<u32> {
        if pct == 0 || def <= 0 || raw <= 1 {
            return None;
        }
        let bonus = u32::from(def as u16) * pct.min(100) / 100;
        (bonus > 0).then(|| raw.saturating_add(bonus).min(9_999_999))
    }

    pub fn set_play_clock_ms(ms: u64) {
        PLAY_CLOCK_MS.store(ms, Ordering::Relaxed);
    }

    pub fn play_clock_ms() -> u64 {
        PLAY_CLOCK_MS.load(Ordering::Relaxed)
    }

    /// The damage a hit on the player comes to after the reductions, pure: 교단의 가호
    /// (-[`BLESSING_REDUCE_PCT`] %) and the accessories' 피해감소 (-`taken_pct` %, capped at
    /// [`ACC_TAKEN_CAP_PCT`]) MULTIPLY — each item's words stay true whatever else is worn —
    /// rounded down once, and a hit that did damage still does at least 1.
    pub fn taken_after(dmg: u32, mask: u32, taken_pct: u32) -> u32 {
        let set = if mask & SET_BLESSING != 0 { BLESSING_REDUCE_PCT } else { 0 };
        let acc = taken_pct.min(ACC_TAKEN_CAP_PCT);
        if dmg == 0 || (set == 0 && acc == 0) {
            return dmg;
        }
        (u64::from(dmg) * u64::from(100 - set) * u64::from(100 - acc) / 10_000).max(1) as u32
    }

    /// What one hit on the player comes to, pure: `hp` true current, `dmg` true damage
    /// (> 0), `mask` the worn sets, `taken_pct` the accessories' 피해감소 (150차), `now` the
    /// play clock, `until`/`ready` the guard's two clocks. Returns `(new hp, Some((until,
    /// ready)) when the guard fired)`.
    pub fn player_hit(
        hp: u32,
        dmg: u32,
        mask: u32,
        taken_pct: u32,
        now: u64,
        until: u64,
        ready: u64,
    ) -> (u32, Option<(u64, u64)>) {
        if mask & SET_GUARD != 0 && now < until {
            return (hp, None);
        }
        let dmg = taken_after(dmg, mask, taken_pct);
        let left = hp.saturating_sub(dmg);
        if mask & SET_GUARD != 0 && now >= ready && left <= GUARD_TRIGGER_HP && hp > 0 {
            return (left.max(1), Some((now + GUARD_INVULN_MS, now + GUARD_COOLDOWN_MS)));
        }
        (left, None)
    }

    pub fn snapshot() -> (u64, u64, u64) {
        (
            GUEST_INSTRUCTIONS.load(Ordering::Relaxed),
            SVC_CALLS.load(Ordering::Relaxed),
            RUN_BATCHES.load(Ordering::Relaxed),
        )
    }
}

pub trait ByteRead {
    fn read_bytes(&self, address: u32, result: &mut [u8]) -> Result<usize>;
}

pub trait ByteWrite {
    fn write_bytes(&mut self, address: u32, data: &[u8]) -> Result<()>;
}

pub fn read_generic<T, R>(reader: &R, address: u32) -> Result<T>
where
    T: Copy + AnyBitPattern + NoUninit,
    R: ?Sized + ByteRead,
{
    if address == 0 {
        return Err(WieError::InvalidMemoryAccess(address));
    }

    let mut destination = MaybeUninit::<T>::uninit();
    let destination_bytes = unsafe { from_raw_parts_mut(destination.as_mut_ptr().cast::<u8>(), size_of::<T>()) };
    let read = reader.read_bytes(address, destination_bytes)?;
    if read != destination_bytes.len() {
        return Err(WieError::FatalError(format!(
            "Short read at {address:#x}: expected {}, got {read}",
            destination_bytes.len()
        )));
    }

    Ok(unsafe { destination.assume_init() })
}

pub fn read_null_terminated_string_bytes<R>(reader: &R, address: u32) -> Result<Vec<u8>>
where
    R: ?Sized + ByteRead,
{
    if address == 0 {
        return Err(WieError::InvalidMemoryAccess(address));
    }

    let mut result = Vec::with_capacity(20);
    let mut cursor = address;
    let mut byte = [0; 1];
    loop {
        let read = reader.read_bytes(cursor, &mut byte)?;
        if read != 1 {
            return Err(WieError::FatalError(format!("Short read at {cursor:#x}: expected 1, got {read}")));
        }

        if byte[0] == 0 {
            break;
        }

        result.push(byte[0]);
        cursor += 1;
    }

    Ok(result)
}

pub fn write_null_terminated_string_bytes<W>(writer: &mut W, address: u32, bytes: &[u8]) -> Result<()>
where
    W: ?Sized + ByteWrite,
{
    if address == 0 {
        return Err(WieError::InvalidMemoryAccess(address));
    }

    // TODO temp
    writer.write_bytes(address, bytes)?;
    writer.write_bytes(address + bytes.len() as u32, &[0])?;

    Ok(())
}

pub fn write_generic<W, T>(writer: &mut W, address: u32, data: T) -> Result<()>
where
    W: ?Sized + ByteWrite,
    T: NoUninit,
{
    if address == 0 {
        return Err(WieError::InvalidMemoryAccess(address));
    }

    let data_slice = bytes_of(&data);

    writer.write_bytes(address, data_slice)
}

pub fn read_null_terminated_table<R>(reader: &R, base_address: u32) -> Result<Vec<u32>>
where
    R: ?Sized + ByteRead,
{
    if base_address == 0 {
        return Err(WieError::InvalidMemoryAccess(base_address));
    }

    let mut cursor = base_address;
    let mut result = Vec::new();
    loop {
        let item: u32 = read_generic(reader, cursor)?;
        if item == 0 {
            break;
        }
        result.push(item);

        cursor += 4;
    }

    Ok(result)
}

pub fn write_null_terminated_table<W>(writer: &mut W, base_address: u32, items: &[u32]) -> Result<()>
where
    W: ?Sized + ByteWrite,
{
    if base_address == 0 {
        return Err(WieError::InvalidMemoryAccess(base_address));
    }

    let mut cursor = base_address;
    for &item in items {
        write_generic(writer, cursor, item)?;

        cursor += 4;
    }
    write_generic(writer, cursor, 0u32)
}

pub trait AsAny {
    fn as_any(&self) -> &dyn Any;

    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T> AsAny for T
where
    T: Any,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    /// 160차 T1/T2: an in-range stat is the game's own number; past the i16 the halfword
    /// saturates and the true value is taken back only while the halfword still says so.
    #[test]
    fn the_wide_stats_saturate_the_halfword_and_believe_it_over_a_stale_value() {
        assert_eq!(stats::sat_i16(0), 0);
        assert_eq!(stats::sat_i16(32767), 32767);
        assert_eq!(stats::sat_i16(32768), 32767, "was -32768 through lsls/asrs");
        assert_eq!(stats::sat_i16(37391), 32767, "장로회의관 + 정령 x4 read as -28145 before");
        assert_eq!(stats::sat_i16(-5), -5);
        assert_eq!(stats::sat_i16(-40000), -32768);
        // the reader: a published value is used only when the halfword is its saturation
        assert_eq!(stats::wide_or_guest(18_542, 18_542), 18_542);
        assert_eq!(stats::wide_or_guest(37_391, 32_767), 37_391, "the true value past the i16");
        assert_eq!(stats::wide_or_guest(37_391, 12_000), 12_000, "the halfword moved behind the hooks");
        assert_eq!(stats::wide_or_guest(i32::MIN, 32_767), 32_767, "never published = the halfword");
        assert_eq!(stats::wide_or_guest(100, 32_767), 32_767, "a stale small value does not undo a saturated field");
        // the cap: two full orbs and fifty of options, nothing below it changes
        assert_eq!(stats::DEF_PCT_CAP, 500, "user decision 2026-10-03 (T2)");
        assert_eq!(stats::capped_def_pct(0), 0);
        assert_eq!(stats::capped_def_pct(480), 480, "정령 x2 + 이안의맹세 30");
        assert_eq!(stats::capped_def_pct(500), 500);
        assert_eq!(stats::capped_def_pct(1130), 500, "전설 방어구 + 정령 x4");
        assert_eq!(stats::capped_def_pct(-20), -20, "a negative stays the game's own");
    }

    struct StrictMemory {
        memory: Vec<u8>,
    }

    impl ByteRead for StrictMemory {
        fn read_bytes(&self, address: u32, result: &mut [u8]) -> Result<usize> {
            let address = address as usize;
            let end = address + result.len();
            if end > self.memory.len() {
                return Err(WieError::InvalidMemoryAccess(address as u32));
            }

            result.copy_from_slice(&self.memory[address..end]);

            Ok(result.len())
        }
    }

    #[test]
    fn read_generic_reads_into_initialized_storage() {
        let memory = StrictMemory {
            memory: vec![0, 0x78, 0x56, 0x34, 0x12],
        };

        let value: u32 = read_generic(&memory, 1).unwrap();

        assert_eq!(value, 0x1234_5678);
    }

    #[test]
    fn read_null_terminated_string_handles_four_byte_boundaries() {
        let memory = StrictMemory {
            memory: vec![0, b't', b'e', b's', b't', 0],
        };

        let value = read_null_terminated_string_bytes(&memory, 1).unwrap();

        assert_eq!(value, b"test");
    }

    /// ★ 150차 — the third-option table and the accessory effects contract.
    #[test]
    fn the_accessory_contract_holds() {
        use super::acc_options::{ACC_THIRDS, REGEN_STAT, Third, guest_stat, third_of};
        use super::stats::{self, AccEffects};

        // The table: 11 rows, sorted by idx, unique, every Stat id an addStat arm, summaries
        // in the painter's buffer.
        assert_eq!(ACC_THIRDS.len(), 11);
        for w in ACC_THIRDS.windows(2) {
            assert!(w[0].idx < w[1].idx, "sorted and unique");
        }
        for t in &ACC_THIRDS {
            assert_eq!(t.cat, 0x0c);
            assert!(t.value > 0);
            assert!(!t.summary_euc.is_empty() && t.summary_euc.len() <= 51);
            assert!(!t.summary_euc.contains(&0), "no NUL inside");
            if let Third::Stat(id) = t.third {
                assert!(guest_stat(id), "{} -> {id:#x}", t.name);
            }
        }
        assert!(!guest_stat(0x04) && !guest_stat(0x13) && !guest_stat(REGEN_STAT) && !guest_stat(0x76));
        assert!(third_of(0x0c, 0x2e).is_some_and(|t| t.third == Third::KillMaxHp));
        assert!(third_of(0x0b, 0x2e).is_none(), "the category counts");
        assert!(third_of(0x0c, 0x27).is_none(), "a stock accessory has none");

        // Pack / unpack round trip; NONE is 0.
        let e = AccEffects { taken_pct: 10, dealt_pct: 25, kill_max_hp_pct: 20, hit_max_hp_pct: 40 };
        assert_eq!(AccEffects::unpack(e.pack()), e);
        assert_eq!(AccEffects::NONE.pack(), 0);
        assert!(AccEffects::NONE.is_none() && !e.is_none());

        // Summing the worn slots: every copy counts, Stat / 공격속도 thirds are not effects.
        let worn = [(0x0c, 0x2d), (0x0c, 0x2d), (0x0c, 0x29), (0x0c, 0x2e), (0x0c, 0x31), (0x0c, 0x2a), (0, 0x30)];
        let got = stats::acc_effects_of(worn);
        assert_eq!(got, AccEffects { taken_pct: 20, dealt_pct: 25, kill_max_hp_pct: 20, hit_max_hp_pct: 20 });
        assert_eq!(stats::acc_effects_of([(0x0c, 0x2a), (0x0c, 0x28)]), AccEffects::NONE);
        let many = [(0x0c, 0x2d); 9];
        assert_eq!(stats::acc_effects_of(many).taken_pct as u32, stats::ACC_TAKEN_CAP_PCT, "capped");
        assert_eq!(stats::host_third_pair(0x0c, 0x28), Some((stats::ATTACK_SPEED_STAT, 50)), "타이탄의 파편");
        assert_eq!(stats::host_third_pair(0x0c, 0x2a), None, "a Stat third is the guest's");

        // The regen tick (레아의목걸이 option 2).
        assert_eq!(stats::regen_amount(100, 1_000, 20), 200, "20 % of the max");
        assert_eq!(stats::regen_amount(900, 1_000, 20), 100, "never past the max");
        assert_eq!(stats::regen_amount(1_000, 1_000, 20), 0, "full");
        assert_eq!(stats::regen_amount(0, 1_000, 20), 0, "dead: the death path owns it");
        assert_eq!(stats::regen_amount(1, 7, 20), 2, "rounded up");
        assert_eq!(stats::regen_amount(1, 200_000, 20), 32_767, "one addHP delta");
        assert_eq!(stats::regen_amount(10, 1_000, 0), 0, "not worn");
        assert_eq!(stats::regen_tick(1_000, 0, 5_000), (false, 6_000), "armed one period out");
        assert_eq!(stats::regen_tick(5_999, 6_000, 5_000), (false, 6_000));
        assert_eq!(stats::regen_tick(6_000, 6_000, 5_000), (true, 11_000));
        assert_eq!(stats::regen_tick(7_000, 6_000, 5_000), (true, 11_000), "late: the rhythm holds");
        assert_eq!(stats::regen_tick(60_000, 6_000, 5_000), (true, 65_000), "far behind: ONE heal, re-armed from now");
        // ★ 150차 review — 8 s late: one heal, and the next poll (0.5 s on) is not a second.
        assert_eq!(stats::regen_tick(14_000, 6_000, 5_000), (true, 19_000));
        assert_eq!(stats::regen_tick(14_500, 19_000, 5_000), (false, 19_000), "no burst");
        // ★ 150차 review — a load forgets the character who left.
        stats::WITCH_UNTIL_MS.store(9_000, core::sync::atomic::Ordering::Relaxed);
        stats::HIT_UNTIL_MS.store(7_000, core::sync::atomic::Ordering::Relaxed);
        stats::set_player_hp(45_000, 32_767);
        stats::PLAYER_HP_MAX.store(50_000, core::sync::atomic::Ordering::Relaxed);
        // 160차: a published wide stat is the leaving character's too.
        stats::set_player_wide(stats::WIDE_MDEF, 40_000);
        assert_eq!(stats::player_wide(stats::WIDE_MDEF, 32_767), 40_000);
        assert_eq!(stats::forget_character(), (9_000, 7_000));
        assert_eq!(stats::player_wide(stats::WIDE_MDEF, 32_767), 32_767, "the old character's defence is not borrowed");
        assert_eq!(stats::PLAYER_HP_SEEN.load(core::sync::atomic::Ordering::Relaxed), u32::MAX);
        assert_eq!(stats::hp_true_current(32_767, 60_000, 0, u32::MAX), 60_000, "a full gauge is a full max again");
        assert_eq!(stats::hp_true_max(32_767, 0), 32_767, "the old character's max is not borrowed");
        assert!(stats::acc_effects().is_none());
        assert_eq!(stats::regen_tick(1_000, 90_000, 5_000), (false, 6_000), "the clock went back");
    }

    /// The guest frame-timer ceiling (`stats::clamp_timer_ms`). It must be OFF unless a
    /// front end asks for it — every non-Hero3 target shares this code path — and it
    /// must never LENGTHEN an interval, only shorten one.
    #[test]
    fn timer_clamp_is_off_by_default_and_only_shortens() {
        use super::stats;

        stats::set_timer_clamp_ms(0);
        let _ = stats::timer_clamp_take();
        for ms in [1, 20, 50, 244, 1000] {
            assert_eq!(stats::clamp_timer_ms(ms), ms, "a zero cap must pass {ms} through");
        }
        assert_eq!(stats::timer_clamp_take().0, 0, "nothing was clamped");

        stats::set_timer_clamp_ms(50);
        assert_eq!(stats::clamp_timer_ms(244), 50, "the modal interval is the point of this");
        assert_eq!(stats::clamp_timer_ms(50), 50, "a request AT the cap is untouched");
        assert_eq!(stats::clamp_timer_ms(20), 20, "a faster request is never slowed down");
        let (clamped, cap) = stats::timer_clamp_take();
        assert_eq!((clamped, cap), (1, 50), "only the 244 counts as a clamp");
        assert_eq!(stats::timer_clamp_take().0, 0, "the counter resets, the cap does not");

        stats::set_timer_clamp_ms(0);
    }

    /// ★ The regression vc60 shipped: a flat cap rewrote the player's 게임속도.
    /// Every interval the option can produce (`110 - 10*speed`, speed 1..10, measured on
    /// device 2026-08-14) must survive the clamp untouched, and the modal 244 must not.
    #[test]
    fn timer_clamp_never_touches_the_game_speed_setting() {
        use super::stats;

        stats::set_timer_clamp_ms(50);
        let _ = stats::timer_clamp_take();
        for speed in 1..=10u64 {
            let ms = 110 - 10 * speed; // 100 (slowest) .. 10 (fastest)
            assert!(ms < stats::TIMER_CLAMP_FLOOR_MS, "speed {speed} = {ms} ms must sit below the floor");
            assert_eq!(
                stats::clamp_timer_ms(ms),
                ms,
                "게임속도 {speed} ({ms} ms) must reach the guest unchanged — vc60 halved 1..5"
            );
        }
        assert_eq!(stats::timer_clamp_take().0, 0, "no speed setting may count as a clamp");

        // …and the thing the clamp exists for is still clamped.
        assert_eq!(stats::clamp_timer_ms(244), 50, "the modal 244 ms is above the floor");
        assert_eq!(stats::timer_clamp_take().0, 1);

        // The floor is a floor, not a window: anything between it and the cap still gets cut.
        assert_eq!(stats::clamp_timer_ms(stats::TIMER_CLAMP_FLOOR_MS), 50);
        // One millisecond below it is the player's territory and is left alone.
        assert_eq!(stats::clamp_timer_ms(stats::TIMER_CLAMP_FLOOR_MS - 1), stats::TIMER_CLAMP_FLOOR_MS - 1);

        stats::set_timer_clamp_ms(0);
    }

    /// ★ 153차 — the portrait capture's run, blit by blit, the way the blitters feed it:
    /// the player's shadow opens it (and counts as a drawing of the player), the pieces
    /// wholly inside the window are taken, the NEXT actor's shadow ends it, and a blended
    /// piece marks the picture. (The only test touching these statics.)
    #[test]
    fn the_portrait_run_is_the_players_own_pieces() {
        use super::stats::{self, CAPTURE_RUN, PORTRAIT_W, SHADOW_H, SHADOW_W};

        // The measured standing player: corner (176, 81), window origin (160, 37), shadow (171, 75).
        let (origin, anchor) = ((160, 37), (171, 75));
        stats::set_portrait_window(Some((origin, anchor)));
        let (seq0, _, _) = stats::portrait_run_state();
        // Before the shadow nothing is taken (a tile under the player).
        assert_eq!(stats::portrait_takes(176, 63, 9, 11, false), None);
        // The shadow: opens the run, is not copied itself, bumps the count.
        assert_eq!(stats::portrait_takes(171, 75, SHADOW_W, SHADOW_H, true), None);
        let (seq1, left, blended) = stats::portrait_run_state();
        assert_eq!((seq1, left, blended), (seq0 + 1, CAPTURE_RUN, false));
        // The pieces: inside → taken; a piece hanging off the window → counted, not taken.
        assert_eq!(stats::portrait_takes(176, 63, 9, 11, false), Some(origin));
        assert_eq!(stats::portrait_takes(176, 42, 18, 26, false), Some(origin), "the helmet fits the 153차 window");
        assert_eq!(stats::portrait_takes(origin.0 + PORTRAIT_W as i32 - 4, 60, 8, 8, false), None);
        assert_eq!(stats::portrait_run_state().1, CAPTURE_RUN - 3);
        // The next actor's shadow ends the run — its own pieces, even inside the window, are not taken.
        assert_eq!(stats::portrait_takes(171, 91, SHADOW_W, SHADOW_H, true), None);
        assert_eq!(stats::portrait_run_state().1, 0);
        assert_eq!(stats::portrait_takes(176, 80, 16, 16, false), None);
        assert!(!stats::portrait_run_state().2, "nothing blended was taken");

        // A drawing of the player see-through: the blended piece is taken and marks it.
        stats::portrait_takes(171, 75, SHADOW_W, SHADOW_H, true);
        assert_eq!(stats::portrait_takes(176, 48, 18, 21, true), Some(origin));
        assert!(stats::portrait_run_state().2, "a blended piece marks the picture");
        // …and the next drawing starts clean.
        stats::portrait_takes(171, 75, SHADOW_W, SHADOW_H, true);
        assert!(!stats::portrait_run_state().2);
        // A shadow-SIZED opaque piece is a piece, not an actor: it does not end the run.
        assert_eq!(stats::portrait_takes(170, 60, SHADOW_W, SHADOW_H, false), Some(origin));
        assert_eq!(stats::portrait_run_state().1, CAPTURE_RUN - 1);

        stats::set_portrait_window(None);
        assert_eq!(stats::portrait_takes(176, 63, 9, 11, false), None, "disarmed takes nothing");
    }
}

pub mod gameplay;
