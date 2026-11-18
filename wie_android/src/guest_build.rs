//! Which carrier build of the game this process booted, and the few guest anchors
//! that differ between builds.
//!
//! Every guest ADDRESS in this crate (`save::FN_SERIALISE`, the quickbind sites,
//! the HUD/EXP/level-cap patch sites, …) was reverse engineered from the KTF
//! `client.bin` (md5 `3792a476…`, Ver 01.00.03). The LGT `binary.mod` (Ver 01.00.07)
//! is a DIFFERENT compile — no byte-level overlap, different addresses, different
//! statics model (absolute instead of RWPI `sl`) — so on a non-KTF boot those
//! addresses point at unrelated LGT code/data.
//!
//! Two things are now separated, because they stopped being the same question:
//!
//! * [`char_anchor`] — WHERE the character record is. Known for both builds
//!   (2026-08-10, 4차): KTF reaches it through a pointer global, LGT's record IS a
//!   `.bss` static. Everything read-only (HUD gauges, quickbar scan, window/state
//!   gating, minimap) hangs off this and now works on LGT.
//! * [`guest_code_addresses_valid`] — whether the reverse-engineered CODE addresses
//!   are the ones running. Only KTF, and it stays that way until each LGT routine is
//!   found. It gates every guest CALL (`call_guest_function` at a stale address jumps
//!   into the middle of an unrelated routine and corrupts the guest) and every guest
//!   WRITE, including writes into the character record at offsets not yet verified on
//!   LGT. The byte-validation guards in front of the code patches are probabilistic;
//!   this flag is the airtight version.
//!
//! A third, [`container_data_off`], is neither: it is the carrier RUNTIME's pointer
//! convention, so it is known for every build the moment the build is known.
//!
//! The blit565 sprite-blitter perf patch is NOT gated here: it is keyed to the md5 of
//! the loaded binary in `vendor/wie/data/binary_patches.toml` and already has
//! verified LGT entries (`blit565_lgt`).
//!
//! The anchor TABLE itself lives in `guest_anchors.rs`, which is dependency-free so
//! that `wie_probe` can `include!` the shipping numbers instead of keeping a copy
//! (MIGRATION_LGT.md §3.4). Only the "which build booted" atomic is here.

use std::sync::atomic::{AtomicU8, Ordering};

// GuestBuild / CharAnchor and every per-build value, shared verbatim with wie_probe.
include!("guest_anchors.rs");

const UNKNOWN: u8 = 0;
const KTF: u8 = 1;
const LGT: u8 = 2;
const SKT: u8 = 3;

static KIND: AtomicU8 = AtomicU8::new(UNKNOWN);

pub fn set_ktf() {
    KIND.store(KTF, Ordering::Relaxed);
}

pub fn set_lgt() {
    KIND.store(LGT, Ordering::Relaxed);
}

pub fn set_skt() {
    KIND.store(SKT, Ordering::Relaxed);
}

/// The build that booted, or `None` before the archive has been identified.
pub fn kind() -> Option<GuestBuild> {
    match KIND.load(Ordering::Relaxed) {
        KTF => Some(GuestBuild::Ktf),
        LGT => Some(GuestBuild::Lgt),
        SKT => Some(GuestBuild::Skt),
        _ => None,
    }
}

/// True only when the KTF build (the one all guest CODE addresses were lifted from)
/// is the one running. Every guest call and every guest write asks this first.
pub fn guest_code_addresses_valid() -> bool {
    KIND.load(Ordering::Relaxed) == KTF
}

pub fn char_anchor() -> Option<CharAnchor> {
    kind()?.char_anchor()
}

pub fn state_off() -> u32 {
    kind().map_or(0x7c, GuestBuild::state_off)
}

/// CHAR offset of the online module's screen word, when located — see
/// [`GuestBuild::shell_screen_off`]. `None` disables the creation-screen save pin (76차).
pub fn shell_screen_off() -> Option<u32> {
    kind().and_then(GuestBuild::shell_screen_off)
}

/// The slot list's label cells, when located — see [`GuestBuild::slot_label_base`].
/// `None` keeps the save-slot window (130차) at its identity.
pub fn slot_label_base() -> Option<u32> {
    kind().and_then(GuestBuild::slot_label_base)
}

/// `*(handle) + off` vs. a direct pointer, for every field holding a WIPI allocation.
/// Defaults to the KTF shape before the build is known; each caller validates its
/// hops anyway, so an unresolvable chain yields `None` rather than a bad read.
pub fn container_data_off() -> Option<u32> {
    kind().map_or(Some(8), GuestBuild::container_data_off)
}

/// The minimap's blend-suppression global, when located on this build.
pub fn minimap_noblend_global() -> Option<u32> {
    kind().and_then(GuestBuild::minimap_noblend_global)
}

