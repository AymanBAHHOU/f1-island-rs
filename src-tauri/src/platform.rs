//! Small differences between operating systems that the rest of the app asks about.
//! (Port of the helpers in the Electron app's `src/platform.js`.)

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Os {
    Windows,
    Mac,
    Linux,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

/// "CommandOrControl+Alt+F" the way the platform writes it ("Ctrl+Alt+F", "Cmd+Option+F").
pub fn format_hotkey(hotkey: &str, os: Os) -> String {
    let mac = os == Os::Mac;
    hotkey
        .replace("CommandOrControl", if mac { "Cmd" } else { "Ctrl" })
        .replace("Command", "Cmd")
        .replace("Control", "Ctrl")
        .replace("Alt", if mac { "Option" } else { "Alt" })
}

/// The menu entry's name for starting with the system.
pub fn login_item_label(os: Os) -> &'static str {
    match os {
        Os::Mac => "Start at Login",
        _ => "Start with Windows",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys_are_written_the_way_the_platform_does() {
        let key = "CommandOrControl+Alt+F";
        assert_eq!(format_hotkey(key, Os::Windows), "Ctrl+Alt+F");
        assert_eq!(format_hotkey(key, Os::Linux), "Ctrl+Alt+F");
        assert_eq!(format_hotkey(key, Os::Mac), "Cmd+Option+F");
        assert_eq!(format_hotkey("Command+Shift+K", Os::Windows), "Cmd+Shift+K");
        assert_eq!(format_hotkey("Control+Space", Os::Mac), "Ctrl+Space");
    }

    #[test]
    fn the_login_entry_is_named_for_the_system() {
        assert_eq!(login_item_label(Os::Mac), "Start at Login");
        assert_eq!(login_item_label(Os::Windows), "Start with Windows");
    }
}
