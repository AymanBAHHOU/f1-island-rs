//! Session analysis from OpenF1 data: classification, short-run (one-lap) pace, long-run
//! pace, tyre usage, speed traps, weather and key race-control notes. This file is the
//! calculation only; fetching is in `analysis_data.rs`.
//! (Port of `analyse` and its helpers in the Electron app's `src/analysis.js`; the answer
//! has the same shape, because the Analysis page was written for it.)

use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::LazyLock;

// Lap classification, relative to the driver's best lap in the session.
const PUSH_FACTOR: f64 = 1.08; // within 8%: a representative lap
const COOL_FACTOR: f64 = 1.15; // slower than 15%: cool-down / in-lap, ends a run
const MIN_RUN: usize = 3; // shortest run returned; the view lets the user raise the minimum
const RACE_PACE_MIN: usize = 5; // clean laps needed for a race-pace figure
const RUN_OUTLIER: f64 = 1.02; // within a run, laps 2% slower than its median count as traffic

pub fn is_race(session_name: &str) -> bool {
    matches!(session_name, "Race" | "Sprint")
}

// ---------- reading OpenF1's rows ----------

fn rows(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

fn num(row: &Value, key: &str) -> Option<f64> {
    row.get(key)?.as_f64().filter(|n| n.is_finite())
}

fn int(row: &Value, key: &str) -> Option<i64> {
    row.get(key)?.as_i64()
}

fn text<'a>(row: &'a Value, key: &str) -> Option<&'a str> {
    row.get(key)?.as_str()
}

#[derive(Clone)]
struct Lap {
    number: i64,
    driver: i64,
    duration: Option<f64>,
    pit_out: bool,
    sectors: [Option<f64>; 3],
    speed_trap: Option<f64>,
    i1: Option<f64>,
    i2: Option<f64>,
    /// When the lap started, in milliseconds since 1970.
    start_ms: Option<i64>,
}

fn read_laps(laps: &Value) -> Vec<Lap> {
    rows(laps)
        .iter()
        .filter_map(|l| {
            Some(Lap {
                number: int(l, "lap_number")?,
                driver: int(l, "driver_number")?,
                duration: num(l, "lap_duration"),
                pit_out: l.get("is_pit_out_lap").and_then(Value::as_bool) == Some(true),
                sectors: [
                    num(l, "duration_sector_1"),
                    num(l, "duration_sector_2"),
                    num(l, "duration_sector_3"),
                ],
                speed_trap: num(l, "st_speed"),
                i1: num(l, "i1_speed"),
                i2: num(l, "i2_speed"),
                start_ms: text(l, "date_start").and_then(crate::results::parse_ms),
            })
        })
        .collect()
}

#[derive(Clone)]
struct Stint {
    driver: i64,
    lap_start: i64,
    lap_end: i64,
    compound: Option<String>,
    tyre_age_at_start: f64,
}

impl Stint {
    fn compound(&self) -> &str {
        self.compound.as_deref().unwrap_or("UNKNOWN")
    }
    fn age_at(&self, lap: i64) -> f64 {
        self.tyre_age_at_start + (lap - self.lap_start) as f64
    }
}

fn read_stints(stints: &Value) -> Vec<Stint> {
    rows(stints)
        .iter()
        .filter_map(|s| {
            Some(Stint {
                driver: int(s, "driver_number")?,
                lap_start: int(s, "lap_start")?,
                lap_end: int(s, "lap_end")?,
                compound: text(s, "compound").map(str::to_string),
                tyre_age_at_start: num(s, "tyre_age_at_start").unwrap_or(0.0),
            })
        })
        .collect()
}

// ---------- the drivers ----------

#[derive(Clone)]
pub struct DriverInfo {
    pub number: i64,
    pub code: String,
    pub name: String,
    pub team: String,
    pub color: String,
}

impl DriverInfo {
    fn unknown(number: i64) -> Self {
        DriverInfo {
            number,
            code: format!("#{number}"),
            name: format!("#{number}"),
            team: "Unknown".into(),
            color: "#888888".into(),
        }
    }

    fn to_json(&self) -> Value {
        json!({"number": self.number, "code": self.code, "name": self.name,
               "team": self.team, "color": self.color})
    }
}

/// In the order OpenF1 lists them.
fn driver_info(drivers: &Value) -> Vec<DriverInfo> {
    rows(drivers)
        .iter()
        .filter_map(|d| {
            let number = int(d, "driver_number")?;
            let code = text(d, "name_acronym")
                .filter(|s| !s.is_empty())
                .map_or_else(|| format!("#{number}"), str::to_string);
            let name = text(d, "full_name")
                .filter(|s| !s.is_empty())
                .or_else(|| text(d, "broadcast_name").filter(|s| !s.is_empty()))
                .map_or_else(|| code.clone(), str::to_string);
            Some(DriverInfo {
                number,
                code,
                name,
                team: text(d, "team_name")
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Unknown")
                    .to_string(),
                color: text(d, "team_colour")
                    .filter(|s| !s.is_empty())
                    .map_or_else(|| "#888888".to_string(), |c| format!("#{c}")),
            })
        })
        .collect()
}

