//! The window around the island: creating it, placing it, following the pointer, dragging
//! it, docking it, growing it for the analysis view. The numbers come from
//! `f1_island::geometry`; this file is the part that talks to the operating system.
//! (Port of the window half of the Electron app's `main.js`.)

use f1_island::dock::Point;
use f1_island::geometry::{self, Area, EDGE};
use f1_island::platform::Os;
use f1_island::settings::{Position, SettingsStore};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

pub const LABEL: &str = "main";

/// Everything about the window that changes while the app runs. Shared by the page's
/// commands and the background threads, so each part sits in an atomic or a `Mutex`.
#[derive(Default)]
pub struct Shell {
    /// The island's hover area, in CSS pixels inside the window (the page reports it).
    hit_rect: Mutex<Option<Area>>,
    /// Was the pointer over the island at the last check?
    pub hovering: AtomicBool,
    /// The island's menu is showing: keep the island open under it.
    pub menu_open: AtomicBool,
    /// Mirrors the page's "keep open" state.
    pub pinned: AtomicBool,
    dragging: AtomicBool,
    /// Counts slides, so starting a new one (or a drag) cancels the one running.
    slide_id: AtomicU64,
    sliding: AtomicBool,
    big: Mutex<Option<BigState>>,
}

/// The window while the analysis view is open.
struct BigState {
    /// The window's bounds to go back to.
    normal: Area,
    size: (f64, f64),
}

/// A rectangle as the page sends it.
#[derive(Deserialize, Clone, Copy)]
pub struct RectArg {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

// ---------- screens ----------

#[derive(Clone, Copy)]
struct Screen {
    work: Area,
    scale: f64,
    primary: bool,
}

fn screens(win: &WebviewWindow) -> Vec<Screen> {
    let primary = win.primary_monitor().ok().flatten();
    win.available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let w = m.work_area();
            Screen {
                work: Area::new(
                    f64::from(w.position.x),
                    f64::from(w.position.y),
                    f64::from(w.size.width),
                    f64::from(w.size.height),
                ),
                scale: m.scale_factor(),
                primary: primary
                    .as_ref()
                    .is_some_and(|p| p.position() == m.position() && p.size() == m.size()),
            }
        })
        .collect()
}

/// The screen the window (at `pos`) mostly belongs to; the primary one if none fits.
fn screen_for(win: &WebviewWindow, pos: Point) -> Option<Screen> {
    let all = screens(win);
    let guess = win.scale_factor().unwrap_or(1.0);
    geometry::nearest_screen(&all.iter().map(|s| s.work).collect::<Vec<_>>(), pos, guess)
        .map(|i| all[i])
        .or_else(|| all.iter().find(|s| s.primary).or(all.first()).copied())
}

fn primary_screen(win: &WebviewWindow) -> Option<Screen> {
    let all = screens(win);
    all.iter().find(|s| s.primary).or(all.first()).copied()
}

// ---------- the window ----------

fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

fn origin(win: &WebviewWindow) -> Point {
    let p = win.outer_position().unwrap_or_default();
    Point {
        x: f64::from(p.x),
        y: f64::from(p.y),
    }
}

fn set_pos(win: &WebviewWindow, pos: Point) {
    let _ = win.set_position(PhysicalPosition::new(pos.x as i32, pos.y as i32));
}

fn is_mac() -> bool {
    Os::current() == Os::Mac
}

fn is_docked(app: &AppHandle) -> bool {
    !is_mac() && app.state::<SettingsStore>().get().docked
}

/// Creates the window: transparent, undecorated, always on top, never taking focus, and
/// click-through until the page says otherwise. Debug flags (`--pinned`, `--big`,
/// `--tab=`, `--page=`, `--start=`) are passed to the page as a query.
pub fn create_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| {
        args.iter()
            .find_map(|a| a.strip_prefix(name).map(str::to_string))
    };
    let mut query: Vec<String> = Vec::new();
    if flag("--pinned") {
        query.push("pinned=1".into());
    }
    if flag("--big") {
        query.push("big=1".into());
    }
    for (arg, key) in [
        ("--tab=", "tab"),
        ("--page=", "page"),
        ("--start=", "start"),
    ] {
        if let Some(v) = value(arg) {
            query.push(format!("{key}={v}"));
        }
    }
    let url = if query.is_empty() {
        "index.html".to_string()
    } else {
        format!("index.html?{}", query.join("&"))
    };

    let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(url.into()))
        .title("F1 Island")
        .inner_size(geometry::WIN_W, geometry::WIN_H)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .focusable(false)
        .focused(false)
        .visible(false)
        .build()?;
    win.set_ignore_cursor_events(true)?;
    place_window(app);
    win.show()?;
    Ok(win)
}

