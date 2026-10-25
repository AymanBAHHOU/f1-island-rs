//! Turns live timing events into island alerts: a coloured tile, a title and a line of
//! detail, shown for a few seconds. (Port of the Electron app's `src/alerts.js`, and of
//! the alert rules in its `main.js`.)

use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::sync::LazyLock;

/// `level` is one of red, sc, vsc, yellow, green, chequered, info (the page colours by it);
/// `tile` is the short text on the coloured flag tile.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Alert {
    pub level: &'static str,
    pub tile: &'static str,
    pub title: String,
    pub sub: String,
    pub ms: u32,
}

fn alert(level: &'static str, tile: &'static str, title: &str, sub: &str, ms: u32) -> Alert {
    Alert {
        level,
        tile,
        title: title.to_string(),
        sub: sub.to_string(),
        ms,
    }
}

/// Track status codes from live timing.
#[derive(Clone, Copy, PartialEq)]
enum Track {
    Clear,
    Yellow,
    SafetyCar,
    Red,
    Vsc,
    VscEnding,
}

fn track_of(code: Option<&str>) -> Option<Track> {
    match code? {
        "1" => Some(Track::Clear),
        "2" => Some(Track::Yellow),
        "4" => Some(Track::SafetyCar),
        "5" => Some(Track::Red),
        "6" => Some(Track::Vsc),
        "7" => Some(Track::VscEnding),
        _ => None,
    }
}

pub fn from_track(status: Option<&str>, prev: Option<&str>) -> Option<Alert> {
    let neutralised = |t| {
        matches!(
            t,
            Some(Track::SafetyCar | Track::Red | Track::Vsc | Track::VscEnding)
        )
    };
    match track_of(status)? {
        Track::Red => Some(alert("red", "", "Red flag", "Session suspended", 10_000)),
        Track::SafetyCar => Some(alert(
            "sc",
            "SC",
            "Safety Car",
            "Safety Car deployed",
            10_000,
        )),
        Track::Vsc => Some(alert(
            "vsc",
            "VSC",
            "Virtual Safety Car",
            "VSC deployed",
            9_000,
        )),
        Track::VscEnding => Some(alert(
            "vsc",
            "VSC",
            "VSC ending",
            "Racing resumes shortly",
            6_000,
        )),
        // Only worth a pop-up after a neutralisation.
        Track::Clear => neutralised(track_of(prev))
            .then(|| alert("green", "", "Track clear", "Green flag", 6_000)),
        // Single yellows are frequent: shown as pill status only.
        Track::Yellow => None,
    }
}

macro_rules! pattern {
    ($name:ident, $re:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new(concat!("(?i)", $re)).unwrap());
    };
}

pattern!(IGNORE, r"DELETED|TRACK LIMITS|BLUE FLAG");
pattern!(
    DELAYED,
    r"DELAYED START|START (IS |WILL BE )?DELAYED|\bDELAYED\b"
);
pattern!(SUSPENDED, r"SUSPENDED");
pattern!(
    START_PROCEDURE,
    r"FORMATION LAP|STANDING START|ROLLING START|START BEHIND THE SAFETY CAR"
);
pattern!(RESTART, r"WILL (RE)?START|WILL RESUME|RESUMPTION|\bRESUME");
pattern!(WET, r"WET (RACE|SESSION|TRACK)|DECLARED WET");
pattern!(CONDITIONS, r"SLIPPERY|LOW GRIP|CLIMATIC CONDITIONS");
pattern!(OVERTAKE, r"(OVERTAKE|DRS) (ENABLED|DISABLED)");
pattern!(DISABLED, r"DISABLED");
pattern!(PENALTY, r"PENALTY");
pattern!(INVESTIGATION, r"UNDER INVESTIGATION|WILL BE INVESTIGATED");
pattern!(NO_ACTION, r"NO FURTHER (ACTION|INVESTIGATION)");
pattern!(RAIN, r"RAIN");

