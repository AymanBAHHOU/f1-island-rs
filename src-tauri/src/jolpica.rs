//! Fetching from Jolpica, the race-data service. It allows only about 4 requests a
//! second, and the app asks for many things at start-up, so requests go out one slot at
//! a time with a gap between them, a "too many requests" answer is retried after a wait,
//! and answers are cached so the same URL isn't asked for twice.
//! (Port of `getJson` in the Electron app's `src/data.js`.)

use crate::championship::Kind;
use crate::championship::SeasonResult;
use crate::circuit::{parse_visit, parse_winners, total_winners, CircuitHistory};
use crate::schedule::{parse_schedule, pick_weekend, Race, Weekend};
use crate::season::parse_season_results;
use crate::standings::{parse_constructors, parse_drivers, parse_last_race, Standings};
use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::sleep;

const BASE: &str = "https://api.jolpi.ca/ergast/f1";
/// Gap between requests: Jolpica allows about 4 a second.
const GAP: Duration = Duration::from_millis(350);
/// How many times a "slow down" (429) answer is retried.
const RETRIES: u32 = 3;
/// The calendar rarely changes.
const SCHEDULE_TTL: Duration = Duration::from_secs(6 * 3600);
/// Standings and results change after each race (and lag a little behind it).
const STANDINGS_TTL: Duration = Duration::from_secs(10 * 60);
/// Results of this season's races, which only grow after a race.
const SEASON_TTL: Duration = Duration::from_secs(3600);
/// Past winners of a circuit only change when a race is run there.
const CIRCUIT_TTL: Duration = Duration::from_secs(12 * 3600);
/// Results of a race that is already over.
const PAST_TTL: Duration = Duration::from_secs(24 * 3600);

/// What can go wrong when fetching. An `enum` again: each way to fail is its own case.
#[derive(Debug)]
pub enum FetchError {
    /// Couldn't reach the service (offline, timeout, ...).
    Network(String),
    /// The service answered with an error code (404, 500, ...).
    Status(u16),
    /// It answered, but not with something we understand.
    Parse(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(e) => write!(f, "can't reach the race data service ({e})"),
            Self::Status(code) => write!(f, "the race data service answered {code}"),
            Self::Parse(e) => write!(f, "unexpected answer from the race data service ({e})"),
        }
    }
}

impl std::error::Error for FetchError {}

struct Cached {
    stored: Instant,
    body: String,
}

/// A client for Jolpica. One of these is shared by the whole app (Tauri "manages" it).
pub struct Jolpica {
    client: reqwest::Client,
    // The cache is only ever touched briefly and never across an `.await`, so a plain
    // `std` Mutex is right here.
    cache: Mutex<HashMap<String, Cached>>,
    // The time of the next free request slot. An *async* Mutex: waiting for it must not
    // block the thread.
    next_slot: AsyncMutex<Instant>,
}

impl Default for Jolpica {
    fn default() -> Self {
        Self::new()
    }
}

