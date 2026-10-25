//! Championship maths: points still available, who can still win, the earliest round the
//! leader can clinch, and title chances from a Monte Carlo simulation of the remaining
//! races based on recent form. (Port of the Electron app's `src/championship.js`, step for
//! step, including its seeded random numbers: the same inputs give the same chances.)
//!
//! The chances are an estimate: each driver gets a form score from recent results (newer
//! events weigh more), every remaining race is played out with a finishing order drawn in
//! proportion to exp(BETA × form) (a Plackett-Luce draw) plus a per-driver retirement
//! chance, and points are awarded. Upgrades, penalties, weather and team orders are
//! unknown to it.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};

const RACE_POINTS: [f64; 10] = [25.0, 18.0, 15.0, 12.0, 10.0, 8.0, 6.0, 4.0, 2.0, 1.0];
const SPRINT_POINTS: [f64; 8] = [8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0];

const FORM_DECAY: f64 = 0.8; // each older event counts 20% less
const BETA: f64 = 5.0; // how strongly form separates drivers
/// Pace isn't known exactly: each simulated season redraws every driver's form with this
/// much spread (tracks, upgrades). Without it the model is over-confident in close fights.
const FORM_SPREAD: f64 = 0.25;
/// Shrinks retirement rates towards ~8%.
const DNF_PRIOR: (f64, f64) = (1.0, 12.0);
pub const RUNS: u32 = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Race,
    Sprint,
}

impl Kind {
    fn table(self) -> &'static [f64] {
        match self {
            Kind::Race => &RACE_POINTS,
            Kind::Sprint => &SPRINT_POINTS,
        }
    }
    /// Most a single driver can score in one event of this kind.
    fn max_driver(self) -> f64 {
        match self {
            Kind::Race => 25.0,
            Kind::Sprint => 8.0,
        }
    }
    /// Most a two-car team can score.
    fn max_team(self) -> f64 {
        match self {
            Kind::Race => 25.0 + 18.0,
            Kind::Sprint => 8.0 + 7.0,
        }
    }
}

/// A race or sprint still to be scored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    #[serde(rename = "type")]
    pub kind: Kind,
    pub name: String,
    pub start: String,
}

type PerEvent = fn(Kind) -> f64;
const DRIVER_MAX: PerEvent = Kind::max_driver;
const TEAM_MAX: PerEvent = Kind::max_team;

/// A result from this season, to measure form.
#[derive(Debug, Clone, PartialEq)]
pub struct SeasonResult {
    pub driver: String,
    pub kind: Kind,
    pub round: u32,
    pub points: f64,
    pub position: u32,
    pub dnf: bool,
}

#[derive(Debug, Clone)]
pub struct DriverIn {
    pub code: String,
    pub points: f64,
    pub team: Option<String>,
    pub team_id: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TeamIn {
    pub name: String,
    pub points: f64,
    pub team_id: Option<String>,
    pub color: Option<String>,
}

fn available(events: &[Event], per_event: PerEvent) -> f64 {
    events.iter().map(|e| per_event(e.kind)).sum()
}

// ---------- exact maths ----------

/// Earliest event after which the leader can be champion (winning everything while the
/// runner-up scores nothing).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Clinch {
    /// It's already over.
    Decided { decided: bool },
    /// The leader needs `lead` points of lead after `event`.
    At {
        event: Event,
        index: usize,
        lead: f64,
    },
}

fn earliest_clinch(
    leader: f64,
    second: f64,
    events: &[Event],
    per_event: PerEvent,
) -> Option<Clinch> {
    let total = available(events, per_event);
    let lead = leader - second;
    if lead > total {
        return Some(Clinch::Decided { decided: true });
    }
    let mut gained = 0.0;
    for (index, event) in events.iter().enumerate() {
        gained += per_event(event.kind);
        let remaining = total - gained;
        if lead + gained > remaining {
            return Some(Clinch::At {
                event: event.clone(),
                index,
                lead: remaining,
            });
        }
    }
    None
}

struct Row {
    points: f64,
    gap: f64,
    max_possible: f64,
    contender: bool,
}

