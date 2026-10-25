//! The session clock. The live feed reports "time remaining" at a moment in time and
//! whether the clock is running; a running clock is counted down locally between
//! reports. (Port of `renderer/clock.js`.)

/// What the feed says about the clock.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionClock {
    /// Time left when it was reported, "H:MM:SS" (e.g. "00:03:03").
    pub remaining: String,
    /// When that was reported, in milliseconds since 1970.
    pub reported_at_ms: i64,
    /// Whether it is counting down.
    pub running: bool,
}

/// Milliseconds left now, or `None` if there's no clock or it carries no time.
pub fn remaining_ms(clock: Option<&SessionClock>, now_ms: i64) -> Option<i64> {
    let clock = clock?; // no clock at all: None
    if clock.remaining.is_empty() {
        return None;
    }

    // "H:MM:SS" -> hours, minutes, seconds; a missing or odd part counts as 0.
    let mut parts = clock
        .remaining
        .split(':')
        .map(|p| p.parse::<i64>().unwrap_or(0));
    let (h, m, s) = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    let at_report = (h * 3600 + m * 60 + s) * 1000;

    if !clock.running {
        return Some(at_report); // stopped (red flag, before the start): holds its value
    }
    let elapsed = (now_ms - clock.reported_at_ms).max(0);
    Some((at_report - elapsed).max(0)) // never below zero
}

/// "1:03:05" for an hour or more, "12:07" below that.
pub fn format_clock(ms: i64) -> String {
    let total = (ms as f64 / 1000.0).round() as i64;
    let (h, m, s) = (total / 3600, total % 3600 / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORTED: i64 = 1_792_000_000_000; // any moment will do
    fn clock(running: bool) -> SessionClock {
        SessionClock {
            remaining: "00:03:03".into(),
            reported_at_ms: REPORTED,
            running,
        }
    }

    #[test]
    fn a_running_clock_counts_down_from_when_it_was_reported() {
        let c = clock(true);
        assert_eq!(remaining_ms(Some(&c), REPORTED), Some(183_000));
        assert_eq!(remaining_ms(Some(&c), REPORTED + 60_000), Some(123_000));
        assert_eq!(format_clock(123_000), "2:03");
    }

    #[test]
    fn a_stopped_clock_holds_its_value() {
        let c = clock(false);
        assert_eq!(remaining_ms(Some(&c), REPORTED + 3_600_000), Some(183_000));
    }

    #[test]
    fn the_countdown_never_goes_below_zero() {
        let c = clock(true);
        assert_eq!(remaining_ms(Some(&c), REPORTED + 10_000_000), Some(0));
    }

    #[test]
    fn a_report_from_the_future_does_not_add_time() {
        let c = clock(true);
        assert_eq!(remaining_ms(Some(&c), REPORTED - 5_000), Some(183_000));
    }

    #[test]
    fn no_clock_or_no_time_means_none() {
        assert_eq!(remaining_ms(None, REPORTED), None);
        let empty = SessionClock {
            remaining: String::new(),
            reported_at_ms: REPORTED,
            running: true,
        };
        assert_eq!(remaining_ms(Some(&empty), REPORTED), None);
    }

    #[test]
    fn formats_hours_only_when_there_are_some() {
        assert_eq!(format_clock(3_785_000), "1:03:05");
        assert_eq!(format_clock(727_000), "12:07");
        assert_eq!(format_clock(5_000), "0:05");
        assert_eq!(format_clock(0), "0:00");
    }
}
