//! What the Title page needs besides the simulation: which Sprints and races are still to
//! be scored, how fresh Jolpica's standings are, and OpenF1's fresher standings (Jolpica
//! can lag hours behind a Sprint or a race).
//! (Port of the matching parts of the Electron app's `src/data.js`.)

use crate::championship::{DriverIn, Event, Kind, TeamIn};
use crate::results::parse_ms;
use crate::schedule::Race;
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Value};
use std::collections::HashMap;

const MIN: i64 = 60_000;
const RACE_MINS: i64 = 120;
/// OpenF1 publishes results about half an hour after a session.
pub const OPENF1_DELAY_MS: i64 = 30 * MIN;

fn iso(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// "United States Grand Prix" -> "United States GP".
pub fn short_name(name: &str) -> String {
    match name.to_lowercase().find("grand prix") {
        Some(i) => format!("{}GP{}", &name[..i], &name[i + "grand prix".len()..]),
        None => name.to_string(),
    }
}

/// Sprints and races not yet in the standings (those starting after `as_of_ms`).
pub fn events_after(races: &[Race], as_of_ms: i64) -> Vec<Event> {
    let mut out = Vec::new();
    for race in races {
        let name = short_name(race.name());
        if let Some(start) = race.sprint_start_ms().filter(|&s| s > as_of_ms) {
            out.push(Event {
                kind: Kind::Sprint,
                name: format!("{name} Sprint"),
                start: iso(start),
            });
        }
        if let Some(start) = race.race_start_ms().filter(|&s| s > as_of_ms) {
            out.push(Event {
                kind: Kind::Race,
                name,
                start: iso(start),
            });
        }
    }
    out
}

/// The moment Jolpica's standings for `round` reflect. "Round 17" standings can include
/// only that round's Sprint (race results not published yet), in which case the race is
/// still to be counted.
pub fn standings_as_of(round: &str, races: &[Race], last_race_round: i64) -> i64 {
    let Some(race) = races.iter().find(|r| r.round() == round) else {
        return 0;
    };
    let race_start = race.race_start_ms().unwrap_or(0);
    if round.parse::<i64>().is_ok_and(|n| n <= last_race_round) {
        return race_start + RACE_MINS * MIN;
    }
    if let Some(sprint) = race.sprint_start_ms() {
        return sprint + 60 * MIN;
    }
    race_start - 1 // nothing from this round yet
}

// ---------- OpenF1's championship ----------

/// The latest finished Sprint or race.
#[derive(Debug, Clone, PartialEq)]
pub struct Latest {
    pub key: i64,
    pub meeting_key: Option<i64>,
    pub start_ms: i64,
    pub session_name: String,
    pub country: String,
}

fn text(row: &Value, key: &str) -> Option<String> {
    row.get(key)?.as_str().map(str::to_string)
}

fn rows(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

/// The newest Sprint or race whose results should be out by `now_ms`, if it started after
/// `after_ms` (the time Jolpica's standings reflect).
pub fn latest_finished_race(sessions: &Value, now_ms: i64, after_ms: i64) -> Option<Latest> {
    let mut finished: Vec<(i64, &Value)> = rows(sessions)
        .iter()
        .filter(|s| s.get("is_cancelled").and_then(Value::as_bool) != Some(true))
        .filter_map(|s| {
            let end = parse_ms(&text(s, "date_end")?)?;
            let start = parse_ms(&text(s, "date_start")?)?;
            (end + OPENF1_DELAY_MS < now_ms).then_some((start, s))
        })
        .collect();
    finished.sort_by_key(|f| std::cmp::Reverse(f.0));
    let (start_ms, s) = finished.into_iter().next()?;
    if start_ms <= after_ms {
        return None;
    }
    Some(Latest {
        key: s.get("session_key")?.as_i64()?,
        meeting_key: s.get("meeting_key").and_then(Value::as_i64),
        start_ms,
        session_name: text(s, "session_name").unwrap_or_default(),
        country: text(s, "country_name").unwrap_or_default(),
    })
}

/// Fresher standings: what the page shows, and the full field for the title fight.
#[derive(Debug, Clone)]
pub struct Fresher {
    pub standings: Vec<Value>,
    pub constructors: Vec<Value>,
    pub note: String,
    pub as_of_ms: i64,
    pub drivers: Vec<DriverIn>,
    pub teams: Vec<TeamIn>,
}

/// A number the way JavaScript's `String()` writes it (25 not 25.0).
fn num_str(value: &Value) -> String {
    match value {
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 => format!("{}", f as i64),
            _ => n.to_string(),
        },
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

fn by_position(list: &Value) -> Vec<&Value> {
    let mut sorted: Vec<&Value> = rows(list).iter().collect();
    sorted.sort_by_key(|r| {
        r.get("position_current")
            .and_then(Value::as_i64)
            .unwrap_or(i64::MAX)
    });
    sorted
}

pub fn build_fresher(
    latest: &Latest,
    champ: &Value,
    teams: &Value,
    drivers: &Value,
    meeting: Option<&Value>,
) -> Option<Fresher> {
    if rows(champ).is_empty() {
        return None;
    }
    let by_number: HashMap<i64, &Value> = rows(drivers)
        .iter()
        .filter_map(|d| Some((d.get("driver_number")?.as_i64()?, d)))
        .collect();
    let colour_of = |d: &Value| text(d, "team_colour").map(|c| format!("#{c}"));
    // Later drivers of a team overwrite earlier ones (they share a colour anyway).
    let team_colour: HashMap<String, String> = rows(drivers)
        .iter()
        .filter_map(|d| Some((text(d, "team_name")?, colour_of(d)?)))
        .collect();
    let code_of = |c: &Value| {
        let number = c.get("driver_number").and_then(Value::as_i64);
        number
            .and_then(|n| by_number.get(&n))
            .and_then(|d| text(d, "name_acronym"))
            .unwrap_or_else(|| format!("#{}", number.unwrap_or(0)))
    };
    let driver_of = |c: &Value| {
        c.get("driver_number")
            .and_then(Value::as_i64)
            .and_then(|n| by_number.get(&n).copied())
    };

    let standings = by_position(champ)
        .into_iter()
        .take(10)
        .map(|c| {
            json!({
                "pos": num_str(&c["position_current"]),
                "code": code_of(c),
                "points": num_str(&c["points_current"]),
                "color": driver_of(c).and_then(colour_of),
            })
        })
        .collect();
    let constructors = by_position(teams)
        .into_iter()
        .map(|t| {
            let name = text(t, "team_name").unwrap_or_default();
            json!({
                "pos": num_str(&t["position_current"]),
                "color": team_colour.get(&name),
                "name": name,
                "points": num_str(&t["points_current"]),
            })
        })
        .collect();

    let meeting_name = meeting
        .and_then(|m| text(m, "meeting_name"))
        .unwrap_or_else(|| latest.country.clone());
    let sprint = if latest.session_name == "Sprint" {
        " Sprint"
    } else {
        ""
    };

    Some(Fresher {
        standings,
        constructors,
        note: format!("After {}{sprint}", short_name(&meeting_name)),
        as_of_ms: latest.start_ms,
        drivers: rows(champ)
            .iter()
            .map(|c| DriverIn {
                code: code_of(c),
                points: c["points_current"].as_f64().unwrap_or(0.0),
                team: driver_of(c).and_then(|d| text(d, "team_name")),
                team_id: None,
                color: driver_of(c).and_then(colour_of),
            })
            .collect(),
        teams: rows(teams)
            .iter()
            .map(|t| {
                let name = text(t, "team_name").unwrap_or_default();
                TeamIn {
                    points: t["points_current"].as_f64().unwrap_or(0.0),
                    color: team_colour.get(&name).cloned(),
                    team_id: None,
                    name,
                }
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::parse_schedule;

    fn races() -> Vec<Race> {
        parse_schedule(
            r#"{"MRData":{"RaceTable":{"Races":[
            {"season":"2026","round":"1","raceName":"Australian Grand Prix",
             "Circuit":{"circuitId":"albert_park","circuitName":"A","Location":{"locality":"M","country":"Australia"}},
             "date":"2026-03-08","time":"04:00:00Z"},
            {"season":"2026","round":"2","raceName":"Chinese Grand Prix",
             "Circuit":{"circuitId":"shanghai","circuitName":"S","Location":{"locality":"S","country":"China"}},
             "date":"2026-03-15","time":"07:00:00Z",
             "Sprint":{"date":"2026-03-14","time":"03:00:00Z"}}]}}}"#,
        )
        .unwrap()
    }

    #[test]
    fn grand_prix_becomes_gp() {
        assert_eq!(short_name("United States Grand Prix"), "United States GP");
        assert_eq!(short_name("Monaco GRAND PRIX 2"), "Monaco GP 2");
        assert_eq!(short_name("Imola"), "Imola");
    }

    #[test]
    fn events_are_what_starts_after_the_standings() {
        let races = races();
        let all = events_after(&races, 0);
        let kinds: Vec<_> = all.iter().map(|e| (e.kind, e.name.as_str())).collect();
        assert_eq!(
            kinds,
            vec![
                (Kind::Race, "Australian GP"),
                (Kind::Sprint, "Chinese GP Sprint"),
                (Kind::Race, "Chinese GP"),
            ]
        );
        assert_eq!(all[0].start, "2026-03-08T04:00:00.000Z");
        let after_sprint = parse_ms("2026-03-14T05:00:00+00:00").unwrap();
        let rest = events_after(&races, after_sprint);
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].name, "Chinese GP");
    }

    #[test]
    fn standings_as_of_knows_about_sprints() {
        let races = races();
        let race1 = parse_ms("2026-03-08T04:00:00+00:00").unwrap();
        assert_eq!(standings_as_of("1", &races, 1), race1 + 120 * MIN);
        // Round 2 standings with no race result yet: only the Sprint is in.
        let sprint = parse_ms("2026-03-14T03:00:00+00:00").unwrap();
        assert_eq!(standings_as_of("2", &races, 1), sprint + 60 * MIN);
        assert_eq!(standings_as_of("9", &races, 1), 0);
    }

    fn sessions() -> Value {
        json!([
            {"session_key": 1, "session_name": "Race", "meeting_key": 7, "country_name": "Australia",
             "date_start": "2026-03-08T04:00:00+00:00", "date_end": "2026-03-08T06:00:00+00:00"},
            {"session_key": 2, "session_name": "Sprint", "meeting_key": 8, "country_name": "China",
             "date_start": "2026-03-14T03:00:00+00:00", "date_end": "2026-03-14T04:00:00+00:00"},
            {"session_key": 3, "session_name": "Race", "meeting_key": 8, "country_name": "China",
             "date_start": "2026-03-15T07:00:00+00:00", "date_end": "2026-03-15T09:00:00+00:00"},
        ])
    }

    #[test]
    fn the_latest_finished_race_waits_for_results_to_be_out() {
        let after_race1 = parse_ms("2026-03-08T06:00:00+00:00").unwrap();
        // The Chinese race ended at 09:00; its results are expected 30 minutes later.
        let too_early = parse_ms("2026-03-15T09:10:00+00:00").unwrap();
        let latest = latest_finished_race(&sessions(), too_early, after_race1).unwrap();
        assert_eq!(latest.key, 2);
        let later = parse_ms("2026-03-15T09:40:00+00:00").unwrap();
        assert_eq!(
            latest_finished_race(&sessions(), later, after_race1)
                .unwrap()
                .key,
            3
        );
        // Jolpica is already up to date.
        assert_eq!(latest_finished_race(&sessions(), later, later), None);
    }

    #[test]
    fn builds_the_page_rows_and_the_full_field() {
        let latest = Latest {
            key: 2,
            meeting_key: Some(8),
            start_ms: 1,
            session_name: "Sprint".into(),
            country: "China".into(),
        };
        let champ = json!([
            {"driver_number": 4, "position_current": 2, "points_current": 30.5},
            {"driver_number": 1, "position_current": 1, "points_current": 33},
            {"driver_number": 99, "position_current": 3, "points_current": 0},
        ]);
        let teams = json!([
            {"team_name": "McLaren", "position_current": 1, "points_current": 63},
        ]);
        let drivers = json!([
            {"driver_number": 1, "name_acronym": "VER", "team_name": "Red Bull", "team_colour": "3671C6"},
            {"driver_number": 4, "name_acronym": "NOR", "team_name": "McLaren", "team_colour": "FF8000"},
        ]);
        let meeting = json!({"meeting_name": "Chinese Grand Prix"});
        let f = build_fresher(&latest, &champ, &teams, &drivers, Some(&meeting)).unwrap();
        assert_eq!(f.note, "After Chinese GP Sprint");
        assert_eq!(f.standings[0]["code"], "VER");
        assert_eq!(f.standings[0]["points"], "33");
        assert_eq!(f.standings[1]["points"], "30.5");
        assert_eq!(f.standings[0]["color"], "#3671C6");
        assert_eq!(f.standings[2]["code"], "#99");
        assert!(f.standings[2]["color"].is_null());
        assert_eq!(f.constructors[0]["color"], "#FF8000");
        assert_eq!(f.drivers.len(), 3);
        assert_eq!(f.drivers[0].team.as_deref(), Some("McLaren")); // champ order: NOR first
        assert_eq!(f.teams[0].points, 63.0);
    }

    #[test]
    fn no_championship_rows_means_nothing_fresher() {
        let latest = Latest {
            key: 1,
            meeting_key: None,
            start_ms: 1,
            session_name: "Race".into(),
            country: "X".into(),
        };
        assert!(build_fresher(&latest, &json!([]), &json!([]), &json!([]), None).is_none());
    }
}