// ---------- small maths ----------

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

fn median(xs: &[f64]) -> f64 {
    let mut s = xs.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let m = s.len() >> 1;
    if s.len() % 2 == 1 {
        s[m]
    } else {
        (s[m - 1] + s[m]) / 2.0
    }
}

/// Least-squares slope of y over x (seconds per lap).
fn slope(xs: &[f64], ys: &[f64]) -> f64 {
    let (mx, my) = (mean(xs), mean(ys));
    let (mut num, mut den) = (0.0, 0.0);
    for (x, y) in xs.iter().zip(ys) {
        num += (x - mx) * (y - my);
        den += (x - mx).powi(2);
    }
    if den != 0.0 {
        num / den
    } else {
        0.0
    }
}

fn min_of(xs: impl IntoIterator<Item = f64>) -> Option<f64> {
    xs.into_iter().reduce(f64::min)
}

fn max_of(xs: impl IntoIterator<Item = f64>) -> Option<f64> {
    xs.into_iter().reduce(f64::max)
}

/// A number for JSON; JavaScript's `NaN` and `Infinity` become `null`.
fn jnum(x: f64) -> Value {
    if x.is_finite() {
        json!(x)
    } else {
        Value::Null
    }
}

fn jopt(x: Option<f64>) -> Value {
    x.map_or(Value::Null, jnum)
}

// ---------- runs ----------

struct RunLap {
    lap: i64,
    time: f64,
    push: bool,
    tyre_age: Option<f64>,
}

struct Run<'a> {
    stint: Option<&'a Stint>,
    laps: Vec<RunLap>,
}

impl Run<'_> {
    fn push(&self) -> impl Iterator<Item = &RunLap> {
        self.laps.iter().filter(|l| l.push)
    }
}

/// Splits a driver's laps into runs: blocks of consecutive laps with no cool-down lap,
/// pit stop or missing time in between.
fn find_runs<'a>(laps: &[Lap], stints: &'a [Stint], best: f64, race: bool) -> Vec<Run<'a>> {
    let mut runs = Vec::new();
    let in_stint = |s: &Stint| -> Vec<Lap> {
        laps.iter()
            .filter(|l| l.number >= s.lap_start && l.number <= s.lap_end)
            .cloned()
            .collect()
    };
    let segments: Vec<(Option<usize>, Vec<Lap>)> = if stints.is_empty() {
        vec![(None, laps.to_vec())]
    } else {
        stints
            .iter()
            .enumerate()
            .map(|(i, s)| (Some(i), in_stint(s)))
            .collect()
    };

    for (index, seg_laps) in segments {
        let stint = index.map(|i| &stints[i]);
        // The final lap of a stint is an in-lap, except at the end of a race.
        let keep_last = race && index.is_some() && index == stints.len().checked_sub(1);
        let usable = if keep_last || seg_laps.is_empty() {
            &seg_laps[..]
        } else {
            &seg_laps[..seg_laps.len() - 1]
        };

        let mut block: Vec<RunLap> = Vec::new();
        let close = |block: &mut Vec<RunLap>, runs: &mut Vec<Run<'a>>| {
            // Second pass: drop laps well off the run's own median (traffic, mistakes).
            let pushed: Vec<f64> = block.iter().filter(|l| l.push).map(|l| l.time).collect();
            if pushed.len() >= 3 {
                let m = median(&pushed);
                for l in block.iter_mut() {
                    if l.push && l.time > m * RUN_OUTLIER {
                        l.push = false;
                    }
                }
            }
            if block.iter().any(|l| l.push) {
                runs.push(Run {
                    stint,
                    laps: std::mem::take(block),
                });
            }
            block.clear();
        };

        for l in usable {
            let Some(t) = l.duration.filter(|_| !l.pit_out) else {
                close(&mut block, &mut runs);
                continue;
            };
            if t > best * COOL_FACTOR {
                close(&mut block, &mut runs);
                continue;
            }
            block.push(RunLap {
                lap: l.number,
                time: t,
                push: t <= best * PUSH_FACTOR,
                tyre_age: stint.map(|s| s.age_at(l.number)),
            });
        }
        close(&mut block, &mut runs);
    }
    runs
}

fn summarise_run(run: &Run, driver: i64) -> Map<String, Value> {
    let times: Vec<f64> = run.push().map(|l| l.time).collect();
    let laps: Vec<f64> = run.push().map(|l| l.lap as f64).collect();
    let first = run.push().next().expect("a run has push laps");
    let mut out = Map::new();
    out.insert("driver".into(), json!(driver));
    out.insert(
        "compound".into(),
        json!(run.stint.map_or("UNKNOWN", Stint::compound)),
    );
    out.insert("tyreAgeStart".into(), jopt(first.tyre_age));
    out.insert(
        "laps".into(),
        Value::Array(
            run.laps
                .iter()
                .map(|l| json!({"lap": l.lap, "time": l.time, "push": l.push}))
                .collect(),
        ),
    );
    out.insert("pushLaps".into(), json!(times.len()));
    out.insert("avg".into(), jnum(mean(&times)));
    out.insert("median".into(), jnum(median(&times)));
    out.insert("best".into(), jopt(min_of(times.iter().copied())));
    // Raw trend: tyre wear makes it positive, fuel burn pulls it down.
    out.insert("trend".into(), jnum(slope(&laps, &times)));
    out
}

