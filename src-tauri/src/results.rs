//! Which sessions have finished this season, and the top three of each one of this
//! weekend's (for the Weekend page). Data from OpenF1.
//! (Port of `listSessions`, `podiumOf` and `getWeekendResults` in the Electron app's
//! `src/analysis.js`.)

use crate::openf1::{OpenF1, FINAL_AFTER_MS};
use chrono::{DateTime, Datelike, Utc};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const SESSIONS_TTL: Duration = Duration::from_secs(10 * 60);
const RESULTS_TTL: Duration = Duration::from_secs(5 * 60);

// ---------- reading OpenF1's rows ----------

fn text(row: &Value, key: &str) -> Option<String> {
    row.get(key)?.as_str().map(str::to_string)
}

fn rows(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

/// A time from OpenF1 (`2025-10-19T19:00:00+00:00`) as unix milliseconds.
pub fn parse_ms(iso: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|t| t.timestamp_millis())
}

#[derive(Debug, Clone, PartialEq)]
pub struct DriverInfo {
    pub number: i64,
    pub code: String,
    pub name: String,
    pub team: String,
    pub color: String,
}

pub fn driver_info(drivers: &Value) -> HashMap<i64, DriverInfo> {
    rows(drivers)
        .iter()
        .filter_map(|d| {
            let number = d.get("driver_number")?.as_i64()?;
            let code = text(d, "name_acronym").unwrap_or_else(|| format!("#{number}"));
            let name = text(d, "full_name")
                .or_else(|| text(d, "broadcast_name"))
                .unwrap_or_else(|| code.clone());
            Some((
                number,
                DriverInfo {
                    number,
                    code,
                    name,
                    team: text(d, "team_name").unwrap_or_else(|| "Unknown".to_string()),
                    color: text(d, "team_colour")
                        .map_or_else(|| "#888888".to_string(), |c| format!("#{c}")),
                },
            ))
        })
        .collect()
}

// ---------- the season's finished sessions ----------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionRef {
    pub key: i64,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub start: String,
    pub end: String,
    pub meeting: String,
    pub location: String,
}

/// Finished, not cancelled, newest first.
pub fn finished_sessions(sessions: &Value, meetings: &Value, now_ms: i64) -> Vec<SessionRef> {
    let meeting_name: HashMap<i64, String> = rows(meetings)
        .iter()
        .filter_map(|m| Some((m.get("meeting_key")?.as_i64()?, text(m, "meeting_name")?)))
        .collect();
    let mut out: Vec<SessionRef> = rows(sessions)
        .iter()
        .filter(|s| s.get("is_cancelled").and_then(Value::as_bool) != Some(true))
        .filter_map(|s| {
            let end = text(s, "date_end")?;
            if parse_ms(&end)? >= now_ms {
                return None;
            }
            let meeting = s
                .get("meeting_key")
                .and_then(Value::as_i64)
                .and_then(|k| meeting_name.get(&k).cloned())
                .or_else(|| text(s, "country_name"))
                .unwrap_or_default();
            Some(SessionRef {
                key: s.get("session_key")?.as_i64()?,
                name: text(s, "session_name")?,
                kind: text(s, "session_type").unwrap_or_default(),
                start: text(s, "date_start")?,
                end,
                meeting,
                location: text(s, "location").unwrap_or_default(),
            })
        })
        .collect();
    out.sort_by(|a, b| b.start.cmp(&a.start));
    out
}

// ---------- a session's top three ----------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Podium {
    pub pos: i64,
    pub code: String,
    pub color: String,
    /// The winner's time, for sessions that are not races.
    pub time: Option<f64>,
    /// Seconds behind the winner, or text such as "+1 LAP".
    pub gap: Option<Value>,
}

/// Qualifying gives one value per phase (Q1, Q2, Q3): the last one reached counts.
fn last_phase(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::Array(items) => items.iter().rfind(|v| !v.is_null()).cloned(),
        Value::Null => None,
        other => Some(other.clone()),
    }
}

