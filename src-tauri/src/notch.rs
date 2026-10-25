//! The camera notch on some Macs: the island can attach to it and hang from it.
//! (Port of the geometry in the Electron app's `src/notch.js`. Reading the notch from the
//! system needs a small macOS helper program, `notch-info`, which prints JSON.)

use crate::dock::Point;
use serde::{Deserialize, Serialize};

/// A notch as reported by the helper: in a screen's own coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Reported {
    pub id: i64,
    pub width: f64,
    pub height: f64,
    #[serde(rename = "centerX")]
    pub center_x: f64,
}

/// The screen a notch belongs to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Screen {
    pub id: i64,
    pub x: f64,
    pub y: f64,
    pub width: f64,
}

/// A notch in global screen coordinates, as the page receives it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notch {
    pub id: i64,
    pub width: f64,
    pub height: f64,
    pub center_x: f64,
    pub top: f64,
}

/// The notch of `screen`, if one was reported and its numbers make sense.
pub fn notch_for_screen(screen: Screen, reported: &[Reported]) -> Option<Notch> {
    let n = reported.iter().find(|n| n.id == screen.id)?;
    let finite = [n.width, n.height, n.center_x]
        .iter()
        .all(|v| v.is_finite());
    let sane = finite
        && n.width > 0.0
        && n.width < screen.width / 2.0
        && n.height > 0.0
        && n.height <= 100.0
        && n.center_x - n.width / 2.0 >= 0.0
        && n.center_x + n.width / 2.0 <= screen.width;
    sane.then_some(Notch {
        id: n.id,
        width: n.width,
        height: n.height,
        center_x: screen.x + n.center_x,
        top: screen.y,
    })
}

/// Window origin that puts the island under the notch.
pub fn notch_origin(notch: &Notch, window_width: f64, edge: f64) -> Point {
    Point {
        x: (notch.center_x - window_width / 2.0).round(),
        y: notch.top - edge,
    }
}

/// Is the cursor over the notch (with a few pixels around it)?
pub fn contains_notch(point: Point, notch: Option<&Notch>) -> bool {
    let Some(n) = notch else { return false };
    point.x >= n.center_x - n.width / 2.0 - 8.0
        && point.x <= n.center_x + n.width / 2.0 + 8.0
        && point.y >= n.top
        && point.y <= n.top + n.height + 8.0
}

/// The island's centre entering a forgiving target around the camera attaches it. Only on
/// drop: changing layouts while a pointer is captured causes jumps.
pub fn magnet_origin(origin: Point, notch: Option<&Notch>, width: f64, edge: f64) -> Option<Point> {
    let n = notch?;
    let home = notch_origin(n, width, edge);
    ((origin.x - home.x).abs() <= n.width / 2.0 + 24.0
        && (origin.y - home.y).abs() <= 72.0_f64.max(n.height + 32.0))
    .then_some(home)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Screen = Screen {
        id: 1,
        x: 0.0,
        y: 0.0,
        width: 1512.0,
    };

    fn reported() -> Reported {
        Reported {
            id: 1,
            width: 180.0,
            height: 32.0,
            center_x: 756.0,
        }
    }

    #[test]
    fn a_sensible_notch_is_found_in_global_coordinates() {
        let on_second = Screen {
            x: 1512.0,
            ..SCREEN
        };
        let n = notch_for_screen(on_second, &[reported()]).unwrap();
        assert_eq!(n.center_x, 1512.0 + 756.0);
        assert_eq!(n.top, 0.0);
        assert!(notch_for_screen(
            SCREEN,
            &[Reported {
                id: 9,
                ..reported()
            }]
        )
        .is_none());
    }

    #[test]
    fn nonsense_notches_are_ignored() {
        for bad in [
            Reported {
                width: 0.0,
                ..reported()
            },
            Reported {
                width: 800.0,
                ..reported()
            }, // wider than half the screen
            Reported {
                height: 300.0,
                ..reported()
            },
            Reported {
                center_x: 20.0,
                ..reported()
            }, // hangs off the left
            Reported {
                width: f64::NAN,
                ..reported()
            },
        ] {
            assert!(notch_for_screen(SCREEN, &[bad]).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn the_island_hangs_from_the_notch() {
        let n = notch_for_screen(SCREEN, &[reported()]).unwrap();
        let o = notch_origin(&n, 960.0, 32.0);
        assert_eq!((o.x, o.y), (276.0, -32.0));
        assert!(contains_notch(Point { x: 756.0, y: 10.0 }, Some(&n)));
        assert!(!contains_notch(Point { x: 756.0, y: 100.0 }, Some(&n)));
        assert!(!contains_notch(Point { x: 756.0, y: 10.0 }, None));
    }

    #[test]
    fn dropping_near_the_notch_attaches() {
        let n = notch_for_screen(SCREEN, &[reported()]).unwrap();
        let near = Point { x: 300.0, y: -20.0 };
        assert_eq!(
            magnet_origin(near, Some(&n), 960.0, 32.0),
            Some(Point { x: 276.0, y: -32.0 })
        );
        let far = Point { x: 900.0, y: 200.0 };
        assert_eq!(magnet_origin(far, Some(&n), 960.0, 32.0), None);
        assert_eq!(magnet_origin(near, None, 960.0, 32.0), None);
    }
}
