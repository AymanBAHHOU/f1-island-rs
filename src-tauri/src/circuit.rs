//! A circuit's history: its recent winners, and a recap of the last race held there
//! (pole, podium, fastest lap, race time). Pure parsing of Jolpica's answers.
//! (Port of `getCircuitHistory` in the Electron app's `src/data.js`.)

use crate::ergast::{Constructor, Driver, PodiumEntry};
use serde::{Deserialize, Serialize};

/// How many recent winners the island lists.
const WINNERS_SHOWN: usize = 5;

// ---------- the JSON ----------

#[derive(Deserialize)]
struct Total {
    #[serde(rename = "MRData")]
    mr_data: TotalData,
}

#[derive(Deserialize)]
struct TotalData {
    /// Jolpica writes numbers as text: "13".
    total: String,
}

#[derive(Deserialize)]
struct ResultsResponse {
    #[serde(rename = "MRData")]
    mr_data: ResultsData,
}

#[derive(Deserialize)]
struct ResultsData {
    #[serde(rename = "RaceTable")]
    table: ResultsTable,
}

#[derive(Deserialize)]
struct ResultsTable {
    #[serde(rename = "Races")]
    races: Vec<ResultsRace>,
}

#[derive(Deserialize)]
struct ResultsRace {
    season: String,
    #[serde(rename = "raceName")]
    race_name: String,
    #[serde(rename = "Results")]
    results: Vec<ResultRow>,
}

#[derive(Deserialize)]
struct ResultRow {
    position: String,
    #[serde(rename = "Driver")]
    driver: Driver,
    #[serde(rename = "Constructor")]
    constructor: Constructor,
    laps: Option<String>,
    #[serde(rename = "Time")]
    time: Option<TimeInfo>,
    #[serde(rename = "FastestLap")]
    fastest_lap: Option<FastestLapInfo>,
}

#[derive(Deserialize)]
struct TimeInfo {
    time: Option<String>,
}

#[derive(Deserialize)]
struct FastestLapInfo {
    rank: Option<String>,
    lap: Option<String>,
    #[serde(rename = "Time")]
    time: Option<TimeInfo>,
}

#[derive(Deserialize)]
struct QualiResponse {
    #[serde(rename = "MRData")]
    mr_data: QualiData,
}

#[derive(Deserialize)]
struct QualiData {
    #[serde(rename = "RaceTable")]
    table: QualiTable,
}

#[derive(Deserialize)]
struct QualiTable {
    #[serde(rename = "Races")]
    races: Vec<QualiRace>,
}

#[derive(Deserialize)]
struct QualiRace {
    #[serde(rename = "QualifyingResults")]
    results: Vec<QualiRow>,
}

#[derive(Deserialize)]
struct QualiRow {
    position: String,
    #[serde(rename = "Driver")]
    driver: Driver,
    #[serde(rename = "Constructor")]
    constructor: Constructor,
    #[serde(rename = "Q1")]
    q1: Option<String>,
    #[serde(rename = "Q2")]
    q2: Option<String>,
    #[serde(rename = "Q3")]
    q3: Option<String>,
}

