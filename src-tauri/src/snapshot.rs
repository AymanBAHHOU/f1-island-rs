//! Everything the island shows besides live timing, in one answer: the weekend, the last
//! race, the standings (fresher ones from OpenF1 when Jolpica lags), the title fight and
//! the circuit's history. (Port of `getSnapshot` in the Electron app's `src/data.js`.)

use crate::championship::{self, DriverIn, TeamIn, TitleFight};
use crate::jolpica::Jolpica;
use crate::openf1::OpenF1;
use crate::schedule::pick_weekend;
use crate::title::{build_fresher, events_after, latest_finished_race, standings_as_of, Fresher};
use chrono::{DateTime, Datelike, SecondsFormat, Utc};
use serde_json::{json, Map, Value};
use std::sync::Mutex;
use std::time::Duration;

const MIN: u64 = 60;

/// Remembers the last simulation: it takes a moment, and only changes when its inputs do.
#[derive(Default)]
pub struct Snapshots {
    title: Mutex<Option<(String, TitleFight)>>,
}

fn iso(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The weekend as the page wants it: each session also carries `start` and `end` as ISO
/// text (the page parses those).
fn weekend_json(weekend: &impl serde::Serialize) -> Value {
    let mut value = serde_json::to_value(weekend).unwrap_or(Value::Null);
    if let Some(sessions) = value.get_mut("sessions").and_then(Value::as_array_mut) {
        for session in sessions {
            let start = session["startMs"].as_i64();
            let end = session["endMs"].as_i64();
            if let (Some(start), Some(end)) = (start, end) {
                session["start"] = json!(iso(start));
                session["end"] = json!(iso(end));
            }
        }
    }
    value
}

/// OpenF1's championship after the latest finished Sprint or race, if it is newer than
/// `as_of_ms`. Anything going wrong (OpenF1 is locked during a live session) just means
/// "keep Jolpica's".
async fn fresher_standings(openf1: &OpenF1, now_ms: i64, as_of_ms: i64) -> Option<Fresher> {
    let year = DateTime::<Utc>::from_timestamp_millis(now_ms)?.year();
    let sessions = openf1
        .get_for(
            &format!("sessions?year={year}&session_type=Race"),
            Duration::from_secs(30 * MIN),
        )
        .await
        .ok()?;
    let latest = latest_finished_race(&sessions, now_ms, as_of_ms)?;

    // One at a time: OpenF1 allows only a few requests per second.
    let q = format!("session_key={}", latest.key);
    let champ = openf1
        .get_for(
            &format!("championship_drivers?{q}"),
            Duration::from_secs(10 * MIN),
        )
        .await
        .ok()?;
    let teams = openf1
        .get_for(
            &format!("championship_teams?{q}"),
            Duration::from_secs(10 * MIN),
        )
        .await
        .ok()?;
    let drivers = openf1
        .get_for(&format!("drivers?{q}"), Duration::from_secs(60 * MIN))
        .await
        .ok()?;
    let meeting = match latest.meeting_key {
        Some(key) => openf1
            .get_for(
                &format!("meetings?meeting_key={key}"),
                Duration::from_secs(12 * 60 * MIN),
            )
            .await
            .ok(),
        None => None,
    };
    let meeting = meeting.as_ref().and_then(|m| m.get(0));
    build_fresher(&latest, &champ, &teams, &drivers, meeting)
}

impl Snapshots {
    /// `extras: false` skips the slow parts (title fight, circuit history) so the island
    /// can show the weekend first; the answer then says `extrasPending`.
    pub async fn build(
        &self,
        jolpica: &Jolpica,
        openf1: &OpenF1,
        now_ms: i64,
        extras: bool,
    ) -> Value {
        let races = jolpica.races().await;
        let standings = jolpica.standings().await;

        let mut snap = Map::new();
        snap.insert("fetchedAt".into(), json!(iso(now_ms)));
        snap.insert("weekend".into(), Value::Null);
        snap.insert("lastRace".into(), Value::Null);
        snap.insert("standings".into(), json!([]));
        snap.insert("constructors".into(), json!([]));
        snap.insert("circuit".into(), Value::Null);
        snap.insert("title".into(), Value::Null);
        snap.insert("error".into(), Value::Null);

        let weekend = match &races {
            Ok(races) => pick_weekend(races, now_ms),
            Err(err) => {
                snap.insert(
                    "error".into(),
                    json!(format!("Schedule unavailable ({err})")),
                );
                None
            }
        };
        if let Some(w) = &weekend {
            snap.insert("weekend".into(), weekend_json(w));
        }

        // The full field and when these standings were true, for the title fight.
        let mut title_drivers: Vec<DriverIn> = Vec::new();
        let mut title_teams: Vec<TeamIn> = Vec::new();
        let mut as_of_ms = 0;
        let mut last_race_round = 0;

        if let Ok(standings) = &standings {
            if let Some(last) = &standings.last_race {
                last_race_round = last.round.parse().unwrap_or(0);
                snap.insert("lastRace".into(), json!(last));
            }
            if let Some(table) = &standings.drivers {
                snap.insert("standings".into(), json!(table.rows));
                title_drivers = table
                    .all
                    .iter()
                    .map(|d| DriverIn {
                        code: d.code.clone(),
                        points: d.points,
                        team: d.team.clone(),
                        team_id: d.team_id.clone(),
                        color: None,
                    })
                    .collect();
            }
            snap.insert("constructors".into(), json!(standings.constructors));
            title_teams = standings
                .constructors
                .iter()
                .map(|c| TeamIn {
                    name: c.name.clone(),
                    points: c.points.parse().unwrap_or(0.0),
                    team_id: Some(c.team_id.clone()),
                    color: None,
                })
                .collect();

            // Jolpica can lag hours behind a Sprint or race; prefer OpenF1's championship
            // when it reflects a later session.
            if let Ok(races) = &races {
                let round = standings.drivers.as_ref().map_or("", |t| t.round.as_str());
                as_of_ms = standings_as_of(round, races, last_race_round);
                if let Some(fresher) = fresher_standings(openf1, now_ms, as_of_ms).await {
                    snap.insert("standings".into(), Value::Array(fresher.standings));
                    snap.insert("constructors".into(), Value::Array(fresher.constructors));
                    snap.insert("standingsNote".into(), json!(fresher.note));
                    title_drivers = fresher.drivers;
                    title_teams = fresher.teams;
                    as_of_ms = fresher.as_of_ms;
                }
            }
        }

        if !extras {
            snap.insert("extrasPending".into(), json!(true));
            return Value::Object(snap);
        }

        if let (Ok(races), false) = (&races, title_drivers.is_empty()) {
            if let Some(title) = self
                .title_fight(jolpica, races, title_drivers, title_teams, as_of_ms)
                .await
            {
                snap.insert("title".into(), json!(title));
            }
            // else: an optional extra; the Title page says it's unavailable.
        }

        if let Some(w) = &weekend {
            if let Ok(season) = w.season.parse() {
                if let Ok(circuit) = jolpica.circuit_history(&w.circuit_id, season).await {
                    snap.insert("circuit".into(), json!(circuit));
                }
            }
        }

        Value::Object(snap)
    }

    async fn title_fight(
        &self,
        jolpica: &Jolpica,
        races: &[crate::schedule::Race],
        drivers: Vec<DriverIn>,
        teams: Vec<TeamIn>,
        as_of_ms: i64,
    ) -> Option<TitleFight> {
        let events = events_after(races, as_of_ms);
        let mut history = jolpica.season_results(false).await.ok()?;
        history.extend(jolpica.season_results(true).await.ok()?);

        let key = serde_json::to_string(&json!([
            drivers
                .iter()
                .map(|d| (&d.code, d.points))
                .collect::<Vec<_>>(),
            teams.iter().map(|t| t.points).collect::<Vec<_>>(),
            events.len(),
            history.len(),
        ]))
        .ok()?;
        if let Some((cached_key, result)) = &*self.title.lock().unwrap() {
            if *cached_key == key {
                return Some(result.clone());
            }
        }

        // About a second of arithmetic: keep it off the thread that serves the page.
        let result = tokio::task::spawn_blocking(move || {
            championship::title_fight(&drivers, &teams, &events, &history, championship::RUNS)
        })
        .await
        .ok()?;
        *self.title.lock().unwrap() = Some((key, result.clone()));
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::{Session, Weekend};

    #[test]
    fn sessions_carry_iso_start_and_end_for_the_page() {
        let weekend = Weekend {
            season: "2026".into(),
            round: "18".into(),
            name: "United States Grand Prix".into(),
            circuit_id: "americas".into(),
            circuit: "COTA".into(),
            locality: "Austin".into(),
            country: "USA".into(),
            sessions: vec![Session {
                name: "Race".into(),
                short: "RACE".into(),
                start_ms: 1_792_958_400_000,
                end_ms: 1_792_965_600_000,
            }],
        };
        let json = weekend_json(&weekend);
        assert_eq!(json["sessions"][0]["start"], "2026-10-25T20:00:00.000Z");
        assert_eq!(json["sessions"][0]["end"], "2026-10-25T22:00:00.000Z");
        assert_eq!(json["circuitId"], "americas");
    }
}