/// An alert for a race control message (`Message`, `Flag`, `Category`), if it deserves one.
pub fn from_message(m: &Value) -> Option<Alert> {
    let text = m["Message"].as_str().unwrap_or("").trim();
    if text.is_empty() {
        return None;
    }
    let sub: String = if text.chars().count() > 90 {
        format!("{}…", text.chars().take(88).collect::<String>())
    } else {
        text.to_string()
    };
    let flag = m["Flag"].as_str();

    if flag == Some("CHEQUERED") {
        return Some(alert(
            "chequered",
            "",
            "Chequered flag",
            "Session over",
            8_000,
        ));
    }
    if flag == Some("DOUBLE YELLOW") {
        return Some(alert("yellow", "!!", "Double yellow", &sub, 6_000));
    }
    // Red / SC / VSC arrive through track status as well; avoid doubles.
    if flag == Some("RED") || m["Category"].as_str() == Some("SafetyCar") {
        return None;
    }

    // Session control: delays, suspensions, restarts, start procedures.
    if DELAYED.is_match(text) {
        return Some(alert("yellow", "DLY", "Start delayed", &sub, 9_000));
    }
    if SUSPENDED.is_match(text) {
        return Some(alert("red", "", "Session suspended", &sub, 9_000));
    }
    // Start procedures first: "WILL START BEHIND THE SAFETY CAR" isn't a restart.
    if START_PROCEDURE.is_match(text) {
        return Some(alert("info", "LAP", "Start procedure", &sub, 8_000));
    }
    if RESTART.is_match(text) {
        return Some(alert("green", "GO", "Restart", &sub, 8_000));
    }
    if WET.is_match(text) {
        return Some(alert("info", "RAIN", "Wet conditions", &sub, 8_000));
    }

    // Track conditions (often one message per sector: the caller rate-limits these).
    if CONDITIONS.is_match(text) {
        return Some(alert("yellow", "GRIP", "Track conditions", &sub, 7_000));
    }
    if OVERTAKE.is_match(text) {
        let title = if DISABLED.is_match(text) {
            "Overtake disabled"
        } else {
            "Overtake enabled"
        };
        return Some(alert("info", "OVT", title, &sub, 6_000));
    }

    if PENALTY.is_match(text) {
        return Some(alert("info", "PEN", "Penalty", &sub, 8_000));
    }
    if INVESTIGATION.is_match(text) {
        return Some(alert("info", "INV", "Under investigation", &sub, 7_000));
    }
    if NO_ACTION.is_match(text) {
        return Some(alert("info", "FIA", "Stewards", &sub, 6_000));
    }
    if RAIN.is_match(text) && !IGNORE.is_match(text) {
        return Some(alert("info", "RAIN", "Weather", &sub, 7_000));
    }
    None
}

pub fn from_session_status(status: &str, session_name: Option<&str>) -> Option<Alert> {
    match status {
        "Started" => Some(alert(
            "green",
            "",
            &format!("{} underway", session_name.unwrap_or("Session")),
            "Green light",
            6_000,
        )),
        "Aborted" => Some(alert(
            "red",
            "",
            "Session suspended",
            session_name.unwrap_or(""),
            8_000,
        )),
        _ => None,
    }
}

/// "Preview alerts": one of each kind.
pub fn demo() -> Vec<Alert> {
    vec![
        alert("green", "", "Sprint underway", "Green light", 4_000),
        alert(
            "yellow",
            "!!",
            "Double yellow",
            "DOUBLE YELLOW IN TRACK SECTOR 7",
            4_000,
        ),
        alert("vsc", "VSC", "Virtual Safety Car", "VSC deployed", 4_000),
        alert("sc", "SC", "Safety Car", "Safety Car deployed", 4_000),
        alert("red", "", "Red flag", "Session suspended", 4_000),
        alert(
            "info",
            "PEN",
            "Penalty",
            "FIA STEWARDS: 5 SECOND TIME PENALTY FOR CAR 44 (HAM) - CAUSING A COLLISION",
            5_000,
        ),
        alert("chequered", "", "Chequered flag", "Session over", 4_000),
    ]
}