// ---------- what the page gets ----------

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Winner {
    pub season: String,
    pub code: String,
    pub team_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pole {
    pub code: String,
    pub team_id: String,
    pub time: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FastestLap {
    pub code: String,
    pub team_id: String,
    pub time: Option<String>,
    pub lap: Option<String>,
}

/// The last race held at this circuit.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Visit {
    pub season: String,
    pub name: String,
    pub laps: Option<String>,
    pub winner_time: Option<String>,
    pub podium: Vec<PodiumEntry>,
    pub pole: Option<Pole>,
    pub fastest_lap: Option<FastestLap>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CircuitHistory {
    /// Most recent first.
    pub winners: Vec<Winner>,
    pub last: Option<Visit>,
}

// ---------- reading it ----------

/// How many races have been won at this circuit, from `.../results/1.json?limit=1`.
/// Used to ask for the *last* few winners (Jolpica lists oldest first).
pub fn total_winners(json: &str) -> Result<u32, serde_json::Error> {
    let total: Total = serde_json::from_str(json)?;
    // A total we can't read as a number counts as none, rather than failing the page.
    Ok(total.mr_data.total.parse().unwrap_or(0))
}

/// The winners in a page of `.../results/1.json`, before `current_season`, most recent
/// first and at most five.
pub fn parse_winners(json: &str, current_season: u32) -> Result<Vec<Winner>, serde_json::Error> {
    let response: ResultsResponse = serde_json::from_str(json)?;
    let mut winners: Vec<Winner> = response
        .mr_data
        .table
        .races
        .into_iter()
        // This year's race (if it has been run) isn't "history".
        .filter(|race| {
            race.season
                .parse::<u32>()
                .is_ok_and(|season| season < current_season)
        })
        .filter_map(|race| {
            let winner = race.results.into_iter().next()?;
            Some(Winner {
                season: race.season,
                code: winner.driver.short_code(),
                team_id: winner.constructor.id,
            })
        })
        .collect();

    winners.reverse(); // oldest first -> most recent first
    winners.truncate(WINNERS_SHOWN);
    Ok(winners)
}

/// The recap of a past race, from its results and (if we have it) its qualifying.
pub fn parse_visit(
    results_json: &str,
    quali_json: Option<&str>,
) -> Result<Option<Visit>, serde_json::Error> {
    let response: ResultsResponse = serde_json::from_str(results_json)?;
    let Some(race) = response.mr_data.table.races.into_iter().next() else {
        return Ok(None);
    };
    let mut rows = race.results;
    if rows.is_empty() {
        return Ok(None);
    }
    // The rows don't reliably come back in finishing order.
    rows.sort_by_key(|r| r.position.parse::<u32>().unwrap_or(u32::MAX));

    let podium = rows
        .iter()
        .take(3)
        .map(|r| PodiumEntry {
            pos: r.position.clone(),
            code: r.driver.short_code(),
            team_id: r.constructor.id.clone(),
        })
        .collect();

    let fastest_lap = rows
        .iter()
        .find(|r| {
            r.fastest_lap
                .as_ref()
                .is_some_and(|f| f.rank.as_deref() == Some("1"))
        })
        .map(|r| {
            let lap = r.fastest_lap.as_ref();
            FastestLap {
                code: r.driver.short_code(),
                team_id: r.constructor.id.clone(),
                time: lap
                    .and_then(|f| f.time.as_ref())
                    .and_then(|t| t.time.clone()),
                lap: lap.and_then(|f| f.lap.clone()),
            }
        });

    // Qualifying is a nice extra: if it's missing or unreadable, the recap just has no pole.
    let pole = quali_json.and_then(|json| parse_pole(json).ok().flatten());

    Ok(Some(Visit {
        season: race.season,
        name: race.race_name,
        laps: rows[0].laps.clone(),
        winner_time: rows[0].time.as_ref().and_then(|t| t.time.clone()),
        podium,
        pole,
        fastest_lap,
    }))
}

fn parse_pole(json: &str) -> Result<Option<Pole>, serde_json::Error> {
    let response: QualiResponse = serde_json::from_str(json)?;
    let Some(race) = response.mr_data.table.races.into_iter().next() else {
        return Ok(None);
    };
    // The rows aren't reliably in order either, so look for position 1 explicitly.
    Ok(race
        .results
        .into_iter()
        .find(|r| r.position == "1")
        .map(|r| Pole {
            code: r.driver.short_code(),
            team_id: r.constructor.id,
            // The time of the last qualifying round they reached. A round they didn't reach
            // can come as "" rather than missing, so skip blanks: `flatten` drops the `None`s,
            // `find` takes the first that has something in it.
            time: [r.q3, r.q2, r.q1]
                .into_iter()
                .flatten()
                .find(|t| !t.is_empty()),
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed copies of the real answers for the United States GP.
    const WINNERS: &str = r#"{"MRData":{"total":"13","RaceTable":{"Races":[
      {"season":"2019","raceName":"United States Grand Prix","Results":[{"position":"1","Driver":{"code":"BOT","familyName":"Bottas"},"Constructor":{"constructorId":"mercedes","name":"Mercedes"}}]},
      {"season":"2021","raceName":"United States Grand Prix","Results":[{"position":"1","Driver":{"code":"VER","familyName":"Verstappen"},"Constructor":{"constructorId":"red_bull","name":"Red Bull"}}]},
      {"season":"2022","raceName":"United States Grand Prix","Results":[{"position":"1","Driver":{"code":"VER","familyName":"Verstappen"},"Constructor":{"constructorId":"red_bull","name":"Red Bull"}}]},
      {"season":"2023","raceName":"United States Grand Prix","Results":[{"position":"1","Driver":{"code":"VER","familyName":"Verstappen"},"Constructor":{"constructorId":"red_bull","name":"Red Bull"}}]},
      {"season":"2024","raceName":"United States Grand Prix","Results":[{"position":"1","Driver":{"code":"LEC","familyName":"Leclerc"},"Constructor":{"constructorId":"ferrari","name":"Ferrari"}}]},
      {"season":"2025","raceName":"United States Grand Prix","Results":[{"position":"1","Driver":{"code":"VER","familyName":"Verstappen"},"Constructor":{"constructorId":"red_bull","name":"Red Bull"}}]},
      {"season":"2026","raceName":"United States Grand Prix","Results":[{"position":"1","Driver":{"code":"XXX","familyName":"Future"},"Constructor":{"constructorId":"ferrari","name":"Ferrari"}}]}
    ]}}}"#;

    // Rows deliberately out of order, as Jolpica sometimes sends them.
    const RESULTS: &str = r#"{"MRData":{"RaceTable":{"Races":[{"season":"2025","raceName":"United States Grand Prix","Results":[
      {"position":"3","Driver":{"code":"LEC","familyName":"Leclerc"},"Constructor":{"constructorId":"ferrari","name":"Ferrari"},"laps":"56"},
      {"position":"1","Driver":{"code":"VER","familyName":"Verstappen"},"Constructor":{"constructorId":"red_bull","name":"Red Bull"},"laps":"56",
       "Time":{"millis":"5640161","time":"1:34:00.161"},"FastestLap":{"rank":"3","lap":"35","Time":{"time":"1:37.991"}}},
      {"position":"2","Driver":{"code":"NOR","familyName":"Norris"},"Constructor":{"constructorId":"mclaren","name":"McLaren"},"laps":"56"},
      {"position":"9","Driver":{"code":"ANT","familyName":"Antonelli"},"Constructor":{"constructorId":"mercedes","name":"Mercedes"},"laps":"56",
       "FastestLap":{"rank":"1","lap":"33","Time":{"time":"1:37.577"}}}
    ]}]}}}"#;

    const QUALI: &str = r#"{"MRData":{"RaceTable":{"Races":[{"QualifyingResults":[
      {"position":"2","Driver":{"code":"NOR","familyName":"Norris"},"Constructor":{"constructorId":"mclaren","name":"McLaren"},"Q1":"1:33.4","Q2":"1:32.8","Q3":"1:32.6"},
      {"position":"1","Driver":{"code":"VER","familyName":"Verstappen"},"Constructor":{"constructorId":"red_bull","name":"Red Bull"},"Q1":"1:33.207","Q2":"1:32.701","Q3":"1:32.510"}
    ]}]}}}"#;

    #[test]
    fn counts_the_winners() {
        assert_eq!(total_winners(WINNERS).unwrap(), 13);
        assert_eq!(total_winners(r#"{"MRData":{"total":"many"}}"#).unwrap(), 0);
    }

    #[test]
    fn the_five_most_recent_winners_before_this_season_newest_first() {
        let winners = parse_winners(WINNERS, 2026).unwrap();
        let seasons: Vec<&str> = winners.iter().map(|w| w.season.as_str()).collect();
        assert_eq!(seasons, ["2025", "2024", "2023", "2022", "2021"]); // 2019 cut, 2026 isn't history
        assert_eq!(winners[1].code, "LEC");
        assert_eq!(winners[1].team_id, "ferrari");
    }

    #[test]
    fn a_circuit_with_few_visits_lists_what_there_is() {
        let winners = parse_winners(WINNERS, 2020).unwrap();
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].season, "2019");
    }

    #[test]
    fn recaps_the_last_race_with_the_rows_put_in_order() {
        let visit = parse_visit(RESULTS, Some(QUALI)).unwrap().unwrap();
        assert_eq!(
            (visit.season.as_str(), visit.name.as_str()),
            ("2025", "United States Grand Prix")
        );
        let podium: Vec<&str> = visit.podium.iter().map(|p| p.code.as_str()).collect();
        assert_eq!(podium, ["VER", "NOR", "LEC"]);
        assert_eq!(visit.laps.as_deref(), Some("56"));
        assert_eq!(visit.winner_time.as_deref(), Some("1:34:00.161"));
    }

    #[test]
    fn the_fastest_lap_is_the_one_ranked_first_not_the_winners() {
        let lap = parse_visit(RESULTS, None)
            .unwrap()
            .unwrap()
            .fastest_lap
            .unwrap();
        assert_eq!(
            (lap.code.as_str(), lap.time.as_deref(), lap.lap.as_deref()),
            ("ANT", Some("1:37.577"), Some("33"))
        );
    }

    #[test]
    fn the_pole_is_found_by_position_and_uses_the_last_round_reached() {
        let pole = parse_visit(RESULTS, Some(QUALI))
            .unwrap()
            .unwrap()
            .pole
            .unwrap();
        assert_eq!(
            (pole.code.as_str(), pole.time.as_deref()),
            ("VER", Some("1:32.510"))
        );
    }

    #[test]
    fn missing_or_broken_qualifying_just_means_no_pole() {
        assert_eq!(parse_visit(RESULTS, None).unwrap().unwrap().pole, None);
        assert_eq!(
            parse_visit(RESULTS, Some("not json"))
                .unwrap()
                .unwrap()
                .pole,
            None
        );
    }

    #[test]
    fn a_pole_who_went_out_in_q1_shows_the_q1_time() {
        let json = r#"{"MRData":{"RaceTable":{"Races":[{"QualifyingResults":[
          {"position":"1","Driver":{"code":"VER","familyName":"Verstappen"},"Constructor":{"constructorId":"red_bull","name":"Red Bull"},"Q1":"1:33.0","Q2":"","Q3":""}]}]}}}"#;
        assert_eq!(
            parse_pole(json).unwrap().unwrap().time.as_deref(),
            Some("1:33.0")
        );
    }

    #[test]
    fn no_race_or_no_results_means_no_recap() {
        assert_eq!(
            parse_visit(r#"{"MRData":{"RaceTable":{"Races":[]}}}"#, None).unwrap(),
            None
        );
        let empty =
            r#"{"MRData":{"RaceTable":{"Races":[{"season":"2025","raceName":"X","Results":[]}]}}}"#;
        assert_eq!(parse_visit(empty, None).unwrap(), None);
    }
}