/// May the host read the smith's 제련 session? See [`GuestBuild::refine_odds_ok`].
/// `false` before the build is known, so the bar cannot flash during boot.
pub fn refine_odds_ok() -> bool {
    kind().is_some_and(GuestBuild::refine_odds_ok)
}

/// 메뉴 터치화 (66차): the per-screen cursor bytes, when measured on this build. `None`
/// gates menu taps off — the badge taps do not go through here at all.
/// See [`GuestBuild::menu_cursors`].
pub fn menu_cursors() -> Option<MenuCursors> {
    kind().and_then(GuestBuild::menu_cursors)
}

/// The game's own derived-field refresh helpers, per build. `None` turns the whole
/// in-game 오브결합 / 소켓확장 off — including the popup row that offers it, because a
/// build that cannot refresh the cache must not write the bytes either.
/// See [`ItemRefresh`].
pub fn item_refresh() -> Option<ItemRefresh> {
    kind()?.item_refresh()
}

/// The game's `0x52b14(cat, idx, -1, 0)`, when located. See [`GuestBuild::give_item_fn`].
pub fn give_item_fn() -> Option<u32> {
    kind()?.give_item_fn()
}

/// `ComputeMaxHP` (142차-C 마녀의 생명력). `None` = the buff only reaches the max at the
/// game's own recomputes (level-up, equip) on this build.
pub fn compute_max_hp_fn() -> Option<u32> {
    kind()?.compute_max_hp_fn()
}

/// `addHP` (150차 레아의목걸이's 5-second heal). `None` = no host heal on this build.
/// See [`GuestBuild::add_hp_fn`] for why a host call's delta is true HP.
pub fn add_hp_fn() -> Option<u32> {
    kind()?.add_hp_fn()
}

/// `useItem(tab, slot)` (164차 — a potion pressed during a skill cast). `None` = the host
/// never calls it on this build. See [`GuestBuild::use_item_fn`].
pub fn use_item_fn() -> Option<u32> {
    kind()?.use_item_fn()
}

/// The paper-doll layer setter (167차 아바타 옷장 — re-dress one layer). See
/// [`GuestBuild::layer_set_fn`].
pub fn layer_set_fn() -> Option<u32> {
    kind()?.layer_set_fn()
}

/// The EQUIPMENT page's mode byte (0 = worn cells, 1 = candidate list). See
/// [`GuestBuild::equip_mode_addr`].
pub fn equip_mode_addr() -> Option<u32> {
    kind()?.equip_mode_addr()
}

/// The `.bss` word holding the string table's base (150차 — the 상세보기 name box's orb
/// label). See [`GuestBuild::string_table_ptr`].
pub fn string_table_ptr() -> Option<u32> {
    kind()?.string_table_ptr()
}

/// Where the popup's per-row label allocations are listed, per build. Read-only, and
/// the single gate every cash buff has to pass — see [`GuestBuild::popup_handles`].
pub fn popup_handles() -> Option<PopupHandles> {
    kind()?.popup_handles()
}

/// How to make the game take a popup row on this build. `None` = the 퀵슬롯등록 rewrite
/// stays off, because there would be no way to commit what it offers.
pub fn popup_commit() -> Option<PopupCommit> {
    kind()?.popup_commit()
}

/// The inventory page's pre-emptor bytes to check before treating the ITEM popup as
/// live, per build. Empty before the build is known — harmless, because `popup_view`
/// cannot resolve CHAR then either. See [`GuestBuild::popup_preempt_offsets`] for why
/// only the build whose commit bypasses the game's key router carries any.
pub fn popup_preempt_offsets() -> &'static [u32] {
    kind().map_or(&[], GuestBuild::popup_preempt_offsets)
}

/// Where the auto-loot flag lives on this build, or `None` where it has not been located.
/// It is an ARRAY indexed by the save slot, and the value is a bit-7 flag, not a boolean —
/// see [`GuestBuild::autoloot_flag`] and [`AutolootFlag`].
pub fn autoloot_flag() -> Option<AutolootFlag> {
    kind()?.autoloot_flag()
}

/// `(site, original, branch)` for the game's own bottom-HUD draw, per build.
///
/// ★ This is the first guest CODE patch to be split PER ITEM rather than hidden behind
/// [`guest_code_addresses_valid`], which is the discipline MIGRATION_LGT.md §3.6 asks
/// for: the flag stays false on LGT so the routines still unported cannot fire, and
/// each item that gets reverse engineered moves into a table like this one instead of
/// loosening the flag for everything at once.
pub fn give_item_count_patch() -> Option<(u32, &'static [u8], &'static [u8])> {
    kind()?.give_item_count_patch()
}

pub fn hud_bottom_patch() -> Option<(u32, u16, u16)> {
    kind()?.hud_bottom_patch()
}