impl Jolpica {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("f1-island-rs (https://github.com/AymanBAHHOU/f1-island-rs)")
            .build()
            .expect("the HTTP client builds");
        Self {
            client,
            cache: Mutex::new(HashMap::new()),
            next_slot: AsyncMutex::new(Instant::now()),
        }
    }

    /// GETs `path` (such as `current.json?limit=100`) and returns the body text.
    /// A cached answer younger than `ttl` is returned without a request; if the request
    /// fails, an older cached answer is better than nothing.
    pub async fn get(&self, path: &str, ttl: Duration) -> Result<String, FetchError> {
        if let Some(body) = self.cached(path, Some(ttl), Instant::now()) {
            return Ok(body);
        }
        match self.fetch(path).await {
            Ok(body) => {
                self.cache.lock().unwrap().insert(
                    path.to_string(),
                    Cached {
                        stored: Instant::now(),
                        body: body.clone(),
                    },
                );
                Ok(body)
            }
            // `or` picks the stale copy if there is one, otherwise the error.
            Err(err) => self.cached(path, None, Instant::now()).ok_or(err),
        }
    }

    /// The cached body for `path`. With `Some(ttl)` it must be younger than that; with
    /// `None` any age will do (stale data).
    fn cached(&self, path: &str, ttl: Option<Duration>, now: Instant) -> Option<String> {
        let cache = self.cache.lock().unwrap();
        let entry = cache.get(path)?;
        let fresh = ttl.is_none_or(|ttl| now.duration_since(entry.stored) < ttl);
        fresh.then(|| entry.body.clone())
    }

    /// Waits for the next free slot, so requests are at least `GAP` apart.
    async fn wait_for_slot(&self) {
        let wait = {
            let mut next = self.next_slot.lock().await;
            let now = Instant::now();
            let at = (*next).max(now);
            *next = at + GAP; // the slot after this one
            at - now
        }; // the lock is released here, before sleeping
        sleep(wait).await;
    }

    async fn fetch(&self, path: &str) -> Result<String, FetchError> {
        let url = format!("{BASE}/{path}");
        let mut attempt = 0;
        loop {
            self.wait_for_slot().await;
            let response = self
                .client
                .get(&url)
                .send()
                .await
                .map_err(|e| FetchError::Network(e.to_string()))?;

            let status = response.status();
            if status.as_u16() == 429 && attempt < RETRIES {
                // Slow down: wait as long as asked (Retry-After), or back off 2s, 4s, 8s.
                let asked = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok());
                let wait = asked.unwrap_or(2 << attempt).min(15);
                sleep(Duration::from_secs(wait)).await;
                attempt += 1;
                continue;
            }
            if !status.is_success() {
                return Err(FetchError::Status(status.as_u16()));
            }
            return response
                .text()
                .await
                .map_err(|e| FetchError::Network(e.to_string()));
        }
    }

    /// The season's calendar.
    pub async fn races(&self) -> Result<Vec<Race>, FetchError> {
        let text = self.get("current.json?limit=100", SCHEDULE_TTL).await?;
        parse_schedule(&text).map_err(parse_error)
    }

    /// Every race (or Sprint) result of the season so far, for each driver's form.
    /// Jolpica pages its answers, so this asks until it has them all.
    pub async fn season_results(&self, sprint: bool) -> Result<Vec<SeasonResult>, FetchError> {
        let file = if sprint { "sprint" } else { "results" };
        let kind = if sprint { Kind::Sprint } else { Kind::Race };
        let mut all = Vec::new();
        let mut offset = 0;
        loop {
            let text = self
                .get(
                    &format!("current/{file}.json?limit=100&offset={offset}"),
                    SEASON_TTL,
                )
                .await?;
            let page = parse_season_results(&text, kind).map_err(parse_error)?;
            all.extend(page.rows);
            offset += 100;
            if offset >= page.total {
                return Ok(all);
            }
        }
    }

    /// The weekend to show at `now_ms`, from the season's calendar.
    pub async fn weekend(&self, now_ms: i64) -> Result<Option<Weekend>, FetchError> {
        let text = self.get("current.json?limit=100", SCHEDULE_TTL).await?;
        let races = parse_schedule(&text).map_err(parse_error)?;
        Ok(pick_weekend(&races, now_ms))
    }

    /// The championship tables and the last race's podium. The three requests are
    /// independent: if one fails the others still show, and only if all three fail is
    /// that an error.
    pub async fn standings(&self) -> Result<Standings, FetchError> {
        let drivers = self
            .get("current/driverStandings.json?limit=40", STANDINGS_TTL)
            .await;
        let teams = self
            .get("current/constructorStandings.json?limit=11", STANDINGS_TTL)
            .await;
        let last = self
            .get("current/last/results.json?limit=3", STANDINGS_TTL)
            .await;

        // Match on all three answers at once: only "all three failed" is an error.
        match (drivers, teams, last) {
            (Err(err), Err(_), Err(_)) => Err(err),
            // Otherwise, for each part: no answer, or one we can't read, is simply
            // "nothing to show".
            (drivers, teams, last) => Ok(Standings {
                drivers: drivers.ok().and_then(|t| parse_drivers(&t).ok().flatten()),
                constructors: teams
                    .ok()
                    .and_then(|t| parse_constructors(&t).ok())
                    .unwrap_or_default(),
                last_race: last.ok().and_then(|t| parse_last_race(&t).ok().flatten()),
            }),
        }
    }

    /// A circuit's recent winners and the recap of the last race held there before
    /// `season`.
    pub async fn circuit_history(
        &self,
        circuit_id: &str,
        season: u32,
    ) -> Result<CircuitHistory, FetchError> {
        // The id goes into a web address, so only plain characters are allowed.
        if circuit_id.is_empty()
            || !circuit_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(FetchError::Parse(format!(
                "not a circuit id: {circuit_id:?}"
            )));
        }

        // Jolpica lists winners oldest first, so ask how many there are, then for the last few.
        let head = self
            .get(
                &format!("circuits/{circuit_id}/results/1.json?limit=1"),
                CIRCUIT_TTL,
            )
            .await?;
        let total = total_winners(&head).map_err(parse_error)?;
        let offset = total.saturating_sub(6);
        let page = self
            .get(
                &format!("circuits/{circuit_id}/results/1.json?limit=6&offset={offset}"),
                CIRCUIT_TTL,
            )
            .await?;
        let winners = parse_winners(&page, season).map_err(parse_error)?;

        let Some(last_season) = winners.first().map(|w| w.season.clone()) else {
            return Ok(CircuitHistory {
                winners,
                last: None,
            }); // never raced here before
        };
        let results = self
            .get(
                &format!("{last_season}/circuits/{circuit_id}/results.json?limit=40"),
                PAST_TTL,
            )
            .await?;
        // Qualifying is optional: if it can't be had, the recap just has no pole.
        let quali = self
            .get(
                &format!("{last_season}/circuits/{circuit_id}/qualifying.json?limit=40"),
                PAST_TTL,
            )
            .await
            .ok();
        let last = parse_visit(&results, quali.as_deref()).map_err(parse_error)?;
        Ok(CircuitHistory { winners, last })
    }
}

