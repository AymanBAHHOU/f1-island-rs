//! The race calendar from Jolpica (the Ergast successor): the shape of its JSON, the
//! sessions of a weekend, and which weekend to show. Pure logic, no network: the text
//! comes in as a `&str`, so it is tested with samples. (Port of `buildSessions`,
//! `pickWeekend` and friends in the Electron app's `src/data.js`.)

use chrono::DateTime;
use serde::{Deserialize, Serialize};

const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 3_600_000;
const RACE_MINUTES: i64 = 120;
/// A weekend stays the one shown until this long after the race's scheduled end.
const WEEKEND_TAIL_MS: i64 = 6 * HOUR_MS;

// ---------- the JSON Jolpica sends ----------
//
// Serde matches JSON fields to struct fields by name. Jolpica's names don't follow
// Rust's style (`raceName`, `Circuit`, `FirstPractice`), so `#[serde(rename = "...")]`
// says which JSON name belongs to which field. Fields we don't list are ignored.

#[derive(Debug, Deserialize)]
struct Response {
    #[serde(rename = "MRData")]
    mr_data: MrData,
}

#[derive(Debug, Deserialize)]
struct MrData {
    #[serde(rename = "RaceTable")]
    race_table: RaceTable,
}

#[derive(Debug, Deserialize)]
struct RaceTable {
    #[serde(rename = "Races")]
    races: Vec<Race>,
}

/// A date and (usually) a time, as Jolpica writes them: "2026-10-25" and "20:00:00Z".
#[derive(Debug, Clone, Deserialize)]
pub struct Slot {
    date: String,
    time: Option<String>,
}