pub fn podium_of(results: &Value, drivers: &Value, session_name: &str) -> Vec<Podium> {
    let info = driver_info(drivers);
    let is_race = matches!(session_name, "Race" | "Sprint");
    let mut top: Vec<Podium> = rows(results)
        .iter()
        .filter_map(|r| {
            let pos = r
                .get("position")?
                .as_i64()
                .filter(|p| (1..=3).contains(p))?;
            let number = r.get("driver_number")?.as_i64()?;
            let driver = info.get(&number);
            let time = last_phase(r.get("duration")).and_then(|v| v.as_f64());
            Some(Podium {
                pos,
                code: driver.map_or_else(|| format!("#{number}"), |d| d.code.clone()),
                color: driver.map_or_else(|| "#888888".to_string(), |d| d.color.clone()),
                time: if pos == 1 && !is_race { time } else { None },
                gap: if pos == 1 {
                    None
                } else {
                    last_phase(r.get("gap_to_leader"))
                },
            })
        })
        .collect();
    top.sort_by_key(|p| p.pos);
    top
}

// ---------- fetching it ----------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Entry {
    pub key: i64,
    pub name: String,
    pub start: String,
    pub top: Vec<Podium>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unavailable: bool,
}

#[derive(Default)]
pub struct Results {
    sessions: Mutex<Option<(Instant, Vec<SessionRef>)>>,
    /// session key -> (when fetched, can no longer change, what we got)
    entries: Mutex<HashMap<i64, (Instant, bool, Entry)>>,
}

impl Results {
    /// The season's finished sessions, newest first.
    pub async fn sessions(&self, openf1: &OpenF1, now_ms: i64) -> Result<Vec<SessionRef>, String> {
        if let Some((at, list)) = &*self.sessions.lock().unwrap() {
            if at.elapsed() < SESSIONS_TTL {
                return Ok(list.clone());
            }
        }
        let year = DateTime::<Utc>::from_timestamp_millis(now_ms).map_or(2026, |t| t.year());
        let sessions = openf1.get(&format!("sessions?year={year}"), None).await?;
        let meetings = openf1.get(&format!("meetings?year={year}"), None).await?;
        let list = finished_sessions(&sessions, &meetings, now_ms);
        *self.sessions.lock().unwrap() = Some((Instant::now(), list.clone()));
        Ok(list)
    }

