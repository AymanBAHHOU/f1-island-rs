//! Is the Live tab the main one? It is from when the app connects (15 minutes before a
//! session) until a while after the session finishes; after that it moves to the end
//! of the tabs and the island opens on the Weekend again. (Port of `renderer/livetab.js`.)

const FIFTEEN_MINUTES_MS: i64 = 15 * 60 * 1000;

/// What live timing says a session is doing. An `enum` is a value that is exactly one
/// of a fixed list of cases, so the compiler can check we handled every one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    /// Not started yet (also the break between qualifying segments).
    Inactive,
    Started,
    /// Red flag.
    Aborted,
    Finished,
    Finalised,
    Ends,
    /// A status we don't know; ignored.
    Unknown,
}

impl SessionStatus {
    /// From the text in the feed ("Started", "Finished", ...).
    pub fn parse(text: &str) -> Self {
        match text {
            "Inactive" => Self::Inactive,
            "Started" => Self::Started,
            "Aborted" => Self::Aborted,
            "Finished" => Self::Finished,
            "Finalised" => Self::Finalised,
            "Ends" => Self::Ends,
            _ => Self::Unknown,
        }
    }

    pub fn is_running(self) -> bool {
        // A red flag (Aborted) is the session still being on, not over.
        matches!(self, Self::Started | Self::Aborted)
    }

    fn is_finished(self) -> bool {
        matches!(self, Self::Finished | Self::Finalised | Self::Ends)
    }
}

/// Remembers when the current session finished, to decide when Live stops being the main tab.
#[derive(Debug, Clone)]
pub struct LiveTab {
    after_ms: i64,
    saw_running: bool,
    finished_at_ms: Option<i64>,
}

impl Default for LiveTab {
    fn default() -> Self {
        Self::new(FIFTEEN_MINUTES_MS)
    }
}

impl LiveTab {
    /// `after_ms`: how long after the finish Live stays the main tab.
    pub fn new(after_ms: i64) -> Self {
        Self {
            after_ms,
            saw_running: false,
            finished_at_ms: None,
        }
    }

    pub fn reset(&mut self) {
        self.saw_running = false;
        self.finished_at_ms = None;
    }

    /// Feed it each live-timing status. `live` is `None` once disconnected.
    /// `scheduled_end_ms` is when the session was due to end: used if the app started
    /// after the session finished, so it never saw it run.
    pub fn update(
        &mut self,
        live: Option<SessionStatus>,
        now_ms: i64,
        scheduled_end_ms: Option<i64>,
    ) {
        let Some(status) = live else {
            self.reset();
            return;
        };

        if status.is_running() {
            self.saw_running = true;
            self.finished_at_ms = None;
        } else if status.is_finished() {
            // Only the first "finished" counts: Finalised after Finished doesn't restart the timer.
            if self.finished_at_ms.is_none() {
                self.finished_at_ms = Some(if self.saw_running {
                    now_ms
                } else {
                    scheduled_end_ms.unwrap_or(0)
                });
            }
        } else if status == SessionStatus::Inactive {
            self.reset(); // the next session (or a break between segments), not started yet
        }
    }

    pub fn is_main(&self, connected: bool, now_ms: i64) -> bool {
        // `is_none_or`: true if there is no finish time, otherwise check the closure.
        connected
            && self
                .finished_at_ms
                .is_none_or(|finished| now_ms - finished <= self.after_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SessionStatus::*;

    const MIN: i64 = 60_000;
    const T0: i64 = 1_792_000_000_000;

    #[test]
    fn main_before_the_session_while_it_runs_and_for_15_minutes_after() {
        let mut tab = LiveTab::default();
        tab.update(Some(Inactive), T0 - 10 * MIN, None);
        assert!(tab.is_main(true, T0 - 10 * MIN));
        tab.update(Some(Started), T0, None);
        assert!(tab.is_main(true, T0 + 30 * MIN));
        tab.update(Some(Finished), T0 + 60 * MIN, None);
        assert!(tab.is_main(true, T0 + 70 * MIN));
        assert!(tab.is_main(true, T0 + 75 * MIN)); // exactly 15 minutes
        assert!(!tab.is_main(true, T0 + 76 * MIN));
    }

    #[test]
    fn a_red_flag_is_not_a_finish() {
        let mut tab = LiveTab::default();
        tab.update(Some(Started), T0, None);
        tab.update(Some(Aborted), T0 + 20 * MIN, None);
        assert!(tab.is_main(true, T0 + 120 * MIN));
    }

    #[test]
    fn finalised_after_finished_keeps_the_first_finish_time() {
        let mut tab = LiveTab::default();
        tab.update(Some(Started), T0, None);
        tab.update(Some(Finished), T0 + 60 * MIN, None);
        tab.update(Some(Finalised), T0 + 80 * MIN, None);
        assert!(!tab.is_main(true, T0 + 80 * MIN)); // 20 minutes after the first
    }

    #[test]
    fn started_after_the_session_ended_uses_the_scheduled_end() {
        let mut late = LiveTab::default();
        late.update(Some(Finalised), T0 + 120 * MIN, Some(T0 + 60 * MIN)); // ended an hour ago
        assert!(!late.is_main(true, T0 + 120 * MIN));

        let mut fresh = LiveTab::default();
        fresh.update(Some(Finalised), T0 + 70 * MIN, Some(T0 + 60 * MIN)); // 10 minutes ago
        assert!(fresh.is_main(true, T0 + 70 * MIN));

        let mut unknown = LiveTab::default();
        unknown.update(Some(Finalised), T0, None); // no schedule: assume long ago
        assert!(!unknown.is_main(true, T0));
    }

    #[test]
    fn the_next_session_makes_it_main_again_and_disconnecting_resets() {
        let mut tab = LiveTab::default();
        tab.update(Some(Started), T0, None);
        tab.update(Some(Finished), T0 + 60 * MIN, None);
        assert!(!tab.is_main(true, T0 + 90 * MIN));
        tab.update(Some(Inactive), T0 + 95 * MIN, None); // e.g. Q2 after the Q1 break
        assert!(tab.is_main(true, T0 + 95 * MIN));
        tab.update(Some(Started), T0 + 100 * MIN, None);
        tab.update(Some(Finished), T0 + 130 * MIN, None);
        tab.update(None, T0 + 200 * MIN, None);
        assert!(tab.is_main(true, T0 + 200 * MIN)); // reset: no finish time any more
    }

    #[test]
    fn not_main_without_live_data() {
        assert!(!LiveTab::default().is_main(false, T0));
    }

    #[test]
    fn parses_the_feeds_status_text() {
        assert_eq!(SessionStatus::parse("Started"), Started);
        assert_eq!(SessionStatus::parse("Ends"), Ends);
        assert_eq!(SessionStatus::parse("something new"), Unknown);
    }
}
