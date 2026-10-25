//! The parts of F1 Island that need no window: plain logic you can test on its own.
//! Each module is a port of a piece of the Electron app (`src/*.js`, `renderer/*.js`).
//!
//! Run the tests with `cargo test --lib` from `src-tauri/`.

pub mod alerts;
pub mod analysis;
pub mod analysis_data;
pub mod championship;
pub mod circuit;
pub mod clock;
pub mod demo;
pub mod dock;
pub mod ergast;
pub mod geometry;
pub mod jolpica;
pub mod live_client;
pub mod live_state;
pub mod live_tab;
pub mod live_window;
pub mod merge;
pub mod news;
pub mod notch;
pub mod openf1;
pub mod platform;
pub mod results;
pub mod schedule;
pub mod season;
pub mod settings;
pub mod snapshot;
pub mod standings;
pub mod status_log;
pub mod title;
pub mod updater;
pub mod version;

/// Chooses the TLS (secure connection) engine for the whole program. Two engines are
/// compiled in (the updater brings one, the HTTP client the other), and the live-timing
/// socket cannot pick between them by itself: without this it panics when it connects.
pub fn install_tls_provider() {
    // Already installed (by a test, say): fine.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}