/// Gap to the leader, best possible total, and whether they can still win (a tie is still
/// possible on countback). Returned in the order of `sorted_points`, which is best first.
fn contention(sorted_points: &[f64], left: f64) -> Vec<Row> {
    let leader = sorted_points.first().copied().unwrap_or(0.0);
    sorted_points
        .iter()
        .map(|&points| Row {
            points,
            gap: leader - points,
            max_possible: points + left,
            contender: points + left >= leader,
        })
        .collect()
}

// ---------- form ----------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Form {
    pub form: f64,
    pub dnf_rate: f64,
    pub wins: f64,
}

fn event_key(h: &SeasonResult) -> String {
    format!(
        "{:03}-{}",
        h.round,
        if h.kind == Kind::Sprint { 0 } else { 1 }
    )
}

/// Each driver's recent pace (newer events weigh more), retirement rate and win count.
pub fn form_from_history(history: &[SeasonResult], decay: f64) -> HashMap<String, Form> {
    let events: BTreeSet<String> = history.iter().map(event_key).collect();
    let age: HashMap<&String, i32> = events
        .iter()
        .enumerate()
        .map(|(i, e)| (e, (events.len() - 1 - i) as i32))
        .collect();

    #[derive(Default)]
    struct Sum {
        sum: f64,
        weight: f64,
        dnfs: f64,
        starts: f64,
        wins: f64,
    }
    let mut out: HashMap<String, Sum> = HashMap::new();
    for h in history {
        let s = out.entry(h.driver.clone()).or_default();
        s.starts += 1.0;
        if h.dnf {
            s.dnfs += 1.0;
        } else {
            // Retirements count towards reliability, not pace.
            let w = decay.powi(age[&event_key(h)]);
            s.sum += w * (h.points / h.kind.max_driver()).min(1.0);
            s.weight += w;
        }
        if h.kind == Kind::Race && h.position == 1 {
            s.wins += 1.0;
        }
    }
    out.into_iter()
        .map(|(driver, s)| {
            (
                driver,
                Form {
                    form: if s.weight > 0.0 {
                        s.sum / s.weight
                    } else {
                        0.0
                    },
                    dnf_rate: (s.dnfs + DNF_PRIOR.0) / (s.starts + DNF_PRIOR.1),
                    wins: s.wins,
                },
            )
        })
        .collect()
}

// ---------- simulation ----------

/// Small seeded generator (mulberry32), so the same inputs always give the same chances.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b_79f5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }

    /// Standard normal draw (Box-Muller).
    fn gauss(&mut self) -> f64 {
        let a = self.next();
        let radius = (-2.0 * if a == 0.0 { 1e-12 } else { a }.ln()).sqrt();
        radius * (2.0 * std::f64::consts::PI * self.next()).cos()
    }
}

struct SimDriver {
    team: Option<String>,
    points: f64,
    form: Form,
}

/// Title chances of each driver and team (same order as the inputs).
struct Chances {
    drivers: Vec<f64>,
    teams: Vec<f64>,
}

