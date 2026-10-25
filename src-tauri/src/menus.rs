//! The menus: the tray icon's, the island's own (the ⋯ button or a right click), and the
//! pop-up lists the Analysis page asks for. Also the global hotkey.
//! (Port of the menu, tray and hotkey parts of the Electron app's `main.js`.)

use crate::shell::{self, Shell, LABEL};
use f1_island::demo;
use f1_island::platform::{format_hotkey, login_item_label, Os};
use f1_island::settings::SettingsStore;
use f1_island::updater::{self, Updater};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Duration;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt as AutostartExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tokio::sync::oneshot;

const TRAY_ID: &str = "f1-island";

/// Whether the global hotkey could be registered (another app may hold it).
#[derive(Default)]
pub struct Hotkey(pub std::sync::atomic::AtomicBool);

/// Who is waiting for the choice, and what each item chooses.
type Waiting = (oneshot::Sender<Option<Value>>, HashMap<String, Value>);

/// A pop-up list waiting for a choice.
#[derive(Default)]
pub struct Choices {
    pending: Mutex<Option<Waiting>>,
}

fn os() -> Os {
    Os::current()
}

fn hotkey_label(app: &AppHandle) -> String {
    format_hotkey(&app.state::<SettingsStore>().get().hotkey, os())
}

// ---------- the main menu ----------

/// The tray menu offers Show / hide; the island's own menu (right-click or ⋯) is only
/// reachable while it's open, so it offers Keep open instead.
fn build_menu(app: &AppHandle, from_island: bool) -> tauri::Result<Menu<tauri::Wry>> {
    let settings = app.state::<SettingsStore>().get();
    let shell = app.state::<Shell>();
    let mac = os() == Os::Mac;
    let hotkey_ok = app.state::<Hotkey>().0.load(Ordering::Relaxed);
    let menu = Menu::new(app)?;

    if from_island {
        menu.append(&CheckMenuItem::with_id(
            app,
            "pinned",
            "Keep open",
            true,
            shell.pinned.load(Ordering::Relaxed),
            None::<&str>,
        )?)?;
    } else {
        let label = if hotkey_ok {
            format!("Show / hide\t{}", hotkey_label(app))
        } else {
            "Show / hide".to_string()
        };
        menu.append(&MenuItem::with_id(
            app,
            "toggle",
            label,
            true,
            None::<&str>,
        )?)?;
    }

    if mac && shell::is_attached(app) {
        // Hanging from the notch, "hidden" means notch only; otherwise stretched.
        let idle = Submenu::with_id(app, "idle-menu", "Idle appearance", true)?;
        idle.append(&CheckMenuItem::with_id(
            app,
            "idle:notch",
            "Notch only",
            true,
            settings.auto_hide,
            None::<&str>,
        )?)?;
        idle.append(&CheckMenuItem::with_id(
            app,
            "idle:stretched",
            "Stretched",
            true,
            !settings.auto_hide,
            None::<&str>,
        )?)?;
        menu.append(&idle)?;
    } else {
        menu.append(&CheckMenuItem::with_id(
            app,
            "autohide",
            "Auto-hide until hovered",
            true,
            settings.auto_hide,
            None::<&str>,
        )?)?;
    }
    if !mac {
        menu.append(&CheckMenuItem::with_id(
            app,
            "dock",
            "Dock to top edge",
            true,
            settings.docked,
            None::<&str>,
        )?)?;
    }
    menu.append(&MenuItem::with_id(
        app,
        "reset",
        "Reset position",
        true,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    menu.append(&MenuItem::with_id(
        app,
        "analysis",
        "Session analysis",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "preview-alerts",
        "Preview alerts",
        true,
        None::<&str>,
    )?)?;
    if mac {
        let current = app
            .state::<crate::DemoState>()
            .0
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|d| d["mode"].as_str().map(str::to_string));
        let sub = Submenu::with_id(app, "demo-menu", "Preview stretched bar", true)?;
        sub.append(&CheckMenuItem::with_id(
            app,
            "demo:off",
            "Live data (stop preview)",
            true,
            current.is_none(),
            None::<&str>,
        )?)?;
        for mode in demo::MODES {
            let name = match mode {
                "next" => "Next session",
                "race" => "Race / overtakes",
                "safety-car" => "Safety Car",
                "red-flag" => "Red flag",
                _ => "Penalty",
            };
            sub.append(&CheckMenuItem::with_id(
                app,
                format!("demo:{mode}"),
                format!("{name} (dummy)"),
                true,
                current.as_deref() == Some(mode),
                None::<&str>,
            )?)?;
        }
        menu.append(&sub)?;
    }
    menu.append(&update_item(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "refresh",
        "Refresh data",
        true,
        None::<&str>,
    )?)?;

    // A debug build would register the development program, not the installed app.
    let can_autostart = !cfg!(debug_assertions);
    let starts = can_autostart && app.autolaunch().is_enabled().unwrap_or(false);
    menu.append(&CheckMenuItem::with_id(
        app,
        "login",
        login_item_label(os()),
        can_autostart,
        starts,
        None::<&str>,
    )?)?;

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit",
        if mac { "Quit F1 Island" } else { "Quit" },
        true,
        None::<&str>,
    )?)?;
    Ok(menu)
}