/// Adds the lap to an alert that is about the race itself ("Safety Car deployed · Lap 12").
pub fn with_lap(mut alert: Alert, lap: Option<i64>) -> Alert {
    if let (Some(lap), true) = (lap.filter(|l| *l != 0), alert.level != "info") {
        alert.sub = format!("{} · Lap {lap}", alert.sub);
    }
    alert
}

/// Remembers when each bursty kind of alert last showed, to space them out.
#[derive(Default)]
pub struct Limiter {
    last_double_yellow: Option<i64>,
    last_conditions: Option<i64>,
}

/// Double yellows come in bursts (marshals clearing the track, pre-start inspections): pop
/// them only while the session runs, at most every 45 s.
const DOUBLE_YELLOW_GAP_MS: i64 = 45_000;
/// Conditions messages arrive per sector; one pop-up per couple of minutes is plenty.
const CONDITIONS_GAP_MS: i64 = 120_000;

impl Limiter {
    /// The alert for a race control message, unless it comes too soon after a similar one.
    pub fn message(&mut self, m: &Value, session_running: bool, now_ms: i64) -> Option<Alert> {
        let due = |last: Option<i64>, gap| last.is_none_or(|l| now_ms - l >= gap);
        if m["Flag"].as_str() == Some("DOUBLE YELLOW") {
            if !session_running || !due(self.last_double_yellow, DOUBLE_YELLOW_GAP_MS) {
                return None;
            }
            self.last_double_yellow = Some(now_ms);
        }
        let found = from_message(m)?;
        if found.title == "Track conditions" {
            if !due(self.last_conditions, CONDITIONS_GAP_MS) {
                return None;
            }
            self.last_conditions = Some(now_ms);
        }
        Some(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(text: &str) -> Value {
        json!({"Message": text})
    }

    #[test]
    fn track_flags() {
        assert_eq!(from_track(Some("5"), Some("1")).unwrap().title, "Red flag");
        assert_eq!(from_track(Some("4"), None).unwrap().tile, "SC");
        assert_eq!(from_track(Some("6"), None).unwrap().ms, 9_000);
        assert_eq!(from_track(Some("7"), None).unwrap().title, "VSC ending");
        assert_eq!(from_track(Some("2"), Some("1")), None); // single yellow: no pop-up
        assert_eq!(from_track(Some("9"), None), None);
    }

    #[test]
    fn green_only_after_a_neutralisation() {
        assert_eq!(from_track(Some("1"), Some("2")), None);
        assert_eq!(from_track(Some("1"), None), None);
        for before in ["4", "5", "6", "7"] {
            assert_eq!(
                from_track(Some("1"), Some(before)).unwrap().title,
                "Track clear"
            );
        }
    }

    #[test]
    fn messages_are_recognised() {
        let title = |t: &str| from_message(&msg(t)).map(|a| a.title);
        assert_eq!(
            title("RACE START DELAYED").as_deref(),
            Some("Start delayed")
        );
        assert_eq!(
            title("SESSION SUSPENDED").as_deref(),
            Some("Session suspended")
        );
        assert_eq!(
            title("FORMATION LAP WILL START AT 17:30").as_deref(),
            Some("Start procedure")
        );
        assert_eq!(
            title("RACE WILL START BEHIND THE SAFETY CAR").as_deref(),
            Some("Start procedure")
        );
        assert_eq!(
            title("SESSION WILL RESUME AT 15:10").as_deref(),
            Some("Restart")
        );
        assert_eq!(title("RISK OF RAIN FOR TURN 3").as_deref(), Some("Weather"));
        assert_eq!(
            title("DECLARED WET RACE").as_deref(),
            Some("Wet conditions")
        );
        assert_eq!(
            title("SLIPPERY TRACK SECTOR 2").as_deref(),
            Some("Track conditions")
        );
        assert_eq!(title("DRS ENABLED").as_deref(), Some("Overtake enabled"));
        assert_eq!(
            title("OVERTAKE DISABLED").as_deref(),
            Some("Overtake disabled")
        );
        assert_eq!(
            title("FIA STEWARDS: 5 SECOND TIME PENALTY FOR CAR 44").as_deref(),
            Some("Penalty")
        );
        assert_eq!(
            title("CAR 4 UNDER INVESTIGATION").as_deref(),
            Some("Under investigation")
        );
        assert_eq!(title("NO FURTHER ACTION").as_deref(), Some("Stewards"));
        assert_eq!(title("CAR 1 TIME 1:30.1 DELETED - TRACK LIMITS"), None);
        assert_eq!(title("   "), None);
    }

    #[test]
    fn flags_come_first() {
        let chequered = json!({"Message": "CHEQUERED FLAG", "Flag": "CHEQUERED"});
        assert_eq!(from_message(&chequered).unwrap().level, "chequered");
        let double = json!({"Message": "DOUBLE YELLOW IN TRACK SECTOR 7", "Flag": "DOUBLE YELLOW"});
        let a = from_message(&double).unwrap();
        assert_eq!(
            (a.tile, a.sub.as_str()),
            ("!!", "DOUBLE YELLOW IN TRACK SECTOR 7")
        );
        // Red and safety car arrive via track status too.
        assert_eq!(
            from_message(&json!({"Message": "RED FLAG", "Flag": "RED"})),
            None
        );
        assert_eq!(
            from_message(&json!({"Message": "SAFETY CAR DEPLOYED", "Category": "SafetyCar"})),
            None
        );
    }

    #[test]
    fn long_details_are_cut() {
        let long = "PENALTY ".to_string() + &"X".repeat(200);
        let a = from_message(&msg(&long)).unwrap();
        assert_eq!(a.sub.chars().count(), 89);
        assert!(a.sub.ends_with('…'));
    }

    #[test]
    fn session_status_alerts() {
        assert_eq!(
            from_session_status("Started", Some("Race")).unwrap().title,
            "Race underway"
        );
        assert_eq!(
            from_session_status("Started", None).unwrap().title,
            "Session underway"
        );
        assert_eq!(
            from_session_status("Aborted", Some("Race")).unwrap().sub,
            "Race"
        );
        assert_eq!(from_session_status("Finished", Some("Race")), None);
    }

    #[test]
    fn the_lap_is_added_except_to_info_alerts() {
        let red = from_track(Some("5"), None).unwrap();
        assert_eq!(
            with_lap(red.clone(), Some(12)).sub,
            "Session suspended · Lap 12"
        );
        assert_eq!(with_lap(red, None).sub, "Session suspended");
        let info = from_message(&msg("NO FURTHER ACTION")).unwrap();
        assert_eq!(with_lap(info, Some(3)).sub, "NO FURTHER ACTION");
    }

    #[test]
    fn bursts_are_spaced_out() {
        let mut limiter = Limiter::default();
        let dy = json!({"Message": "DOUBLE YELLOW IN TRACK SECTOR 7", "Flag": "DOUBLE YELLOW"});
        assert!(limiter.message(&dy, false, 0).is_none()); // session not running
        assert!(limiter.message(&dy, true, 1_000).is_some());
        assert!(limiter.message(&dy, true, 30_000).is_none()); // too soon
        assert!(limiter.message(&dy, true, 50_000).is_some());

        let grip = msg("SLIPPERY TRACK SECTOR 2");
        assert!(limiter.message(&grip, true, 0).is_some());
        assert!(limiter.message(&grip, true, 100_000).is_none());
        assert!(limiter.message(&grip, true, 130_000).is_some());
        // everything else always passes
        assert!(limiter
            .message(&msg("NO FURTHER ACTION"), true, 1)
            .is_some());
    }

    #[test]
    fn there_is_a_demo_of_each_kind() {
        let levels: Vec<_> = demo().iter().map(|a| a.level).collect();
        assert_eq!(
            levels,
            ["green", "yellow", "vsc", "sc", "red", "info", "chequered"]
        );
    }
}
