//! APK flavour (`BRANCHES.md`): `main` (개정판) or `vanilla` (편의성 개선 원본).
//!
//! `android_app/flavor.properties` `flavor=vanilla` → `build_android.ps1` → rustc
//! `--cfg hero3_vanilla`. That is the ONLY source; nothing at runtime can flip it, and no
//! file on the phone can either. Every host feature that differs between the flavours
//! asks one of the constants below BY NAME, so a gate reads as intent (`if flavor::TOWER`)
//! and this file is the complete list of what vanilla lacks.
//!
//! ★★ Player decision 2026-09-19 (114차) — vanilla is main minus exactly three things:
//!   1. **하늘성: no entrance.** The host never mirrors the difficulty into the guest's
//!      HARD flag (`items::GUARDIAN_FLAG_HARD`; on vanilla the mirror writes 0, which also
//!      erases a flag a main save carried in), so the two entrance NPCs (엔자크 행인 ·
//!      네오 솔티아 로빈) fall through to the ORIGINAL block of their condition container on
//!      normal and hard alike. The floor table and the `wie_guardian.txt` lever (which
//!      carries the `hard 1` testing knob) are ignored, so `guardian::Run` stays `Idle`
//!      for the life of the process and the ENTER flag can never start a run.
//!   2. **New item drops off.** The reaver / 검은 눈물 / 타이탄의 파편 hook is not installed
//!      (`wie_core_arm` `hook.rs`). Items already in a save load, equip and show — the
//!      item TABLES are shared by design (77차-B).
//!   3. **Level cap 70.** The five 111차 guest patches carry `flavor = "main"` in
//!      `binary_patches.toml` and are not installed, so `doLevelUp` is byte-original.
//!      The host ceilings (`items::LEVEL_CAP` 99 …) stay — looser than the guest's clamp
//!      is fine, tighter would read a Lv 71+ character from main as "no character".
//!   4. **The 개정판 balance patches (163차, player decision 2026-10-05).** *"바닐라에서는
//!      쿨감 제한 적용되면안돼. 신규템으로 인한 밸런스 관련 패치했던거 적용되면 안되는거야."*
//!      Guest side these are `flavor = "main"` hooks/patches in `binary_patches.toml`
//!      (명중 255 · 쿨타임 감소 70 % · 방어% 500 · 공격력/방어력 i16 해제 · 공격속도 ·
//!      장신구 3번째 옵션 · the damage caps 9,999,999 / 32,767 — vanilla keeps the game's
//!      9,999). The ORIGINAL orbs reach several of them (폭풍 쿨타임 +90 %, 사신 명중 +120 %,
//!      금강/정령 방어 +225 %, 직격 방패무시 +90 % per full item), so they were not inert
//!      on vanilla. Host side: [`REBALANCE`] (the cooldown ring's cap, 방패무시).
//! Everything else — the three bags, the damage folds (a hit past 32767 clamps instead of
//! reading 1), 흡수 on big hits, the HP gauge (inert there: vanilla's max HP stays far below
//! 32767), 제련 odds, touch UI, save vault, the in-app warehouse and cash shop — is identical
//! on both flavours.
//!
//! ⛔ Do not read `cfg!(hero3_vanilla)` anywhere else in this crate; name the feature here.

/// This build is the vanilla flavour.
pub const VANILLA: bool = cfg!(hero3_vanilla);

/// The 하늘성 (guardian tower) exists on this flavour: floor table, control file, HARD
/// mirror, ENTER poll, entrance-confirm cursor reset (`emulator.rs`).
pub const TOWER: bool = !VANILLA;

/// 헬모드 (133차) exists on this flavour: the ending-edge promotion (difficulty byte
/// 1 → 2), the all-scene monster fork, and — via the reaver hook, which vanilla never
/// installs — the legacy drops. Content, like the tower (vanilla keeps its NG+ loop
/// exactly as shipped: hard forever).
pub const HELL: bool = !VANILLA;

/// The 개정판 balance patches exist on this flavour (163차): the host's cooldown ring caps
/// the 쿨타임 감소 sum at `wie_util::stats::COOLDOWN_REDUCE_CAP_PCT` like the main-only guest
/// hook, and 방패무시 (option id 0x12 — 직격의 오브 is an ORIGINAL orb) actually pierces.
/// Vanilla keeps the game's own rules: no cap, and 방패무시 summed but never read.
pub const REBALANCE: bool = !VANILLA;

/// The one boot line that says which flavour a logcat came from. `versionCode` alone
/// cannot — both flavours share the package and one versionCode sequence (BRANCHES.md).
pub fn boot_line() -> &'static str {
    if VANILLA {
        "flavor: vanilla — 하늘성 OFF (entrance NPCs keep the shipped dialogue on hard too; a pushed floor table is ignored) · reaver/boss-accessory drop hook OFF · level cap 70 (the five 111차 patches are main-only) · 개정판 balance OFF (명중 255 · 쿨감 60 % · 방어% 500 · i16 해제 · 공속 · 3번째 옵션 · 방패무시 · damage cap stays 9,999); the three bags, 흡수 and everything else are main's"
    } else {
        "flavor: main — 하늘성 on · reaver/boss-accessory drops on · level cap 99 · 헬모드 on (hardcore clear -> difficulty 2) · 개정판 balance on"
    }
}
