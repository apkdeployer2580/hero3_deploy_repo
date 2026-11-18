// Standalone self-test of the host-driven save chain (quick-save + autosave).
//
//   rustc --edition 2021 --test save_selftest.rs -o tt_save.exe && ./tt_save.exe
//
// ★ `include!`s the REAL source, so the guest addresses, the scratch location and the
// step order under test are exactly the ones the app issues. `wie_android/src/save.rs`
// must therefore stay dependency-free (same contract as endpoint.rs / tls_buf.rs /
// minimap_render.rs / buffs.rs; see CLAUDE.md).
//
// Worth testing without a device because every step here writes to the player's SAVE:
// a wrong length, a stale buffer pointer or a step issued out of order does not draw
// something ugly, it produces a corrupt record — and the game's loader is the only
// thing that would ever tell us.

include!("wie_android/src/save.rs");
