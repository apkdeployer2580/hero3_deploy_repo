#![allow(dead_code)]
extern crate self as log;
extern crate self as wie_backend;
#[macro_export]
macro_rules! warn { ($($arg:tt)*) => { eprintln!($($arg)*) }; }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    UP,
    DOWN,
    LEFT,
    RIGHT,
    OK,
    CLEAR,
}
#[derive(Clone, Copy)]
pub enum StickMode {
    Float,
    Dpad,
    Pinned,
}
impl StickMode {
    fn next(self) -> Self {
        match self {
            Self::Float => Self::Dpad,
            Self::Dpad => Self::Pinned,
            Self::Pinned => Self::Float,
        }
    }
}
fn stick_mode() -> StickMode {
    StickMode::Float
}
fn set_stick_mode(_: StickMode) {}
mod config {
    pub fn write_stick_mode(_: &std::path::Path, _: crate::StickMode) {}
}
mod menutouch {
    include!("wie_android/src/menutouch.rs");
}
mod gamefont_data {
    include!("wie_android/src/gamefont_data.rs");
}
mod gamefont {
    include!("wie_android/src/gamefont.rs");
}
mod game_options {
    include!("wie_android/src/game_options.rs");
}
mod gameplay {
    include!("vendor/wie/wie_util/src/gameplay.rs");
}
