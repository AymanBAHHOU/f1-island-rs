//! Telling the user a newer version exists: the update chip on the island and the menu
//! entry follow this state. (Port of the "notify" way in the Electron app's
//! `src/updater.js`: ask GitHub for the latest release, and offer its page.)
//!
//! Three ways, by how the app was installed:
//! - **Auto**: the Windows installer. The app downloads a newer signed release in the
//!   background and installs it when asked ("Restart to update"). That part is done by
//!   Tauri's updater plugin (see `self_update.rs`); this file holds what the page shows.
//! - **Notify**: everything else that is packaged (the portable Windows exe, macOS): these
//!   can't replace themselves, so a newer release is announced with a link to its page.
//! - **Off**: a development build.

use crate::platform::Os;
use crate::version::{is_newer, parse_version};
use serde::Serialize;
use serde_json::Value;
use std::sync::Mutex;

pub const REPO: &str = "AymanBAHHOU/f1-island-rs";
pub const RELEASES_URL: &str = "https://github.com/AymanBAHHOU/f1-island-rs/releases/latest";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Off,
    Notify,
    Auto,
}

/// How this copy of the app updates. `installed`: it runs from where the installer put it.
pub fn update_mode(debug_build: bool, os: Os, installed: bool) -> Mode {
    if debug_build {
        Mode::Off
    } else if os == Os::Windows && installed {
        Mode::Auto
    } else {
        Mode::Notify
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Idle,
    Available,
    Downloading,
    Ready,
}

/// What the page's update chip shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub status: Status,
    pub version: Option<String>,
    pub url: Option<String>,
    /// When we last checked, in milliseconds since 1970 (0 = never).
    pub checked_at: i64,
}

impl Default for State {
    fn default() -> Self {
        State {
            status: Status::Idle,
            version: None,
            url: None,
            checked_at: 0,
        }
    }
}

/// GitHub's "latest release" JSON as (version, page), or `None` for anything that isn't a
/// published, final release.
pub fn release_info(json: &Value) -> Option<(String, String)> {
    if json["draft"].as_bool() == Some(true) || json["prerelease"].as_bool() == Some(true) {
        return None;
    }
    let tag = json["tag_name"].as_str()?;
    parse_version(tag)?;
    let url = json["html_url"]
        .as_str()
        .filter(|u| u.starts_with("https://github.com/"))
        .unwrap_or(RELEASES_URL);
    Some((tag.trim_start_matches('v').to_string(), url.to_string()))
}

/// The state after a check that found `latest` (or nothing), running version `current`.
pub fn after_check(latest: Option<(String, String)>, current: &str, now_ms: i64) -> State {
    match latest {
        Some((version, url)) if is_newer(&version, current) => State {
            status: Status::Available,
            version: Some(version),
            url: Some(url),
            checked_at: now_ms,
        },
        _ => State {
            status: Status::Idle,
            checked_at: now_ms,
            ..State::default()
        },
    }
}

/// What choosing the chip or menu entry does.
#[derive(Debug, PartialEq)]
pub enum Act {
    /// Open this page in the browser.
    Open(String),
    /// Look for an update now.
    Check,
    /// Install the downloaded update and restart.
    Install,
    Nothing,
}

pub fn act(state: &State) -> Act {
    match (state.status, &state.url) {
        (Status::Ready, _) => Act::Install,
        (Status::Available, Some(url)) => Act::Open(url.clone()),
        (Status::Idle, _) => Act::Check,
        _ => Act::Nothing,
    }
}

pub struct Updater {
    current: String,
    http: reqwest::Client,
    state: Mutex<State>,
    mode: Mutex<Mode>,
}

impl Updater {
    pub fn new(current: &str) -> Self {
        Updater {
            current: current.to_string(),
            http: reqwest::Client::builder()
                .user_agent("F1Island")
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("a plain HTTP client can always be built"),
            state: Mutex::new(State::default()),
            mode: Mutex::new(Mode::Off),
        }
    }