// ---------- classification ----------

fn classify(results: &Value, best_by_driver: &[(i64, Option<Best>)]) -> Value {
    if !rows(results).is_empty() {
        let mut sorted: Vec<&Value> = rows(results).iter().collect();
        sorted.sort_by_key(|r| int(r, "position").unwrap_or(99));
        return Value::Array(
            sorted
                .into_iter()
                .map(|r| {
                    // Qualifying returns per-phase arrays; use the last phase reached.
                    let as_list = |key: &str| -> Vec<Value> {
                        match r.get(key) {
                            Some(Value::Array(a)) => a.clone(),
                            Some(v) => vec![v.clone()],
                            None => vec![Value::Null],
                        }
                    };
                    let durations = as_list("duration");
                    let gaps = as_list("gap_to_leader");
                    let mut phase = durations.len().saturating_sub(1);
                    while phase > 0 && durations[phase].is_null() {
                        phase -= 1;
                    }
                    let flag = |k: &str| r.get(k).is_some_and(|v| v.as_bool() == Some(true));
                    let status = if flag("dsq") {
                        json!("DSQ")
                    } else if flag("dns") {
                        json!("DNS")
                    } else if flag("dnf") {
                        json!("DNF")
                    } else {
                        Value::Null
                    };
                    json!({
                        "position": r.get("position").cloned().unwrap_or(Value::Null),
                        "driver": r.get("driver_number").cloned().unwrap_or(Value::Null),
                        "time": durations.get(phase).cloned().unwrap_or(Value::Null),
                        "gap": gaps.get(phase).cloned().unwrap_or(Value::Null),
                        "phase": if durations.len() > 1 { json!(phase + 1) } else { Value::Null },
                        "laps": r.get("number_of_laps").cloned().unwrap_or(Value::Null),
                        "status": status,
                    })
                })
                .collect(),
        );
    }
    // No official classification yet: order by best lap.
    let mut ordered: Vec<(i64, &Best)> = best_by_driver
        .iter()
        .filter_map(|(n, b)| b.as_ref().map(|b| (*n, b)))
        .collect();
    ordered.sort_by(|a, b| a.1.time.total_cmp(&b.1.time));
    let top = ordered.first().map(|(_, b)| b.time);
    Value::Array(
        ordered
            .iter()
            .enumerate()
            .map(|(i, (num, b))| {
                json!({
                    "position": i + 1,
                    "driver": num,
                    "time": b.time,
                    "gap": if i > 0 { jnum(b.time - top.unwrap_or(b.time)) } else { json!(0) },
                    "phase": null,
                    "laps": b.laps,
                    "status": null,
                })
            })
            .collect(),
    )
}

/// Keeps each team's best (lowest) value, best team first.
fn team_ranking(entries: &[(Map<String, Value>, Option<f64>)]) -> Value {
    struct Team<'a> {
        name: Value,
        color: Value,
        driver: Value,
        value: f64,
        entry: &'a Map<String, Value>,
    }
    let mut order: Vec<String> = Vec::new();
    let mut by_team: HashMap<String, Team> = HashMap::new();
    for (entry, value) in entries {
        let Some(v) = value else { continue };
        let team = entry["team"].as_str().unwrap_or("").to_string();
        let better = by_team.get(&team).is_none_or(|cur| *v < cur.value);
        if better {
            if !by_team.contains_key(&team) {
                order.push(team.clone());
            }
            by_team.insert(
                team,
                Team {
                    name: entry["team"].clone(),
                    color: entry["color"].clone(),
                    driver: entry["driver"].clone(),
                    value: *v,
                    entry,
                },
            );
        }
    }
    let mut list: Vec<&Team> = order.iter().map(|t| &by_team[t]).collect();
    list.sort_by(|a, b| a.value.total_cmp(&b.value));
    let top = list.first().map(|t| t.value);
    Value::Array(
        list.iter()
            .map(|t| {
                json!({
                    "team": t.name, "color": t.color, "driver": t.driver, "value": t.value,
                    "entry": t.entry, "gap": t.value - top.unwrap_or(t.value),
                })
            })
            .collect(),
    )
}

// ---------- race control ----------

static CAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bCAR (\d+)").unwrap());
static LAP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bLAP (\d+)").unwrap());
static DELETED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bDELETED\b").unwrap());
static REINSTATED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)REINSTATED").unwrap());
static TIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bTIME (?:(\d+):)?(\d+\.\d+)").unwrap());
static KEY_MESSAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)PENALTY|INVESTIGATION|REPRIMAND|NO FURTHER ACTION|\bRED FLAG").unwrap()
});