/// The menu entry follows the updater: check, wait for a download, restart to install, or
/// open the download page.
fn update_item(app: &AppHandle) -> tauri::Result<MenuItem<tauri::Wry>> {
    let updater = app.state::<Updater>();
    if updater.mode() == updater::Mode::Off {
        return MenuItem::with_id(app, "update", "Check for updates", false, None::<&str>);
    }
    let state = updater.state();
    let version = state.version.clone().unwrap_or_default();
    let (label, enabled) = match state.status {
        updater::Status::Ready => (format!("Restart to update to v{version}"), true),
        updater::Status::Downloading => (format!("Downloading v{version}…"), false),
        updater::Status::Available => (format!("Download v{version}…"), true),
        updater::Status::Idle if state.checked_at > 0 => (
            format!("Up to date (v{}) · check again", env!("CARGO_PKG_VERSION")),
            true,
        ),
        updater::Status::Idle => ("Check for updates".to_string(), true),
    };
    MenuItem::with_id(app, "update", label, enabled, None::<&str>)
}

/// What choosing a menu item does (the tray's, the island's, or a pop-up list's).
pub fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref();
    let settings = app.state::<SettingsStore>();
    if let Some(choice) = id.strip_prefix("choice:") {
        if let Some((tx, values)) = app.state::<Choices>().pending.lock().unwrap().take() {
            let _ = tx.send(values.get(choice).cloned());
        }
        return;
    }
    if let Some(idle) = id.strip_prefix("idle:") {
        let updated = settings.update(|s| s.auto_hide = idle == "notch");
        crate::set_demo(app, None);
        let _ = app.emit("island:settings", updated);
        refresh_tray_menu(app);
        return;
    }
    if let Some(mode) = id.strip_prefix("demo:") {
        crate::set_demo(app, (mode != "off").then_some(mode));
        refresh_tray_menu(app);
        return;
    }
    match id {
        "toggle" => {
            let _ = app.emit("island:toggle", ());
        }
        "pinned" => {
            // The check mark has already flipped: the page learns the new state.
            let on = !app.state::<Shell>().pinned.load(Ordering::Relaxed);
            let _ = app.emit("island:set-pinned", on);
        }
        "autohide" => {
            let on = !settings.get().auto_hide;
            let updated = settings.update(|s| s.auto_hide = on);
            crate::set_demo(app, None);
            let _ = app.emit("island:settings", updated);
            refresh_tray_menu(app);
        }
        "dock" => {
            let on = !settings.get().docked;
            let updated = settings.update(|s| s.docked = on);
            shell::place_window(app);
            let _ = app.emit("island:settings", updated);
            refresh_tray_menu(app);
        }
        "reset" => shell::reset_position(app),
        "analysis" => {
            let _ = app.emit("island:big", ());
        }
        "preview-alerts" => crate::live::preview_alerts(app),
        "update" => crate::act_on_update(app),
        "refresh" => {
            let _ = app.emit("f1:refresh", ());
        }
        "login" => {
            let launch = app.autolaunch();
            let _ = if launch.is_enabled().unwrap_or(false) {
                launch.disable()
            } else {
                launch.enable()
            };
            refresh_tray_menu(app);
        }
        "quit" => app.exit(0),
        _ => {}
    }
}

