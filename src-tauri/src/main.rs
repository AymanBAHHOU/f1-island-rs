// F1 Island, stages 1, 3 and 4: a transparent, always-on-top window that shows the island,
// lets clicks fall through to whatever is underneath, and opens when the mouse
// is over the island. Same idea as the Electron app's main.js, in Rust.

// In release builds, don't open a console window next to the app (Windows only).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod live;

use f1_island::circuit::CircuitHistory;
use f1_island::jolpica::Jolpica;
use f1_island::schedule::Weekend;
use f1_island::standings::Standings;
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
        .manage(Jolpica::new())
        .manage(live::Live::default())
        .invoke_handler(tauri::generate_handler![
            weekend,
            standings,
            circuit_history,
            live::live_state,
            report_rect,
            set_interactive,
            rehover
        ])
        .setup(|app| {
            let win = app
                .get_webview_window("main")
                .expect("the window from tauri.conf.json");

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
