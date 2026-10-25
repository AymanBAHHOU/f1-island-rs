//! The connection to F1's live-timing feed: SignalR over a WebSocket.
//! (Port of the connection half of the Electron app's `src/livetiming.js`.)
//!
//! How it works:
//! 1. `negotiate`: a plain request that returns a connection token (and a load-balancer
//!    cookie, which the socket must send back too).
//! 2. Open a WebSocket with the token, say hello (a "handshake"), and subscribe to the
//!    topics we want. The first answer is a snapshot of everything; after that, deltas.
//! 3. Keep the connection alive with a ping every 15 seconds, and reconnect, waiting
//!    longer each time, if it drops.
//!
//! This is one long-running `async` task. It doesn't know about windows: it sends
//! [`LiveEvent`]s down a channel, and whoever started it decides what to do with them.

use crate::live_state::{Event, LiveState, Summary};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::time::{interval, sleep, MissedTickBehavior};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::COOKIE;
use tokio_tungstenite::tungstenite::Message;

const BASE: &str = "https://livetiming.formula1.com/signalrcore";
/// The topics that work without an F1 TV account.
const TOPICS: [&str; 11] = [
    "Heartbeat",
    "SessionInfo",
    "SessionStatus",
    "TrackStatus",
    "RaceControlMessages",
    "TimingData",
    "TimingAppData", // tyres per stint
    "DriverList",
    "LapCount",
    "ExtrapolatedClock",
    "PitLaneTimeCollection", // how long each pit stop took in the pit lane
];
/// SignalR ends each message with this character, so one WebSocket frame can hold several.
const RECORD_SEPARATOR: char = '\u{1e}';
const PING_EVERY: Duration = Duration::from_secs(15);
/// A burst of updates becomes at most one summary this often.
const SUMMARY_EVERY: Duration = Duration::from_secs(1);

/// What the connection tells the app.
#[derive(Debug, Clone)]
pub enum LiveEvent {
    /// Something that happened in the session: a flag, a message, a status change.
    Feed(Event),
    /// The current state of the session, at most once a second. `None` once we've stopped.
    State(Option<Box<Summary>>),
}

/// Why one connection ended.
#[derive(Debug)]
enum Ended {
    /// We were asked to stop.
    Stopped,
    /// The server closed it, or it dropped. `snapshot` says whether we got as far as the
    /// first snapshot, which means the connection worked and the retry delay can start over.
    Dropped { snapshot: bool },
}

// ---------- the small pure pieces ----------

/// How long to wait before reconnecting: 2 s, 4 s, 8 s ... up to a minute.
pub fn backoff(retries: u32) -> Duration {
    Duration::from_secs((2u64 << retries.min(5)).min(60))
}

/// A WebSocket frame holds one or more records, each ending in the separator.
pub fn split_records(frame: &str) -> impl Iterator<Item = &str> {
    frame
        .split(RECORD_SEPARATOR)
        .filter(|record| !record.is_empty())
}

fn record(value: &Value) -> Message {
    Message::text(format!("{value}{RECORD_SEPARATOR}"))
}

/// The first thing we send: which protocol we speak.
fn handshake() -> Message {
    record(&json!({ "protocol": "json", "version": 1 }))
}

/// The second: subscribe to the topics. Its `invocationId` is how the answer (the
/// snapshot) is recognised.
fn subscribe() -> Message {
    record(
        &json!({ "type": 1, "invocationId": "sub", "target": "Subscribe", "arguments": [TOPICS] }),
    )
}

fn ping() -> Message {
    record(&json!({ "type": 6 }))
}

/// The address of the socket for a connection token. The token may contain characters
/// that must be escaped in a web address, hence `append_pair`.
pub fn socket_url(token: &str) -> Result<String, String> {
    let ws_base = BASE.replacen("https", "wss", 1);
    let mut url = reqwest::Url::parse(&ws_base).map_err(|e| e.to_string())?;
    url.query_pairs_mut().append_pair("id", token);
    Ok(url.to_string())
}

