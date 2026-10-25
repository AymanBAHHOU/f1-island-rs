//! The championship tables and the last race's podium, from Jolpica.
//! (Port of the standings part of `getSnapshot` in the Electron app's `src/data.js`.)

use crate::ergast::{Constructor, Driver, PodiumEntry};
use serde::{Deserialize, Serialize};

/// The island shows the top ten drivers.
const DRIVERS_SHOWN: usize = 10;

// ---------- the JSON ----------

#[derive(Deserialize)]
struct StandingsResponse<T> {
    #[serde(rename = "MRData")]
    mr_data: StandingsData<T>,
}

#[derive(Deserialize)]
struct StandingsData<T> {
    #[serde(rename = "StandingsTable")]
    table: StandingsTable<T>,
}

#[derive(Deserialize)]
struct StandingsTable<T> {
    #[serde(rename = "StandingsLists")]
    lists: Vec<T>,
}

#[derive(Deserialize)]
struct DriverList {
    round: String,
    #[serde(rename = "DriverStandings")]
    standings: Vec<DriverEntry>,
}

#[derive(Deserialize)]
struct DriverEntry {
    position: String,
    points: String,
    #[serde(rename = "Driver")]
    driver: Driver,
    #[serde(rename = "Constructors")]
    constructors: Vec<Constructor>,
}

#[derive(Deserialize)]
struct ConstructorList {
    #[serde(rename = "ConstructorStandings")]
    standings: Vec<ConstructorEntry>,
}

#[derive(Deserialize)]
struct ConstructorEntry {
    position: String,
    points: String,
    #[serde(rename = "Constructor")]
    constructor: Constructor,
}

#[derive(Deserialize)]
struct LastRaceResponse {
    #[serde(rename = "MRData")]
    mr_data: LastRaceData,
}

#[derive(Deserialize)]
struct LastRaceData {
    #[serde(rename = "RaceTable")]
    table: LastRaceTable,
}

#[derive(Deserialize)]
struct LastRaceTable {
    #[serde(rename = "Races")]
    races: Vec<LastRaceEntry>,
}

#[derive(Deserialize)]
struct LastRaceEntry {
    round: String,
    #[serde(rename = "raceName")]
    race_name: String,
    #[serde(rename = "Results")]
    results: Vec<LastRaceResult>,
}

#[derive(Deserialize)]
struct LastRaceResult {
    position: String,
    #[serde(rename = "Driver")]
    driver: Driver,
    #[serde(rename = "Constructor")]
    constructor: Constructor,
}

// ---------- what the page gets ----------

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverRow {
    pub pos: String,
    pub code: String,
    pub points: String,
    /// The team they drive for now (`None` if the data lists none).
    pub team_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamRow {
    pub pos: String,
    pub name: String,
    pub team_id: String,
    pub points: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastRace {
    pub round: String,
    pub name: String,
    pub podium: Vec<PodiumEntry>,
}

/// The drivers' table: the top ten, and the round it is as of.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverTable {
    pub round: String,
    pub rows: Vec<DriverRow>,
}

/// Everything the Standings page shows. Each part may be missing (early in a season, or
/// if one request failed) without taking the others down with it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Standings {
    pub drivers: Option<DriverTable>,
    pub constructors: Vec<TeamRow>,
    pub last_race: Option<LastRace>,
}

// ---------- reading it ----------

pub fn parse_drivers(json: &str) -> Result<Option<DriverTable>, serde_json::Error> {
    let response: StandingsResponse<DriverList> = serde_json::from_str(json)?;
    // `.into_iter().next()`: the first list, if there is one. Early in a season there isn't.
    let Some(list) = response.mr_data.table.lists.into_iter().next() else {
        return Ok(None);
    };
    let rows = list
        .standings
        .into_iter()
        .take(DRIVERS_SHOWN)
        .map(|entry| DriverRow {
            pos: entry.position,
            code: entry.driver.short_code(),
            points: entry.points,
            // The last team in the list is the one they drive for now (they may have changed).
            team_id: entry.constructors.last().map(|c| c.id.clone()),
        })
        .collect();
    Ok(Some(DriverTable {
        round: list.round,
        rows,
    }))
}

pub fn parse_constructors(json: &str) -> Result<Vec<TeamRow>, serde_json::Error> {
    let response: StandingsResponse<ConstructorList> = serde_json::from_str(json)?;
    Ok(response
        .mr_data
        .table
        .lists
        .into_iter()
        .next()
        .map(|list| {
            list.standings
                .into_iter()
                .map(|entry| TeamRow {
                    pos: entry.position,
                    name: entry.constructor.name,
                    team_id: entry.constructor.id,
                    points: entry.points,
                })
                .collect()
        })
        .unwrap_or_default()) // no table yet: an empty list
}

