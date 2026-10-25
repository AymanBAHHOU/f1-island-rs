//! The state of a live session, built from what the live-timing feed sends, and turned
//! into the running order and summary the island shows. No network here: messages go in
//! as JSON values and events and summaries come out, so it is tested with samples.
//! (Port of the state half of the Electron app's `src/livetiming.js`.)
//!
//! The feed is "SignalR": JSON messages with a number for their kind. Type 3 answers our
//! subscription with a snapshot of every topic; type 1 pushes a delta for one topic;
//! type 7 says the server is closing the connection.

use crate::merge::merge_into;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;

/// How many race control messages the summary carries (newest first).
const MESSAGES_SHOWN: usize = 40;

// ---------- reading the feed's JSON ----------
//
// The feed is loose about its types: a time may be a plain string or an object with a
// `Value` inside, a list may arrive as an array or as an object keyed by index. These
// helpers hide that.

/// Text in a JSON value; empty text counts as nothing.
fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A time or gap: `"+0.213"` or `{"Value": "+0.213", ...}`.
fn value(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Object(o) => o.get("Value").and_then(text),
        other => text(other),
    }
}

/// Truthiness, the way the feed means it (a missing value, `false`, `0` and `""` are off).
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// A list that may arrive as an array or, when built from deltas, as an object keyed by index.
fn list(v: Option<&Value>) -> Vec<&Value> {
    match v {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(map)) => {
            let mut entries: Vec<(u64, &Value)> = map
                .iter()
                .map(|(k, v)| (k.parse().unwrap_or(u64::MAX), v))
                .collect();
            entries.sort_by_key(|(index, _)| *index); // numerically: "10" after "2"
            entries.into_iter().map(|(_, v)| v).collect()
        }
        _ => Vec::new(),
    }
}

fn string_at(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(text)
}

// ---------- what the page gets ----------

