//! Where the island's window goes: top centre of a screen, clamped to the screen, snapped
//! to the dock when dropped near it, grown for the analysis view, slid into place.
//! (Port of the positioning half of the Electron app's `main.js`, as plain functions on
//! numbers, so it can be tested without a screen.)
//!
//! Everything is in physical pixels. The layout constants are in logical pixels (what the
//! page uses), so each function takes the screen's `scale` to turn them into pixels.

use crate::dock::{near_dock, Point};

/// A transparent margin around the island so its shadow and alert glow aren't clipped by
/// the window; it must match `--edge` in the page's CSS. The window sits `EDGE` above the
/// island, so it may start above the screen edge.
pub const EDGE: f64 = 32.0;
/// The window is wider than the island so the Live tab can open wide; the extra is
/// transparent and click-through.
pub const WIN_W: f64 = 960.0;
pub const WIN_H: f64 = 680.0 + EDGE;
const BASE_WIN_W: f64 = 520.0;
/// Positions behave as if the window were still `BASE_WIN_W` wide.
const SIDE_SLACK: f64 = (WIN_W - BASE_WIN_W) / 2.0;
/// Gap between the screen top and a floating island.
pub const TOP_GAP: f64 = 8.0;
/// The window while the analysis view is open ("big" mode).
pub const BIG_W: f64 = 1000.0;
pub const BIG_H: f64 = 740.0 + EDGE;
pub const SNAP_PX: f64 = 48.0;

/// A rectangle: a screen's usable area, or a window's bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Area {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Area {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }
}

/// The window's size on a screen with this scale.
pub fn window_size(scale: f64) -> (f64, f64) {
    ((WIN_W * scale).round(), (WIN_H * scale).round())
}

/// Window origin at the top centre of a screen. `docked`: flush with the top edge, instead
/// of floating `TOP_GAP` below it.
pub fn top_centre(work: Area, scale: f64, docked: bool) -> Point {
    let (w, _) = window_size(scale);
    Point {
        x: (work.x + (work.width - w) / 2.0).round(),
        y: work.y + if docked { 0.0 } else { TOP_GAP * scale } - EDGE * scale,
    }
}

/// Keeps the whole window on the screen. The transparent sides may hang past the screen,
/// so the island itself can go as far to the edge as it could with a narrower window.
pub fn clamp_to_screen(pos: Point, work: Area, scale: f64) -> Point {
    let (w, h) = window_size(scale);
    let slack = SIDE_SLACK * scale;
    Point {
        x: pos
            .x
            .max(work.x - slack)
            .min(work.right() - w + slack)
            .round(),
        y: pos
            .y
            .max(work.y - EDGE * scale)
            .min(work.bottom() - h)
            .round(),
    }
}

/// The screen whose usable area is nearest the window's centre-top point.
pub fn nearest_screen(screens: &[Area], pos: Point, scale_guess: f64) -> Option<usize> {
    let (w, _) = window_size(scale_guess);
    let p = Point {
        x: pos.x + w / 2.0,
        y: pos.y + EDGE * scale_guess,
    };
    screens
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| distance(**a, p).total_cmp(&distance(**b, p)))
        .map(|(i, _)| i)
}

/// 0 inside the area, else how far outside.
fn distance(a: Area, p: Point) -> f64 {
    let dx = (a.x - p.x).max(0.0).max(p.x - a.right());
    let dy = (a.y - p.y).max(0.0).max(p.y - a.bottom());
    dx.hypot(dy)
}

// ---------- dropping ----------

/// Where the island ends up when dropped, and whether it docked.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drop {
    pub pos: Point,
    /// Dropped near the top: it docks (and slides to the centre).
    pub dock_snap: bool,
    /// Dropped on its home spot (docked, or top centre).
    pub snapped: bool,
}

/// `pos` is the window origin after the drag, already clamped to the screen.
pub fn drop_island(pos: Point, work: Area, scale: f64) -> Drop {
    let snap = SNAP_PX * scale;
    let dock_home = top_centre(work, scale, true);
    let dock_snap = near_dock(pos, dock_home, snap);
    let home = if dock_snap {
        dock_home
    } else {
        top_centre(work, scale, false)
    };
    let snapped = dock_snap || ((pos.x - home.x).abs() < snap && pos.y - home.y < snap);
    Drop {
        pos: if snapped { home } else { pos },
        dock_snap,
        snapped,
    }
}

// ---------- big (analysis) mode ----------

/// The window while the analysis view is open, and what the page needs to know.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Big {
    pub bounds: Area,
    /// In CSS pixels.
    pub anchor_x: f64,
    pub width: f64,
    pub height: f64,
}

/// Grows the window so the island can expand into the analysis view. The island stays
/// where it is unless the screen edge forces the window to shift.
pub fn enter_big(normal: Area, work: Area, scale: f64) -> Big {
    let cx = normal.x + normal.width / 2.0;
    let width = (BIG_W * scale).min(work.width);
    let height = (BIG_H * scale).min(work.bottom() - normal.y);
    let x = (cx - width / 2.0)
        .max(work.x)
        .min(work.right() - width)
        .round();
    Big {
        bounds: Area::new(x, normal.y, width, height),
        anchor_x: (cx - x) / scale,
        width: width / scale,
        height: height / scale,
    }
}