/// `Set-Cookie` headers ("a=1; Path=/; Secure") into one `Cookie` header ("a=1; b=2").
pub fn cookie_header<'a>(set_cookies: impl IntoIterator<Item = &'a str>) -> String {
    set_cookies
        .into_iter()
        .filter_map(|c| c.split(';').next())
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

// ---------- the connection ----------

/// Runs until `stop` says true: connects, and reconnects when the connection drops.
/// Events go down `events`; if nobody is listening any more, it stops.
pub async fn run(events: mpsc::UnboundedSender<LiveEvent>, mut stop: watch::Receiver<bool>) {
    let mut state = LiveState::new();
    let mut retries = 0;

    while !*stop.borrow() {
        match connect_once(&events, &mut stop, &mut state).await {
            Ok(Ended::Stopped) => break,
            Ok(Ended::Dropped { snapshot }) => {
                if snapshot {
                    retries = 0;
                }
            }
            Err(_) => {} // couldn't connect at all: the same, wait and try again
        }
        // Wait, but wake up straight away if asked to stop.
        tokio::select! {
            _ = sleep(backoff(retries)) => {}
            _ = stop.changed() => {}
        }
        retries += 1;
    }

    state.clear();
    let _ = events.send(LiveEvent::State(None));
}

/// One connection, from negotiating to being dropped or stopped.
async fn connect_once(
    events: &mpsc::UnboundedSender<LiveEvent>,
    stop: &mut watch::Receiver<bool>,
    state: &mut LiveState,
) -> Result<Ended, String> {
    // 1. Negotiate.
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let response = http
        .post(format!("{BASE}/negotiate?negotiateVersion=1"))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("negotiate answered {}", response.status()));
    }
    // The load balancer wants its cookie on the socket too.
    let cookie = cookie_header(
        response
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok()),
    );
    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    let token = body["connectionToken"]
        .as_str()
        .ok_or("no connectionToken in the negotiate answer")?;

    // 2. Open the socket, say hello, subscribe.
    let mut request = socket_url(token)?
        .into_client_request()
        .map_err(|e| e.to_string())?;
    if !cookie.is_empty() {
        request
            .headers_mut()
            .insert(COOKIE, cookie.parse().map_err(|_| "bad cookie")?);
    }
    let (socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| e.to_string())?;
    let (mut write, mut read) = socket.split();
    write.send(handshake()).await.map_err(|e| e.to_string())?;
    write.send(subscribe()).await.map_err(|e| e.to_string())?;

    // 3. Listen. `select!` waits for whichever of these happens first.
    let mut pings = interval(PING_EVERY);
    let mut summaries = interval(SUMMARY_EVERY);
    summaries.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut changed = false; // updates since the last summary
    let mut got_snapshot = false;

    loop {
        tokio::select! {
            frame = read.next() => {
                let text = match frame {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None => return Ok(Ended::Dropped { snapshot: got_snapshot }),
                    Some(Ok(_)) => continue, // pings, pongs and binary frames need nothing from us
                    Some(Err(_)) => return Ok(Ended::Dropped { snapshot: got_snapshot }),
                };
                for part in split_records(text.as_str()) {
                    let Ok(message) = serde_json::from_str::<Value>(part) else { continue };
                    for event in state.handle(&message) {
                        match event {
                            Event::ServerClosed => return Ok(Ended::Dropped { snapshot: got_snapshot }),
                            Event::Snapshot(_) => got_snapshot = true,
                            _ => {}
                        }
                        if events.send(LiveEvent::Feed(event)).is_err() {
                            return Ok(Ended::Stopped); // nobody is listening any more
                        }
                    }
                    changed = true;
                }
            }
            _ = pings.tick() => {
                write.send(ping()).await.map_err(|e| e.to_string())?;
            }
            _ = summaries.tick() => {
                if changed {
                    changed = false;
                    if events.send(LiveEvent::State(Some(Box::new(state.summary())))).is_err() {
                        return Ok(Ended::Stopped);
                    }
                }
            }
            _ = stop.changed() => {
                let _ = write.close().await;
                return Ok(Ended::Stopped);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn the_wait_grows_and_stops_at_a_minute() {
        let waits: Vec<u64> = (0..8).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(waits, [2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(backoff(u32::MAX).as_secs(), 60); // no overflow
    }

    #[test]
    fn a_frame_can_hold_several_records_and_blanks_are_skipped() {
        let frame = "{\"type\":6}\u{1e}{\"type\":1}\u{1e}";
        assert_eq!(
            split_records(frame).collect::<Vec<_>>(),
            ["{\"type\":6}", "{\"type\":1}"]
        );
        assert_eq!(split_records("").count(), 0);
        assert_eq!(split_records("\u{1e}\u{1e}").count(), 0);
    }

    #[test]
    fn the_messages_we_send_end_with_the_separator_and_say_the_right_things() {
        let hello = handshake().into_text().unwrap();
        assert_eq!(
            hello.as_str(),
            "{\"protocol\":\"json\",\"version\":1}\u{1e}"
        );

        let sub = subscribe().into_text().unwrap();
        let body: Value = serde_json::from_str(sub.trim_end_matches(RECORD_SEPARATOR)).unwrap();
        assert_eq!(body["target"], "Subscribe");
        assert_eq!(body["invocationId"], "sub");
        assert_eq!(body["arguments"][0].as_array().unwrap().len(), TOPICS.len());
        assert!(body["arguments"][0]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "TimingData"));
        assert!(sub.ends_with(RECORD_SEPARATOR));
        assert_eq!(ping().into_text().unwrap().as_str(), "{\"type\":6}\u{1e}");
    }

    #[test]
    fn the_token_goes_into_the_address_escaped() {
        let url = socket_url("abc DEF/+=").unwrap();
        assert!(
            url.starts_with("wss://livetiming.formula1.com/signalrcore?id="),
            "{url}"
        );
        assert!(!url.contains(' '), "{url}");
        assert_eq!(
            reqwest::Url::parse(&url)
                .unwrap()
                .query_pairs()
                .next()
                .unwrap()
                .1,
            "abc DEF/+="
        );
    }

    #[test]
    fn cookies_are_joined_without_their_attributes() {
        let header = cookie_header([
            "AWSALBCORS=abc; Expires=Fri, 01 Jan 2027; Path=/; Secure",
            " other=1 ; HttpOnly",
            "",
        ]);
        assert_eq!(header, "AWSALBCORS=abc; other=1");
        assert_eq!(cookie_header([]), "");
    }

    #[tokio::test]
    async fn stopping_before_connecting_ends_straight_away_with_no_state() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (stop_tx, stop_rx) = watch::channel(true); // already stopped
        let started = Instant::now();
        run(tx, stop_rx).await;
        drop(stop_tx);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(matches!(rx.recv().await, Some(LiveEvent::State(None))));
    }

    // This one touches the real feed, so it only runs on request:
    //     cargo test --lib -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "needs the network"]
    async fn the_real_feed_sends_a_snapshot_and_then_a_summary() {
        crate::install_tls_provider();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (stop_tx, stop_rx) = watch::channel(false);
        let task = tokio::spawn(run(tx, stop_rx));

        let mut snapshot = None;
        let mut summary = None;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
        while tokio::time::Instant::now() < deadline && (snapshot.is_none() || summary.is_none()) {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(LiveEvent::Feed(Event::Snapshot(status)))) => snapshot = Some(status),
                Ok(Some(LiveEvent::State(Some(s)))) => summary = Some(s),
                Ok(Some(_)) => {}
                _ => break,
            }
        }
        stop_tx.send(true).unwrap();
        task.await.unwrap();

        let snapshot = snapshot.expect("a snapshot arrived");
        let summary = summary.expect("a summary arrived");
        println!(
            "session {:?} ({:?}), status {:?} (snapshot said {:?}), part {:?}, {} drivers, {} messages",
            summary.session, summary.meeting, summary.status, snapshot, summary.part, summary.order.len(), summary.messages.len()
        );
        if let Some(first) = summary.order.first() {
            println!("P1: {} best {:?}", first.code, first.best);
        }
    }
}
