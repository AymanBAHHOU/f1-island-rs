//! Docking: dropping the island near the top edge of a display, anywhere along it,
//! docks it there. (Port of `src/dock.js`.)

/// A window origin, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// Is `pos` close enough to the docked spot `home` to dock?
///
/// Only the height matters: anything above the spot counts as near (the window is
/// clamped to the screen top anyway), and below it must be within `snap_px`.
pub fn near_dock(pos: Point, home: Point, snap_px: f64) -> bool {
    pos.y - home.y < snap_px
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: Point = Point { x: 700.0, y: -32.0 };
    const SNAP: f64 = 48.0;

    fn at(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    #[test]
    fn docks_at_the_spot() {
        assert!(near_dock(at(700.0, -32.0), HOME, SNAP));
    }

    #[test]
    fn docks_anywhere_along_the_top_edge() {
        assert!(near_dock(at(0.0, -32.0), HOME, SNAP));
        assert!(near_dock(at(1400.0, -20.0), HOME, SNAP));
        assert!(near_dock(at(100.0, -32.0 + 40.0), HOME, SNAP));
    }

    #[test]
    fn does_not_dock_when_too_far_down() {
        assert!(!near_dock(at(700.0, -32.0 + 48.0), HOME, SNAP));
        assert!(!near_dock(at(700.0, 300.0), HOME, SNAP));
    }

    #[test]
    fn above_the_spot_counts_as_near() {
        assert!(near_dock(at(700.0, -200.0), HOME, SNAP));
    }
}