fn simulate(
    drivers: &[SimDriver],
    team_names: &[String],
    team_points: &[f64],
    events: &[Event],
    runs: u32,
    seed: u32,
) -> Chances {
    let mut rand = Rng(seed);
    let n = drivers.len();
    let mut driver_titles = vec![0u32; n];
    let mut team_titles = vec![0u32; team_names.len()];
    let team_index: HashMap<&str, usize> = team_names
        .iter()
        .enumerate()
        .map(|(i, t)| (t.as_str(), i))
        .collect();

    let mut strength = vec![0.0; n];
    let mut keys = vec![0.0; n];
    let mut total = vec![0.0; n];
    let mut wins = vec![0.0; n];
    let mut team_total = vec![0.0; team_names.len()];
    let mut order: Vec<usize> = vec![0; n];

    for _ in 0..runs {
        for i in 0..n {
            strength[i] = BETA * (drivers[i].form.form + FORM_SPREAD * rand.gauss());
            total[i] = drivers[i].points;
            wins[i] = drivers[i].form.wins;
        }
        team_total.copy_from_slice(team_points);

        for event in events {
            // Gumbel-max trick: sorting by strength + Gumbel noise is a Plackett-Luce
            // draw. Retirements sink to the back.
            for i in 0..n {
                order[i] = i;
                keys[i] = if rand.next() < drivers[i].form.dnf_rate {
                    -1e9
                } else {
                    let u = rand.next();
                    strength[i] - (-(if u == 0.0 { 1e-12 } else { u }).ln()).ln()
                };
            }
            order.sort_by(|&a, &b| keys[b].partial_cmp(&keys[a]).unwrap_or(Ordering::Equal));
            let table = event.kind.table();
            for (p, &i) in order.iter().enumerate().take(table.len().min(n)) {
                if keys[i] <= -1e9 {
                    break; // retired: no points
                }
                total[i] += table[p];
                if let Some(&t) = drivers[i].team.as_deref().and_then(|t| team_index.get(t)) {
                    team_total[t] += table[p];
                }
            }
            if event.kind == Kind::Race && keys[order[0]] > -1e9 {
                wins[order[0]] += 1.0;
            }
        }

        // Champion: most points, then most wins (countback, simplified).
        let mut best = 0;
        for i in 1..n {
            if total[i] > total[best] || (total[i] == total[best] && wins[i] > wins[best]) {
                best = i;
            }
        }
        driver_titles[best] += 1;
        if !team_names.is_empty() {
            let mut best_team = 0;
            for t in 1..team_names.len() {
                if team_total[t] > team_total[best_team] {
                    best_team = t;
                }
            }
            team_titles[best_team] += 1;
        }
    }

    let share = |count: &Vec<u32>| {
        count
            .iter()
            .map(|&c| f64::from(c) / f64::from(runs))
            .collect()
    };
    Chances {
        drivers: share(&driver_titles),
        teams: share(&team_titles),
    }
}

