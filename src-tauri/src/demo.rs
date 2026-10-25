//! Explicitly labelled synthetic data, used by previews only: a pretend weekend, and
//! (for the racing modes) a pretend race with a running order and race control messages.
//! (Port of the Electron app's `src/demo.js`.)

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Value};

pub const MODES: [&str; 5] = ["next", "race", "safety-car", "red-flag", "penalty"];

/// A plausible running order for the timing tower: (code, team colour).
const FIELD: [(&str, &str); 20] = [
    ("VER", "#3671C6"),
    ("NOR", "#FF8000"),
    ("LEC", "#E8002D"),
    ("ANT", "#27F4D2"),
    ("RUS", "#27F4D2"),
    ("HAM", "#E8002D"),
    ("PIA", "#FF8000"),
    ("HAD", "#3671C6"),
    ("ALO", "#229971"),
    ("GAS", "#00A1E8"),
    ("LAW", "#6692FF"),
    ("SAI", "#64C4FF"),
    ("ALB", "#64C4FF"),
    ("HUL", "#F50537"),
    ("OCO", "#B6BABD"),
    ("BEA", "#B6BABD"),
    ("STR", "#229971"),
    ("LIN", "#6692FF"),
    ("BOR", "#F50537"),
    ("COL", "#00A1E8"),
];

