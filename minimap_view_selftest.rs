// Standalone self-test of hero2's hero5-style minimap VIEW (fixed zoom, follows the
// player, walls not drawn, exits as blue bars).
//
//   rustc --edition 2021 --test minimap_view_selftest.rs -o t.exe && ./t.exe
//
// `include!`s the REAL source — the file `overlay.rs` and `wie_probe` splice in. The tests
// live in `minimap_view.rs` under `#[cfg(test)]`; a `use crate::…` there breaks this build,
// which is the intended alarm (it must stay dependency-free for the probe).

include!("wie_android/src/minimap_view.rs");