/// Puts the window where the settings say: docked at the top centre, the saved spot, or
/// the default top centre.
pub fn place_window(app: &AppHandle) {
    let (Some(win), shell) = (window(app), app.state::<Shell>()) else {
        return;
    };
    if shell.dragging.load(Ordering::Relaxed) {
        return;
    }
    cancel_slide(&shell);
    let Some(primary) = primary_screen(&win) else {
        return;
    };
    let saved = app
        .state::<SettingsStore>()
        .get()
        .position
        .map(|p| Point { x: p.x, y: p.y });

    let pos = if is_docked(app) {
        // Docked: top centre of the screen the island was last on.
        let on = saved.and_then(|s| screen_for(&win, s)).unwrap_or(primary);
        geometry::top_centre(on.work, on.scale, true)
    } else if let Some(saved) = saved {
        let on = screen_for(&win, saved).unwrap_or(primary);
        geometry::clamp_to_screen(saved, on.work, on.scale)
    } else {
        geometry::top_centre(primary.work, primary.scale, false)
    };

    let in_big = shell.big.lock().unwrap().is_some();
    if in_big {
        // The analysis view is open: re-centre the big window around the island's new home.
        let (w, h) = geometry::window_size(primary.scale);
        let normal = Area::new(pos.x, pos.y, w, h);
        let on = screen_for(&win, pos).unwrap_or(primary);
        let big = geometry::enter_big(normal, on.work, on.scale);
        if let Some(state) = shell.big.lock().unwrap().as_mut() {
            state.normal = normal;
            state.size = (big.bounds.width, big.bounds.height);
        }
        apply_bounds(&win, big.bounds);
        let _ = app.emit("island:big-geometry", big_json(&big));
    } else {
        set_pos(&win, pos);
    }
}

fn apply_bounds(win: &WebviewWindow, b: Area) {
    let _ = win.set_size(PhysicalSize::new(b.width as u32, b.height as u32));
    set_pos(win, Point { x: b.x, y: b.y });
}

fn big_json(big: &geometry::Big) -> Value {
    json!({"anchorX": big.anchor_x, "width": big.width, "height": big.height, "edge": EDGE})
}

pub fn reset_position(app: &AppHandle) {
    app.state::<SettingsStore>().update(|s| {
        s.position = None;
        s.notch_attached = true;
    });
    place_window(app);
}

pub fn set_docked(app: &AppHandle, on: bool) {
    if is_mac() {
        return;
    }
    let settings = app.state::<SettingsStore>();
    if settings.get().docked == on {
        return;
    }
    let updated = settings.update(|s| s.docked = on);
    let _ = app.emit("island:settings", updated);
    crate::menus::refresh_tray_menu(app);
}

// ---------- sliding ----------

fn cancel_slide(shell: &Shell) {
    shell.slide_id.fetch_add(1, Ordering::Relaxed);
    shell.sliding.store(false, Ordering::Relaxed);
}

/// Slides the window to `to` (docking: from wherever it was dropped along the top edge
/// back to the centre). Cancelled if a drag starts or the island is placed again.
fn slide_to(app: &AppHandle, to: Point, done: impl FnOnce() + Send + 'static) {
    let (Some(win), shell) = (window(app), app.state::<Shell>()) else {
        return;
    };
    cancel_slide(&shell);
    let from = origin(&win);
    let distance = (to.x - from.x).hypot(to.y - from.y);
    if distance < 2.0 {
        set_pos(&win, to);
        done();
        return;
    }
    let scale = win.scale_factor().unwrap_or(1.0);
    let duration = geometry::slide_ms(distance, scale);
    let id = shell.slide_id.load(Ordering::Relaxed);
    shell.sliding.store(true, Ordering::Relaxed);
    let app = app.clone();
    std::thread::spawn(move || {
        let started = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(8));
            let shell = app.state::<Shell>();
            if shell.slide_id.load(Ordering::Relaxed) != id {
                return; // cancelled
            }
            let Some(win) = window(&app) else { return };
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            let (pos, over) = geometry::slide_step(from, to, elapsed, duration);
            set_pos(&win, pos);
            if over {
                shell.sliding.store(false, Ordering::Relaxed);
                done();
                return;
            }
        }
    });
}