// ---------- everything for the island ----------

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverFight {
    pub code: String,
    pub color: Option<String>,
    pub team_id: Option<String>,
    pub points: f64,
    pub gap: f64,
    pub max_possible: f64,
    pub contender: bool,
    pub chance: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamFight {
    pub name: String,
    pub color: Option<String>,
    pub team_id: Option<String>,
    pub points: f64,
    pub gap: f64,
    pub max_possible: f64,
    pub contender: bool,
    pub chance: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleFight {
    pub races: usize,
    pub sprints: usize,
    pub points_left: f64,
    pub team_points_left: f64,
    pub runs: u32,
    pub drivers: Vec<DriverFight>,
    pub teams: Vec<TeamFight>,
    pub clinch: Option<Clinch>,
    pub team_clinch: Option<Clinch>,
}

pub fn title_fight(
    drivers: &[DriverIn],
    teams: &[TeamIn],
    events: &[Event],
    history: &[SeasonResult],
    runs: u32,
) -> TitleFight {
    let form = form_from_history(history, FORM_DECAY);

    // Best first (ties keep their incoming order).
    let mut d_sorted: Vec<&DriverIn> = drivers.iter().collect();
    d_sorted.sort_by(|a, b| b.points.partial_cmp(&a.points).unwrap_or(Ordering::Equal));
    let mut t_sorted: Vec<&TeamIn> = teams.iter().collect();
    t_sorted.sort_by(|a, b| b.points.partial_cmp(&a.points).unwrap_or(Ordering::Equal));

    let d_rows = contention(
        &d_sorted.iter().map(|d| d.points).collect::<Vec<_>>(),
        available(events, DRIVER_MAX),
    );
    let t_rows = contention(
        &t_sorted.iter().map(|t| t.points).collect::<Vec<_>>(),
        available(events, TEAM_MAX),
    );

    let chances = (!events.is_empty() && !d_rows.is_empty()).then(|| {
        let sim_drivers: Vec<SimDriver> = d_sorted
            .iter()
            .map(|d| SimDriver {
                team: d.team.clone(),
                points: d.points,
                form: form.get(&d.code).copied().unwrap_or(Form {
                    form: 0.0,
                    dnf_rate: 0.1,
                    wins: 0.0,
                }),
            })
            .collect();
        let names: Vec<String> = t_sorted.iter().map(|t| t.name.clone()).collect();
        let points: Vec<f64> = t_sorted.iter().map(|t| t.points).collect();
        simulate(&sim_drivers, &names, &points, events, runs, 1)
    });

    // Out of contention means zero, whatever the simulation's rounding says.
    let chance = |contender: bool, gap: f64, simulated: Option<f64>| {
        if !contender {
            0.0
        } else if chances.is_some() {
            simulated.unwrap_or(0.0)
        } else if gap == 0.0 {
            1.0
        } else {
            0.0
        }
    };

    TitleFight {
        races: events.iter().filter(|e| e.kind == Kind::Race).count(),
        sprints: events.iter().filter(|e| e.kind == Kind::Sprint).count(),
        points_left: available(events, DRIVER_MAX),
        team_points_left: available(events, TEAM_MAX),
        runs: if chances.is_some() { runs } else { 0 },
        drivers: d_sorted
            .iter()
            .zip(&d_rows)
            .enumerate()
            .map(|(i, (d, row))| DriverFight {
                code: d.code.clone(),
                color: d.color.clone(),
                team_id: d.team_id.clone(),
                points: row.points,
                gap: row.gap,
                max_possible: row.max_possible,
                contender: row.contender,
                chance: chance(
                    row.contender,
                    row.gap,
                    chances.as_ref().map(|c| c.drivers[i]),
                ),
            })
            .collect(),
        teams: t_sorted
            .iter()
            .zip(&t_rows)
            .enumerate()
            .map(|(i, (t, row))| TeamFight {
                name: t.name.clone(),
                color: t.color.clone(),
                team_id: t.team_id.clone(),
                points: row.points,
                gap: row.gap,
                max_possible: row.max_possible,
                contender: row.contender,
                chance: chance(row.contender, row.gap, chances.as_ref().map(|c| c.teams[i])),
            })
            .collect(),
        clinch: if d_rows.len() > 1 {
            earliest_clinch(d_rows[0].points, d_rows[1].points, events, DRIVER_MAX)
        } else {
            None
        },
        team_clinch: if t_rows.len() > 1 {
            earliest_clinch(t_rows[0].points, t_rows[1].points, events, TEAM_MAX)
        } else {
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: Kind) -> Event {
        Event {
            kind,
            name: "x".into(),
            start: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn driver(code: &str, points: f64, team: &str) -> DriverIn {
        DriverIn {
            code: code.into(),
            points,
            team: Some(team.into()),
            team_id: None,
            color: None,
        }
    }

    #[test]
    fn the_generator_matches_the_electron_apps() {
        // First values of mulberry32(1), computed with the JavaScript original.
        let mut rand = Rng(1);
        let first: Vec<f64> = (0..3).map(|_| rand.next()).collect();
        assert!(
            (first[0] - 0.627_073_940_588_161_3).abs() < 1e-15,
            "{first:?}"
        );
        assert!(
            (first[1] - 0.002_735_721_180_215_478).abs() < 1e-15,
            "{first:?}"
        );
        assert!(
            (first[2] - 0.527_447_039_959_952_2).abs() < 1e-15,
            "{first:?}"
        );
    }

    #[test]
    fn points_left_count_races_and_sprints() {
        let events = [event(Kind::Race), event(Kind::Sprint)];
        assert_eq!(available(&events, DRIVER_MAX), 33.0);
        assert_eq!(available(&events, TEAM_MAX), 58.0);
    }

    #[test]
    fn a_big_enough_lead_is_decided() {
        let events = [event(Kind::Race)];
        assert_eq!(
            earliest_clinch(100.0, 50.0, &events, DRIVER_MAX),
            Some(Clinch::Decided { decided: true })
        );
    }

    #[test]
    fn the_leader_can_clinch_at_the_event_where_the_rest_cannot_catch_up() {
        let events = [event(Kind::Race), event(Kind::Race), event(Kind::Race)];
        // lead 30: after race 1 gains 25 => 55 > remaining 50 ✓.
        match earliest_clinch(130.0, 100.0, &events, DRIVER_MAX) {
            Some(Clinch::At { index, lead, .. }) => assert_eq!((index, lead), (0, 50.0)),
            other => panic!("{other:?}"),
        }
        assert_eq!(earliest_clinch(100.0, 100.0, &[], DRIVER_MAX), None); // nothing left to race
    }

    #[test]
    fn form_weighs_recent_events_and_ignores_retirements_for_pace() {
        let history = [
            SeasonResult {
                driver: "AAA".into(),
                kind: Kind::Race,
                round: 1,
                points: 0.0,
                position: 12,
                dnf: false,
            },
            SeasonResult {
                driver: "AAA".into(),
                kind: Kind::Race,
                round: 2,
                points: 25.0,
                position: 1,
                dnf: false,
            },
            SeasonResult {
                driver: "BBB".into(),
                kind: Kind::Race,
                round: 2,
                points: 0.0,
                position: 20,
                dnf: true,
            },
        ];
        let form = form_from_history(&history, 0.8);
        let a = form["AAA"];
        assert!((a.form - 1.0 / 1.8).abs() < 1e-12); // (0.8*0 + 1*1) / 1.8
        assert_eq!(a.wins, 1.0);
        assert!((a.dnf_rate - 1.0 / 14.0).abs() < 1e-12);
        assert_eq!(form["BBB"].form, 0.0);
        assert!(form["BBB"].dnf_rate > form["AAA"].dnf_rate);
    }

    #[test]
    fn nobody_can_win_from_beyond_the_points_left() {
        let drivers = [driver("AAA", 300.0, "T1"), driver("BBB", 100.0, "T2")];
        let teams = [
            TeamIn {
                name: "T1".into(),
                points: 400.0,
                team_id: None,
                color: None,
            },
            TeamIn {
                name: "T2".into(),
                points: 200.0,
                team_id: None,
                color: None,
            },
        ];
        let fight = title_fight(&drivers, &teams, &[event(Kind::Race)], &[], 200);
        assert!(fight.drivers[0].contender);
        assert!(!fight.drivers[1].contender);
        assert_eq!(fight.drivers[1].chance, 0.0);
        assert_eq!(fight.drivers[0].chance, 1.0);
        assert_eq!(fight.clinch, Some(Clinch::Decided { decided: true }));
    }

    #[test]
    fn without_events_the_leader_has_the_title() {
        let drivers = [driver("AAA", 10.0, "T"), driver("BBB", 9.0, "T")];
        let fight = title_fight(&drivers, &[], &[], &[], 200);
        assert_eq!(fight.runs, 0);
        assert_eq!(fight.drivers[0].chance, 1.0);
        assert_eq!(fight.drivers[1].chance, 0.0);
    }

    #[test]
    fn a_close_fight_splits_the_chances_and_they_add_up() {
        let drivers = [driver("AAA", 100.0, "T1"), driver("BBB", 98.0, "T2")];
        let events = [event(Kind::Race), event(Kind::Race)];
        let fight = title_fight(&drivers, &[], &events, &[], 2000);
        let (a, b) = (fight.drivers[0].chance, fight.drivers[1].chance);
        assert!(a > 0.2 && b > 0.2, "{a} {b}");
        assert!((a + b - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_chances_match_the_electron_apps_for_the_same_inputs() {
        // Expected values come from running src/championship.js on the same input.
        let drivers = [
            driver("AAA", 100.0, "T1"),
            driver("BBB", 98.0, "T2"),
            driver("CCC", 80.0, "T1"),
        ];
        let team = |name: &str, points| TeamIn {
            name: name.into(),
            points,
            team_id: None,
            color: None,
        };
        let teams = [team("T1", 180.0), team("T2", 98.0)];
        let events = [event(Kind::Race), event(Kind::Sprint), event(Kind::Race)];
        let h = |driver: &str, kind, round, points, position, dnf| SeasonResult {
            driver: driver.into(),
            kind,
            round,
            points,
            position,
            dnf,
        };
        let history = [
            h("AAA", Kind::Race, 1, 25.0, 1, false),
            h("BBB", Kind::Race, 1, 18.0, 2, false),
            h("CCC", Kind::Race, 1, 0.0, 15, true),
            h("AAA", Kind::Sprint, 2, 5.0, 4, false),
            h("BBB", Kind::Sprint, 2, 8.0, 1, false),
            h("BBB", Kind::Race, 2, 25.0, 1, false),
            h("AAA", Kind::Race, 2, 15.0, 3, false),
            h("CCC", Kind::Race, 2, 10.0, 5, false),
        ];
        let fight = title_fight(&drivers, &teams, &events, &history, 20_000);
        let chances: Vec<f64> = fight.drivers.iter().map(|d| d.chance).collect();
        assert_eq!(chances, vec![0.4459, 0.55075, 0.00335]);
        let team_chances: Vec<f64> = fight.teams.iter().map(|t| t.chance).collect();
        assert_eq!(team_chances, vec![1.0, 0.0]);
    }
}
