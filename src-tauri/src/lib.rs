//! The parts of F1 Island that need no window: plain logic you can test on its own.
//! Each module is a port of a piece of the Electron app (`src/*.js`, `renderer/*.js`).
//!
//! Run the tests with `cargo test --lib` from `src-tauri/`.

pub mod circuit;
pub mod clock;
pub mod dock;
pub mod ergast;
pub mod jolpica;
pub mod live_tab;
pub mod schedule;
pub mod standings;
pub mod status_log;
pub mod version;