    /// The top three of each finished session between `start_ms` and `end_ms`.
    pub async fn weekend(
        &self,
        openf1: &OpenF1,
        start_ms: i64,
        end_ms: i64,
        now_ms: i64,
    ) -> Result<Vec<Entry>, String> {
        let t0 = start_ms - 6 * 3600 * 1000;
        let t1 = end_ms + 24 * 3600 * 1000;
        let mut these: Vec<SessionRef> = self
            .sessions(openf1, now_ms)
            .await?
            .into_iter()
            .filter(|s| parse_ms(&s.start).is_some_and(|t| (t0..=t1).contains(&t)))
            .collect();
        these.sort_by(|a, b| a.start.cmp(&b.start));

        let mut out = Vec::new();
        for s in these {
            let hit = self.entries.lock().unwrap().get(&s.key).cloned();
            if let Some((at, final_, entry)) = &hit {
                if *final_ || at.elapsed() < RESULTS_TTL {
                    out.push(entry.clone());
                    continue;
                }
            }
            let ended = parse_ms(&s.end).unwrap_or(now_ms);
            let after = Some(ended + FINAL_AFTER_MS);
            let fetched = async {
                let results = openf1
                    .get(
                        &format!("session_result?session_key={}&position%3C%3D3", s.key),
                        after,
                    )
                    .await?;
                let drivers = if rows(&results).is_empty() {
                    Value::Array(Vec::new())
                } else {
                    openf1
                        .get(&format!("drivers?session_key={}", s.key), after)
                        .await?
                };
                Ok::<_, String>(podium_of(&results, &drivers, &s.name))
            }
            .await;
            match fetched {
                Ok(top) => {
                    // Classifications can still change for a while (penalties).
                    let final_ = !top.is_empty() && now_ms - ended > FINAL_AFTER_MS;
                    let entry = Entry {
                        key: s.key,
                        name: s.name.clone(),
                        start: s.start.clone(),
                        top,
                        unavailable: false,
                    };
                    self.entries
                        .lock()
                        .unwrap()
                        .insert(s.key, (Instant::now(), final_, entry.clone()));
                    out.push(entry);
                }
                Err(_) => out.push(hit.map(|(_, _, e)| e).unwrap_or(Entry {
                    key: s.key,
                    name: s.name,
                    start: s.start,
                    top: Vec::new(),
                    unavailable: true,
                })),
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lists_finished_sessions_newest_first() {
        let sessions = json!([
            {"session_key": 1, "session_name": "Practice 1", "session_type": "Practice", "meeting_key": 10,
             "date_start": "2026-10-23T17:30:00+00:00", "date_end": "2026-10-23T18:30:00+00:00", "location": "Austin"},
            {"session_key": 2, "session_name": "Race", "session_type": "Race", "meeting_key": 10,
             "date_start": "2026-10-25T19:00:00+00:00", "date_end": "2026-10-25T21:00:00+00:00", "location": "Austin"},
            {"session_key": 3, "session_name": "Sprint", "session_type": "Race", "meeting_key": 10, "is_cancelled": true,
             "date_start": "2026-10-24T14:00:00+00:00", "date_end": "2026-10-24T14:30:00+00:00"},
            {"session_key": 4, "session_name": "Qualifying", "session_type": "Qualifying", "country_name": "USA",
             "date_start": "2026-10-24T21:00:00+00:00", "date_end": "2026-10-24T22:00:00+00:00", "location": "Austin"},
        ]);
        let meetings = json!([{"meeting_key": 10, "meeting_name": "United States Grand Prix"}]);
        let now = parse_ms("2026-10-25T10:00:00+00:00").unwrap();
        let list = finished_sessions(&sessions, &meetings, now);
        let keys: Vec<i64> = list.iter().map(|s| s.key).collect();
        assert_eq!(keys, vec![4, 1]); // race not over, sprint cancelled
        assert_eq!(list[1].meeting, "United States Grand Prix");
        assert_eq!(list[0].meeting, "USA"); // no meeting row: falls back to the country
    }

    #[test]
    fn podium_for_a_non_race_shows_the_winners_time() {
        let results = json!([
            {"position": 2, "driver_number": 4, "duration": [90.1, 89.5, 89.0], "gap_to_leader": [0.5, 0.4, 0.2]},
            {"position": 1, "driver_number": 1, "duration": [89.9, 89.1, 88.8], "gap_to_leader": [0, 0, 0]},
            {"position": 3, "driver_number": 99, "duration": 91.0, "gap_to_leader": "+1 LAP"},
            {"position": 4, "driver_number": 5, "duration": 92.0},
        ]);
        let drivers = json!([
            {"driver_number": 1, "name_acronym": "VER", "team_colour": "3671C6"},
            {"driver_number": 4, "name_acronym": "NOR"},
        ]);
        let top = podium_of(&results, &drivers, "Qualifying");
        assert_eq!(top.len(), 3);
        assert_eq!((top[0].code.as_str(), top[0].time), ("VER", Some(88.8)));
        assert_eq!(top[0].color, "#3671C6");
        assert_eq!(top[0].gap, None);
        assert_eq!(top[1].code, "NOR");
        assert_eq!(top[1].gap, Some(json!(0.2)));
        assert_eq!(top[1].color, "#888888");
        assert_eq!(top[2].code, "#99"); // unknown driver
        assert_eq!(top[2].gap, Some(json!("+1 LAP")));
    }

    #[test]
    fn a_races_winner_has_no_time() {
        let results = json!([{"position": 1, "driver_number": 1, "duration": 5400.0}]);
        let top = podium_of(&results, &json!([]), "Race");
        assert_eq!(top[0].time, None);
    }
}
