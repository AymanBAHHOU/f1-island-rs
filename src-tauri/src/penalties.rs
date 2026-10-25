//! What the stewards have done to each driver, read from the race control messages: time
//! penalties, drive-throughs, stop-go penalties, investigations still open, black and white
//! flags. The Live tab shows these as small tags beside the driver.
//!
//! The messages are text, so this reads them the way a person would. The wording was checked
//! against a whole race's messages (see the tests).

use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::LazyLock;

/// A small tag beside a driver. `kind` is what the page colours by: `penalty`,
/// `investigation` or `warning`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tag {
    pub kind: &'static str,
    pub text: String,
}

macro_rules! pattern {
    ($name:ident, $re:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($re).unwrap());
    };
}

// "(15:04:59)" at the end names the incident: every message about it ends the same way.
pattern!(INCIDENT, r"\((\d\d:\d\d:\d\d)\)\s*$");
pattern!(
    INVOLVING,
    r"INVOLVING CARS? (\d+) \([A-Z]{3}\)(?: AND (\d+) \([A-Z]{3}\))?"
);
pattern!(TIME_PENALTY, r"(\d+) SECOND TIME PENALTY FOR CAR (\d+)");
pattern!(DRIVE_THROUGH, r"DRIVE[ -]THROUGH PENALTY FOR CAR (\d+)");
pattern!(STOP_GO, r"(\d+) SECOND STOP[ /-]?GO PENALTY FOR CAR (\d+)");
pattern!(BLACK_WHITE, r"BLACK AND WHITE FLAG FOR CAR (\d+)");
pattern!(REPRIMAND, r"REPRIMAND FOR CAR (\d+)");
pattern!(
    DISQUALIFIED,
    r"CAR (\d+) \([A-Z]{3}\) (?:HAS BEEN )?DISQUALIFIED"
);

#[derive(Debug, Clone, PartialEq)]
enum Penalty {
    Time(u32),
    DriveThrough,
    StopGo(u32),
}

#[derive(Default)]
struct Car {
    /// (incident, penalty) not yet served.
    penalties: Vec<(Option<String>, Penalty)>,
    /// Incidents that are under investigation.
    investigations: Vec<String>,
    black_and_white: bool,
    reprimanded: bool,
    disqualified: bool,
}

fn incident(text: &str) -> Option<String> {
    INCIDENT.captures(text).map(|c| c[1].to_string())
}

/// The tags for each car (by racing number), from the race control messages in order.
pub fn tags_by_car(messages: &[&Value]) -> HashMap<String, Vec<Tag>> {
    let mut cars: HashMap<String, Car> = HashMap::new();

    for m in messages {
        let text = m["Message"].as_str().unwrap_or("").trim();
        let id = incident(text);
        let involved: Vec<String> = INVOLVING
            .captures(text)
            .map(|c| {
                c.iter()
                    .skip(1)
                    .flatten()
                    .map(|n| n.as_str().to_string())
                    .collect()
            })
            .unwrap_or_default();

        if let Some(c) = DISQUALIFIED.captures(text) {
            cars.entry(c[1].to_string()).or_default().disqualified = true;
            continue;
        }
        if let Some(c) = BLACK_WHITE.captures(text) {
            cars.entry(c[1].to_string()).or_default().black_and_white = true;
            continue;
        }
        if let Some(c) = REPRIMAND.captures(text) {
            cars.entry(c[1].to_string()).or_default().reprimanded = true;
            close_investigation(&mut cars, id.as_deref());
            continue;
        }

        // "FIA STEWARDS: PENALTY SERVED - 5 SECOND TIME PENALTY FOR CAR 63 (RUS) - ..."
        let served = text.contains("PENALTY SERVED");
        let penalty = if let Some(c) = STOP_GO.captures(text) {
            Some((
                c[2].to_string(),
                Penalty::StopGo(c[1].parse().unwrap_or(10)),
            ))
        } else if let Some(c) = TIME_PENALTY.captures(text) {
            Some((c[2].to_string(), Penalty::Time(c[1].parse().unwrap_or(0))))
        } else {
            DRIVE_THROUGH
                .captures(text)
                .map(|c| (c[1].to_string(), Penalty::DriveThrough))
        };
        if let Some((car, kind)) = penalty {
            let car = cars.entry(car).or_default();
            if served {
                if let Some(i) = car
                    .penalties
                    .iter()
                    .position(|(pid, k)| *k == kind && (id.is_none() || *pid == id))
                {
                    car.penalties.remove(i);
                }
            } else {
                car.penalties.push((id.clone(), kind));
                close_investigation(&mut cars, id.as_deref());
            }
            continue;
        }

        if text.contains("UNDER INVESTIGATION") {
            if let Some(id) = id {
                for n in involved {
                    let car = cars.entry(n).or_default();
                    if !car.investigations.contains(&id) {
                        car.investigations.push(id.clone());
                    }
                }
            }
        } else if text.contains("NO FURTHER ACTION")
            || text.contains("NO FURTHER INVESTIGATION")
            || text.contains("NO INVESTIGATION")
        {
            close_investigation(&mut cars, id.as_deref());
        }
    }

    cars.into_iter()
        .filter_map(|(num, car)| {
            let tags = tags_of(&car);
            (!tags.is_empty()).then_some((num, tags))
        })
        .collect()
}