pub fn refresh_tray_menu(app: &AppHandle) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY_ID), build_menu(app, false)) {
        let _ = tray.set_menu(Some(menu));
    }
}

/// The island's menu (the ⋯ button or a right click), as a pop-up next to the pointer.
#[tauri::command]
pub async fn open_menu(app: AppHandle) {
    let (Some(win), Ok(menu)) = (app.get_webview_window(LABEL), build_menu(&app, true)) else {
        return;
    };
    let shell = app.state::<Shell>();
    shell.menu_open.store(true, Ordering::Relaxed);
    let _ = win.popup_menu(&menu);
    // Hover polling resumes when the menu closes; if the pointer has left by then, the
    // island collapses.
    shell.menu_open.store(false, Ordering::Relaxed);
}

// ---------- pop-up lists for the page ----------

/// An entry of a list the page wants shown: `{ label, value, checked }`, a separator or a
/// header.
#[derive(Deserialize)]
pub struct Item {
    #[serde(default)]
    label: Value,
    #[serde(default)]
    value: Value,
    #[serde(default)]
    checked: bool,
    #[serde(default)]
    separator: bool,
    #[serde(default)]
    header: bool,
}

/// Shows a pop-up list and resolves with the chosen value, or `null` if it is dismissed.
#[tauri::command]
pub async fn choose(items: Vec<Item>, app: AppHandle) -> Result<Option<Value>, String> {
    let Some(win) = app.get_webview_window(LABEL) else {
        return Ok(None);
    };
    let menu = Menu::new(&app).map_err(|e| e.to_string())?;
    let mut values = HashMap::new();
    for (i, item) in items.iter().take(80).enumerate() {
        let label = match &item.label {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        if item.separator {
            menu.append(&PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        } else if item.header {
            menu.append(
                &MenuItem::with_id(&app, format!("header:{i}"), label, false, None::<&str>)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        } else {
            let id = format!("choice:{i}");
            values.insert(i.to_string(), item.value.clone());
            menu.append(
                &CheckMenuItem::with_id(&app, id, label, true, item.checked, None::<&str>)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        }
    }
    if values.is_empty() {
        return Ok(None);
    }
    Ok(pop_up(&app, &win, &menu, values).await)
}

/// Pops the menu up and waits for what the user chooses.
async fn pop_up(
    app: &AppHandle,
    win: &tauri::WebviewWindow,
    menu: &Menu<tauri::Wry>,
    values: HashMap<String, Value>,
) -> Option<Value> {
    let (tx, rx) = oneshot::channel();
    *app.state::<Choices>().pending.lock().unwrap() = Some((tx, values));

    let shell = app.state::<Shell>();
    shell.menu_open.store(true, Ordering::Relaxed);
    let _ = win.popup_menu(menu);
    // A chosen item answers through `on_menu_event`; dismissing the menu doesn't, so give
    // the answer a moment to arrive and then call it a dismissal.
    let answer = tokio::time::timeout(Duration::from_millis(300), rx).await;
    shell.menu_open.store(false, Ordering::Relaxed);
    app.state::<Choices>().pending.lock().unwrap().take();
    answer.ok().and_then(Result::ok).flatten()
}

/// The session picker of the Analysis page, as a native menu (a <select> can't open
/// reliably in a window that never takes focus): the latest meeting's sessions, then the
/// older meetings as sub-menus. Resolves with the chosen session's key.
#[tauri::command]
pub async fn analysis_pick(
    current_key: Option<i64>,
    app: AppHandle,
    openf1: tauri::State<'_, f1_island::openf1::OpenF1>,
    results: tauri::State<'_, f1_island::results::Results>,
) -> Result<Option<Value>, String> {
    let sessions = results
        .sessions(&openf1, chrono::Utc::now().timestamp_millis())
        .await?;
    let Some(win) = app.get_webview_window(LABEL) else {
        return Ok(None);
    };
    let e = |err: tauri::Error| err.to_string();

    // Group by meeting, keeping newest-first order.
    let mut groups: Vec<(String, Vec<&f1_island::results::SessionRef>)> = Vec::new();
    for s in &sessions {
        match groups.iter_mut().find(|(m, _)| *m == s.meeting) {
            Some((_, list)) => list.push(s),
            None => groups.push((s.meeting.clone(), vec![s])),
        }
    }

    let menu = Menu::new(&app).map_err(e)?;
    let mut values = HashMap::new();
    let item = |s: &f1_island::results::SessionRef| {
        CheckMenuItem::with_id(
            &app,
            format!("choice:{}", s.key),
            &s.name,
            true,
            Some(s.key) == current_key,
            None::<&str>,
        )
    };
    let mut groups = groups.into_iter();
    let Some((latest, latest_list)) = groups.next() else {
        menu.append(
            &MenuItem::with_id(
                &app,
                "none",
                "No completed sessions yet",
                false,
                None::<&str>,
            )
            .map_err(e)?,
        )
        .map_err(e)?;
        let _ = win.popup_menu(&menu);
        return Ok(None);
    };
    menu.append(&MenuItem::with_id(&app, "latest", latest, false, None::<&str>).map_err(e)?)
        .map_err(e)?;
    for s in latest_list {
        values.insert(s.key.to_string(), Value::from(s.key));
        menu.append(&item(s).map_err(e)?).map_err(e)?;
    }
    menu.append(&PredefinedMenuItem::separator(&app).map_err(e)?)
        .map_err(e)?;
    for (meeting, list) in groups {
        let sub = Submenu::new(&app, meeting, true).map_err(e)?;
        for s in list {
            values.insert(s.key.to_string(), Value::from(s.key));
            sub.append(&item(s).map_err(e)?).map_err(e)?;
        }
        menu.append(&sub).map_err(e)?;
    }
    Ok(pop_up(&app, &win, &menu, values).await)
}

// ---------- the tray ----------

/// 16x16 red dot for the tray.
fn tray_icon() -> Image<'static> {
    let size = 16usize;
    let mut rgba = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f64 - 7.5).powi(2) + (y as f64 - 7.5).powi(2)).sqrt();
            let alpha = (7.0 - d).clamp(0.0, 1.0);
            let i = (y * size + x) * 4;
            rgba[i] = 0xe1;
            rgba[i + 1] = 0x06;
            rgba[i + 2] = 0x00;
            rgba[i + 3] = (alpha * 255.0).round() as u8;
        }
    }
    Image::new_owned(rgba, size as u32, size as u32)
}

