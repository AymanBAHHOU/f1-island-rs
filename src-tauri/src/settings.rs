//! The user's settings: a small JSON file in the app's data folder.
//! (Port of the Electron app's `src/settings.js`; the file has the same names, so a
//! settings file from the Electron app works here.)

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Hide the island until the pointer comes to the top of the screen.
    pub auto_hide: bool,
    /// Windows/Linux: attach the island flush to the top edge, centred.
    pub docked: bool,
    /// Updated by dragging; not a manual toggle.
    pub notch_attached: bool,
    pub hotkey: String,
    /// Window position; `None` means top-centre of the primary display.
    pub position: Option<Position>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            auto_hide: false,
            docked: false,
            notch_attached: true,
            hotkey: "CommandOrControl+Alt+F".to_string(),
            position: None,
        }
    }
}

/// Reads settings from file text. Anything unreadable gives the defaults.
pub fn parse(text: &str) -> Settings {
    let Ok(saved) = serde_json::from_str::<serde_json::Value>(text) else {
        return Settings::default();
    };
    let mut settings: Settings = serde_json::from_value(saved.clone()).unwrap_or_default();
    // Preserve an older manual detachment, but allow magnetic docking now.
    if saved.get("notchAttached").is_none() && saved.get("attachNotch") == Some(&false.into()) {
        settings.notch_attached = false;
    }
    settings
}

pub struct SettingsStore {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl SettingsStore {
    pub fn load(path: PathBuf) -> Self {
        let current = std::fs::read_to_string(&path)
            .map(|text| parse(&text))
            .unwrap_or_default();
        SettingsStore {
            path,
            current: Mutex::new(current),
        }
    }

    pub fn get(&self) -> Settings {
        self.current.lock().unwrap().clone()
    }

    /// Changes the settings and saves them. Failing to save is not fatal: the change
    /// still holds until the app closes.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) -> Settings {
        let mut current = self.current.lock().unwrap();
        change(&mut current);
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&*current) {
            let _ = std::fs::write(&self.path, text);
        }
        current.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_broken_files_give_the_defaults() {
        assert_eq!(parse("not json"), Settings::default());
        assert_eq!(parse("{}"), Settings::default());
        assert_eq!(parse(r#"{"autoHide": "yes"}"#), Settings::default());
    }

    #[test]
    fn saved_values_win_and_the_rest_are_default() {
        let s = parse(r#"{"autoHide": true, "position": {"x": 10, "y": 20}}"#);
        assert!(s.auto_hide);
        assert_eq!(s.position, Some(Position { x: 10.0, y: 20.0 }));
        assert!(s.notch_attached);
        assert_eq!(s.hotkey, "CommandOrControl+Alt+F");
    }

    #[test]
    fn an_older_manual_detachment_is_kept() {
        assert!(!parse(r#"{"attachNotch": false}"#).notch_attached);
        assert!(parse(r#"{"attachNotch": false, "notchAttached": true}"#).notch_attached);
    }

    #[test]
    fn updates_are_saved_and_loaded_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("settings.json");
        let store = SettingsStore::load(path.clone());
        assert_eq!(store.get(), Settings::default());
        store.update(|s| {
            s.docked = true;
            s.position = Some(Position { x: 1.0, y: 2.0 });
        });
        let again = SettingsStore::load(path);
        assert!(again.get().docked);
        assert_eq!(again.get().position, Some(Position { x: 1.0, y: 2.0 }));
    }

    #[test]
    fn the_page_sees_camel_case_names() {
        let json = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(json["autoHide"], false);
        assert_eq!(json["notchAttached"], true);
        assert!(json["position"].is_null());
    }
}
