//! Installing updates by itself: asks the update server (the GitHub release's `latest.json`)
//! for a newer signed version, downloads it in the background, and installs it when the
//! user chooses "Restart to update". (Port of the "auto" way in the Electron app's
//! `src/updater.js`, with Tauri's updater plugin in the place of electron-updater.)

use f1_island::updater::{State, Status, Updater};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

/// A downloaded update, waiting for the user to restart.
#[derive(Default)]
pub struct Pending(Mutex<Option<(Update, Vec<u8>)>>);

fn publish(app: &AppHandle, state: State) {
    app.state::<Updater>().set(state.clone());
    let _ = app.emit("update:state", state);
    crate::menus::refresh_tray_menu(app);
}

fn busy(app: &AppHandle) -> bool {
    matches!(
        app.state::<Updater>().state().status,
        Status::Downloading | Status::Ready
    )
}

/// Looks for an update; if there is one, downloads it. Offline, no release yet, a bad
/// signature: log it and try again at the next check.
pub async fn check(app: &AppHandle) {
    if busy(app) {
        return;
    }
    let now_ms = chrono::Utc::now().timestamp_millis();
    // For trying the update out against a server of one's own.
    let builder = app.updater_builder();
    let builder = match std::env::var("F1_ISLAND_UPDATE_URL")
        .ok()
        .and_then(|u| u.parse().ok())
    {
        Some(url) => match builder.endpoints(vec![url]) {
            Ok(b) => b,
            Err(e) => return eprintln!("update check failed: {e}"),
        },
        None => builder,
    };
    let updater = match builder.build() {
        Ok(u) => u,
        Err(e) => return eprintln!("update check failed: {e}"),
    };

    match updater.check().await {
        Ok(Some(update)) => {
            let version = update.version.clone();
            publish(
                app,
                State {
                    status: Status::Downloading,
                    version: Some(version.clone()),
                    url: None,
                    checked_at: now_ms,
                },
            );
            match update.download(|_, _| {}, || {}).await {
                Ok(bytes) => {
                    *app.state::<Pending>().0.lock().unwrap() = Some((update, bytes));
                    publish(
                        app,
                        State {
                            status: Status::Ready,
                            version: Some(version),
                            url: None,
                            checked_at: now_ms,
                        },
                    );
                }
                Err(e) => {
                    eprintln!("update download failed: {e}");
                    publish(
                        app,
                        State {
                            checked_at: now_ms,
                            ..State::default()
                        },
                    );
                }
            }
        }
        Ok(None) => publish(
            app,
            State {
                checked_at: now_ms,
                ..State::default()
            },
        ),
        Err(e) => eprintln!("update check failed: {e}"),
    }
}

/// The user chose "Restart to update": install it (the installer closes the app and starts
/// the new one).
pub fn install(app: &AppHandle) {
    let Some((update, bytes)) = app.state::<Pending>().0.lock().unwrap().take() else {
        return;
    };
    if let Err(e) = update.install(bytes) {
        eprintln!("update install failed: {e}");
        return;
    }
    // On Windows the installer ends the process; elsewhere restart by hand.
    app.restart();
}
