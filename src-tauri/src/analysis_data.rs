//! Getting the data for the Analysis page and working it into the shapes the page wants:
//! a session's analysis, lap telemetry, positions on track (track maps and the replay).
//! (Port of the fetching half of the Electron app's `src/analysis.js`.)

use crate::analysis::{self, Raw};
use crate::openf1::{OpenF1, FINAL_AFTER_MS};
use crate::results::{parse_ms, Results};
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const FRESH_FOR: Duration = Duration::from_secs(5 * 60);
const REPLAY_CHUNK_MS: i64 = 60_000;
const CHUNK_CACHE_MAX: usize = 40;

const NO_LAPS: &str =
    "No lap data for this session yet. It usually appears about 30 minutes after the session ends.";

fn rows(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

fn num(row: &Value, key: &str) -> Option<f64> {
    row.get(key)?.as_f64().filter(|n| n.is_finite())
}

fn time_ms(row: &Value, key: &str) -> Option<i64> {
    parse_ms(row.get(key)?.as_str()?)
}

/// JavaScript's `Date.toISOString()`.
fn iso(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// A time in a web address: the colons are escaped.
fn in_url(iso: &str) -> String {
    iso.replace(':', "%3A")
}

/// JavaScript's `Math.round` (halves go up).
fn round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// Everything fetched for one session, kept for the calls that follow.
pub struct Cached {
    at: Instant,
    /// Data of a session that ended long ago no longer changes.
    is_final: bool,
    pub data: Value,
    pub laps: Value,
    pub session: Value,
    pub race_control: Value,
    pub drivers: Value,
}

impl Cached {
    fn session_start_ms(&self) -> Option<i64> {
        time_ms(&self.session, "date_start")
    }

    /// A saved copy made after this is final (see `OpenF1::get`).
    fn final_after(&self) -> Option<i64> {
        time_ms(&self.session, "date_end").map(|end| end + FINAL_AFTER_MS)
    }

    fn lap(&self, driver: i64, number: i64) -> Option<&Value> {
        rows(&self.laps).iter().find(|l| {
            l["driver_number"].as_i64() == Some(driver) && l["lap_number"].as_i64() == Some(number)
        })
    }
}

#[derive(Default)]
pub struct Analyses {
    sessions: Mutex<HashMap<i64, Arc<Cached>>>,
    locations: Mutex<HashMap<String, Value>>,
    telemetry: Mutex<HashMap<String, Value>>,
    maps: Mutex<HashMap<i64, Value>>,
    /// Oldest first, so the oldest can be dropped.
    chunks: Mutex<Vec<(String, Value)>>,
}

impl Analyses {
    /// The session's analysis (what the Analysis page shows), fetched on first use.
    pub async fn get(
        &self,
        openf1: &OpenF1,
        session_key: i64,
        now_ms: i64,
    ) -> Result<Arc<Cached>, String> {
        if let Some(hit) = self.sessions.lock().unwrap().get(&session_key) {
            if hit.is_final || hit.at.elapsed() < FRESH_FOR {
                return Ok(hit.clone());
            }
        }

        // A session's own details don't change: any saved copy will do.
        let found = openf1
            .get(&format!("sessions?session_key={session_key}"), Some(0))
            .await?;
        let session = rows(&found).first().cloned().ok_or("Session not found")?;
        let end_ms = time_ms(&session, "date_end");
        let after = end_ms.map(|e| e + FINAL_AFTER_MS);
        let q = format!("session_key={session_key}");
        // One at a time on purpose (rate limit); the client spaces them out.
        let drivers = openf1.get(&format!("drivers?{q}"), after).await?;
        let laps = openf1.get(&format!("laps?{q}"), after).await?;
        let stints = openf1.get(&format!("stints?{q}"), after).await?;
        let results = openf1.get(&format!("session_result?{q}"), after).await?;
        let weather = openf1.get(&format!("weather?{q}"), after).await?;
        let race_control = openf1.get(&format!("race_control?{q}"), after).await?;

        if rows(&laps).is_empty() {
            return Err(NO_LAPS.to_string());
        }
        let data = analysis::analyse(
            &session,
            &Raw {
                drivers: &drivers,
                laps: &laps,
                stints: &stints,
                results: &results,
                weather: &weather,
                race_control: &race_control,
            },
        );
        let cached = Arc::new(Cached {
            at: Instant::now(),
            // Data is considered final a couple of hours after the session ends.
            is_final: end_ms.is_some_and(|e| now_ms - e > FINAL_AFTER_MS),
            data,
            laps,
            session,
            race_control,
            drivers,
        });
        self.sessions
            .lock()
            .unwrap()
            .insert(session_key, cached.clone());
        Ok(cached)
    }

    // ---------- telemetry ----------

    /// Car telemetry for one lap, with distance (m) from the start line obtained by
    /// integrating speed. Samples arrive about 3.7 times a second.
    pub async fn telemetry(
        &self,
        openf1: &OpenF1,
        session_key: i64,
        driver: i64,
        lap_number: i64,
        now_ms: i64,
    ) -> Result<Value, String> {
        let cache_key = format!("{session_key}/{driver}/{lap_number}");
        if let Some(hit) = self.telemetry.lock().unwrap().get(&cache_key) {
            return Ok(hit.clone());
        }
        let a = self.get(openf1, session_key, now_ms).await?;
        let lap = a.lap(driver, lap_number).ok_or("No timing for that lap.")?;
        let (Some(start), Some(lap_time)) = (time_ms(lap, "date_start"), num(lap, "lap_duration"))
        else {
            return Err("No timing for that lap.".into());
        };

        let from = iso(start - 1500);
        let to = iso(start + (lap_time * 1000.0) as i64 + 1500);
        let found = openf1
            .get(
                &format!(
                    "car_data?session_key={session_key}&driver_number={driver}&date>={}&date<={}",
                    in_url(&from),
                    in_url(&to)
                ),
                a.final_after(),
            )
            .await?;
        if rows(&found).len() < 20 {
            return Err("No car telemetry for that lap.".into());
        }

        let result = build_telemetry(rows(&found), start, lap_time, driver, lap_number);
        self.telemetry
            .lock()
            .unwrap()
            .insert(cache_key, result.clone());
        Ok(result)
    }

    // ---------- positions on track ----------

    /// x/y positions (decimetres) through one lap, with t = seconds from the lap start.
    pub async fn lap_location(
        &self,
        openf1: &OpenF1,
        session_key: i64,
        driver: i64,
        lap_number: i64,
        now_ms: i64,
    ) -> Result<Value, String> {
        let cache_key = format!("{session_key}/{driver}/{lap_number}");
        if let Some(hit) = self.locations.lock().unwrap().get(&cache_key) {
            return Ok(hit.clone());
        }
        let a = self.get(openf1, session_key, now_ms).await?;
        let lap = a.lap(driver, lap_number).ok_or("No timing for that lap.")?;
        let (Some(start), Some(lap_time)) = (time_ms(lap, "date_start"), num(lap, "lap_duration"))
        else {
            return Err("No timing for that lap.".into());
        };
        let from = iso(start);
        let to = iso(start + (lap_time * 1000.0) as i64);
        let found = openf1
            .get(
                &format!(
                    "location?session_key={session_key}&driver_number={driver}&date>={}&date<={}",
                    in_url(&from),
                    in_url(&to)
                ),
                a.final_after(),
            )
            .await?;
        // (0,0,0) rows are dropouts.
        let mut points: Vec<(f64, Value, Value)> = rows(&found)
            .iter()
            .filter(|r| num(r, "x").unwrap_or(0.0) != 0.0 || num(r, "y").unwrap_or(0.0) != 0.0)
            .filter_map(|r| {
                Some((
                    (time_ms(r, "date")? - start) as f64 / 1000.0,
                    r["x"].clone(),
                    r["y"].clone(),
                ))
            })
            .collect();
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        if points.len() < 30 {
            return Err("No position data for that lap.".into());
        }
        let result = json!({
            "driver": driver,
            "lap": lap_number,
            "points": points.iter().map(|(t, x, y)| json!({"t": t, "x": x, "y": y})).collect::<Vec<_>>(),
        });
        self.locations
            .lock()
            .unwrap()
            .insert(cache_key, result.clone());
        Ok(result)
    }

    /// Track outline from the session's fastest clean lap.
    pub async fn track_map(
        &self,
        openf1: &OpenF1,
        session_key: i64,
        now_ms: i64,
    ) -> Result<Value, String> {
        if let Some(hit) = self.maps.lock().unwrap().get(&session_key) {
            return Ok(hit.clone());
        }
        let a = self.get(openf1, session_key, now_ms).await?;
        let fastest = rows(&a.laps)
            .iter()
            .filter(|l| {
                num(l, "lap_duration").is_some()
                    && l["is_pit_out_lap"].as_bool() != Some(true)
                    && l["date_start"].is_string()
            })
            .min_by(|x, y| {
                num(x, "lap_duration")
                    .unwrap()
                    .total_cmp(&num(y, "lap_duration").unwrap())
            })
            .ok_or("No laps to draw the track from.")?;
        let (Some(driver), Some(lap)) = (
            fastest["driver_number"].as_i64(),
            fastest["lap_number"].as_i64(),
        ) else {
            return Err("No laps to draw the track from.".into());
        };
        let location = self
            .lap_location(openf1, session_key, driver, lap, now_ms)
            .await?;
        let points: Vec<Value> = location["points"]
            .as_array()
            .map(|p| {
                p.iter()
                    .map(|p| json!({"x": p["x"], "y": p["y"]}))
                    .collect()
            })
            .unwrap_or_default();
        let map = json!({ "points": points });
        self.maps.lock().unwrap().insert(session_key, map.clone());
        Ok(map)
    }

    /// Map for a race weekend: this weekend's latest finished session, otherwise last
    /// year's race at the same venue. `None` if there is nothing to draw it from.
    #[allow(clippy::too_many_arguments)]
    pub async fn weekend_track_map(
        &self,
        openf1: &OpenF1,
        results: &Results,
        start_ms: i64,
        end_ms: i64,
        country: &str,
        locality: &str,
        now_ms: i64,
    ) -> Result<Option<Value>, String> {
        let sessions = results.sessions(openf1, now_ms).await?;
        let (t0, t1) = (start_ms - 6 * 3600 * 1000, end_ms + 24 * 3600 * 1000);
        let this_weekend = sessions
            .iter()
            .find(|s| parse_ms(&s.start).is_some_and(|t| (t0..=t1).contains(&t)));
        if let Some(s) = this_weekend {
            return self.track_map(openf1, s.key, now_ms).await.map(Some);
        }

        let year = DateTime::<Utc>::from_timestamp_millis(start_ms)
            .map_or(2025, |t| chrono::Datelike::year(&t))
            - 1;
        let races = openf1
            .get(&format!("sessions?year={year}&session_name=Race"), None)
            .await?;
        let norm = |v: &Value, key: &str| v[key].as_str().unwrap_or("").to_lowercase();
        let country = country.to_lowercase();
        let locality = locality.to_lowercase();
        let same_country: Vec<&Value> = rows(&races)
            .iter()
            .filter(|r| norm(r, "country_name") == country)
            .collect();
        let found = same_country
            .iter()
            .find(|r| {
                let location = norm(r, "location");
                location.contains(&locality) || locality.contains(&location)
            })
            .copied()
            .or_else(|| (same_country.len() == 1).then(|| same_country[0]));
        let Some(key) = found.and_then(|r| r["session_key"].as_i64()) else {
            return Ok(None);
        };
        self.track_map(openf1, key, now_ms).await.map(Some)
    }

    // ---------- replay ----------

    pub async fn replay_info(
        &self,
        openf1: &OpenF1,
        session_key: i64,
        now_ms: i64,
    ) -> Result<Value, String> {
        let a = self.get(openf1, session_key, now_ms).await?;
        let start = a.session_start_ms().ok_or("Session has no start time.")?;
        let end = time_ms(&a.session, "date_end").unwrap_or(start);
        let rel = |iso: Option<i64>| iso.unwrap_or(start) - start;

        let positions = openf1
            .get(&format!("position?session_key={session_key}"), None)
            .await?;
        let mut drivers = Map::new();
        for d in rows(&a.drivers) {
            let Some(n) = d["driver_number"].as_i64() else {
                continue;
            };
            drivers.insert(
                n.to_string(),
                json!({
                    "code": d["name_acronym"].as_str().map_or_else(|| format!("#{n}"), str::to_string),
                    "color": d["team_colour"].as_str().map_or_else(|| "#888888".to_string(), |c| format!("#{c}")),
                    "team": d["team_name"],
                }),
            );
        }
        let mut pos: Vec<(i64, Value, Value)> = rows(&positions)
            .iter()
            .map(|p| {
                (
                    rel(time_ms(p, "date")),
                    p["driver_number"].clone(),
                    p["position"].clone(),
                )
            })
            .collect();
        pos.sort_by_key(|p| p.0);
        let mut laps: Vec<(i64, Value, Value)> = rows(&a.laps)
            .iter()
            .filter_map(|l| {
                Some((
                    rel(Some(time_ms(l, "date_start")?)),
                    l["driver_number"].clone(),
                    l["lap_number"].clone(),
                ))
            })
            .collect();
        laps.sort_by_key(|l| l.0);
        let mut messages: Vec<(i64, Value)> = rows(&a.race_control)
            .iter()
            .filter(|m| m["message"].as_str().is_some_and(|s| !s.is_empty()))
            .map(|m| {
                let t = rel(time_ms(m, "date"));
                (
                    t,
                    json!({"t": t, "message": m["message"], "flag": m["flag"], "category": m["category"]}),
                )
            })
            .collect();
        messages.sort_by_key(|m| m.0);

        let name = a.session["session_name"].as_str().unwrap_or("");
        let track = self.track_map(openf1, session_key, now_ms).await.ok();
        Ok(json!({
            "sessionKey": session_key,
            "name": name,
            "isRace": analysis::is_race(name),
            "duration": end - start,
            "chunkMs": REPLAY_CHUNK_MS,
            "drivers": drivers,
            "positions": pos.iter().map(|(t, d, p)| json!([t, d, p])).collect::<Vec<_>>(),
            "laps": laps.iter().map(|(t, d, l)| json!([t, d, l])).collect::<Vec<_>>(),
            "messages": messages.into_iter().map(|m| m.1).collect::<Vec<_>>(),
            "track": track,
        }))
    }

    /// Car positions for one chunk of the session: `{ cars: { [driver]: [[ms, x, y], ...] } }`.
    pub async fn replay_chunk(
        &self,
        openf1: &OpenF1,
        session_key: i64,
        index: i64,
        now_ms: i64,
    ) -> Result<Value, String> {
        let cache_key = format!("{session_key}/{index}");
        if let Some((_, hit)) = self
            .chunks
            .lock()
            .unwrap()
            .iter()
            .find(|(k, _)| *k == cache_key)
        {
            return Ok(hit.clone());
        }
        let a = self.get(openf1, session_key, now_ms).await?;
        let start = a.session_start_ms().ok_or("Session has no start time.")?;
        let from = iso(start + index * REPLAY_CHUNK_MS);
        let to = iso(start + (index + 1) * REPLAY_CHUNK_MS);
        let found = openf1
            .get(
                &format!(
                    "location?session_key={session_key}&date>={}&date<{}",
                    in_url(&from),
                    in_url(&to)
                ),
                None,
            )
            .await?;

        let mut cars: Map<String, Value> = Map::new();
        for r in rows(&found) {
            if num(r, "x").unwrap_or(0.0) == 0.0 && num(r, "y").unwrap_or(0.0) == 0.0 {
                continue;
            }
            let (Some(driver), Some(t)) = (r["driver_number"].as_i64(), time_ms(r, "date")) else {
                continue;
            };
            let list = cars
                .entry(driver.to_string())
                .or_insert_with(|| Value::Array(Vec::new()));
            list.as_array_mut()
                .unwrap()
                .push(json!([t - start, r["x"], r["y"]]));
        }
        for list in cars.values_mut() {
            if let Some(list) = list.as_array_mut() {
                list.sort_by_key(|p| p[0].as_i64().unwrap_or(0));
            }
        }
        let chunk = json!({"index": index, "cars": cars});
        let mut chunks = self.chunks.lock().unwrap();
        chunks.push((cache_key, chunk.clone()));
        if chunks.len() > CHUNK_CACHE_MAX {
            chunks.remove(0);
        }
        Ok(chunk)
    }
}

// ---------- the telemetry maths ----------

#[derive(Clone, Copy)]
struct Sample {
    t: f64,
    speed: f64,
    throttle: f64,
    brake: f64,
    gear: f64,
    drs: bool,
    d: f64,
}

/// Value of one field at time `t`, linearly interpolated between samples.
fn interp_at(samples: &[Sample], t: f64, field: impl Fn(&Sample) -> f64) -> f64 {
    // (Before the first sample, and after the last, this gives the first sample's value.)
    let i = samples.iter().position(|s| s.t >= t).unwrap_or(0);
    if i == 0 {
        return field(&samples[0]);
    }
    let (a, b) = (&samples[i - 1], &samples[i]);
    let span = b.t - a.t;
    field(a) + (field(b) - field(a)) * (t - a.t) / if span == 0.0 { 1.0 } else { span }
}

fn build_telemetry(rows: &[Value], start: i64, lap_time: f64, driver: i64, lap: i64) -> Value {
    // Time from the lap start, then distance by trapezoid integration of speed.
    let mut raw: Vec<Sample> = rows
        .iter()
        .filter_map(|r| {
            Some(Sample {
                t: (time_ms(r, "date")? - start) as f64 / 1000.0,
                speed: num(r, "speed").unwrap_or(0.0),
                throttle: num(r, "throttle").unwrap_or(0.0).clamp(0.0, 100.0),
                brake: if num(r, "brake").unwrap_or(0.0) > 0.0 {
                    100.0
                } else {
                    0.0
                },
                gear: num(r, "n_gear").unwrap_or(0.0),
                drs: num(r, "drs").unwrap_or(0.0) >= 10.0, // 10/12/14 = flap open
                d: 0.0,
            })
        })
        .collect();
    raw.sort_by(|a, b| a.t.total_cmp(&b.t));
    for i in 1..raw.len() {
        raw[i].d = raw[i - 1].d
            + ((raw[i - 1].speed + raw[i].speed) / 2.0) * (raw[i].t - raw[i - 1].t) / 3.6;
    }

    // Re-zero on the start line and clip to the lap.
    let d0 = interp_at(&raw, 0.0, |s| s.d);
    let length = interp_at(&raw, lap_time, |s| s.d) - d0;
    let sample_json = |s: &Sample| {
        json!({
            "t": round(s.t * 1000.0) / 1000.0,
            "speed": s.speed,
            "throttle": s.throttle,
            "brake": s.brake,
            "gear": s.gear,
            "drs": s.drs,
            "d": round((s.d - d0) * 10.0) / 10.0,
        })
    };
    let mut samples: Vec<Value> = raw
        .iter()
        .filter(|s| s.t >= 0.0 && s.t <= lap_time)
        .map(sample_json)
        .collect();
    let edge = |t: f64, brake: f64, drs: bool| {
        json!({
            "t": t,
            "d": round((interp_at(&raw, t, |s| s.d) - d0) * 10.0) / 10.0,
            "speed": round(interp_at(&raw, t, |s| s.speed)),
            "throttle": round(interp_at(&raw, t, |s| s.throttle)),
            "brake": brake,
            "gear": round(interp_at(&raw, t, |s| s.gear)),
            "drs": drs,
        })
    };
    let first_brake = samples
        .first()
        .map_or(0.0, |s| s["brake"].as_f64().unwrap_or(0.0));
    samples.insert(0, edge(0.0, first_brake, false));
    let last = samples.last().cloned().unwrap_or(Value::Null);
    samples.push(edge(
        lap_time,
        last["brake"].as_f64().unwrap_or(0.0),
        last["drs"].as_bool().unwrap_or(false),
    ));

    json!({"driver": driver, "lap": lap, "lapTime": lap_time, "length": length, "samples": samples})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn car_row(ms_from_start: i64, speed: f64) -> Value {
        // lap starts at 2026-01-01T00:00:10Z
        json!({
            "date": iso(1_767_225_610_000 + ms_from_start),
            "speed": speed, "throttle": 100, "brake": 0, "n_gear": 7, "drs": 12
        })
    }

    #[test]
    fn telemetry_integrates_speed_into_distance_from_the_start_line() {
        // A steady 360 km/h (100 m/s) over a 2 second lap, sampled from 1.5 s before.
        let rows: Vec<Value> = (-15..=35).map(|i| car_row(i * 100, 360.0)).collect();
        let out = build_telemetry(&rows, 1_767_225_610_000, 2.0, 1, 5);
        assert_eq!(out["driver"], 1);
        assert!((out["length"].as_f64().unwrap() - 200.0).abs() < 1e-6);
        let samples = out["samples"].as_array().unwrap();
        assert_eq!(samples[0]["t"], 0.0);
        assert_eq!(samples[0]["d"], 0.0);
        let last = samples.last().unwrap();
        assert_eq!(last["t"], 2.0);
        assert_eq!(last["d"], 200.0);
        assert_eq!(last["drs"], true); // taken from the last real sample
        assert_eq!(samples[1]["drs"], true);
    }

    #[test]
    fn interpolation_is_linear_between_samples() {
        let s = |t, d| Sample {
            t,
            speed: 0.0,
            throttle: 0.0,
            brake: 0.0,
            gear: 0.0,
            drs: false,
            d,
        };
        let samples = [s(0.0, 0.0), s(2.0, 10.0)];
        assert_eq!(interp_at(&samples, 1.0, |s| s.d), 5.0);
        assert_eq!(interp_at(&samples, 5.0, |s| s.d), 0.0); // past the end: the first value, as before
    }

    #[test]
    fn colons_are_escaped_in_times() {
        assert_eq!(
            in_url(&iso(1_767_225_610_123)),
            "2026-01-01T00%3A00%3A10.123Z"
        );
    }

    #[test]
    fn js_style_rounding() {
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(-2.5), -2.0);
    }
}
