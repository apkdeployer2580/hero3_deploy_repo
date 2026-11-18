// Standalone self-test of hero2's STATUS HUD (the hero5-style portrait ring · HP/SP bars ·
// level plate at the top-left, and the EXP bar at the bottom centre).
//
//   rustc --edition 2021 --test hud_view_selftest.rs -o t.exe && ./t.exe
//
// `include!`s the REAL source — the file `overlay.rs` and `wie_probe` splice in. The tests
// live in `hud_view.rs` under `#[cfg(test)]`; a `use crate::…` there breaks this build,
// which is the intended alarm (it must stay dependency-free for the probe).

include!("wie_android/src/hud_view.rs");