/// The buff band's three x immediates, per build. Empty on a build where they have not
/// been located, which is what leaves the band where the game puts it. Same per-item
/// split as [`hud_bottom_patch`].
pub fn band_x_sites() -> &'static [(u32, u16, u8)] {
    kind().map_or(&[], GuestBuild::band_x_sites)
}

/// The target-monster box's x immediates, per build. Empty = not located.
pub fn target_box_x_sites() -> &'static [(u32, u16, i32)] {
    kind().map_or(&[], GuestBuild::target_box_x_sites)
}

/// The scene / warp / live-monster table, when it has been measured on this build.
/// `None` on KTF and SKT, which is what keeps the tower off those builds entirely
/// rather than letting it write plausible-looking offsets into them.
pub fn scene_world() -> Option<SceneWorld> {
    kind()?.scene_world()
}

/// May the host write the five quickbar arrays on this build? See
/// [`GuestBuild::quickbar_writes_ok`] for what "verified" means for each.
pub fn quickbar_writes_ok() -> bool {
    kind().is_some_and(GuestBuild::quickbar_writes_ok)
}

/// The game's own `skill_bind`, when located on this build. `None` puts a page restore on
/// the raw-write path instead of disabling it — see [`GuestBuild::skill_bind_fn`].
pub fn skill_bind_fn() -> Option<u32> {
    kind()?.skill_bind_fn()
}

/// Address of the kill-EXP divisor (`movs r1,#100`), per build. Same per-item split as
/// [`hud_bottom_patch`].
pub fn exp_divisor_site() -> Option<u32> {
    kind()?.exp_divisor_site()
}

/// The game's own `checkLevelUp` (thumb bit included), per build — the guest call the
/// 천상의 영약 cash item makes. Same per-item split as [`hud_bottom_patch`].
pub fn level_up_fn() -> Option<u32> {
    kind()?.level_up_fn()
}

/// The network-warehouse deposit grade gate `(site, orig, patched)`, per build. Same
/// per-item split as [`hud_bottom_patch`]. Un-gates 영웅-이상 (cash) equipment deposit.
pub fn warehouse_grade_gate() -> Option<(u32, u16, u16)> {
    kind()?.warehouse_grade_gate()
}

/// Where and HOW to relax the equip-level check, per build. Same per-item split as
/// [`hud_bottom_patch`]. The LGT site is located but deliberately ships `None` until
/// its verification run — see the table entry for the rule this enforces.
pub fn equip_relief_site() -> Option<EquipReliefSite> {
    kind()?.equip_relief_site()
}

/// The game's own "연결이 끊어졌습니다" modal, per build. `None` = the host can only toast,
/// and the player stays where they are — see [`NetDisconnectModal`] for why nothing
/// simpler works. Same per-item split as [`hud_bottom_patch`].
pub fn net_disconnect_modal() -> Option<NetDisconnectModal> {
    kind()?.net_disconnect_modal()
}

/// The full per-build save chain, when ported — see [`GuestBuild::save_chain`] for the
/// addresses and each build's scratch proof. Same per-item split as [`hud_bottom_patch`].
pub fn save_chain() -> Option<SaveChain> {
    kind()?.save_chain()
}

/// The chain converted to `save::Chain` — the mirror `save.rs` can name while staying
/// dependency-free for `save_selftest.rs`. One conversion point, here, so the two
/// structs cannot drift apart in more than one place.
pub fn save_chain_fns() -> Option<crate::save::Chain> {
    let c = save_chain()?;
    Some(crate::save::Chain {
        fn_serialise: c.fn_serialise,
        fn_slot_summary: c.fn_slot_summary,
        fn_write_record: c.fn_write_record,
        scratch: c.scratch,
    })
}

/// Can the host drive the game's own save primitives on the build that booted?
///
/// ★ DIAGNOSTIC ONLY — it must not hide or disable anything (2026-08-11). An earlier
/// pass this same day used it to remove the SAVE control on LGT, and that was the wrong
/// trade: a feature that already shipped does not get taken away because a carrier port
/// is unfinished. The fix is to finish MIGRATION_LGT.md §4-3.
///
/// It exists because a refusal has two very different causes and a device log has to
/// distinguish them: "the place forbids saving" (the game's own AREA byte) versus "this
/// build's save chain was reverse engineered from a different compile". Without the
/// second one the symptom is only `save: queued Manual retry expired unfulfilled`,
/// which reads like the first.
///
/// §4-3 got its per-build table on 2026-08-11 (9차): this now follows
/// [`GuestBuild::save_chain`], so the LGT port turning real is what flipped it — not a
/// loosened [`guest_code_addresses_valid`].
pub fn save_chain_supported() -> bool {
    save_chain().is_some()
}

pub fn name() -> &'static str {
    kind().map_or("unknown", GuestBuild::name)
}
