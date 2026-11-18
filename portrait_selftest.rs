// Standalone self-test of the PORTRAIT rules (141차 · 153차): where the capture window
// goes, the game's scroll (a small map centred), which square of it is the face, which
// frame may be taken, and when a picture may replace the one in the ring.
//
//   rustc --edition 2021 --test portrait_selftest.rs -o t.exe && ./t.exe
//
// `include!`s the REAL source — the file `items.rs` and `wie_probe` splice in.

include!("wie_android/src/portrait.rs");
