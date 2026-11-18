// Standalone self-test of the host-owned quickbar PAGE store.
//
//   rustc --edition 2021 --test quickpage_selftest.rs -o t.exe && ./t.exe
//
// `cargo test -p wie_android` cannot run on the Windows host (the linked
// winit/android deps abort at startup), so this compiles the REAL source — not a
// copy — as its own test crate. `quickpage.rs` is dependency-free for exactly this
// reason; if it ever grows a `use crate::...` this file stops building, which is the
// intended alarm.

include!("wie_android/src/quickpage.rs");
