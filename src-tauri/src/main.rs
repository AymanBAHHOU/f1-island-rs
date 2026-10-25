// F1 Island, stages 1, 3 and 4: a transparent, always-on-top window that shows the island,
// lets clicks fall through to whatever is underneath, and opens when the mouse
// is over the island. Same idea as the Electron app's main.js, in Rust.

// In release builds, don't open a console window next to the app (Windows only).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod live;

use f1_island::analysis_data::Analyses;
use f1_island::circuit::CircuitHistory;
use f1_island::demo;
use f1_island::jolpica::Jolpica;
use f1_island::news::{self, News};
use f1_island::openf1::OpenF1;
use f1_island::results::{self, Results};
use f1_island::schedule::Weekend;
use f1_island::settings::{Settings, SettingsStore};
use f1_island::snapshot::Snapshots;
use f1_island::standings::Standings;
use f1_island::updater::{self, Updater};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{Emitter, Manager, PhysicalPosition};

// The window is bigger than the island and transparent. The island sits EDGE
// pixels below the window's top, and the window is moved up by EDGE so the
// island ends up TOP_GAP from the top of the screen (these match the Electron app).
const EDGE: f64 = 32.0;
const TOP_GAP: f64 = 8.0;

// ---------- data for the page ----------

// A command is a Rust function the page can call by name:
//   await window.__TAURI__.core.invoke('weekend')
//
// This one is `async`: while it waits for the network, the app keeps running. The
// `Jolpica` client was handed to Tauri with `.manage(...)` below; `State` gives the
// command a reference to it (the `'_` is a lifetime Rust needs to see for async commands).
// Errors become text, which the page receives as a rejected promise.
#[tauri::command]
async fn weekend(jolpica: tauri::State<'_, Jolpica>) -> Result<Option<Weekend>, String> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    jolpica.weekend(now_ms).await.map_err(|e| e.to_string())
}

// The championship tables and the last race's podium.
#[tauri::command]
async fn standings(jolpica: tauri::State<'_, Jolpica>) -> Result<Standings, String> {
    jolpica.standings().await.map_err(|e| e.to_string())
}

// Commands can take arguments: the page sends `{ circuitId, season }` and Tauri turns
// those camelCase names into these snake_case parameters.
#[tauri::command]
async fn circuit_history(
    jolpica: tauri::State<'_, Jolpica>,
    circuit_id: String,
    season: String,
) -> Result<CircuitHistory, String> {
    let season: u32 = season
        .parse()
        .map_err(|_| format!("not a season: {season:?}"))?;
    jolpica
        .circuit_history(&circuit_id, season)
        .await
        .map_err(|e| e.to_string())
}

// The headlines, merged from several feeds.
#[tauri::command]
async fn news(news: tauri::State<'_, News>) -> Result<Vec<news::Item>, String> {
    news.headlines().await
}

// Opens a headline in the browser, but only links we handed out ourselves.
#[tauri::command]
fn open_article(url: String, news: tauri::State<News>) {
    if news.is_known_link(&url) {
        let _ = open::that(url);
    }
}

// The whole snapshot the island shows (weekend, standings, title fight, circuit).
#[tauri::command]
async fn snapshot(
    extras: Option<bool>,
    jolpica: tauri::State<'_, Jolpica>,
    openf1: tauri::State<'_, OpenF1>,
    snapshots: tauri::State<'_, Snapshots>,
) -> Result<serde_json::Value, String> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    Ok(snapshots
        .build(&jolpica, &openf1, now_ms, extras.unwrap_or(true))
        .await)
}

// Shows one alert of each kind (also reachable from the menu, and with `--demo-alerts`).
#[tauri::command]
fn preview_alerts(app: tauri::AppHandle) {
    live::preview_alerts(&app);
}

// ---------- settings and the preview mode ----------

#[tauri::command]
fn get_settings(settings: tauri::State<SettingsStore>) -> Settings {
    settings.get()
}

// The pretend race being previewed (see `demo`), if any.
#[derive(Default)]
struct DemoState(Mutex<Option<serde_json::Value>>);

#[tauri::command]
fn get_demo(demo: tauri::State<DemoState>) -> Option<serde_json::Value> {
    demo.0.lock().unwrap().clone()
}

// Starts a preview (`None` stops it) and tells the page.
fn set_demo(app: &tauri::AppHandle, mode: Option<&str>) {
    let demo = mode.and_then(|m| demo::create(m, chrono::Utc::now().timestamp_millis()));
    *app.state::<DemoState>().0.lock().unwrap() = demo.clone();
    let _ = app.emit("island:demo", demo);
}

// ---------- updates ----------

#[tauri::command]
fn update_state(updater: tauri::State<Updater>) -> updater::State {
    updater.state()
}

// The user chose the update chip: open the download page, or check now.
#[tauri::command]
fn update_act(app: tauri::AppHandle) {
    act_on_update(&app);
}