/// The stewards have decided: the incident is no longer under investigation.
fn close_investigation(cars: &mut HashMap<String, Car>, id: Option<&str>) {
    let Some(id) = id else { return };
    for car in cars.values_mut() {
        car.investigations.retain(|i| i != id);
    }
}

fn tags_of(car: &Car) -> Vec<Tag> {
    let mut tags = Vec::new();
    let mut add = |kind, text: &str| {
        tags.push(Tag {
            kind,
            text: text.to_string(),
        })
    };
    if car.disqualified {
        add("penalty", "DSQ");
    }
    if car
        .penalties
        .iter()
        .any(|(_, p)| *p == Penalty::DriveThrough)
    {
        add("penalty", "DT");
    }
    if let Some(secs) = car.penalties.iter().find_map(|(_, p)| match p {
        Penalty::StopGo(s) => Some(*s),
        _ => None,
    }) {
        add("penalty", &format!("S/G {secs}s"));
    }
    let time: u32 = car
        .penalties
        .iter()
        .map(|(_, p)| match p {
            Penalty::Time(s) => *s,
            _ => 0,
        })
        .sum();
    if time > 0 {
        add("penalty", &format!("+{time}s"));
    }
    if !car.investigations.is_empty() {
        add("investigation", "INV");
    }
    if car.black_and_white {
        add("warning", "B/W");
    }
    if car.reprimanded {
        add("warning", "REP");
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(lines: &[&str]) -> HashMap<String, Vec<Tag>> {
        let values: Vec<Value> = lines.iter().map(|l| json!({ "Message": l })).collect();
        let refs: Vec<&Value> = values.iter().collect();
        tags_by_car(&refs)
    }

    fn texts(tags: &HashMap<String, Vec<Tag>>, car: &str) -> Vec<String> {
        tags.get(car)
            .map(|t| t.iter().map(|t| t.text.clone()).collect())
            .unwrap_or_default()
    }

    // The messages of the 2026 United States Grand Prix, in order.
    const RACE: [&str; 12] = [
        "TURN 13 INCIDENT INVOLVING CAR 63 (RUS) NOTED - LEAVING THE TRACK AND GAINING AN ADVANTAGE (15:04:59)",
        "FIA STEWARDS: TURN 13 INCIDENT INVOLVING CAR 63 (RUS) UNDER INVESTIGATION - LEAVING THE TRACK AND GAINING AN ADVANTAGE (15:04:59)",
        "FIA STEWARDS: TURN 11 INCIDENT INVOLVING CAR 5 (BOR) UNDER INVESTIGATION - LEAVING THE TRACK AND GAINING AN ADVANTAGE (15:06:39)",
        "FIA STEWARDS: 5 SECOND TIME PENALTY FOR CAR 63 (RUS) - LEAVING THE TRACK AND GAINING AN ADVANTAGE (15:04:59)",
        "FIA STEWARDS: TURN 11 INCIDENT INVOLVING CAR 5 (BOR) NO FURTHER ACTION - LEAVING THE TRACK AND GAINING AN ADVANTAGE (15:06:39)",
        "FIA STEWARDS: TURN 13 INCIDENT INVOLVING CARS 87 (BEA) AND 10 (GAS) UNDER INVESTIGATION - CAUSING A COLLISION (15:06:57)",
        "FIA STEWARDS: 10 SECOND TIME PENALTY FOR CAR 87 (BEA) - CAUSING A COLLISION (15:06:57)",
        "FIA STEWARDS: PENALTY SERVED - 5 SECOND TIME PENALTY FOR CAR 63 (RUS) - LEAVING THE TRACK AND GAINING AN ADVANTAGE (15:04:59)",
        "FIA STEWARDS: PENALTY SERVED - 10 SECOND TIME PENALTY FOR CAR 87 (BEA) - CAUSING A COLLISION (15:06:57)",
        "TURN 1 INCIDENT INVOLVING CARS 43 (COL) AND 41 (LIN) NOTED (15:45:42)",
        "FIA STEWARDS: TURN 1 INCIDENT INVOLVING CARS 43 (COL) AND 41 (LIN) REVIEWED NO FURTHER INVESTIGATION (15:45:42)",
        "BLACK AND WHITE FLAG FOR CAR 63 (RUS) - TRACK LIMITS",
    ];

    #[test]
    fn an_investigation_shows_until_the_stewards_decide() {
        let tags = run(&RACE[..3]);
        assert_eq!(texts(&tags, "63"), ["INV"]);
        assert_eq!(texts(&tags, "5"), ["INV"]);
        assert_eq!(texts(&tags, "87"), Vec::<String>::new());
    }

    #[test]
    fn a_penalty_replaces_the_investigation_and_no_action_clears_it() {
        let tags = run(&RACE[..5]);
        assert_eq!(texts(&tags, "63"), ["+5s"]);
        assert!(texts(&tags, "5").is_empty()); // no further action
    }

    #[test]
    fn an_investigation_of_two_cars_marks_both_until_one_decision_closes_it() {
        let tags = run(&RACE[..6]);
        assert_eq!(texts(&tags, "87"), ["INV"]);
        assert_eq!(texts(&tags, "10"), ["INV"]);
        let tags = run(&RACE[..7]);
        assert_eq!(texts(&tags, "87"), ["+10s"]);
        assert!(texts(&tags, "10").is_empty()); // the incident is decided
    }

    #[test]
    fn a_served_penalty_goes_away() {
        let tags = run(&RACE[..9]);
        assert!(texts(&tags, "63").is_empty());
        assert!(texts(&tags, "87").is_empty());
    }

    #[test]
    fn the_whole_race_leaves_only_the_warning() {
        let tags = run(&RACE);
        assert_eq!(texts(&tags, "63"), ["B/W"]);
        assert_eq!(tags["63"][0].kind, "warning");
        assert_eq!(tags.len(), 1); // nobody else has anything outstanding
    }

    #[test]
    fn time_penalties_add_up_and_other_kinds_get_their_own_tag() {
        let tags = run(&[
            "FIA STEWARDS: 5 SECOND TIME PENALTY FOR CAR 1 (VER) - SPEEDING IN THE PIT LANE (10:00:00)",
            "FIA STEWARDS: 10 SECOND TIME PENALTY FOR CAR 1 (VER) - CAUSING A COLLISION (10:05:00)",
            "FIA STEWARDS: DRIVE THROUGH PENALTY FOR CAR 4 (NOR) - JUMP START (10:06:00)",
            "FIA STEWARDS: 10 SECOND STOP/GO PENALTY FOR CAR 44 (HAM) - CAUSING A COLLISION (10:07:00)",
        ]);
        assert_eq!(texts(&tags, "1"), ["+15s"]);
        assert_eq!(texts(&tags, "4"), ["DT"]);
        assert_eq!(texts(&tags, "44"), ["S/G 10s"]);
        assert_eq!(tags["4"][0].kind, "penalty");
    }

    #[test]
    fn serving_one_of_two_penalties_leaves_the_other() {
        let tags = run(&[
            "FIA STEWARDS: 5 SECOND TIME PENALTY FOR CAR 1 (VER) - A (10:00:00)",
            "FIA STEWARDS: 10 SECOND TIME PENALTY FOR CAR 1 (VER) - B (10:05:00)",
            "FIA STEWARDS: PENALTY SERVED - 5 SECOND TIME PENALTY FOR CAR 1 (VER) - A (10:00:00)",
        ]);
        assert_eq!(texts(&tags, "1"), ["+10s"]);
    }

    #[test]
    fn disqualification_and_reprimand() {
        let tags = run(&[
            "FIA STEWARDS: REPRIMAND FOR CAR 5 (BOR) - A (10:00:00)",
            "CAR 12 (ANT) DISQUALIFIED FROM THE RESULTS",
        ]);
        assert_eq!(texts(&tags, "5"), ["REP"]);
        assert_eq!(texts(&tags, "12"), ["DSQ"]);
    }

    #[test]
    fn unrelated_messages_do_nothing() {
        let tags = run(&[
            "RACE START",
            "DRS ENABLED",
            "",
            "TRACK SURFACE SLIPPERY IN TRACK SECTOR 2",
        ]);
        assert!(tags.is_empty());
    }
}
