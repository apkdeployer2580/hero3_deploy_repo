#![allow(dead_code)]

extern crate self as wie_backend;

#[path = "../../../vendor/wie/wie_backend/src/audio_sink.rs"]
mod backend;
pub use backend::{AudioSink, volume_gain};

#[path = "../../../wie_android/src/audio_mix.rs"]
pub mod audio_mix;
#[path = "../../../wie_android/src/audio_quality.rs"]
pub mod audio_quality;
#[path = "../../../wie_android/src/audio_sink.rs"]
mod audio_sink;

pub fn sfx_level() -> u8 {
    audio_mix::SFX_LEVEL_DEFAULT
}