/// A time with the timing screens' colour flags.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct Timed {
    pub value: Option<String>,
    /// Personal best (green).
    pub pb: bool,
    /// Fastest overall (purple).
    pub overall: bool,
    /// A sector that still shows the previous lap's time.
    pub previous: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tyre {
    pub compound: String,
    pub laps: Option<i64>,
    pub fresh: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub pos: u32,
    pub num: String,
    pub code: String,
    pub color: Option<String>,
    pub gap: Option<String>,
    pub interval: Option<String>,
    pub best: Option<String>,
    pub last: Option<String>,
    pub last_lap: Timed,
    pub sectors: Vec<Timed>,
    pub laps: Option<i64>,
    pub pits: i64,
    pub in_pit: bool,
    pub pit_out: bool,
    pub out: bool,
    pub knocked_out: bool,
    pub tyre: Option<Tyre>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Track {
    pub status: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Clock {
    pub remaining: Option<String>,
    pub utc: Option<String>,
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Message {
    /// With a "Z": the feed's times are UTC but come without a zone.
    pub utc: String,
    pub category: Option<String>,
    pub flag: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Top {
    pub pos: u32,
    pub code: String,
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// "race", "qualifying" or "practice".
    pub kind: String,
    pub order: Vec<Row>,
    pub session: Option<String>,
    pub meeting: Option<String>,
    pub status: Option<String>,
    /// The qualifying segment: 1, 2 or 3.
    pub part: Option<u32>,
    pub track: Option<Track>,
    pub lap: Option<i64>,
    pub total_laps: Option<i64>,
    pub remaining: Option<String>,
    pub clock: Option<Clock>,
    /// The track's local time zone offset, e.g. "08:00:00".
    pub gmt_offset: Option<String>,
    pub messages: Vec<Message>,
    pub top: Vec<Top>,
}

// ---------- events ----------

/// Things that happened that the app may want to react to (an alert, a log line).
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The track flag changed (green, yellow, safety car, ...).
    Track {
        status: Option<String>,
        message: Option<String>,
        prev: Option<String>,
    },
    /// A new race control message.
    RaceControl(Value),
    /// The session status changed ("Started", "Finished", ...).
    Status(String),
    /// We (re)connected and got the full state; carries the session's status at that moment.
    Snapshot(Option<String>),
    /// The server is closing the connection: reconnect.
    ServerClosed,
}

fn message_key(m: &Value) -> String {
    format!(
        "{}|{}",
        string_at(m, "Utc").unwrap_or_default(),
        string_at(m, "Message").unwrap_or_default()
    )
}

// ---------- the state ----------

#[derive(Debug, Default)]
pub struct LiveState {
    state: Value,
    /// Race control messages already seen, so one isn't announced twice.
    seen: HashSet<String>,
}

impl LiveState {
    pub fn new() -> Self {
        Self {
            state: Value::Object(Default::default()),
            seen: HashSet::new(),
        }
    }

    /// Forgets everything (when we disconnect).
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    fn at(&self, pointer: &str) -> Option<&Value> {
        self.state.pointer(pointer)
    }

    fn at_text(&self, pointer: &str) -> Option<String> {
        self.at(pointer).and_then(text)
    }

    /// Handles one SignalR message from the feed and says what happened.
    pub fn handle(&mut self, msg: &Value) -> Vec<Event> {
        match msg.get("type").and_then(Value::as_i64) {
            Some(3) if msg.get("invocationId").and_then(Value::as_str) == Some("sub") => {
                self.apply_snapshot(msg.get("result").cloned().unwrap_or(Value::Null))
            }
            Some(1) => match msg.get("arguments").and_then(Value::as_array) {
                Some(args) => match args.first().and_then(Value::as_str) {
                    Some(topic) => self.apply(topic, args.get(1).cloned().unwrap_or(Value::Null)),
                    None => vec![],
                },
                None => vec![],
            },
            Some(7) => vec![Event::ServerClosed],
            _ => vec![],
        }
    }

    /// The snapshot replaces everything. The race control messages in it are history, not
    /// news: they are remembered but not announced.
    fn apply_snapshot(&mut self, result: Value) -> Vec<Event> {
        let prev_track = self.at_text("/TrackStatus/Status");
        self.state = if result.is_object() {
            result
        } else {
            Value::Object(Default::default())
        };
        let keys: Vec<String> = self.messages().iter().map(|m| message_key(m)).collect();
        self.seen.extend(keys);

        let mut events = Vec::new();
        let track = self.at_text("/TrackStatus/Status");
        if prev_track.is_some() && track.is_some() && track != prev_track {
            events.push(Event::Track {
                status: track,
                message: self.at_text("/TrackStatus/Message"),
                prev: prev_track,
            });
        }
        events.push(Event::Snapshot(self.at_text("/SessionStatus/Status")));
        events
    }

    fn apply(&mut self, topic: &str, data: Value) -> Vec<Event> {
        let mut events = Vec::new();
        match topic {
            "TrackStatus" => {
                let prev = self.at_text("/TrackStatus/Status");
                self.merge_topic(topic, data);
                let status = self.at_text("/TrackStatus/Status");
                if status != prev {
                    events.push(Event::Track {
                        status,
                        message: self.at_text("/TrackStatus/Message"),
                        prev,
                    });
                }
            }
            "SessionStatus" => {
                let prev = self.at_text("/SessionStatus/Status");
                self.merge_topic(topic, data);
                if let Some(status) = self
                    .at_text("/SessionStatus/Status")
                    .filter(|s| Some(s) != prev.as_ref())
                {
                    events.push(Event::Status(status));
                }
            }
            "RaceControlMessages" => {
                self.merge_topic(topic, data);
                let fresh: Vec<Value> = self
                    .messages()
                    .into_iter()
                    .filter(|m| !self.seen_before(m))
                    .cloned()
                    .collect();
                for m in fresh {
                    self.seen.insert(message_key(&m));
                    events.push(Event::RaceControl(m));
                }
            }
            _ => self.merge_topic(topic, data),
        }
        events
    }

    fn merge_topic(&mut self, topic: &str, data: Value) {
        if let Value::Object(map) = &mut self.state {
            merge_into(map.entry(topic.to_string()).or_insert(Value::Null), data);
        }
    }

    fn seen_before(&self, m: &Value) -> bool {
        self.seen.contains(&message_key(m))
    }

    /// The race control messages, oldest first.
    fn messages(&self) -> Vec<&Value> {
        list(self.at("/RaceControlMessages/Messages"))
    }

    // ---------- the running order ----------

    /// The whole field, in order. Races report the gap to the leader and the interval to
    /// the car ahead; practice and qualifying report the gap to the fastest lap.
    pub fn order(&self) -> Vec<Row> {
        let race = self.at_text("/SessionInfo/Type").as_deref() == Some("Race");
        let quali = self.at_text("/SessionInfo/Type").as_deref() == Some("Qualifying");
        let part = self
            .at("/TimingData/SessionPart")
            .and_then(as_u32)
            .unwrap_or(0);
        let empty = Value::Null;
        let drivers = self.at("/DriverList").unwrap_or(&empty);
        let tyres = self.at("/TimingAppData/Lines").unwrap_or(&empty);

        let Some(Value::Object(lines)) = self.at("/TimingData/Lines") else {
            return Vec::new();
        };

        let mut rows: Vec<Row> = lines
            .iter()
            .filter_map(|(num, l)| {
                let pos = l.get("Position").and_then(as_u32).filter(|p| *p > 0)?;
                // Qualifying has a best lap and gaps per segment; the other sessions have one.
                let seg = if quali && part > 0 {
                    Some(segment(l, part))
                } else {
                    None
                };
                let (best, gap, interval) = match seg {
                    Some(Some(s)) => (s.best, s.gap, s.interval),
                    Some(None) => (None, None, None), // no time in this segment yet
                    None => (
                        value(l.get("BestLapTime")).or_else(|| {
                            list(l.get("BestLapTimes"))
                                .into_iter()
                                .filter_map(|t| value(Some(t)))
                                .next_back() // the newest: the last one that has a time
                        }),
                        value(l.get(if race {
                            "GapToLeader"
                        } else {
                            "TimeDiffToFastest"
                        })),
                        value(l.get(if race {
                            "IntervalToPositionAhead"
                        } else {
                            "TimeDiffToPositionAhead"
                        })),
                    ),
                };
                let sectors = list(l.get("Sectors"));
                let stint = list(tyres.get(num).and_then(|t| t.get("Stints")))
                    .last()
                    .copied();
                let driver = drivers.get(num);

                Some(Row {
                    pos,
                    num: num.clone(),
                    code: driver
                        .and_then(|d| string_at(d, "Tla"))
                        .unwrap_or_else(|| format!("#{num}")),
                    color: driver
                        .and_then(|d| string_at(d, "TeamColour"))
                        .map(|c| format!("#{c}")),
                    gap,
                    interval,
                    best,
                    last: value(l.get("LastLapTime")),
                    last_lap: timed(l.get("LastLapTime")),
                    sectors: (0..3).map(|i| sector(sectors.get(i).copied())).collect(),
                    laps: l.get("NumberOfLaps").and_then(Value::as_i64),
                    pits: l
                        .get("NumberOfPitStops")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                    in_pit: truthy(l.get("InPit")),
                    pit_out: truthy(l.get("PitOut")),
                    out: truthy(l.get("Retired")) || truthy(l.get("Stopped")),
                    knocked_out: truthy(l.get("KnockedOut")),
                    tyre: stint.and_then(|s| {
                        Some(Tyre {
                            compound: string_at(s, "Compound")?,
                            laps: s.get("TotalLaps").and_then(Value::as_i64),
                            fresh: matches!(s.get("New"), Some(Value::Bool(true)))
                                || s.get("New").and_then(Value::as_str) == Some("true"),
                        })
                    }),
                })
            })
            .collect();
        rows.sort_by_key(|r| r.pos);
        rows
    }

    pub fn summary(&self) -> Summary {
        let order = self.order();
        let top = order
            .iter()
            .take(3)
            .map(|r| Top {
                pos: r.pos,
                code: r.code.clone(),
                color: r.color.clone(),
            })
            .collect();
        let kind = match self.at_text("/SessionInfo/Type").as_deref() {
            Some("Race") => "race",
            Some("Qualifying") => "qualifying",
            _ => "practice",
        };
        let clock = self
            .at("/ExtrapolatedClock")
            .filter(|c| c.is_object())
            .map(|c| Clock {
                remaining: string_at(c, "Remaining"),
                utc: string_at(c, "Utc"),
                running: truthy(c.get("Extrapolating")),
            });
        let messages: Vec<Message> = self
            .messages()
            .into_iter()
            .rev() // newest first
            .take(MESSAGES_SHOWN)
            .map(|m| Message {
                utc: format!("{}Z", string_at(m, "Utc").unwrap_or_default()),
                category: string_at(m, "Category"),
                flag: string_at(m, "Flag"),
                message: string_at(m, "Message"),
            })
            .collect();

        Summary {
            kind: kind.to_string(),
            order,
            session: self.at_text("/SessionInfo/Name"),
            meeting: self.at_text("/SessionInfo/Meeting/Name"),
            status: self.at_text("/SessionStatus/Status"),
            part: self
                .at("/TimingData/SessionPart")
                .and_then(as_u32)
                .filter(|p| *p > 0),
            track: self
                .at("/TrackStatus")
                .filter(|t| t.is_object())
                .map(|t| Track {
                    status: string_at(t, "Status"),
                    message: string_at(t, "Message"),
                }),
            lap: self.at("/LapCount/CurrentLap").and_then(Value::as_i64),
            total_laps: self.at("/LapCount/TotalLaps").and_then(Value::as_i64),
            remaining: self.at_text("/ExtrapolatedClock/Remaining"),
            clock,
            gmt_offset: self.at_text("/SessionInfo/GmtOffset"),
            messages,
            top,
        }
    }
}

/// A number the feed may send as `1` or `"1"`.
fn as_u32(v: &Value) -> Option<u32> {
    match v {
        Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn timed(v: Option<&Value>) -> Timed {
    match v {
        Some(Value::Object(o)) => Timed {
            value: o.get("Value").and_then(text),
            pb: truthy(o.get("PersonalFastest")),
            overall: truthy(o.get("OverallFastest")),
            previous: false,
        },
        other => Timed {
            value: other.and_then(text),
            ..Default::default()
        },
    }
}

/// A sector of the lap in progress fills in as it's completed; until then it shows the
/// previous lap's time, marked as such.
fn sector(v: Option<&Value>) -> Timed {
    let t = timed(v);
    if t.value.is_some() {
        return t;
    }
    let previous = v.and_then(|s| s.get("PreviousValue")).and_then(text);
    Timed {
        previous: previous.is_some(),
        value: previous,
        pb: false,
        overall: false,
    }
}

/// One driver's best lap and gaps in one qualifying segment.
struct Segment {
    best: Option<String>,
    gap: Option<String>,
    interval: Option<String>,
}

/// A driver's qualifying numbers for the segment in progress (`part`, 1 to 3). A driver who
/// has set a time in it shows that; an eliminated driver keeps the segment they went out
/// in; a driver yet to set one shows none (`None`).
fn segment(l: &Value, part: u32) -> Option<Segment> {
    let times = list(l.get("BestLapTimes"));
    let time_at = |i: usize| times.get(i).and_then(|t| value(Some(t)));

    let mut i = part as usize - 1;
    if time_at(i).is_none() && truthy(l.get("KnockedOut")) {
        // Walk back to the last segment they set a time in.
        loop {
            if time_at(i).is_some() {
                break;
            }
            i = i.checked_sub(1)?;
        }
    }
    let best = time_at(i)?;

    let stats = list(l.get("Stats"));
    let stat = stats.get(i).copied();
    Some(Segment {
        best: Some(best),
        gap: stat.and_then(|s| value(s.get("TimeDiffToFastest"))),
        // The feed spells it TimeDifftoPositionAhead here.
        interval: stat.and_then(|s| {
            value(
                s.get("TimeDifftoPositionAhead")
                    .or_else(|| s.get("TimeDiffToPositionAhead")),
            )
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot() -> Value {
        json!({
            "type": 3, "invocationId": "sub",
            "result": {
                "SessionInfo": { "Name": "Sprint", "GmtOffset": "08:00:00", "Meeting": { "Name": "Singapore Grand Prix" } },
                "SessionStatus": { "Status": "Inactive" },
                "TrackStatus": { "Status": "2", "Message": "Yellow" },
                "RaceControlMessages": { "Messages": [{ "Utc": "2026-10-10T08:52:43", "Category": "Other", "Message": "DELAYED START" }] },
                "LapCount": { "CurrentLap": 1, "TotalLaps": 20 },
                "DriverList": { "3": { "Tla": "VER", "TeamColour": "4781D7" }, "44": { "Tla": "HAM", "TeamColour": "ED1131" } },
                "TimingData": { "Lines": { "3": { "Position": "1" }, "44": { "Position": "2" } } }
            }
        })
    }

    fn feed(topic: &str, data: Value) -> Value {
        json!({ "type": 1, "target": "feed", "arguments": [topic, data, "2026-10-10T09:30:00Z"] })
    }

    /// A snapshot with some of its parts replaced.
    fn snapshot_with(edit: impl FnOnce(&mut Value)) -> Value {
        let mut snap = snapshot();
        edit(&mut snap["result"]);
        snap
    }

    #[test]
    fn snapshot_messages_are_history_not_alerts() {
        let mut live = LiveState::new();
        let events = live.handle(&snapshot());
        assert!(!events.iter().any(|e| matches!(e, Event::RaceControl(_))));
        assert_eq!(events, vec![Event::Snapshot(Some("Inactive".into()))]);
    }

    #[test]
    fn new_messages_track_and_status_changes_are_emitted_once() {
        let mut live = LiveState::new();
        live.handle(&snapshot());
        let message = feed(
            "RaceControlMessages",
            json!({ "Messages": { "1": { "Utc": "2026-10-10T09:17:00", "Message": "FORMATION LAP WILL START AT 17:30" } } }),
        );

        let first = live.handle(&message);
        assert!(
            matches!(&first[..], [Event::RaceControl(m)] if m["Message"] == "FORMATION LAP WILL START AT 17:30")
        );
        // The same message again (e.g. after a re-send) is not repeated.
        assert!(live.handle(&message).is_empty());

        assert_eq!(
            live.handle(&feed("SessionStatus", json!({ "Status": "Started" }))),
            vec![Event::Status("Started".into())]
        );
        assert_eq!(
            live.handle(&feed(
                "TrackStatus",
                json!({ "Status": "4", "Message": "SCDeployed" })
            )),
            vec![Event::Track {
                status: Some("4".into()),
                message: Some("SCDeployed".into()),
                prev: Some("2".into())
            }]
        );
        // The same status again is not a change.
        assert!(live
            .handle(&feed("SessionStatus", json!({ "Status": "Started" })))
            .is_empty());
    }

    #[test]
    fn a_reconnect_that_finds_a_different_flag_says_so() {
        let mut live = LiveState::new();
        live.handle(&snapshot());
        let events = live.handle(&snapshot_with(|r| {
            r["TrackStatus"] = json!({ "Status": "1", "Message": "AllClear" })
        }));
        assert_eq!(
            events[0],
            Event::Track {
                status: Some("1".into()),
                message: Some("AllClear".into()),
                prev: Some("2".into())
            }
        );
    }

    #[test]
    fn the_server_closing_is_an_event() {
        assert_eq!(
            LiveState::new().handle(&json!({ "type": 7 })),
            vec![Event::ServerClosed]
        );
        assert!(LiveState::new().handle(&json!({ "type": 6 })).is_empty()); // a ping
        assert!(LiveState::new().handle(&json!("junk")).is_empty());
    }

    #[test]
    fn summary_session_flag_lap_running_order_and_messages_newest_first() {
        let mut live = LiveState::new();
        live.handle(&snapshot());
        live.handle(&feed("LapCount", json!({ "CurrentLap": 12 })));
        live.handle(&feed(
            "TimingData",
            json!({ "Lines": { "3": { "Position": "2" }, "44": { "Position": "1" } } }),
        ));
        let s = live.summary();
        assert_eq!(s.session.as_deref(), Some("Sprint"));
        assert_eq!(s.status.as_deref(), Some("Inactive"));
        assert_eq!(
            s.track,
            Some(Track {
                status: Some("2".into()),
                message: Some("Yellow".into())
            })
        );
        assert_eq!((s.lap, s.total_laps), (Some(12), Some(20)));
        assert_eq!(s.gmt_offset.as_deref(), Some("08:00:00"));
        let top: Vec<String> = s
            .top
            .iter()
            .map(|t| format!("{} {} {}", t.pos, t.code, t.color.as_deref().unwrap()))
            .collect();
        assert_eq!(top, ["1 HAM #ED1131", "2 VER #4781D7"]);
        assert_eq!(s.messages[0].message.as_deref(), Some("DELAYED START"));
        assert_eq!(s.messages[0].utc, "2026-10-10T08:52:43Z");
    }

    #[test]
    fn running_order_in_a_race_has_gaps_intervals_tyres_pits_and_status() {
        let mut live = LiveState::new();
        live.handle(&snapshot_with(|r| {
            r["SessionInfo"]["Type"] = json!("Race");
            r["DriverList"]["16"] = json!({ "Tla": "LEC", "TeamColour": "ED1131" });
            r["TimingData"] = json!({ "Lines": {
                "3": { "Position": "1", "GapToLeader": "LAP 12", "IntervalToPositionAhead": { "Value": "LAP 12" }, "NumberOfPitStops": 1, "LastLapTime": { "Value": "1:36.100" } },
                "44": { "Position": "2", "GapToLeader": "+3.214", "IntervalToPositionAhead": { "Value": "+3.214", "Catching": true }, "NumberOfPitStops": 1, "InPit": true },
                "16": { "Position": "3", "GapToLeader": "+9.880", "IntervalToPositionAhead": { "Value": "+6.666" }, "NumberOfPitStops": 0, "Retired": true }
            } });
            r["TimingAppData"] = json!({ "Lines": {
                "3": { "Stints": [{ "Compound": "MEDIUM", "TotalLaps": 4, "New": "true" }, { "Compound": "HARD", "TotalLaps": 8, "New": "true" }] },
                // Stints built from deltas arrive as index-keyed objects.
                "44": { "Stints": { "0": { "Compound": "SOFT", "TotalLaps": 10, "New": "false" } } }
            } });
        }));
        let s = live.summary();
        assert_eq!(s.kind, "race");
        let codes: Vec<&str> = s.order.iter().map(|r| r.code.as_str()).collect();
        assert_eq!(codes, ["VER", "HAM", "LEC"]);
        let (ver, ham, lec) = (&s.order[0], &s.order[1], &s.order[2]);
        assert_eq!(
            (ham.gap.as_deref(), ham.interval.as_deref()),
            (Some("+3.214"), Some("+3.214"))
        );
        assert_eq!(
            ver.tyre,
            Some(Tyre {
                compound: "HARD".into(),
                laps: Some(8),
                fresh: true
            })
        );
        assert_eq!(
            ham.tyre,
            Some(Tyre {
                compound: "SOFT".into(),
                laps: Some(10),
                fresh: false
            })
        );
        assert!(ham.in_pit);
        assert!(lec.out);
        assert_eq!(lec.tyre, None);
        assert_eq!(ver.pits, 1);
    }

    #[test]
    fn running_order_in_practice_uses_the_gap_to_the_fastest_lap() {
        let mut live = LiveState::new();
        live.handle(&snapshot_with(|r| {
            r["SessionInfo"]["Type"] = json!("Practice");
            r["TimingData"] = json!({ "Lines": {
                "3": { "Position": "2", "TimeDiffToFastest": "+0.213", "TimeDiffToPositionAhead": "+0.213", "BestLapTime": { "Value": "1:35.010" }, "NumberOfLaps": 14 },
                "44": { "Position": "1", "TimeDiffToFastest": "", "BestLapTime": { "Value": "1:34.797" }, "NumberOfLaps": 11 }
            } });
        }));
        let s = live.summary();
        assert_eq!(s.kind, "practice");
        let rows: Vec<String> = s
            .order
            .iter()
            .map(|r| {
                format!(
                    "{} {} {:?} {:?}",
                    r.pos,
                    r.code,
                    r.best.as_deref(),
                    r.gap.as_deref()
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                r#"1 HAM Some("1:34.797") None"#,
                r#"2 VER Some("1:35.010") Some("+0.213")"#
            ]
        );
        assert_eq!(s.order[0].laps, Some(11));
    }

    #[test]
    fn last_lap_and_sectors_keep_their_flags_and_unfinished_sectors_show_the_previous_lap() {
        let mut live = LiveState::new();
        live.handle(&snapshot_with(|r| {
            r["SessionInfo"]["Type"] = json!("Practice");
            r["TimingData"] = json!({ "Lines": { "3": {
                "Position": "1",
                "LastLapTime": { "Value": "1:35.291", "PersonalFastest": true, "OverallFastest": true },
                "Sectors": [
                    { "Value": "25.101", "PersonalFastest": true, "OverallFastest": false, "PreviousValue": "25.300" },
                    { "Value": "", "PreviousValue": "36.400" },
                    { "Value": "", "PreviousValue": "" }
                ]
            } } });
            r["ExtrapolatedClock"] = json!({ "Utc": "2026-10-23T21:56:56Z", "Remaining": "00:03:03", "Extrapolating": true });
        }));
        let s = live.summary();
        let row = &s.order[0];
        assert_eq!(
            row.last_lap,
            Timed {
                value: Some("1:35.291".into()),
                pb: true,
                overall: true,
                previous: false
            }
        );
        assert_eq!(
            row.sectors[0],
            Timed {
                value: Some("25.101".into()),
                pb: true,
                overall: false,
                previous: false
            }
        );
        assert_eq!(
            row.sectors[1],
            Timed {
                value: Some("36.400".into()),
                pb: false,
                overall: false,
                previous: true
            }
        );
        assert_eq!(
            row.sectors[2],
            Timed {
                value: None,
                pb: false,
                overall: false,
                previous: false
            }
        );
        assert_eq!(
            s.clock,
            Some(Clock {
                remaining: Some("00:03:03".into()),
                utc: Some("2026-10-23T21:56:56Z".into()),
                running: true
            })
        );
    }

    // ---- qualifying: best lap and gaps per segment ----

    fn quali(part: u32, lines: Value) -> Value {
        json!({ "type": 3, "invocationId": "sub", "result": {
            "SessionInfo": { "Name": "Qualifying", "Type": "Qualifying" },
            "SessionStatus": { "Status": "Started" },
            "DriverList": { "12": { "Tla": "ANT" }, "16": { "Tla": "LEC" }, "30": { "Tla": "LAW" }, "10": { "Tla": "GAS" } },
            "TimingData": { "SessionPart": part, "Lines": lines }
        } })
    }

    /// One driver's line: a (best lap, gap, interval) per segment; `None` for a segment not run.
    fn qline(pos: u32, segs: [Option<(&str, &str, &str)>; 3], knocked_out: bool) -> Value {
        let best = segs.iter().flatten().map(|s| s.0).min().unwrap_or("");
        json!({
            "Position": pos.to_string(),
            "KnockedOut": knocked_out,
            "BestLapTime": { "Value": best },
            "BestLapTimes": segs.iter().map(|s| json!({ "Value": s.map_or("", |s| s.0) })).collect::<Vec<_>>(),
            "Stats": segs.iter().map(|s| json!({
                "TimeDiffToFastest": s.map_or("", |s| s.1),
                "TimeDifftoPositionAhead": s.map_or("", |s| s.2),
            })).collect::<Vec<_>>(),
        })
    }

    fn by_code(rows: Vec<Row>) -> std::collections::HashMap<String, Row> {
        rows.into_iter().map(|r| (r.code.clone(), r)).collect()
    }

    #[test]
    fn qualifying_q3_shows_the_q3_best_lap_and_gaps() {
        let mut live = LiveState::new();
        live.handle(&quali(3, json!({
            "12": qline(1, [Some(("1:35.390", "+0.361", "+0.361")), Some(("1:34.278", "+0.080", "+0.080")), Some(("1:34.035", "", ""))], false),
            "16": qline(2, [Some(("1:35.200", "+0.171", "+0.171")), Some(("1:34.198", "", "")), Some(("1:34.267", "+0.232", "+0.232"))], false),
        })));
        let rows = by_code(live.order());
        assert_eq!(
            (rows["ANT"].best.as_deref(), rows["ANT"].gap.as_deref()),
            (Some("1:34.035"), None)
        );
        assert_eq!(
            (
                rows["LEC"].best.as_deref(),
                rows["LEC"].gap.as_deref(),
                rows["LEC"].interval.as_deref()
            ),
            (Some("1:34.267"), Some("+0.232"), Some("+0.232"))
        );
        assert_eq!(live.summary().part, Some(3));
    }

    #[test]
    fn qualifying_no_time_in_the_segment_shows_none_and_a_knocked_out_driver_keeps_their_segment() {
        let mut live = LiveState::new();
        live.handle(&quali(3, json!({
            "12": qline(1, [Some(("1:35.390", "+0.361", "+0.361")), Some(("1:34.278", "+0.080", "+0.080")), Some(("1:34.035", "", ""))], false),
            "30": qline(10, [Some(("1:35.662", "+0.633", "+0.050")), Some(("1:35.058", "+0.860", "+0.100")), None], false),
            "10": qline(11, [Some(("1:35.969", "+0.940", "+0.100")), Some(("1:35.080", "+0.882", "+0.020")), None], true),
        })));
        let rows = by_code(live.order());
        assert_eq!(
            (&rows["LAW"].best, &rows["LAW"].gap, &rows["LAW"].interval),
            (&None, &None, &None)
        );
        assert_eq!(
            (
                rows["GAS"].best.as_deref(),
                rows["GAS"].gap.as_deref(),
                rows["GAS"].interval.as_deref()
            ),
            (Some("1:35.080"), Some("+0.882"), Some("+0.020"))
        );
        assert!(rows["GAS"].knocked_out);
    }

    #[test]
    fn qualifying_at_the_start_of_q2_nobody_has_a_time_but_q1_drop_outs_keep_their_q1_values() {
        let mut live = LiveState::new();
        live.handle(&quali(
            2,
            json!({
                "12": qline(1, [Some(("1:35.390", "+0.361", "+0.361")), None, None], false),
                "10": qline(19, [Some(("1:36.770", "+1.741", "+0.100")), None, None], true),
            }),
        ));
        let rows = by_code(live.order());
        assert_eq!((&rows["ANT"].best, &rows["ANT"].gap), (&None, &None));
        assert_eq!(
            (rows["GAS"].best.as_deref(), rows["GAS"].gap.as_deref()),
            (Some("1:36.770"), Some("+1.741"))
        );
    }

    #[test]
    fn index_keyed_lists_are_ordered_numerically() {
        // "10" sorts before "2" as text; the order must follow the numbers.
        let v = json!({ "2": "c", "10": "j", "1": "b" });
        let ordered: Vec<&str> = list(Some(&v)).iter().filter_map(|x| x.as_str()).collect();
        assert_eq!(ordered, ["b", "c", "j"]);
    }

    #[test]
    fn a_driver_without_a_name_in_the_list_gets_their_number() {
        let mut live = LiveState::new();
        live.handle(&snapshot_with(|r| {
            r["TimingData"]["Lines"]["99"] = json!({ "Position": "3" })
        }));
        assert_eq!(live.order()[2].code, "#99");
    }

    #[test]
    fn drivers_without_a_position_are_left_out() {
        let mut live = LiveState::new();
        live.handle(&snapshot_with(|r| {
            r["TimingData"]["Lines"]["99"] = json!({ "Position": "0" })
        }));
        assert_eq!(live.order().len(), 2);
    }

    #[test]
    fn clearing_forgets_everything() {
        let mut live = LiveState::new();
        live.handle(&snapshot());
        live.clear();
        assert!(live.summary().order.is_empty());
        assert_eq!(live.summary().session, None);
    }
}