impl Slot {
    /// Milliseconds since 1970 (UTC), or `None` if the date doesn't parse.
    fn start_ms(&self) -> Option<i64> {
        let time = self.time.as_deref().unwrap_or("00:00:00Z");
        DateTime::parse_from_rfc3339(&format!("{}T{}", self.date, time))
            .ok()
            .map(|t| t.timestamp_millis())
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Location {
    locality: String,
    country: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Circuit {
    #[serde(rename = "circuitId")]
    circuit_id: String,
    #[serde(rename = "circuitName")]
    circuit_name: String,
    #[serde(rename = "Location")]
    location: Location,
}

/// One race weekend in the calendar. The race itself is `date` + `time`; the other
/// sessions are optional (a Sprint weekend has different ones).
#[derive(Debug, Clone, Deserialize)]
pub struct Race {
    season: String,
    round: String,
    #[serde(rename = "raceName")]
    race_name: String,
    #[serde(rename = "Circuit")]
    circuit: Circuit,
    date: String,
    time: Option<String>,
    #[serde(rename = "FirstPractice")]
    first_practice: Option<Slot>,
    #[serde(rename = "SecondPractice")]
    second_practice: Option<Slot>,
    #[serde(rename = "ThirdPractice")]
    third_practice: Option<Slot>,
    #[serde(rename = "SprintQualifying")]
    sprint_qualifying: Option<Slot>,
    #[serde(rename = "SprintShootout")]
    sprint_shootout: Option<Slot>,
    #[serde(rename = "Sprint")]
    sprint: Option<Slot>,
    #[serde(rename = "Qualifying")]
    qualifying: Option<Slot>,
}

impl Race {
    pub fn round(&self) -> &str {
        &self.round
    }

    pub fn name(&self) -> &str {
        &self.race_name
    }

    /// When the Sprint starts, if this weekend has one.
    pub fn sprint_start_ms(&self) -> Option<i64> {
        self.sprint.as_ref().and_then(Slot::start_ms)
    }

    pub fn race_start_ms(&self) -> Option<i64> {
        Slot {
            date: self.date.clone(),
            time: self.time.clone(),
        }
        .start_ms()
    }
}

/// Reads Jolpica's `current.json` text into the list of races.
pub fn parse_schedule(json: &str) -> Result<Vec<Race>, serde_json::Error> {
    Ok(serde_json::from_str::<Response>(json)?
        .mr_data
        .race_table
        .races)
}

// ---------- what the page gets ----------

/// One session of a weekend. `Serialize` turns it into JSON for the page; `camelCase`
/// so the page sees `startMs`, the way JavaScript names things.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub name: String,
    pub short: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// The weekend the island shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Weekend {
    pub season: String,
    pub round: String,
    pub name: String,
    pub circuit_id: String,
    pub circuit: String,
    pub locality: String,
    pub country: String,
    pub sessions: Vec<Session>,
}

/// The sessions of a weekend in time order, with their scheduled ends.
pub fn build_sessions(race: &Race) -> Vec<Session> {
    // (slot, name, short name, scheduled minutes), in the order a weekend usually runs.
    let slots: [(&Option<Slot>, &str, &str, i64); 7] = [
        (&race.first_practice, "Practice 1", "FP1", 60),
        (&race.second_practice, "Practice 2", "FP2", 60),
        (&race.third_practice, "Practice 3", "FP3", 60),
        (&race.sprint_qualifying, "Sprint Qualifying", "SQ", 45),
        (&race.sprint_shootout, "Sprint Shootout", "SQ", 45),
        (&race.sprint, "Sprint", "SPR", 60),
        (&race.qualifying, "Qualifying", "Q", 60),
    ];

    let mut sessions: Vec<Session> = slots
        .iter()
        // `filter_map` keeps only the slots that exist AND parse, turning `Some(x)` into `x`.
        .filter_map(|(slot, name, short, minutes)| {
            let start_ms = slot.as_ref()?.start_ms()?;
            Some(Session {
                name: name.to_string(),
                short: short.to_string(),
                start_ms,
                end_ms: start_ms + minutes * MINUTE_MS,
            })
        })
        .collect();

    if let Some(start_ms) = race.race_start_ms() {
        sessions.push(Session {
            name: "Race".into(),
            short: "RACE".into(),
            start_ms,
            end_ms: start_ms + RACE_MINUTES * MINUTE_MS,
        });
    }
    sessions.sort_by_key(|s| s.start_ms);
    sessions
}

/// The weekend to show at `now_ms`: the first whose race hasn't ended (plus a tail so
/// the results stay up for a few hours). `None` once the season is over.
pub fn pick_weekend(races: &[Race], now_ms: i64) -> Option<Weekend> {
    let race = races.iter().find(|r| {
        r.race_start_ms()
            .is_some_and(|start| start + RACE_MINUTES * MINUTE_MS + WEEKEND_TAIL_MS > now_ms)
    })?;
    Some(Weekend {
        season: race.season.clone(),
        round: race.round.clone(),
        name: race.race_name.clone(),
        circuit_id: race.circuit.circuit_id.clone(),
        circuit: race.circuit.circuit_name.clone(),
        locality: race.circuit.location.locality.clone(),
        country: race.circuit.location.country.clone(),
        sessions: build_sessions(race),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two weekends in Jolpica's shape: a normal one (round 17) and a Sprint one (round 18).
    const SAMPLE: &str = r#"{
      "MRData": { "RaceTable": { "season": "2026", "Races": [
        { "season": "2026", "round": "17", "raceName": "Singapore Grand Prix",
          "Circuit": { "circuitId": "marina_bay", "circuitName": "Marina Bay Street Circuit",
                       "Location": { "lat": "1.29", "long": "103.86", "locality": "Marina Bay", "country": "Singapore" } },
          "date": "2026-10-11", "time": "12:00:00Z",
          "FirstPractice": { "date": "2026-10-09", "time": "09:30:00Z" },
          "SecondPractice": { "date": "2026-10-09", "time": "13:00:00Z" },
          "ThirdPractice": { "date": "2026-10-10", "time": "09:30:00Z" },
          "Qualifying": { "date": "2026-10-10", "time": "13:00:00Z" } },
        { "season": "2026", "round": "18", "raceName": "United States Grand Prix",
          "Circuit": { "circuitId": "americas", "circuitName": "Circuit of the Americas",
                       "Location": { "lat": "30.13", "long": "-97.64", "locality": "Austin", "country": "USA" } },
          "date": "2026-10-25", "time": "20:00:00Z",
          "FirstPractice": { "date": "2026-10-23", "time": "17:30:00Z" },
          "SprintQualifying": { "date": "2026-10-23", "time": "21:30:00Z" },
          "Sprint": { "date": "2026-10-24", "time": "17:00:00Z" },
          "Qualifying": { "date": "2026-10-24", "time": "21:00:00Z" } }
      ] } }
    }"#;

    // 2026-10-23T12:00:00Z and friends, in milliseconds.
    const OCT_11_12H: i64 = 1_791_720_000_000; // the Singapore race start
    const OCT_25_20H: i64 = 1_792_958_400_000; // the US race start

    fn races() -> Vec<Race> {
        parse_schedule(SAMPLE).expect("the sample parses")
    }

    #[test]
    fn reads_the_races() {
        let races = races();
        assert_eq!(races.len(), 2);
        assert_eq!(races[1].race_name, "United States Grand Prix");
        assert_eq!(races[1].circuit.location.locality, "Austin");
    }

    #[test]
    fn bad_json_is_an_error_not_a_crash() {
        assert!(parse_schedule("not json").is_err());
        assert!(parse_schedule(r#"{"MRData":{}}"#).is_err());
    }

    #[test]
    fn a_normal_weekend_has_its_sessions_in_order_with_ends() {
        let sessions = build_sessions(&races()[0]);
        let names: Vec<&str> = sessions.iter().map(|s| s.short.as_str()).collect();
        assert_eq!(names, ["FP1", "FP2", "FP3", "Q", "RACE"]);
        let race = sessions.last().unwrap();
        assert_eq!(race.start_ms, OCT_11_12H);
        assert_eq!(race.end_ms, OCT_11_12H + 2 * HOUR_MS); // the race is scheduled for two hours
        let fp1 = &sessions[0];
        assert_eq!(fp1.end_ms - fp1.start_ms, HOUR_MS);
    }

    #[test]
    fn a_sprint_weekend_has_its_own_sessions() {
        let sessions = build_sessions(&races()[1]);
        let shorts: Vec<&str> = sessions.iter().map(|s| s.short.as_str()).collect();
        assert_eq!(shorts, ["FP1", "SQ", "SPR", "Q", "RACE"]);
        let sq = &sessions[1];
        assert_eq!(sq.name, "Sprint Qualifying");
        assert_eq!(sq.end_ms - sq.start_ms, 45 * MINUTE_MS);
    }

    #[test]
    fn a_missing_time_means_midnight_utc() {
        let slot = Slot {
            date: "2026-10-25".into(),
            time: None,
        };
        assert_eq!(slot.start_ms(), Some(OCT_25_20H - 20 * HOUR_MS));
        assert_eq!(
            Slot {
                date: "garbage".into(),
                time: None
            }
            .start_ms(),
            None
        );
    }

    #[test]
    fn shows_the_next_weekend_until_its_race_is_over() {
        let races = races();
        // Well before Singapore: Singapore.
        assert_eq!(
            pick_weekend(&races, OCT_11_12H - 3 * 24 * HOUR_MS)
                .unwrap()
                .round,
            "17"
        );
        // During and just after its race (within the tail): still Singapore.
        assert_eq!(
            pick_weekend(&races, OCT_11_12H + HOUR_MS).unwrap().round,
            "17"
        );
        assert_eq!(
            pick_weekend(&races, OCT_11_12H + 2 * HOUR_MS + 5 * HOUR_MS)
                .unwrap()
                .round,
            "17"
        );
        // After the tail: the United States.
        let after = OCT_11_12H + 2 * HOUR_MS + WEEKEND_TAIL_MS + 1;
        let us = pick_weekend(&races, after).unwrap();
        assert_eq!(
            (us.round.as_str(), us.name.as_str()),
            ("18", "United States Grand Prix")
        );
        assert_eq!(us.circuit_id, "americas");
        assert_eq!(us.sessions.len(), 5);
    }

    #[test]
    fn nothing_to_show_once_the_season_is_over() {
        let over = OCT_25_20H + 2 * HOUR_MS + WEEKEND_TAIL_MS + 1;
        assert_eq!(pick_weekend(&races(), over), None);
    }

    #[test]
    fn the_page_sees_camel_case_names() {
        let weekend = pick_weekend(&races(), OCT_25_20H).unwrap();
        let json = serde_json::to_value(&weekend).unwrap();
        assert_eq!(json["circuitId"], "americas");
        assert_eq!(json["sessions"][0]["startMs"], 1_792_776_600_000_i64); // FP1, Oct 23 17:30Z
    }
}
