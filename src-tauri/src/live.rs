//! The app's side of live timing: decides when to be connected, runs the connection, keeps
//! the latest state, logs status changes, and passes the state on to the page.
//! (Port of the live-timing parts of the Electron app's `main.js`.)
//!
//! Three tasks cooperate here, each an `async` function running on the side:
//! - `supervise` wakes up every minute and starts or stops the connection;
//! - `f1_island::live_client::run` is the connection itself (a library module);
//! - `consume` receives its events and acts on them.

use f1_island::jolpica::Jolpica;
use f1_island::live_client::{self, LiveEvent};
use f1_island::live_state::{Event, Summary};
use f1_island::live_tab::{LiveTab, SessionStatus};
use f1_island::live_window::should_connect;
use f1_island::schedule::Weekend;
use f1_island::status_log::{self, Entry};
use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, watch};

const CHECK_EVERY: Duration = Duration::from_secs(60);

/// Shared state, handed to Tauri with `.manage(...)`. Several tasks and commands read and
/// write it, so each part sits in its own `Mutex`.
#[derive(Default)]
pub struct Live {
    /// Set while a connection is running; sending `true` through it stops the connection.
    stop: Mutex<Option<watch::Sender<bool>>>,
    /// The newest summary of the session, or `None` when not connected.
    latest: Mutex<Option<Summary>>,
    /// Decides whether the Live tab is the main one.
    tab: Mutex<LiveTab>,
    /// This weekend's schedule, to know when each session was due to end.
    weekend: Mutex<Option<Weekend>>,
}

/// What the page receives: the summary, and whether Live should be the main tab.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LivePayload {
    summary: Option<Summary>,
    main: bool,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl Live {
    pub fn payload(&self) -> LivePayload {
        let summary = self.latest.lock().unwrap().clone();
        // "Connected" means there is something to show: a running order or messages.
        let connected = summary
            .as_ref()
            .is_some_and(|s| !s.order.is_empty() || !s.messages.is_empty());
        let main = self.tab.lock().unwrap().is_main(connected, now_ms());
        LivePayload { summary, main }
    }

    fn is_running(&self) -> bool {
        self.latest
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|s| s.status.as_deref())
            .is_some_and(|status| SessionStatus::parse(status).is_running())
    }
}

/// The page asks for the current state when the island opens.
#[tauri::command]
pub fn live_state(live: tauri::State<'_, Live>) -> LivePayload {
    live.payload()
}

/// Runs for the life of the app: every minute, decides whether we should be connected.
pub async fn supervise(app: AppHandle) {
    // For trying it out between sessions: `F1_ISLAND_LIVE=1` connects regardless of the schedule.
    let forced = std::env::var("F1_ISLAND_LIVE").is_ok_and(|v| v == "1");

    loop {
        let jolpica = app.state::<Jolpica>();
        // If the schedule can't be had right now, leave things as they are.
        if let Ok(weekend) = jolpica.weekend(now_ms()).await {
            let live = app.state::<Live>();
            let running = live.is_running();
            let wanted = forced
                || weekend
                    .as_ref()
                    .is_some_and(|w| should_connect(&w.sessions, now_ms(), running))
                || running;
            *live.weekend.lock().unwrap() = weekend;
            if wanted {
                start(&app);
            } else {
                stop(&app);
            }
        }
        tokio::time::sleep(CHECK_EVERY).await;
    }
}

fn start(app: &AppHandle) {
    let live = app.state::<Live>();
    let mut stop_slot = live.stop.lock().unwrap();
    if stop_slot.is_some() {
        return; // already connected
    }
    let (stop_tx, stop_rx) = watch::channel(false);
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    *stop_slot = Some(stop_tx);
    drop(stop_slot); // release the lock before doing more

    log(app, "connect", None);
    tauri::async_runtime::spawn(live_client::run(events_tx, stop_rx));
    tauri::async_runtime::spawn(consume(app.clone(), events_rx));
}

fn stop(app: &AppHandle) {
    let live = app.state::<Live>();
    // `take()` empties the slot and gives us what was in it.
    let Some(stop_tx) = live.stop.lock().unwrap().take() else {
        return; // not connected
    };
    let _ = stop_tx.send(true);
    log(app, "disconnect", None);
}

/// Receives what the connection reports. Ends when the connection task has finished (it
/// drops its end of the channel).
async fn consume(app: AppHandle, mut events: mpsc::UnboundedReceiver<LiveEvent>) {
    // A status change is logged together with the summary that follows it, so the line
    // carries the session and the clock as they are *after* the change.
    let mut to_log: Option<(&'static str, Option<String>)> = None;

    while let Some(event) = events.recv().await {
        match event {
            LiveEvent::State(summary) => {
                let live = app.state::<Live>();
                {
                    let status = summary
                        .as_ref()
                        .and_then(|s| s.status.as_deref())
                        .map(SessionStatus::parse);
                    // If the app started after the session ended, the schedule says when it ended.
                    let scheduled_end = summary.as_ref().and_then(|s| {
                        let weekend = live.weekend.lock().unwrap();
                        let session = weekend
                            .as_ref()?
                            .sessions
                            .iter()
                            .find(|x| Some(&x.name) == s.session.as_ref())?;
                        Some(session.end_ms)
                    });
                    live.tab
                        .lock()
                        .unwrap()
                        .update(status, now_ms(), scheduled_end);
                    *live.latest.lock().unwrap() = summary.map(|boxed| *boxed);
                }
                if let Some((name, status)) = to_log.take() {
                    log(&app, name, status);
                }
                let _ = app.emit("live:state", live.payload());
            }
            LiveEvent::Feed(Event::Snapshot(status)) => to_log = Some(("snapshot", status)),
            LiveEvent::Feed(Event::Status(status)) => to_log = Some(("status", Some(status))),
            // Flags and race control messages will become alerts in a later stage.
            LiveEvent::Feed(_) => {}
        }
    }
}

/// One line in `live-status.log` (in the app's data folder): the real sequence of what
/// live timing says, kept to learn from. Failing to write it never matters.
fn log(app: &AppHandle, event: &str, status: Option<String>) {
    let Ok(dir) = app.path().app_data_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);

    let live = app.state::<Live>();
    let latest = live.latest.lock().unwrap();
    let entry = Entry {
        event: event.to_string(),
        session: latest.as_ref().and_then(|s| s.session.clone()),
        part: latest.as_ref().and_then(|s| s.part).map(|p| p as u8),
        status,
        clock: latest.as_ref().and_then(|s| s.remaining.clone()),
    };
    let _ = status_log::append_entry(
        &dir.join("live-status.log"),
        &entry,
        chrono::Utc::now(),
        status_log::MAX_BYTES,
    );
}