/// Dropping the big island.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BigDrop {
    /// Where the big window goes.
    pub pos: Point,
    pub snapped: bool,
    /// Where the small island's window origin goes, once the big view closes.
    pub normal: Point,
    /// Where the small island sits inside the big window, in CSS pixels.
    pub anchor_x: f64,
}

pub fn drop_big(pos: Point, big: (f64, f64), work: Area, scale: f64) -> BigDrop {
    let (width, height) = big;
    let (win_w, _) = window_size(scale);
    let mut nx = pos.x.max(work.x).min(work.right() - width).round();
    let mut ny = pos
        .y
        .min(work.bottom() - height)
        .max(work.y - EDGE * scale)
        .round();
    // Dropped near the top centre, the island docks there.
    let home = top_centre(work, scale, true);
    let home_x = (home.x + win_w / 2.0 - width / 2.0).round();
    let snapped = near_dock(
        Point { x: nx, y: ny },
        Point {
            x: home_x,
            y: home.y,
        },
        SNAP_PX * scale,
    );
    if snapped {
        nx = home_x;
        ny = home.y;
    }
    let normal = clamp_to_screen(
        Point {
            x: (nx + width / 2.0 - win_w / 2.0).round(),
            y: ny,
        },
        work,
        scale,
    );
    BigDrop {
        pos: Point { x: nx, y: ny },
        snapped,
        normal,
        anchor_x: (normal.x + win_w / 2.0 - nx) / scale,
    }
}

// ---------- sliding ----------

/// How long a slide of `distance` pixels takes.
pub fn slide_ms(distance: f64, scale: f64) -> f64 {
    (200.0 + distance / scale * 0.25).min(450.0)
}

/// Position `elapsed_ms` into a slide from `from` to `to` (ease out), and whether it is over.
pub fn slide_step(from: Point, to: Point, elapsed_ms: f64, duration_ms: f64) -> (Point, bool) {
    let k = (elapsed_ms / duration_ms).clamp(0.0, 1.0);
    let e = 1.0 - (1.0 - k).powi(3);
    (
        Point {
            x: (from.x + (to.x - from.x) * e).round(),
            y: (from.y + (to.y - from.y) * e).round(),
        },
        k >= 1.0,
    )
}

// ---------- the pointer ----------