    pub fn mode(&self) -> Mode {
        *self.mode.lock().unwrap()
    }

    pub fn set_mode(&self, mode: Mode) {
        *self.mode.lock().unwrap() = mode;
    }

    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    pub fn set(&self, state: State) {
        *self.state.lock().unwrap() = state;
    }

    /// Asks GitHub for the latest release. Offline, rate limited or no release yet: `None`,
    /// and the next check tries again.
    pub async fn check(&self, now_ms: i64) -> Option<State> {
        if matches!(self.state().status, Status::Downloading | Status::Ready) {
            return None;
        }
        let response = self
            .http
            .get(format!(
                "https://api.github.com/repos/{REPO}/releases/latest"
            ))
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?;
        let json: Value = response.json().await.ok()?;
        let state = after_check(release_info(&json), &self.current, now_ms);
        self.set(state.clone());
        Some(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_final_releases_count() {
        let ok =
            json!({"tag_name": "v0.2.0", "html_url": "https://github.com/x/y/releases/tag/v0.2.0"});
        assert_eq!(
            release_info(&ok),
            Some((
                "0.2.0".into(),
                "https://github.com/x/y/releases/tag/v0.2.0".into()
            ))
        );
        assert_eq!(
            release_info(&json!({"tag_name": "v0.2.0", "draft": true})),
            None
        );
        assert_eq!(
            release_info(&json!({"tag_name": "v0.2.0", "prerelease": true})),
            None
        );
        assert_eq!(release_info(&json!({"tag_name": "nightly"})), None);
        assert_eq!(release_info(&json!({})), None);
    }

    #[test]
    fn a_page_outside_github_is_replaced_by_the_releases_page() {
        let evil = json!({"tag_name": "1.0.0", "html_url": "https://evil.example/x"});
        assert_eq!(release_info(&evil).unwrap().1, RELEASES_URL);
    }

    #[test]
    fn a_newer_release_is_available_and_an_older_one_is_not() {
        let release = Some(("0.2.0".to_string(), "https://github.com/a/b".to_string()));
        let newer = after_check(release.clone(), "0.1.0", 5);
        assert_eq!(newer.status, Status::Available);
        assert_eq!(newer.version.as_deref(), Some("0.2.0"));
        assert_eq!(newer.checked_at, 5);
        let same = after_check(release, "0.2.0", 6);
        assert_eq!(same.status, Status::Idle);
        assert_eq!(same.checked_at, 6);
        assert_eq!(after_check(None, "0.1.0", 7).status, Status::Idle);
    }

    #[test]
    fn choosing_the_chip_does_the_right_thing() {
        let available = after_check(
            Some(("0.2.0".into(), "https://github.com/a/b".into())),
            "0.1.0",
            1,
        );
        assert_eq!(act(&available), Act::Open("https://github.com/a/b".into()));
        assert_eq!(act(&State::default()), Act::Check);
        let ready = State {
            status: Status::Ready,
            ..State::default()
        };
        assert_eq!(act(&ready), Act::Install);
        let busy = State {
            status: Status::Downloading,
            ..State::default()
        };
        assert_eq!(act(&busy), Act::Nothing);
    }

    #[test]
    fn how_an_app_updates_depends_on_how_it_was_installed() {
        assert_eq!(update_mode(true, Os::Windows, true), Mode::Off);
        assert_eq!(update_mode(false, Os::Windows, true), Mode::Auto);
        assert_eq!(update_mode(false, Os::Windows, false), Mode::Notify); // portable
        assert_eq!(update_mode(false, Os::Mac, true), Mode::Notify);
        assert_eq!(update_mode(false, Os::Linux, true), Mode::Notify);
    }

    #[test]
    fn the_page_sees_lowercase_status_and_camel_case_names() {
        let json = serde_json::to_value(State::default()).unwrap();
        assert_eq!(json["status"], "idle");
        assert_eq!(json["checkedAt"], 0);
    }
}
