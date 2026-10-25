//! This season's race and Sprint results, read from Jolpica, to measure each driver's form
//! for the title fight. (Port of `seasonHistory` in the Electron app's `src/data.js`.)

use crate::championship::{Kind, SeasonResult};
use crate::ergast::Driver;
use serde::Deserialize;

#[derive(Deserialize)]
struct Response {
    #[serde(rename = "MRData")]
    mr_data: Data,
}

#[derive(Deserialize)]
struct Data {
    total: String,
    #[serde(rename = "RaceTable")]
    race_table: Table,
}

#[derive(Deserialize)]
struct Table {
    #[serde(rename = "Races")]
    races: Vec<RaceEntry>,
}

#[derive(Deserialize)]
struct RaceEntry {
    round: String,
    #[serde(rename = "Results", default)]
    results: Vec<Row>,
    #[serde(rename = "SprintResults", default)]
    sprint_results: Vec<Row>,
}

#[derive(Deserialize)]
struct Row {
    position: String,
    points: String,
    status: String,
    #[serde(rename = "Driver")]
    driver: Driver,
}

/// One page of results, and how many rows the whole season has (to know whether to ask
/// for another page).
pub struct Page {
    pub rows: Vec<SeasonResult>,
    pub total: usize,
}

/// A driver who was classified (finished, or was lapped) is not a retirement.
pub fn finished_status(status: &str) -> bool {
    if matches!(status, "Finished" | "Lapped") {
        return true;
    }
    // "+1 Lap", "+2 Laps"
    status
        .strip_prefix('+')
        .and_then(|rest| {
            rest.strip_suffix(" Laps")
                .or_else(|| rest.strip_suffix(" Lap"))
        })
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

pub fn parse_season_results(json: &str, kind: Kind) -> Result<Page, serde_json::Error> {
    let response: Response = serde_json::from_str(json)?;
    let total = response.mr_data.total.parse().unwrap_or(0);
    let mut rows = Vec::new();
    for race in response.mr_data.race_table.races {
        let round = race.round.parse().unwrap_or(0);
        let list = match kind {
            Kind::Race => race.results,
            Kind::Sprint => race.sprint_results,
        };
        rows.extend(list.into_iter().map(|row| SeasonResult {
            driver: row.driver.short_code(),
            kind,
            round,
            points: row.points.parse().unwrap_or(0.0),
            position: row.position.parse().unwrap_or(0),
            dnf: !finished_status(&row.status),
        }));
    }
    Ok(Page { rows, total })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classified_statuses_are_not_retirements() {
        for ok in ["Finished", "Lapped", "+1 Lap", "+2 Laps", "+12 Laps"] {
            assert!(finished_status(ok), "{ok}");
        }
        for bad in [
            "Retired",
            "Engine",
            "Collision",
            "+ Laps",
            "+1 Lapse",
            "Disqualified",
        ] {
            assert!(!finished_status(bad), "{bad}");
        }
    }

    #[test]
    fn reads_race_and_sprint_rows() {
        let json = r#"{"MRData":{"total":"3","RaceTable":{"Races":[
          {"round":"2","Results":[
            {"position":"1","points":"25","status":"Finished","Driver":{"code":"VER","familyName":"Verstappen"}},
            {"position":"20","points":"0","status":"Engine","Driver":{"familyName":"Lawson"}}]},
          {"round":"3","SprintResults":[
            {"position":"1","points":"8","status":"Finished","Driver":{"code":"NOR","familyName":"Norris"}}]}]}}}"#;
        let races = parse_season_results(json, Kind::Race).unwrap();
        assert_eq!(races.total, 3);
        assert_eq!(races.rows.len(), 2);
        assert_eq!(races.rows[0].driver, "VER");
        assert_eq!(races.rows[0].points, 25.0);
        assert!(!races.rows[0].dnf);
        assert_eq!(races.rows[1].driver, "LAW"); // no code in the data
        assert!(races.rows[1].dnf);
        assert_eq!(races.rows[1].round, 2);

        let sprints = parse_season_results(json, Kind::Sprint).unwrap();
        assert_eq!(sprints.rows.len(), 1);
        assert_eq!(sprints.rows[0].kind, Kind::Sprint);
        assert_eq!(sprints.rows[0].round, 3);
    }
}