/// Is the cursor (screen pixels) over the island's rectangle (CSS pixels in the window)?
pub fn pointer_in_island(cursor: Point, window_origin: Point, scale: f64, island: Area) -> bool {
    let p = Point {
        x: (cursor.x - window_origin.x) / scale,
        y: (cursor.y - window_origin.y) / scale,
    };
    island.contains(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Area = Area {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1040.0,
    };

    #[test]
    fn the_island_sits_at_the_top_centre_with_a_gap_or_flush() {
        let floating = top_centre(WORK, 1.0, false);
        assert_eq!((floating.x, floating.y), (480.0, 8.0 - 32.0));
        let docked = top_centre(WORK, 1.0, true);
        assert_eq!((docked.x, docked.y), (480.0, -32.0));
        // On a 150% screen everything scales.
        let big = Area::new(0.0, 0.0, 2880.0, 1560.0);
        let p = top_centre(big, 1.5, false);
        assert_eq!((p.x, p.y), (720.0, (8.0 - 32.0) * 1.5));
    }

    #[test]
    fn the_window_stays_on_the_screen_but_its_transparent_sides_may_hang_off() {
        let p = clamp_to_screen(
            Point {
                x: -1000.0,
                y: -1000.0,
            },
            WORK,
            1.0,
        );
        assert_eq!((p.x, p.y), (-220.0, -32.0));
        let q = clamp_to_screen(
            Point {
                x: 5000.0,
                y: 5000.0,
            },
            WORK,
            1.0,
        );
        assert_eq!((q.x, q.y), (1920.0 - 960.0 + 220.0, 1040.0 - 712.0));
        let ok = clamp_to_screen(Point { x: 100.0, y: 100.0 }, WORK, 1.0);
        assert_eq!((ok.x, ok.y), (100.0, 100.0));
    }

    #[test]
    fn dropping_near_the_top_docks_and_far_below_floats() {
        let near = drop_island(Point { x: 100.0, y: 0.0 }, WORK, 1.0);
        assert!(near.dock_snap && near.snapped);
        assert_eq!((near.pos.x, near.pos.y), (480.0, -32.0));

        let far = drop_island(Point { x: 100.0, y: 300.0 }, WORK, 1.0);
        assert!(!far.dock_snap && !far.snapped);
        assert_eq!((far.pos.x, far.pos.y), (100.0, 300.0));
    }

    #[test]
    fn dropping_near_the_floating_spot_snaps_without_docking() {
        // dock home y = -32, floating home y = -24; 48 px reach: y = 40 is too far for the dock
        // (72 above) but x is on the centre and y is within reach of the floating spot? No:
        // 40 - (-24) = 64 > 48, so it floats where it is.
        let d = drop_island(Point { x: 480.0, y: 40.0 }, WORK, 1.0);
        assert!(!d.snapped);
        // y = 10: within 48 of the dock spot (-32): that is a dock.
        let d = drop_island(Point { x: 480.0, y: 10.0 }, WORK, 1.0);
        assert!(d.dock_snap);
    }

    #[test]
    fn the_big_window_grows_around_the_island() {
        let normal = Area::new(480.0, -24.0, 960.0, 712.0);
        let big = enter_big(normal, WORK, 1.0);
        assert_eq!(big.bounds.width, 1000.0);
        assert_eq!(big.bounds.x, 460.0);
        assert_eq!(big.anchor_x, 500.0); // the island's centre stays at the window's centre
        assert_eq!(big.height, 772.0);

        // At the screen's left edge the window can't centre: the anchor moves.
        let left = Area::new(-220.0, -24.0, 960.0, 712.0);
        let big = enter_big(left, WORK, 1.0);
        assert_eq!(big.bounds.x, 0.0);
        assert_eq!(big.anchor_x, 260.0);
    }

    #[test]
    fn the_big_window_is_never_larger_than_the_screen() {
        let small = Area::new(0.0, 0.0, 800.0, 600.0);
        let big = enter_big(Area::new(-80.0, 0.0, 960.0, 712.0), small, 1.0);
        assert_eq!(big.bounds.width, 800.0);
        assert_eq!(big.height, 600.0);
    }

    #[test]
    fn dropping_the_big_island_near_the_top_docks_it_and_reports_where_the_small_one_goes() {
        let d = drop_big(Point { x: 200.0, y: 5.0 }, (1000.0, 772.0), WORK, 1.0);
        assert!(d.snapped);
        assert_eq!((d.pos.x, d.pos.y), (460.0, -32.0));
        assert_eq!((d.normal.x, d.normal.y), (480.0, -32.0));
        assert_eq!(d.anchor_x, 500.0);

        let far = drop_big(Point { x: 300.0, y: 200.0 }, (1000.0, 772.0), WORK, 1.0);
        assert!(!far.snapped);
        assert_eq!((far.pos.x, far.pos.y), (300.0, 200.0));
        // the small island's window is centred under the big one's centre
        assert_eq!(far.normal.x, 300.0 + 500.0 - 480.0);
        assert_eq!(far.anchor_x, 500.0);
    }

    #[test]
    fn a_slide_eases_out_and_ends_on_target() {
        let from = Point { x: 0.0, y: 0.0 };
        let to = Point { x: 1000.0, y: 0.0 };
        let (mid, done) = slide_step(from, to, 100.0, 400.0);
        assert!(!done);
        assert!(mid.x > 250.0, "ease out is ahead of linear: {}", mid.x); // 1-(0.75)^3 = 0.578
        let (end, done) = slide_step(from, to, 400.0, 400.0);
        assert!(done);
        assert_eq!(end, to);
        assert_eq!(slide_ms(0.0, 1.0), 200.0);
        assert_eq!(slide_ms(10_000.0, 1.0), 450.0);
    }

    #[test]
    fn the_pointer_is_checked_in_the_pages_pixels() {
        let island = Area::new(330.0, 32.0, 300.0, 38.0);
        let origin = Point { x: 480.0, y: -32.0 };
        // cursor at screen (480 + 400, -32 + 50) on a 100% screen: inside
        assert!(pointer_in_island(
            Point { x: 880.0, y: 18.0 },
            origin,
            1.0,
            island
        ));
        assert!(!pointer_in_island(
            Point { x: 880.0, y: 300.0 },
            origin,
            1.0,
            island
        ));
        // on a 200% screen, the same CSS point is twice as far in pixels
        let origin2 = Point { x: 960.0, y: -64.0 };
        assert!(pointer_in_island(
            Point { x: 1760.0, y: 36.0 },
            origin2,
            2.0,
            island
        ));
    }

    #[test]
    fn the_nearest_screen_is_found_by_the_islands_centre() {
        let left = Area::new(0.0, 0.0, 1920.0, 1040.0);
        let right = Area::new(1920.0, 0.0, 1920.0, 1040.0);
        let screens = [left, right];
        // window origin x=1500: centre x = 1980 -> on the right screen
        assert_eq!(
            nearest_screen(&screens, Point { x: 1500.0, y: 0.0 }, 1.0),
            Some(1)
        );
        assert_eq!(
            nearest_screen(&screens, Point { x: 100.0, y: 0.0 }, 1.0),
            Some(0)
        );
        assert_eq!(nearest_screen(&[], Point { x: 0.0, y: 0.0 }, 1.0), None);
    }
}