fn act_on_update(app: &tauri::AppHandle) {
    let updater = app.state::<Updater>();
    match updater::act(&updater.state()) {
        updater::Act::Open(url) => {
            let _ = open::that(url);
        }
        updater::Act::Check => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { check_for_update(&app).await });
        }
        updater::Act::Nothing => {}
    }
}

async fn check_for_update(app: &tauri::AppHandle) {
    let updater = app.state::<Updater>();
    if let Some(state) = updater.check(chrono::Utc::now().timestamp_millis()).await {
        let _ = app.emit("update:state", state);
    }
}

// Checks half a minute after start, then every six hours.
async fn watch_for_updates(app: tauri::AppHandle) {
    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    loop {
        check_for_update(&app).await;
        tokio::time::sleep(std::time::Duration::from_secs(6 * 3600)).await;
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ---------- the Analysis page ----------

// The season's finished sessions, newest first.
#[tauri::command]
async fn analysis_sessions(
    openf1: tauri::State<'_, OpenF1>,
    results: tauri::State<'_, Results>,
) -> Result<Vec<f1_island::results::SessionRef>, String> {
    results.sessions(&openf1, now()).await
}

#[tauri::command]
async fn analysis_get(
    key: i64,
    openf1: tauri::State<'_, OpenF1>,
    analyses: tauri::State<'_, Analyses>,
) -> Result<serde_json::Value, String> {
    Ok(analyses.get(&openf1, key, now()).await?.data.clone())
}

#[tauri::command]
async fn analysis_telemetry(
    session_key: i64,
    driver: i64,
    lap: i64,
    openf1: tauri::State<'_, OpenF1>,
    analyses: tauri::State<'_, Analyses>,
) -> Result<serde_json::Value, String> {
    analyses
        .telemetry(&openf1, session_key, driver, lap, now())
        .await
}

#[tauri::command]
async fn analysis_location(
    session_key: i64,
    driver: i64,
    lap: i64,
    openf1: tauri::State<'_, OpenF1>,
    analyses: tauri::State<'_, Analyses>,
) -> Result<serde_json::Value, String> {
    analyses
        .lap_location(&openf1, session_key, driver, lap, now())
        .await
}

#[tauri::command]
async fn replay_info(
    session_key: i64,
    openf1: tauri::State<'_, OpenF1>,
    analyses: tauri::State<'_, Analyses>,
) -> Result<serde_json::Value, String> {
    analyses.replay_info(&openf1, session_key, now()).await
}

#[tauri::command]
async fn replay_chunk(
    session_key: i64,
    index: i64,
    openf1: tauri::State<'_, OpenF1>,
    analyses: tauri::State<'_, Analyses>,
) -> Result<serde_json::Value, String> {
    analyses
        .replay_chunk(&openf1, session_key, index, now())
        .await
}

// The map for the weekend shown on the Circuit page.
#[tauri::command]
async fn track_map(
    jolpica: tauri::State<'_, Jolpica>,
    openf1: tauri::State<'_, OpenF1>,
    results: tauri::State<'_, Results>,
    analyses: tauri::State<'_, Analyses>,
) -> Result<Option<serde_json::Value>, String> {
    let now_ms = now();
    let weekend = jolpica.weekend(now_ms).await.map_err(|e| e.to_string())?;
    let Some(w) = weekend else { return Ok(None) };
    let (Some(first), Some(last)) = (w.sessions.first(), w.sessions.last()) else {
        return Ok(None);
    };
    analyses
        .weekend_track_map(
            &openf1,
            &results,
            first.start_ms,
            last.end_ms,
            &w.country,
            &w.locality,
            now_ms,
        )
        .await
}

// The top three of each finished session of this weekend.
#[tauri::command]
async fn weekend_results(
    jolpica: tauri::State<'_, Jolpica>,
    openf1: tauri::State<'_, OpenF1>,
    results: tauri::State<'_, Results>,
) -> Result<Vec<results::Entry>, String> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let weekend = jolpica.weekend(now_ms).await.map_err(|e| e.to_string())?;
    let Some(weekend) = weekend else {
        return Ok(Vec::new());
    };
    let (Some(first), Some(last)) = (weekend.sessions.first(), weekend.sessions.last()) else {
        return Ok(Vec::new());
    };
    results
        .weekend(&openf1, first.start_ms, last.end_ms, now_ms)
        .await
}

// ---------- where the island is, so we know when the mouse is over it ----------

// The page tells us the island's rectangle (in CSS pixels, relative to the window).
#[derive(Clone, Copy, Deserialize)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

// Shared state. The polling thread reads it while commands write it, so it sits
// behind a `Mutex`: only one side can hold the lock at a time. `Option` because
// the page hasn't reported a rectangle yet when we start.
#[derive(Default)]
struct HitRect(Mutex<Option<Rect>>);

#[tauri::command]
fn report_rect(rect: Rect, state: tauri::State<HitRect>) {
    *state.0.lock().unwrap() = Some(rect);
}

// Whether the pointer was over the island at the last check. Shared so the page can ask for
// the next check to report again (see `rehover`).
#[derive(Default)]
struct Hovering(AtomicBool);

// The window is click-through until the page says otherwise: the page asks for clicks
// when the island is open, and gives them back when it closes. (Same split as the Electron
// app: the main process reports where the mouse is, the page decides what that means.)
#[tauri::command]
fn set_interactive(on: bool, window: tauri::WebviewWindow) {
    let _ = window.set_ignore_cursor_events(!on);
}

// "Tell me again whether the pointer is on the island": clears the remembered answer, so
// the next check reports even if nothing changed.
#[tauri::command]
fn rehover(hovering: tauri::State<Hovering>) {
    hovering.0.store(false, Ordering::Relaxed);
}

// ---------- the window ----------

fn main() {
    tauri::Builder::default()
        .manage(HitRect::default())
        .manage(Hovering::default())
        .manage(News::default())
        .manage(OpenF1::default())
        .manage(Results::default())
        .manage(Snapshots::default())
        .manage(Analyses::default())
        .manage(DemoState::default())
        .manage(Updater::new(env!("CARGO_PKG_VERSION")))
        .manage(Jolpica::new())
        .manage(live::Live::default())
        .invoke_handler(tauri::generate_handler![
            weekend,
            standings,
            circuit_history,
            live::live_state,
            report_rect,
            set_interactive,
            rehover,
            news,
            open_article,
            weekend_results,
            snapshot,
            analysis_sessions,
            analysis_get,
            analysis_telemetry,
            analysis_location,
            replay_info,
            replay_chunk,
            track_map,
            preview_alerts,
            get_settings,
            get_demo,
            update_state,
            update_act
        ])
        .setup(|app| {
            let win = app
                .get_webview_window("main")
                .expect("the window from tauri.conf.json");

            let settings_file = app
                .path()
                .app_data_dir()
                .map(|d| d.join("settings.json"))
                .unwrap_or_else(|_| "settings.json".into());
            app.manage(SettingsStore::load(settings_file));

            // `--demo=race` (or next, safety-car, red-flag, penalty) starts in a preview.
            let handle = app.handle();
            if let Some(mode) =
                std::env::args().find_map(|a| a.strip_prefix("--demo=").map(str::to_string))
            {
                set_demo(handle, Some(&mode));
            }

            // `--fake-update=9.9.9` shows the update chip, for trying it out.
            if let Some(version) =
                std::env::args().find_map(|a| a.strip_prefix("--fake-update=").map(str::to_string))
            {
                handle.state::<Updater>().set(updater::State {
                    status: updater::Status::Available,
                    version: Some(version),
                    url: Some(updater::RELEASES_URL.to_string()),
                    checked_at: now(),
                });
            } else if !cfg!(debug_assertions) {
                tauri::async_runtime::spawn(watch_for_updates(handle.clone()));
            }

            if std::env::args().any(|a| a == "--demo-alerts") {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
                    live::preview_alerts(&handle);
                });
            }
            // OpenF1 answers are kept on disk, so sessions already opened keep working
            // while a live session blocks the free API.
            if let Ok(dir) = app.path().app_data_dir() {
                app.state::<OpenF1>().set_cache_dir(&dir.join("openf1"));
            }

            // Centre the window horizontally at the top of the monitor it's on.
            if let Some(monitor) = win.current_monitor()? {
                let scale = monitor.scale_factor();
                let size = win.outer_size()?;
                let x =
                    monitor.position().x + (monitor.size().width as i32 - size.width as i32) / 2;
                let y = monitor.position().y + ((TOP_GAP - EDGE) * scale) as i32;
                win.set_position(PhysicalPosition::new(x, y))?;
            }

            // Start click-through: clicks pass to the apps underneath.
            win.set_ignore_cursor_events(true)?;

            // A window that is click-through can't see the mouse, so a background
            // thread asks the OS where the cursor is, 20 times a second. When it
            // enters or leaves the island it makes the window clickable (or not)
            // and tells the page, which opens or closes the island.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_millis(50));

                    let Some(win) = handle.get_webview_window("main") else {
                        continue;
                    };
                    let (Ok(cursor), Ok(pos)) = (win.cursor_position(), win.outer_position())
                    else {
                        continue;
                    };
                    let scale = win.scale_factor().unwrap_or(1.0);
                    let rect = *handle.state::<HitRect>().0.lock().unwrap();

                    // Cursor position relative to the window, in CSS pixels.
                    let x = (cursor.x - pos.x as f64) / scale;
                    let y = (cursor.y - pos.y as f64) / scale;
                    let inside = rect.is_some_and(|r| {
                        x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
                    });

                    let hovering = handle.state::<Hovering>();
                    if inside != hovering.0.load(Ordering::Relaxed) {
                        hovering.0.store(inside, Ordering::Relaxed);
                        let _ = win.emit("hover", inside);
                    }
                }
            });

            // Live timing: a task that decides when to be connected (see live.rs).
            tauri::async_runtime::spawn(live::supervise(app.handle().clone()));

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running F1 Island");
}