// ---------- dragging ----------
// The page reports drag start/end; while dragging, the window follows the OS cursor,
// which keeps working even if the pointer briefly outruns the window.

#[tauri::command]
pub fn drag_start(app: AppHandle, shell: tauri::State<Shell>) {
    let Some(win) = window(&app) else { return };
    if shell.dragging.swap(true, Ordering::Relaxed) {
        return;
    }
    cancel_slide(&shell);
    if is_docked(&app) {
        set_docked(&app, false); // dragging a docked island detaches it
    }
    let (Ok(c), start) = (win.cursor_position(), origin(&win)) else {
        shell.dragging.store(false, Ordering::Relaxed);
        return;
    };
    let (dx, dy) = (c.x - start.x, c.y - start.y);
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(8));
        let shell = app.state::<Shell>();
        if !shell.dragging.load(Ordering::Relaxed) {
            return;
        }
        let Some(win) = window(&app) else { return };
        if let Ok(c) = win.cursor_position() {
            set_pos(
                &win,
                Point {
                    x: c.x - dx,
                    y: c.y - dy,
                },
            );
        }
    });
}

#[tauri::command]
pub fn drag_end(app: AppHandle, shell: tauri::State<Shell>) {
    if !shell.dragging.swap(false, Ordering::Relaxed) {
        return;
    }
    let Some(win) = window(&app) else { return };
    let settings = app.state::<SettingsStore>();
    let pos = origin(&win);
    let Some(on) = screen_for(&win, pos).or_else(|| primary_screen(&win)) else {
        return;
    };

    let in_big = shell.big.lock().unwrap().as_ref().map(|b| b.size);
    if let Some(size) = in_big {
        end_big_drag(&app, &win, pos, size, on);
        return;
    }

    let clamped = geometry::clamp_to_screen(pos, on.work, on.scale);
    // Windows/Linux: dropped near the top centre, the island docks there.
    let drop = geometry::drop_island(clamped, on.work, on.scale);
    if drop.dock_snap {
        slide_to(&app, drop.pos, || {});
    } else {
        set_pos(&win, drop.pos);
    }
    // Snapping to the primary screen's top centre is the default spot.
    let is_default = drop.snapped && on.primary;
    settings.update(|s| {
        s.position = (!is_default).then_some(Position {
            x: drop.pos.x,
            y: drop.pos.y,
        });
    });
    set_docked(&app, drop.dock_snap);
}

/// Dropping the big (analysis) island: keep it on screen, snap it to the top centre if it's
/// close, and move the small island's home under its top centre so leaving big mode lands
/// there.
fn end_big_drag(app: &AppHandle, win: &WebviewWindow, pos: Point, size: (f64, f64), on: Screen) {
    let drop = geometry::drop_big(pos, size, on.work, on.scale);
    let settings = app.state::<SettingsStore>();
    let (win_w, win_h) = geometry::window_size(on.scale);
    if let Some(big) = app.state::<Shell>().big.lock().unwrap().as_mut() {
        big.normal = Area::new(drop.normal.x, drop.normal.y, win_w, win_h);
    }
    let is_default = drop.snapped && on.primary;
    settings.update(|s| {
        s.position = (!is_default).then_some(Position {
            x: drop.normal.x,
            y: drop.normal.y,
        });
    });
    set_docked(app, drop.snapped);

    // Where the small island now sits inside the big window (once it has slid there).
    let handle = app.clone();
    let anchor = drop.anchor_x;
    let slides = drop.snapped && !is_mac();
    let send_anchor = move || {
        let _ = handle.emit("island:big-anchor", anchor);
        // The panel slid out from under the pointer: keep it open until it's used again.
        if slides {
            let _ = handle.emit("island:summon", ());
        }
    };
    if slides {
        slide_to(app, drop.pos, send_anchor);
    } else {
        set_pos(win, drop.pos);
        send_anchor();
    }
}

// ---------- big (analysis) mode ----------

#[tauri::command]
pub fn set_big(on: bool, app: AppHandle) -> Option<Value> {
    if on {
        enter_big(&app)
    } else {
        exit_big(&app);
        None
    }
}