fn message(m: &Value) -> &str {
    text(m, "message").unwrap_or("")
}

/// Lap times the stewards deleted (mostly track limits), per car, in seconds. OpenF1
/// numbers laps differently from these messages, so laps are matched by their time:
/// "CAR 16 (LEC) TIME 1:34.810 DELETED - TRACK LIMITS AT TURN 10 LAP 13". A later
/// "CAR 43 (COL) LAP 13 WILL BE REINSTATED" gives the lap back. Messages for a lap
/// without a time ("LAP DELETED ... (NEXT LAP PIT)") are pit laps: skipped.
pub fn deleted_lap_times(race_control: &Value) -> HashMap<i64, Vec<f64>> {
    struct Entry {
        car: i64,
        time: f64,
        lap: Option<i64>,
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut sorted: Vec<&Value> = rows(race_control).iter().collect();
    sorted.sort_by(|a, b| {
        text(a, "date")
            .unwrap_or("")
            .cmp(text(b, "date").unwrap_or(""))
    });
    for m in sorted {
        let msg = message(m);
        let car = CAR
            .captures(msg)
            .and_then(|c| c[1].parse::<i64>().ok())
            .or_else(|| int(m, "driver_number"))
            .unwrap_or(0);
        if car == 0 {
            continue;
        }
        let lap = LAP
            .captures(msg)
            .and_then(|c| c[1].parse::<i64>().ok())
            .filter(|&l| l != 0);
        if REINSTATED.is_match(msg) {
            if let Some(lap) = lap {
                entries.retain(|e| !(e.car == car && e.lap == Some(lap)));
            }
        } else if DELETED.is_match(msg) {
            if let Some(t) = TIME.captures(msg) {
                let minutes: f64 = t.get(1).map_or(0.0, |m| m.as_str().parse().unwrap_or(0.0));
                let seconds: f64 = t[2].parse().unwrap_or(0.0);
                entries.push(Entry {
                    car,
                    time: minutes * 60.0 + seconds,
                    lap,
                });
            }
        }
    }
    let mut out: HashMap<i64, Vec<f64>> = HashMap::new();
    for e in entries {
        out.entry(e.car).or_default().push(e.time);
    }
    out
}

// ---------- the analysis ----------

/// A team, its colour and the laps its driver flew on each compound.
type TeamTyres = (String, Value, Vec<(String, i64)>);

#[derive(Clone)]
struct Best {
    /// The qualifying segment (Q1, Q2, Q3) the lap was set in.
    segment: Option<String>,
    time: f64,
    lap: i64,
    compound: String,
    tyre_age: Option<f64>,
    laps: usize,
}

/// The segments of a qualifying session: ("Q1", start, end) in milliseconds, found from the
/// "SESSION STARTED" and "SESSION FINISHED" messages. Empty for other sessions.
fn segments(session_name: &str, race_control: &Value) -> Vec<(String, i64, i64)> {
    let prefix = if session_name.starts_with("Sprint") {
        "SQ"
    } else {
        "Q"
    };
    if !(session_name.contains("Qualifying") || session_name.contains("Shootout")) {
        return Vec::new();
    }
    let mut events: Vec<(i64, bool)> = rows(race_control)
        .iter()
        .filter(|m| text(m, "category") == Some("SessionStatus"))
        .filter_map(|m| {
            let at = crate::results::parse_ms(text(m, "date")?)?;
            match message(m) {
                "SESSION STARTED" => Some((at, true)),
                "SESSION FINISHED" => Some((at, false)),
                _ => None,
            }
        })
        .collect();
    events.sort_by_key(|e| e.0);
    let mut out = Vec::new();
    let mut open: Option<i64> = None;
    for (at, started) in events {
        match (started, open) {
            (true, _) => open = Some(at),
            (false, Some(start)) => {
                out.push((format!("{prefix}{}", out.len() + 1), start, at));
                open = None;
            }
            (false, None) => {}
        }
    }
    out
}

/// What OpenF1 says about one session.
pub struct Raw<'a> {
    pub drivers: &'a Value,
    pub laps: &'a Value,
    pub stints: &'a Value,
    pub results: &'a Value,
    pub weather: &'a Value,
    pub race_control: &'a Value,
}