fn iso(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn demo_order() -> Vec<Value> {
    let mut gap = 0.0;
    FIELD
        .iter()
        .enumerate()
        .map(|(i, (code, color))| {
            let interval = if i > 0 {
                0.4 + ((i * 37) % 23) as f64 / 10.0
            } else {
                0.0
            };
            gap += interval;
            let compound = if i % 4 == 0 {
                "MEDIUM"
            } else if i % 5 == 0 {
                "SOFT"
            } else {
                "HARD"
            };
            let third = json!({
                "value": format!("{:.3}", 28.1 + i as f64 * 0.04),
                "pb": i % 5 == 0,
                "overall": false,
                "previous": i % 6 == 5,
            });
            json!({
                "pos": i + 1,
                "num": (i + 1).to_string(),
                "code": code,
                "color": color,
                "gap": if i > 0 { format!("+{gap:.3}") } else { "LAP 24".to_string() },
                "interval": if i > 0 { format!("+{interval:.3}") } else { "LAP 24".to_string() },
                "best": format!("1:{:.3}", 30.2 + i as f64 * 0.11 + if i == 4 { 0.9 } else { 0.0 }),
                "last": null,
                "laps": 24,
                "pits": if i % 3 == 0 { 2 } else { 1 },
                "pitTime": if i % 4 == 1 { json!(format!("{:.1}", 21.5 + i as f64 * 0.3)) } else { Value::Null },
                "pitLap": if i % 4 == 1 { json!(11 + i) } else { Value::Null },
                "tags": match i {
                    3 => json!([{"kind": "penalty", "text": "+5s"}]),
                    6 => json!([{"kind": "investigation", "text": "INV"}]),
                    9 => json!([{"kind": "warning", "text": "B/W"}, {"kind": "penalty", "text": "+10s"}]),
                    12 => json!([{"kind": "penalty", "text": "DT"}]),
                    _ => json!([]),
                },
                "inPit": *code == "SAI",
                "pitOut": *code == "LAW",
                "out": *code == "COL",
                "knockedOut": false,
                "lastLap": {
                    "value": format!("1:{:.3}", 31.0 + i as f64 * 0.13),
                    "pb": i % 4 == 1,
                    "overall": i == 2,
                },
                "sectors": [
                    {"value": format!("{:.3}", 28.4 + i as f64 * 0.03), "pb": i % 3 == 0, "overall": i == 0},
                    {"value": format!("{:.3}", 34.2 + i as f64 * 0.05), "pb": false, "overall": i == 3},
                    third,
                ],
                "tyre": {"compound": compound, "laps": 4 + ((i * 7) % 15), "fresh": i % 2 == 0},
            })
        })
        .collect()
}

/// A demo for `mode` (one of `MODES`), as of `now_ms`. `None` for an unknown mode.
pub fn create(mode: &str, now_ms: i64) -> Option<Value> {
    if !MODES.contains(&mode) {
        return None;
    }
    let racing = mode != "next";
    let start = now_ms
        + if racing {
            -35 * 60_000
        } else {
            3 * 3_600_000 + 12 * 60_000 + 8_000
        };
    let session = json!({
        "name": if racing { "Race" } else { "Qualifying" },
        "short": if racing { "RACE" } else { "Q" },
        "start": iso(start),
        "end": iso(start + 2 * 3_600_000),
    });
    let message = match mode {
        "safety-car" => "SAFETY CAR DEPLOYED — debris at Turn 4",
        "red-flag" => "RED FLAG — session suspended, barrier repairs at Turn 9",
        "penalty" => "CAR 16 (LEC) — 5 SECOND TIME PENALTY — track limits",
        _ => "OVERTAKE — NOR passes LEC for P2 at Turn 6",
    };
    let flag = match mode {
        "red-flag" => "RED",
        "safety-car" => "YELLOW",
        _ => "",
    };
    let year = chrono::Datelike::year(&DateTime::<Utc>::from_timestamp_millis(now_ms)?);
    let live = if racing {
        json!({
            "session": "Race",
            "kind": "race",
            "order": demo_order(),
            "status": if mode == "red-flag" { "Aborted" } else { "Started" },
            "track": {"status": match mode { "safety-car" => "4", "red-flag" => "5", _ => "1" }},
            "lap": 24,
            "totalLaps": 52,
            "top": [
                {"pos": 1, "code": "VER", "color": "#3671C6"},
                {"pos": 2, "code": "NOR", "color": "#FF8000"},
                {"pos": 3, "code": "LEC", "color": "#E8002D"},
            ],
            "messages": [
                {"utc": iso(now_ms), "flag": flag,
                 "category": if mode == "safety-car" { "SafetyCar" } else { "Other" },
                 "message": message},
                {"utc": iso(now_ms - 45_000), "flag": "", "category": "Other",
                 "message": "PIT STOP — RUS switches to medium tyres"},
            ],
        })
    } else {
        Value::Null
    };
    Some(json!({
        "mode": mode,
        "snapshot": {
            "demo": true,
            "weekend": {
                "name": "DEMO · British Grand Prix",
                "round": "12",
                "season": year.to_string(),
                "circuit": "Silverstone",
                "country": "United Kingdom",
                "sessions": [session],
            },
            "standings": [],
            "constructors": [],
            "lastRace": null,
        },
        "live": live,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_792_900_000_000; // 2026-10-25

    #[test]
    fn unknown_modes_give_nothing() {
        assert!(create("nonsense", NOW).is_none());
    }

    #[test]
    fn the_next_session_demo_has_no_live_data() {
        let demo = create("next", NOW).unwrap();
        assert!(demo["live"].is_null());
        let session = &demo["snapshot"]["weekend"]["sessions"][0];
        assert_eq!(session["short"], "Q");
        assert_eq!(demo["snapshot"]["weekend"]["season"], "2026");
        // starts 3 h 12 min 8 s from now
        let start = chrono::DateTime::parse_from_rfc3339(session["start"].as_str().unwrap())
            .unwrap()
            .timestamp_millis();
        assert_eq!(start - NOW, 3 * 3_600_000 + 12 * 60_000 + 8_000);
    }

    #[test]
    fn racing_demos_have_a_full_grid_and_the_right_flag() {
        let sc = create("safety-car", NOW).unwrap();
        assert_eq!(sc["live"]["order"].as_array().unwrap().len(), 20);
        assert_eq!(sc["live"]["track"]["status"], "4");
        assert_eq!(sc["live"]["messages"][0]["category"], "SafetyCar");
        let red = create("red-flag", NOW).unwrap();
        assert_eq!(red["live"]["status"], "Aborted");
        assert_eq!(red["live"]["messages"][0]["flag"], "RED");
        assert_eq!(create("race", NOW).unwrap()["live"]["status"], "Started");
    }

    #[test]
    fn the_running_order_is_made_of_plausible_numbers() {
        let order = demo_order();
        assert_eq!(order[0]["gap"], "LAP 24");
        assert_eq!(order[1]["interval"], "+1.800"); // 0.4 + (37 % 23) / 10
        assert_eq!(order[0]["lastLap"]["value"], "1:31.000");
        assert_eq!(order[4]["best"], "1:31.540"); // 30.2 + 0.44 + 0.9
        assert_eq!(order[11]["inPit"], true); // SAI
        assert_eq!(order[10]["pitOut"], true); // LAW
        assert_eq!(order[19]["out"], true); // COL
        assert_eq!(order[5]["sectors"][2]["previous"], true);
        assert_eq!(order[4]["sectors"][2]["previous"], false);
    }
}