fn enter_big(app: &AppHandle) -> Option<Value> {
    let win = window(app)?;
    let shell = app.state::<Shell>();
    let normal = shell
        .big
        .lock()
        .unwrap()
        .as_ref()
        .map(|b| b.normal)
        .unwrap_or_else(|| {
            let p = origin(&win);
            let s = win.outer_size().unwrap_or_default();
            Area::new(p.x, p.y, f64::from(s.width), f64::from(s.height))
        });
    let on = screen_for(
        &win,
        Point {
            x: normal.x,
            y: normal.y,
        },
    )
    .or_else(|| primary_screen(&win))?;
    let big = geometry::enter_big(normal, on.work, on.scale);
    *shell.big.lock().unwrap() = Some(BigState {
        normal,
        size: (big.bounds.width, big.bounds.height),
    });
    apply_bounds(&win, big.bounds);
    Some(big_json(&big))
}

fn exit_big(app: &AppHandle) {
    let Some(win) = window(app) else { return };
    let shell = app.state::<Shell>();
    let Some(state) = shell.big.lock().unwrap().take() else {
        return;
    };
    apply_bounds(&win, state.normal);
    place_window(app);
}

// ---------- the page's small reports ----------

#[tauri::command]
pub fn report_rect(rect: RectArg, shell: tauri::State<Shell>) {
    *shell.hit_rect.lock().unwrap() = Some(Area::new(rect.x, rect.y, rect.width, rect.height));
}

/// The window is click-through until the page asks for clicks (the island is open), and
/// gives them back when it closes.
#[tauri::command]
pub fn set_interactive(on: bool, window: WebviewWindow) {
    let _ = window.set_ignore_cursor_events(!on);
}

/// "Tell me again whether the pointer is on the island": clears the remembered answer, so
/// the next check reports even if nothing changed.
#[tauri::command]
pub fn rehover(shell: tauri::State<Shell>) {
    shell.hovering.store(false, Ordering::Relaxed);
}

#[tauri::command]
pub fn report_pinned(on: bool, shell: tauri::State<Shell>) {
    shell.pinned.store(on, Ordering::Relaxed);
}

#[tauri::command]
pub fn get_notch() -> Option<Value> {
    None // only Macs with a camera notch have one
}

// ---------- hover ----------
// The window is click-through, so it can't see the mouse itself. A thread asks the OS where
// the cursor is, and tells the page when it enters or leaves the island; the page then
// decides when to make the window clickable.

pub fn start_hover_polling(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(50));
        let shell = app.state::<Shell>();
        let busy = shell.dragging.load(Ordering::Relaxed)
            // While the island slides into its dock it leaves the pointer behind: don't
            // read that as leaving.
            || shell.sliding.load(Ordering::Relaxed)
            || shell.menu_open.load(Ordering::Relaxed);
        let Some(rect) = *shell.hit_rect.lock().unwrap() else {
            continue;
        };
        let Some(win) = window(&app) else { continue };
        if busy {
            continue;
        }
        let Ok(cursor) = win.cursor_position() else {
            continue;
        };
        let scale = win.scale_factor().unwrap_or(1.0);
        let inside = geometry::pointer_in_island(
            Point {
                x: cursor.x,
                y: cursor.y,
            },
            origin(&win),
            scale,
            rect,
        );
        if inside != shell.hovering.load(Ordering::Relaxed) {
            shell.hovering.store(inside, Ordering::Relaxed);
            let _ = win.emit("hover", inside);
        }
    });
}

// ---------- the screens changing, and waking from sleep ----------

/// Watches for monitors being added, removed or changing size or scale, and for the
/// computer waking from sleep (a long gap between ticks), and puts the island back.
pub fn start_display_watch(app: AppHandle) {
    std::thread::spawn(move || {
        let signature = |win: &WebviewWindow| -> Vec<(i32, i32, u32, u32, u64)> {
            win.available_monitors()
                .unwrap_or_default()
                .iter()
                .map(|m| {
                    let w = m.work_area();
                    (
                        w.position.x,
                        w.position.y,
                        w.size.width,
                        w.size.height,
                        (m.scale_factor() * 100.0) as u64,
                    )
                })
                .collect()
        };
        let mut last_tick = Instant::now();
        let mut known = window(&app).map(|w| signature(&w)).unwrap_or_default();
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let Some(win) = window(&app) else { continue };
            let woke = last_tick.elapsed() > Duration::from_secs(5);
            last_tick = Instant::now();
            let now = signature(&win);
            if now != known || woke {
                known = now;
                std::thread::sleep(Duration::from_millis(150));
                place_window(&app);
                crate::menus::refresh_tray_menu(&app);
                if woke {
                    let _ = app.emit("f1:refresh", ());
                }
            }
        }
    });
}