pub fn analyse(session: &Value, raw: &Raw) -> Value {
    let session_name = text(session, "session_name").unwrap_or("");
    let race = is_race(session_name);
    let info = driver_info(raw.drivers);
    let info_of: HashMap<i64, &DriverInfo> = info.iter().map(|d| (d.number, d)).collect();

    // Group laps and stints by driver, keeping the order drivers first appear in.
    let all_laps = read_laps(raw.laps);
    let all_stints = read_stints(raw.stints);
    let mut driver_order: Vec<i64> = Vec::new();
    let mut laps_by_driver: HashMap<i64, Vec<Lap>> = HashMap::new();
    for l in &all_laps {
        if !laps_by_driver.contains_key(&l.driver) {
            driver_order.push(l.driver);
        }
        laps_by_driver.entry(l.driver).or_default().push(l.clone());
    }

    let deleted_by_car = deleted_lap_times(raw.race_control);
    let quali_segments = segments(session_name, raw.race_control);
    let mut per_driver: Vec<(Map<String, Value>, Option<f64>)> = Vec::new();
    let mut best_by_driver: Vec<(i64, Option<Best>)> = Vec::new();
    let mut long_runs: Vec<Value> = Vec::new();
    let mut race_pace: Vec<(Map<String, Value>, Option<f64>)> = Vec::new();
    let mut sector_rows: Vec<[Option<f64>; 3]> = Vec::new();
    let mut tyres_of: Vec<TeamTyres> = Vec::new();

    for &num in &driver_order {
        let unknown;
        let d = match info_of.get(&num) {
            Some(d) => *d,
            None => {
                unknown = DriverInfo::unknown(num);
                &unknown
            }
        };
        let mut d_laps = laps_by_driver[&num].clone();
        d_laps.sort_by_key(|l| l.number);
        let mut d_stints: Vec<Stint> = all_stints
            .iter()
            .filter(|s| s.driver == num)
            .cloned()
            .collect();
        d_stints.sort_by_key(|s| s.lap_start);
        let stint_at = |lap: i64| {
            d_stints
                .iter()
                .find(|s| lap >= s.lap_start && lap <= s.lap_end)
        };

        // A deleted lap time doesn't count as a best lap: the official results skip it.
        let deleted_times = deleted_by_car.get(&num).cloned().unwrap_or_default();
        let is_deleted = |l: &Lap| {
            l.duration
                .is_some_and(|t| deleted_times.iter().any(|d| (d - t).abs() < 0.002))
        };
        let best_lap = d_laps
            .iter()
            .filter(|l| l.duration.is_some() && !l.pit_out && !is_deleted(l))
            .fold(None::<&Lap>, |b, l| match b {
                Some(cur) if l.duration >= cur.duration => Some(cur),
                _ => Some(l),
            });
        let sector = |k: usize| min_of(d_laps.iter().filter_map(|l| l.sectors[k]));
        let sectors = [sector(0), sector(1), sector(2)];

        let best = best_lap.map(|bl| {
            let st = stint_at(bl.number);
            Best {
                segment: bl.start_ms.and_then(|t| {
                    quali_segments
                        .iter()
                        .find(|(_, from, to)| t >= *from && t <= *to)
                        .map(|(name, _, _)| name.clone())
                }),
                time: bl.duration.unwrap_or(0.0),
                lap: bl.number,
                compound: st.map_or("UNKNOWN", Stint::compound).to_string(),
                tyre_age: st.map(|s| s.age_at(bl.number)),
                laps: d_laps.len(),
            }
        });
        best_by_driver.push((num, best.clone()));

        let lap_list: Vec<Value> = d_laps
            .iter()
            .filter(|l| l.duration.is_some())
            .map(|l| {
                let st = stint_at(l.number);
                json!({
                    "lap": l.number,
                    "time": l.duration,
                    "sectors": l.sectors.iter().map(|s| jopt(*s)).collect::<Vec<_>>(),
                    "compound": st.map_or("UNKNOWN", Stint::compound),
                    "tyreAge": jopt(st.map(|s| s.age_at(l.number))),
                    "pitOut": l.pit_out,
                    "deleted": is_deleted(l),
                    "speedTrap": jopt(l.speed_trap),
                })
            })
            .collect();

        // Laps flown on each compound.
        let last_lap = d_laps.last().map(|l| l.number);
        let mut compounds: Vec<(String, i64)> = Vec::new();
        for s in &d_stints {
            let end = s.lap_end.min(last_lap.unwrap_or(s.lap_end));
            let n = (end - s.lap_start + 1).max(0);
            match compounds.iter_mut().find(|(c, _)| c == s.compound()) {
                Some((_, total)) => *total += n,
                None => compounds.push((s.compound().to_string(), n)),
            }
        }
        let mut tyres = Map::new();
        for (c, n) in &compounds {
            tyres.insert(c.clone(), json!(n));
        }

        let ideal = match sectors {
            [Some(a), Some(b), Some(c)] => Some(a + b + c),
            _ => None,
        };
        let mut row = Map::new();
        row.insert("driver".into(), json!(num));
        row.insert("code".into(), json!(d.code));
        row.insert("lapList".into(), Value::Array(lap_list));
        row.insert("team".into(), json!(d.team));
        row.insert("color".into(), json!(d.color));
        row.insert("laps".into(), json!(d_laps.len()));
        row.insert(
            "best".into(),
            best.as_ref().map_or(Value::Null, |b| {
                json!({"time": b.time, "lap": b.lap, "compound": b.compound,
                       "tyreAge": jopt(b.tyre_age), "laps": b.laps, "segment": b.segment})
            }),
        );
        row.insert(
            "sectors".into(),
            Value::Array(sectors.iter().map(|s| jopt(*s)).collect()),
        );
        row.insert("ideal".into(), jopt(ideal));
        row.insert(
            "speedTrap".into(),
            jopt(max_of(d_laps.iter().filter_map(|l| l.speed_trap))),
        );
        row.insert(
            "i1".into(),
            jopt(max_of(d_laps.iter().filter_map(|l| l.i1))),
        );
        row.insert(
            "i2".into(),
            jopt(max_of(d_laps.iter().filter_map(|l| l.i2))),
        );
        row.insert("tyres".into(), Value::Object(tyres));
        per_driver.push((row, best.as_ref().map(|b| b.time)));
        sector_rows.push(sectors);
        tyres_of.push((d.team.clone(), json!(d.color), compounds));

        let Some(best) = best else { continue };
        let runs = find_runs(&d_laps, &d_stints, best.time, race);

        // Every run long enough to matter; the view picks per driver by the chosen minimum.
        for run in &runs {
            if run.push().count() >= MIN_RUN {
                let mut summary = summarise_run(run, num);
                summary.insert("code".into(), json!(d.code));
                summary.insert("team".into(), json!(d.team));
                summary.insert("color".into(), json!(d.color));
                long_runs.push(Value::Object(summary));
            }
        }

        if race {
            let push: Vec<f64> = runs.iter().flat_map(|r| r.push().map(|l| l.time)).collect();
            if push.len() >= RACE_PACE_MIN {
                let med = median(&push);
                let mut row = Map::new();
                row.insert("driver".into(), json!(num));
                row.insert("code".into(), json!(d.code));
                row.insert("team".into(), json!(d.team));
                row.insert("color".into(), json!(d.color));
                row.insert("laps".into(), json!(push.len()));
                row.insert("median".into(), jnum(med));
                row.insert("avg".into(), jnum(mean(&push)));
                race_pace.push((row, Some(med)));
            }
        }
    }

    // Overall best sectors, for highlighting.
    let best_sectors: Vec<Value> = (0..3)
        .map(|i| jopt(min_of(sector_rows.iter().filter_map(|r| r[i]))))
        .collect();

    // Tyre usage by team.
    let mut team_order: Vec<String> = Vec::new();
    let mut team_tyres: HashMap<String, (Value, Vec<(String, i64)>)> = HashMap::new();
    for (team, color, compounds) in tyres_of {
        let slot = team_tyres.entry(team.clone()).or_insert_with(|| {
            team_order.push(team.clone());
            (color, Vec::new())
        });
        for (c, n) in compounds {
            match slot.1.iter_mut().find(|(k, _)| *k == c) {
                Some((_, total)) => *total += n,
                None => slot.1.push((c, n)),
            }
        }
    }
    let tyres: Vec<Value> = team_order
        .iter()
        .map(|team| {
            let (color, compounds) = &team_tyres[team];
            let mut map = Map::new();
            for (c, n) in compounds {
                map.insert(c.clone(), json!(n));
            }
            json!({"team": team, "color": color, "compounds": map})
        })
        .collect();

    let short_runs_teams = team_ranking(&per_driver);
    let race_pace_teams = team_ranking(&race_pace);
    race_pace.sort_by(|a, b| a.1.unwrap_or(0.0).total_cmp(&b.1.unwrap_or(0.0)));

    // Zero readings are sensor dropouts, not real temperatures.
    let weather = rows(raw.weather);
    let positive = |key: &str| -> Vec<f64> {
        weather
            .iter()
            .filter_map(|w| num(w, key))
            .filter(|t| *t > 0.0)
            .collect()
    };
    let air = positive("air_temperature");
    let track = positive("track_temperature");
    let humidity: Vec<f64> = weather.iter().filter_map(|w| num(w, "humidity")).collect();
    let weather_json = if air.is_empty() {
        Value::Null
    } else {
        json!({
            "air": [jopt(min_of(air.iter().copied())), jopt(max_of(air.iter().copied()))],
            "track": [jopt(min_of(track.iter().copied())), jopt(max_of(track.iter().copied()))],
            "rain": weather.iter().any(|w| num(w, "rainfall").is_some_and(|r| r > 0.0)),
            "humidity": jnum(mean(&humidity)),
            "wind": jopt(max_of(weather.iter().filter_map(|w| num(w, "wind_speed")))),
        })
    };

    let race_control = rows(raw.race_control);
    let notes: Vec<Value> = race_control
        .iter()
        .filter(|m| !DELETED.is_match(message(m)))
        .filter(|m| {
            text(m, "flag") == Some("RED")
                || text(m, "category") == Some("SafetyCar")
                || KEY_MESSAGE.is_match(message(m))
        })
        .map(|m| {
            let mut note = Map::new();
            for (out, key) in [
                ("date", "date"),
                ("lap", "lap_number"),
                ("message", "message"),
                ("flag", "flag"),
                ("category", "category"),
            ] {
                if let Some(v) = m.get(key) {
                    note.insert(out.into(), v.clone());
                }
            }
            Value::Object(note)
        })
        .collect();

    // Lap times deleted (mostly track limits), counted per driver. OpenF1 leaves
    // driver_number empty on these, so read "CAR 18 (STR)" from the text.
    let mut deleted_order: Vec<i64> = Vec::new();
    let mut deleted_count: HashMap<i64, usize> = HashMap::new();
    for m in race_control.iter().filter(|m| DELETED.is_match(message(m))) {
        let number = int(m, "driver_number")
            .or_else(|| CAR.captures(message(m)).and_then(|c| c[1].parse().ok()))
            .unwrap_or(0);
        if number == 0 {
            continue;
        }
        if !deleted_count.contains_key(&number) {
            deleted_order.push(number);
        }
        *deleted_count.entry(number).or_default() += 1;
    }
    deleted_order.sort_by_key(|n| std::cmp::Reverse(deleted_count[n]));
    let deleted_laps: Vec<Value> = deleted_order
        .iter()
        .map(|n| json!({"driver": n, "count": deleted_count[n]}))
        .collect();

    // JavaScript lists whole-number keys in ascending order.
    let mut by_number: Vec<&DriverInfo> = info.iter().collect();
    by_number.sort_by_key(|d| d.number);
    let mut drivers_json = Map::new();
    for d in by_number {
        drivers_json.insert(d.number.to_string(), d.to_json());
    }

    json!({
        "session": {
            "key": session.get("session_key").cloned().unwrap_or(Value::Null),
            "name": session.get("session_name").cloned().unwrap_or(Value::Null),
            "type": session.get("session_type").cloned().unwrap_or(Value::Null),
            "isRace": race,
            "start": session.get("date_start").cloned().unwrap_or(Value::Null),
            "location": session.get("location").cloned().unwrap_or(Value::Null),
            "country": session.get("country_name").cloned().unwrap_or(Value::Null),
        },
        "drivers": drivers_json,
        "classification": classify(raw.results, &best_by_driver),
        "perDriver": per_driver.iter().map(|(r, _)| Value::Object(r.clone())).collect::<Vec<_>>(),
        "bestSectors": best_sectors,
        "shortRuns": {"teams": short_runs_teams},
        "longRuns": {
            "all": long_runs,
            "settings": {"pushFactor": PUSH_FACTOR, "coolFactor": COOL_FACTOR, "minRun": MIN_RUN},
        },
        "racePace": {
            "drivers": race_pace.iter().map(|(r, _)| Value::Object(r.clone())).collect::<Vec<_>>(),
            "teams": race_pace_teams,
        },
        "tyres": tyres,
        "weather": weather_json,
        "notes": notes,
        "deletedLaps": deleted_laps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lap(driver: i64, n: i64, t: f64) -> Value {
        json!({"driver_number": driver, "lap_number": n, "lap_duration": t,
               "is_pit_out_lap": false, "duration_sector_1": 20.0, "duration_sector_2": 30.0,
               "duration_sector_3": t - 50.0, "st_speed": 300})
    }

    fn sample() -> (Value, Value, Value, Value, Value) {
        let drivers = json!([
            {"driver_number": 1, "name_acronym": "VER", "full_name": "Max Verstappen", "team_name": "Red Bull", "team_colour": "3671C6"},
            {"driver_number": 4, "name_acronym": "NOR", "team_name": "McLaren", "team_colour": "FF8000"},
        ]);
        let mut laps = vec![
            json!({"driver_number": 1, "lap_number": 1, "lap_duration": null, "is_pit_out_lap": true}),
        ];
        for n in 2..=7 {
            laps.push(lap(1, n, 90.0 + n as f64 * 0.05));
        }
        laps.push(lap(1, 8, 120.0)); // in-lap, slower than 15%
        for n in 1..=4 {
            laps.push(lap(4, n, 91.0));
        }
        let stints = json!([
            {"driver_number": 1, "lap_start": 1, "lap_end": 8, "compound": "SOFT", "tyre_age_at_start": 0},
            {"driver_number": 4, "lap_start": 1, "lap_end": 4, "compound": "HARD", "tyre_age_at_start": 3},
        ]);
        (drivers, Value::Array(laps), stints, json!([]), json!([]))
    }

    #[test]
    fn finds_best_laps_runs_and_the_ordering() {
        let (drivers, laps, stints, results, weather) = sample();
        let rc = json!([]);
        let session =
            json!({"session_key": 9, "session_name": "Practice 1", "session_type": "Practice"});
        let out = analyse(
            &session,
            &Raw {
                drivers: &drivers,
                laps: &laps,
                stints: &stints,
                results: &results,
                weather: &weather,
                race_control: &rc,
            },
        );
        // No official classification: ordered by best lap.
        let class = out["classification"].as_array().unwrap();
        assert_eq!(class[0]["driver"], 1);
        assert_eq!(class[0]["time"], 90.1);
        assert_eq!(class[1]["gap"].as_f64().unwrap().round(), 1.0);
        // Driver 1: laps 2-7 form one run (lap 8 is an in-lap).
        let runs = out["longRuns"]["all"].as_array().unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0]["driver"], 1);
        assert_eq!(runs[0]["pushLaps"], 6);
        assert_eq!(runs[1]["tyreAgeStart"], 3.0);
        // Slower laps in the run give a positive trend.
        assert!(runs[0]["trend"].as_f64().unwrap() > 0.0);
        assert_eq!(out["perDriver"][0]["tyres"]["SOFT"], 8);
        assert_eq!(out["shortRuns"]["teams"][0]["team"], "Red Bull");
        assert_eq!(out["drivers"]["4"]["name"], "NOR"); // falls back to the code
        assert!(out["weather"].is_null());
    }

    #[test]
    fn a_deleted_lap_is_not_a_best_lap() {
        let (drivers, laps, stints, results, weather) = sample();
        let rc = json!([{
            "date": "2026-01-01T10:00:00", "driver_number": null,
            "message": "CAR 1 (VER) TIME 1:30.100 DELETED - TRACK LIMITS AT TURN 9 LAP 2"
        }]);
        let session = json!({"session_name": "Qualifying"});
        let out = analyse(
            &session,
            &Raw {
                drivers: &drivers,
                laps: &laps,
                stints: &stints,
                results: &results,
                weather: &weather,
                race_control: &rc,
            },
        );
        assert_eq!(out["perDriver"][0]["best"]["lap"], 3);
        assert_eq!(out["deletedLaps"][0], json!({"driver": 1, "count": 1}));
        assert!(out["notes"].as_array().unwrap().is_empty());
        let laps = out["perDriver"][0]["lapList"].as_array().unwrap();
        assert_eq!(laps[0]["deleted"], true);
    }

    #[test]
    fn reinstated_laps_count_again() {
        let rc = json!([
            {"date": "1", "message": "CAR 16 (LEC) TIME 1:34.810 DELETED - TRACK LIMITS AT TURN 10 LAP 13"},
            {"date": "2", "message": "CAR 16 (LEC) LAP 13 WILL BE REINSTATED"},
            {"date": "3", "message": "CAR 43 (COL) TIME 1:35.500 DELETED - TRACK LIMITS LAP 5"},
            {"date": "4", "message": "CAR 43 (COL) LAP DELETED - NEXT LAP PIT"},
        ]);
        let deleted = deleted_lap_times(&rc);
        assert!(!deleted.contains_key(&16));
        assert_eq!(deleted[&43], vec![95.5]);
    }

    #[test]
    fn the_official_classification_uses_the_last_phase_reached() {
        let results = json!([
            {"position": 2, "driver_number": 4, "duration": [90.0, 89.0, null], "gap_to_leader": [1.0, 0.5, null], "number_of_laps": 20},
            {"position": 1, "driver_number": 1, "duration": [89.5, 88.5, 88.0], "gap_to_leader": [0, 0, 0], "number_of_laps": 21},
            {"position": 3, "driver_number": 5, "duration": 99.0, "dnf": true},
        ]);
        let out = classify(&results, &[]);
        assert_eq!(out[0]["driver"], 1);
        assert_eq!(out[0]["time"], 88.0);
        assert_eq!(out[0]["phase"], 3);
        assert_eq!(out[1]["time"], 89.0); // Q3 blank: Q2 time
        assert_eq!(out[1]["phase"], 2);
        assert_eq!(out[2]["phase"], Value::Null);
        assert_eq!(out[2]["status"], "DNF");
    }

    #[test]
    fn a_qualifying_best_lap_says_which_segment_it_was_set_in() {
        let rc = json!([
            {"date": "2026-10-24T21:00:00+00:00", "category": "SessionStatus", "message": "SESSION STARTED"},
            {"date": "2026-10-24T21:18:00+00:00", "category": "SessionStatus", "message": "SESSION FINISHED"},
            {"date": "2026-10-24T21:25:00+00:00", "category": "SessionStatus", "message": "SESSION STARTED"},
            {"date": "2026-10-24T21:40:00+00:00", "category": "SessionStatus", "message": "SESSION FINISHED"},
        ]);
        let found = segments("Qualifying", &rc);
        let names: Vec<_> = found.iter().map(|s| s.0.as_str()).collect();
        assert_eq!(names, ["Q1", "Q2"]);
        assert_eq!(segments("Sprint Qualifying", &rc)[0].0, "SQ1");
        assert!(segments("Practice 1", &rc).is_empty());

        let (drivers, _, stints, results, weather) = sample();
        let laps = json!([
            {"driver_number": 1, "lap_number": 2, "lap_duration": 90.0, "date_start": "2026-10-24T21:05:00+00:00"},
            {"driver_number": 1, "lap_number": 6, "lap_duration": 91.0, "date_start": "2026-10-24T21:30:00+00:00"},
        ]);
        let session = json!({"session_name": "Qualifying"});
        let out = analyse(
            &session,
            &Raw {
                drivers: &drivers,
                laps: &laps,
                stints: &stints,
                results: &results,
                weather: &weather,
                race_control: &rc,
            },
        );
        assert_eq!(out["perDriver"][0]["best"]["segment"], "Q1");
    }

    #[test]
    fn slope_and_median_basics() {
        assert!((slope(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]) - 2.0).abs() < 1e-12);
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
    }
}