pub fn create_tray(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let mac = os() == Os::Mac;
    let hotkey_ok = app.state::<Hotkey>().0.load(Ordering::Relaxed);
    let tooltip = if hotkey_ok {
        format!("F1 Island ({} to show/hide)", hotkey_label(app))
    } else {
        "F1 Island".to_string()
    };
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(tray_icon())
        .tooltip(tooltip)
        .menu(&build_menu(app, false)?)
        // On macOS the left click opens the menu. Toggling as well would expand and
        // collapse the island every time someone opens its settings.
        .show_menu_on_left_click(mac)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if os() != Os::Mac {
                    let _ = tray.app_handle().emit("island:toggle", ());
                }
            }
        })
        .build(app)
}

// ---------- the global hotkey ----------

/// The plugin that listens for the hotkey; each press shows or hides the island.
pub fn hotkey_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                let _ = app.emit("island:toggle", ());
            }
        })
        .build()
}

pub fn register_hotkey(app: &AppHandle) {
    let hotkey = app.state::<SettingsStore>().get().hotkey;
    let ok = app.global_shortcut().register(hotkey.as_str()).is_ok();
    app.state::<Hotkey>().0.store(ok, Ordering::Relaxed);
    if !ok {
        eprintln!("Hotkey {hotkey} is unavailable (in use by another app?)");
    }
}