pub fn parse_last_race(json: &str) -> Result<Option<LastRace>, serde_json::Error> {
    let response: LastRaceResponse = serde_json::from_str(json)?;
    Ok(response
        .mr_data
        .table
        .races
        .into_iter()
        .next()
        .map(|race| LastRace {
            round: race.round,
            name: race.race_name,
            podium: race
                .results
                .into_iter()
                .take(3)
                .map(|r| PodiumEntry {
                    pos: r.position,
                    code: r.driver.short_code(),
                    team_id: r.constructor.id,
                })
                .collect(),
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed copies of real answers (Singapore GP, round 17).
    const DRIVERS: &str = r#"{"MRData":{"StandingsTable":{"season":"2026","round":"17","StandingsLists":[
      {"season":"2026","round":"17","DriverStandings":[
        {"position":"1","positionText":"1","points":"340","wins":"8",
         "Driver":{"driverId":"antonelli","code":"ANT","givenName":"Andrea Kimi","familyName":"Antonelli"},
         "Constructors":[{"constructorId":"mercedes","name":"Mercedes"}]},
        {"position":"2","positionText":"2","points":"238","wins":"2",
         "Driver":{"driverId":"russell","code":"RUS","givenName":"George","familyName":"Russell"},
         "Constructors":[{"constructorId":"mercedes","name":"Mercedes"}]},
        {"position":"3","positionText":"3","points":"90","wins":"0",
         "Driver":{"driverId":"lawson","givenName":"Liam","familyName":"Lawson"},
         "Constructors":[{"constructorId":"rb","name":"RB F1 Team"},{"constructorId":"red_bull","name":"Red Bull"}]},
        {"position":"4","positionText":"4","points":"1","wins":"0",
         "Driver":{"driverId":"nobody","code":"NOB","givenName":"No","familyName":"Body"},
         "Constructors":[]}
      ]}]}}}"#;

    const CONSTRUCTORS: &str = r#"{"MRData":{"StandingsTable":{"StandingsLists":[{"ConstructorStandings":[
      {"position":"1","positionText":"1","points":"578","wins":"11",
       "Constructor":{"constructorId":"mercedes","name":"Mercedes"}},
      {"position":"2","positionText":"2","points":"444","wins":"4",
       "Constructor":{"constructorId":"ferrari","name":"Ferrari"}}]}]}}}"#;

    const LAST_RACE: &str = r#"{"MRData":{"RaceTable":{"Races":[{"season":"2026","round":"17","raceName":"Singapore Grand Prix",
      "Results":[
        {"position":"1","Driver":{"code":"VER","familyName":"Verstappen"},"Constructor":{"constructorId":"red_bull","name":"Red Bull"}},
        {"position":"2","Driver":{"code":"LEC","familyName":"Leclerc"},"Constructor":{"constructorId":"ferrari","name":"Ferrari"}},
        {"position":"3","Driver":{"code":"ANT","familyName":"Antonelli"},"Constructor":{"constructorId":"mercedes","name":"Mercedes"}},
        {"position":"4","Driver":{"code":"NOR","familyName":"Norris"},"Constructor":{"constructorId":"mclaren","name":"McLaren"}}]}]}}}"#;

    #[test]
    fn reads_the_drivers_table() {
        let table = parse_drivers(DRIVERS).unwrap().unwrap();
        assert_eq!(table.round, "17");
        assert_eq!(table.rows.len(), 4);
        assert_eq!(
            table.rows[0],
            DriverRow {
                pos: "1".into(),
                code: "ANT".into(),
                points: "340".into(),
                team_id: Some("mercedes".into())
            }
        );
    }

    #[test]
    fn a_driver_without_a_code_gets_one_from_their_name() {
        let table = parse_drivers(DRIVERS).unwrap().unwrap();
        assert_eq!(table.rows[2].code, "LAW");
    }

    #[test]
    fn uses_the_team_they_drive_for_now() {
        let table = parse_drivers(DRIVERS).unwrap().unwrap();
        assert_eq!(table.rows[2].team_id.as_deref(), Some("red_bull")); // moved from RB
        assert_eq!(table.rows[3].team_id, None); // none listed
    }

    #[test]
    fn only_the_top_ten_are_kept() {
        let entries: Vec<String> = (1..=15)
            .map(|i| {
                format!(
                    r#"{{"position":"{i}","points":"{}","Driver":{{"code":"D{i:02}","familyName":"Driver"}},"Constructors":[]}}"#,
                    100 - i
                )
            })
            .collect();
        let json = format!(
            r#"{{"MRData":{{"StandingsTable":{{"StandingsLists":[{{"round":"1","DriverStandings":[{}]}}]}}}}}}"#,
            entries.join(",")
        );
        assert_eq!(parse_drivers(&json).unwrap().unwrap().rows.len(), 10);
    }

    #[test]
    fn no_table_yet_early_in_a_season() {
        let json = r#"{"MRData":{"StandingsTable":{"StandingsLists":[]}}}"#;
        assert_eq!(parse_drivers(json).unwrap(), None);
        assert_eq!(parse_constructors(json).unwrap(), vec![]);
    }

    #[test]
    fn reads_the_constructors_table() {
        let teams = parse_constructors(CONSTRUCTORS).unwrap();
        assert_eq!(teams.len(), 2);
        assert_eq!(
            teams[1],
            TeamRow {
                pos: "2".into(),
                name: "Ferrari".into(),
                team_id: "ferrari".into(),
                points: "444".into()
            }
        );
    }

    #[test]
    fn the_last_race_podium_is_the_first_three() {
        let race = parse_last_race(LAST_RACE).unwrap().unwrap();
        assert_eq!(
            (race.round.as_str(), race.name.as_str()),
            ("17", "Singapore Grand Prix")
        );
        let podium: Vec<(&str, &str)> = race
            .podium
            .iter()
            .map(|p| (p.pos.as_str(), p.code.as_str()))
            .collect();
        assert_eq!(podium, [("1", "VER"), ("2", "LEC"), ("3", "ANT")]);
        assert_eq!(race.podium[0].team_id, "red_bull");
    }

    #[test]
    fn no_race_yet_means_none() {
        assert_eq!(
            parse_last_race(r#"{"MRData":{"RaceTable":{"Races":[]}}}"#).unwrap(),
            None
        );
    }

    #[test]
    fn garbage_is_an_error_not_a_crash() {
        assert!(parse_drivers("nope").is_err());
        assert!(parse_last_race(r#"{"MRData":{}}"#).is_err());
    }
}
