//! A client for OpenF1 (https://openf1.org): timing, laps, stints, positions and more.
//! (Port of the `openf1()` helper and disk cache in the Electron app's `src/analysis.js`.)
//!
//! OpenF1 allows about three requests a second, so requests go one at a time with a gap.
//! While a real session is live it refuses free requests, even for old sessions, so every
//! answer is also kept on disk and used when the network says no.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

const BASE: &str = "https://api.openf1.org/v1";
const GAP: Duration = Duration::from_millis(350);
const MAX_RETRIES: u32 = 5;

/// A session's data stops changing a couple of hours after it ends. A copy saved after
/// that is final: it is used without asking the network.
pub const FINAL_AFTER_MS: i64 = 2 * 3600 * 1000;

pub const LIVE_LOCK_MESSAGE: &str = "Session analysis pauses while a session is live: the free timing data is members-only until it ends. Sessions you've opened before still work; the rest comes back after the session.";

/// A stable file name for a query (FNV-1a, so it is the same on every run).
fn cache_name(query: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in query.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}.json")
}

/// Positions of every car at once are far too big to keep.
fn keep_on_disk(query: &str) -> bool {
    !(query.starts_with("location?") && !query.contains("driver_number"))
}

/// How long to wait after a 429: what the server asked for, else 2, 4, 8... seconds.
fn retry_wait(retry_after: Option<&str>, attempt: u32) -> Duration {
    match retry_after.and_then(|v| v.trim().parse::<u64>().ok()) {
        Some(seconds) if seconds > 0 => Duration::from_secs(seconds),
        _ => Duration::from_secs(2 * 2u64.pow(attempt)),
    }
}

pub struct OpenF1 {
    http: reqwest::Client,
    /// Held while a request is in flight (and for the gap after), so requests queue up.
    gate: tokio::sync::Mutex<()>,
    cache_dir: Mutex<Option<PathBuf>>,
}

impl Default for OpenF1 {
    fn default() -> Self {
        OpenF1 {
            http: reqwest::Client::builder()
                .user_agent("F1Island/0.1")
                .timeout(Duration::from_secs(20))
                .build()
                .expect("a plain HTTP client can always be built"),
            gate: tokio::sync::Mutex::new(()),
            cache_dir: Mutex::new(None),
        }
    }
}

impl OpenF1 {
    /// Where to keep answers. If the folder can't be made, nothing is kept.
    pub fn set_cache_dir(&self, dir: &Path) {
        let ok = std::fs::create_dir_all(dir).is_ok();
        *self.cache_dir.lock().unwrap() = ok.then(|| dir.to_path_buf());
    }

    fn cache_file(&self, query: &str) -> Option<PathBuf> {
        let dir = self.cache_dir.lock().unwrap();
        dir.as_ref().map(|d| d.join(cache_name(query)))
    }

    fn read_cache(&self, query: &str) -> Option<Value> {
        let text = std::fs::read_to_string(self.cache_file(query)?).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn write_cache(&self, query: &str, data: &Value) {
        if !keep_on_disk(query) {
            return;
        }
        if let Some(file) = self.cache_file(query) {
            let _ = std::fs::write(file, data.to_string());
        }
    }

    /// A saved copy made after `final_after_ms` (unix ms) never changes: use it as is.
    fn final_copy(&self, query: &str, final_after_ms: Option<i64>) -> Option<Value> {
        let after = final_after_ms?;
        let modified = std::fs::metadata(self.cache_file(query)?)
            .ok()?
            .modified()
            .ok()?;
        let saved_ms = modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_millis() as i64;
        if saved_ms <= after {
            return None;
        }
        self.read_cache(query)
    }

    /// Runs a query such as `laps?session_key=9161`. The answer is a JSON array of rows
    /// (empty when OpenF1 has none). `final_after_ms` is described at `final_copy`.
    pub async fn get(&self, query: &str, final_after_ms: Option<i64>) -> Result<Value, String> {
        if let Some(saved) = self.final_copy(query, final_after_ms) {
            return Ok(saved);
        }
        let _turn = self.gate.lock().await;
        let url = format!("{BASE}/{query}");

        for attempt in 0..=MAX_RETRIES {
            let response = match self.http.get(&url).send().await {
                Ok(response) => response,
                // Offline: fall back to what we have.
                Err(err) => return self.read_cache(query).ok_or_else(|| err.to_string()),
            };
            tokio::time::sleep(GAP).await;

            let status = response.status();
            if status.as_u16() == 429 && attempt < MAX_RETRIES {
                let header = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok());
                tokio::time::sleep(retry_wait(header, attempt)).await;
                continue;
            }
            if status.as_u16() == 404 {
                return Ok(Value::Array(Vec::new())); // OpenF1's "no rows"
            }
            if !status.is_success() {
                if let Some(saved) = self.read_cache(query) {
                    return Ok(saved);
                }
                let body = response.text().await.unwrap_or_default();
                if status.as_u16() == 401 && body.to_lowercase().contains("live f1 session") {
                    return Err(LIVE_LOCK_MESSAGE.to_string());
                }
                return Err(format!(
                    "Timing data unavailable right now (error {}). Try again in a minute.",
                    status.as_u16()
                ));
            }
            let data: Value = response.json().await.map_err(|e| e.to_string())?;
            self.write_cache(query, &data);
            return Ok(data);
        }
        Err("Timing data unavailable right now. Try again in a minute.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cache_names_are_stable_and_differ_per_query() {
        assert_eq!(
            cache_name("laps?session_key=1"),
            cache_name("laps?session_key=1")
        );
        assert_ne!(
            cache_name("laps?session_key=1"),
            cache_name("laps?session_key=2")
        );
        assert_eq!(cache_name("a"), "af63dc4c8601ec8c.json"); // FNV-1a of "a"
    }

    #[test]
    fn all_car_positions_are_not_kept() {
        assert!(!keep_on_disk("location?session_key=1&date>=x"));
        assert!(keep_on_disk("location?session_key=1&driver_number=44"));
        assert!(keep_on_disk("laps?session_key=1"));
    }

    #[test]
    fn waits_as_asked_or_doubles() {
        assert_eq!(retry_wait(Some("7"), 0), Duration::from_secs(7));
        assert_eq!(retry_wait(None, 0), Duration::from_secs(2));
        assert_eq!(retry_wait(Some("soon"), 2), Duration::from_secs(8));
        assert_eq!(retry_wait(Some("0"), 1), Duration::from_secs(4));
    }

    #[test]
    fn saved_answers_round_trip_and_final_ones_skip_the_network() {
        let dir = tempfile::tempdir().unwrap();
        let client = OpenF1::default();
        client.set_cache_dir(dir.path());
        let rows = json!([{"lap_number": 1}]);
        client.write_cache("laps?session_key=1", &rows);
        assert_eq!(client.read_cache("laps?session_key=1"), Some(rows.clone()));
        assert_eq!(client.read_cache("laps?session_key=2"), None);

        // saved just now: final if the session ended long ago, not if it ends in the future
        assert_eq!(client.final_copy("laps?session_key=1", Some(0)), Some(rows));
        let future = chrono::Utc::now().timestamp_millis() + 3_600_000;
        assert_eq!(client.final_copy("laps?session_key=1", Some(future)), None);
        assert_eq!(client.final_copy("laps?session_key=1", None), None);
    }

    #[test]
    fn big_responses_are_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let client = OpenF1::default();
        client.set_cache_dir(dir.path());
        client.write_cache("location?session_key=1&date>=x", &json!([1]));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