fn parse_error(e: serde_json::Error) -> FetchError {
    FetchError::Parse(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_cached(path: &str, body: &str, stored: Instant) -> Jolpica {
        let client = Jolpica::new();
        client.cache.lock().unwrap().insert(
            path.into(),
            Cached {
                stored,
                body: body.into(),
            },
        );
        client
    }

    #[test]
    fn a_young_answer_is_fresh_and_an_old_one_is_not() {
        let stored = Instant::now();
        let client = with_cached("a.json", "body", stored);
        let ttl = Duration::from_secs(60);
        assert_eq!(
            client
                .cached("a.json", Some(ttl), stored + Duration::from_secs(30))
                .as_deref(),
            Some("body")
        );
        assert_eq!(
            client.cached("a.json", Some(ttl), stored + Duration::from_secs(61)),
            None
        );
    }

    #[test]
    fn a_stale_answer_is_still_available_when_asked_for() {
        let stored = Instant::now();
        let client = with_cached("a.json", "body", stored);
        let much_later = stored + Duration::from_secs(10 * 3600);
        assert_eq!(
            client.cached("a.json", None, much_later).as_deref(),
            Some("body")
        );
    }

    #[test]
    fn something_never_fetched_is_not_cached() {
        let client = Jolpica::new();
        assert_eq!(client.cached("nope.json", None, Instant::now()), None);
    }

    #[test]
    fn errors_read_as_sentences() {
        assert_eq!(
            FetchError::Status(503).to_string(),
            "the race data service answered 503"
        );
        assert!(FetchError::Network("timed out".into())
            .to_string()
            .contains("timed out"));
    }

    // A bad id is refused before any request is made, so this one needs no network.
    #[tokio::test]
    async fn a_circuit_id_with_odd_characters_is_refused() {
        let client = Jolpica::new();
        for bad in ["", "../etc", "americas/../x", "a b", "amé", "americas?x=1"] {
            let err = client.circuit_history(bad, 2026).await.unwrap_err();
            assert!(matches!(err, FetchError::Parse(_)), "{bad:?} gave {err}");
        }
    }

    // The tests below touch the network, so they only run on request:
    //     cargo test --lib -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "needs the network"]
    async fn the_real_standings_have_a_table_and_a_podium() {
        let standings = Jolpica::new().standings().await.expect("standings load");
        let drivers = standings.drivers.expect("a drivers table");
        assert!(!drivers.rows.is_empty() && drivers.rows.len() <= 10);
        assert!(!standings.constructors.is_empty());
        let last = standings.last_race.expect("a last race");
        assert_eq!(last.podium.len(), 3);
        println!(
            "round {}: leader {} ({} pts); last race {}: {:?}",
            drivers.round,
            drivers.rows[0].code,
            drivers.rows[0].points,
            last.name,
            last.podium.iter().map(|p| &p.code).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    #[ignore = "needs the network"]
    async fn the_real_history_of_the_circuit_of_the_americas() {
        let history = Jolpica::new()
            .circuit_history("americas", 2026)
            .await
            .expect("history loads");
        assert_eq!(history.winners.len(), 5);
        let last = history.last.expect("a recap of the last race");
        assert_eq!(last.podium.len(), 3);
        println!(
            "{} winners; last: {} {} podium {:?}, pole {:?}, fastest {:?}, time {:?}",
            history.winners.len(),
            last.season,
            last.name,
            last.podium.iter().map(|p| &p.code).collect::<Vec<_>>(),
            last.pole,
            last.fastest_lap,
            last.winner_time
        );
    }
    #[tokio::test]
    #[ignore = "needs the network"]
    async fn the_real_calendar_gives_a_weekend() {
        let client = Jolpica::new();
        let now = chrono::Utc::now().timestamp_millis();
        let weekend = client.weekend(now).await.expect("the calendar loads");
        let weekend = weekend.expect("there is a weekend to show");
        assert!(!weekend.sessions.is_empty());
        println!(
            "{} (round {}): {} sessions",
            weekend.name,
            weekend.round,
            weekend.sessions.len()
        );
    }

    #[tokio::test]
    #[ignore = "needs the network"]
    async fn the_second_request_for_the_same_calendar_is_served_from_the_cache() {
        let client = Jolpica::new();
        let first = Instant::now();
        client
            .get("current.json?limit=100", SCHEDULE_TTL)
            .await
            .expect("first request");
        let first = first.elapsed();
        let second = Instant::now();
        client
            .get("current.json?limit=100", SCHEDULE_TTL)
            .await
            .expect("second request");
        let second = second.elapsed();
        assert!(
            second < first / 5 && second < Duration::from_millis(50),
            "{first:?} then {second:?}"
        );
    }
}
