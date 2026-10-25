//! When to be connected to live timing. Connecting all the time would be wasteful, so the
//! app connects from shortly before a session until well after its scheduled end (sessions
//! overrun with red flags, and the final results land afterwards), and stays connected while
//! a session is still running even past that. (Port of `syncLiveTiming` in the Electron
//! app's `main.js`.)

use crate::schedule::Session;

/// Connect this long before a session starts...
const LEAD_MS: i64 = 15 * 60_000;
/// ...and stay this long after its scheduled end.
const TAIL_MS: i64 = 60 * 60_000;

/// Should we be connected at `now_ms`? `running` says a session is on right now (started or
/// red-flagged), whatever the schedule says.
pub fn should_connect(sessions: &[Session], now_ms: i64, running: bool) -> bool {
    running
        || sessions
            .iter()
            .any(|s| now_ms >= s.start_ms - LEAD_MS && now_ms <= s.end_ms + TAIL_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;
    const T0: i64 = 1_792_000_000_000; // the start of a session

    fn session(start: i64, minutes: i64) -> Session {
        Session {
            name: "Qualifying".into(),
            short: "Q".into(),
            start_ms: start,
            end_ms: start + minutes * MIN,
        }
    }

    #[test]
    fn connects_from_15_minutes_before_until_an_hour_after_the_end() {
        let sessions = [session(T0, 60)];
        assert!(!should_connect(&sessions, T0 - 16 * MIN, false));
        assert!(should_connect(&sessions, T0 - 15 * MIN, false)); // exactly 15 minutes
        assert!(should_connect(&sessions, T0 + 30 * MIN, false));
        assert!(should_connect(&sessions, T0 + 120 * MIN, false)); // 60 after the end
        assert!(!should_connect(&sessions, T0 + 121 * MIN, false));
    }

    #[test]
    fn a_running_session_keeps_it_connected_past_the_window() {
        let sessions = [session(T0, 60)];
        assert!(should_connect(&sessions, T0 + 10 * 60 * MIN, true)); // red flag, hours later
        assert!(should_connect(&[], T0, true));
    }

    #[test]
    fn any_session_of_the_weekend_counts() {
        let sessions = [session(T0, 60), session(T0 + 24 * 60 * MIN, 60)];
        assert!(should_connect(&sessions, T0 + 24 * 60 * MIN, false)); // the second one
        assert!(!should_connect(&sessions, T0 + 12 * 60 * MIN, false)); // between them
    }

    #[test]
    fn no_sessions_and_nothing_running_means_no_connection() {
        assert!(!should_connect(&[], T0, false));
    }
}
