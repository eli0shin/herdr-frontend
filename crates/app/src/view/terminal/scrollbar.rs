//! A terminal pane's scrollbar (`pane_scrollbars`), as Herdr app draws and drags it
//! (`app:src/terminal.rs` `scrollbar_thumb`): a lane from the grid's right edge to the pane's (the
//! last 8 points when the grid fills the pane, so scrollback showing up never reflows the
//! terminal), a 1-point rail and a 4-point thumb, hidden on the alternate screen, below five
//! columns and with no scrollback. Dragging the thumb, or pressing the lane, scrolls there.

/// The thumb's width, and its gap from the pane's right edge, in points.
pub const THUMB: f32 = 4.0;
pub const INSET: f32 = 2.0;
/// The narrowest lane, in points.
pub const LANE: f32 = 8.0;

/// The thumb's top and length in a `track`-point lane: the scrollable area is `total` rows, and
/// the `len`-row viewport sits `offset` rows from its top. `None` when nothing scrolls. `min`
/// keeps a huge scrollback's thumb big enough to grab.
pub fn thumb(total: u64, offset: u64, len: u64, track: f32, min: f32) -> Option<(f32, f32)> {
    if total <= len || len == 0 || track <= 0.0 {
        return None;
    }
    let thumb = (track * len as f32 / total as f32).clamp(min.min(track), track);
    let travel = track - thumb;
    let top = if travel <= 0.0 {
        0.0
    } else {
        offset.min(total - len) as f32 / (total - len) as f32 * travel
    };
    Some((top, thumb))
}

/// Where in the thumb a press at `y` holds it: where it pressed, or its middle for a press in
/// the lane, which brings the thumb there.
pub fn grab_at(top: f32, len: f32, y: f32) -> f32 {
    if y >= top && y < top + len {
        y - top
    } else {
        len / 2.0
    }
}

/// The first row the viewport shows with the thumb's top at `top`.
pub fn row_at(total: u64, len: u64, track: f32, min: f32, top: f32) -> u64 {
    let Some((_, thumb)) = thumb(total, 0, len, track, min) else {
        return 0;
    };
    let travel = track - thumb;
    if travel <= 0.0 {
        return 0;
    }
    (top.clamp(0.0, travel) / travel * (total - len) as f32).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thumb_is_the_viewports_share_of_the_track() {
        assert_eq!(thumb(24, 0, 24, 480.0, 20.0), None, "nothing to scroll");
        assert_eq!(thumb(48, 24, 24, 480.0, 20.0), Some((240.0, 240.0)));
        assert_eq!(thumb(48, 0, 24, 480.0, 20.0), Some((0.0, 240.0)));
        // A long scrollback keeps a thumb of at least `min`.
        let (top, len) = thumb(100_000, 100_000 - 24, 24, 480.0, 20.0).unwrap();
        assert_eq!((top, len), (460.0, 20.0));
    }

    #[test]
    fn dragging_maps_the_thumb_back_to_rows() {
        assert_eq!(row_at(48, 24, 480.0, 20.0, 0.0), 0);
        assert_eq!(row_at(48, 24, 480.0, 20.0, 240.0), 24);
        assert_eq!(row_at(48, 24, 480.0, 20.0, 120.0), 12);
        assert_eq!(row_at(48, 24, 480.0, 20.0, 900.0), 24, "clamped");
        assert_eq!(grab_at(100.0, 50.0, 120.0), 20.0);
        assert_eq!(grab_at(100.0, 50.0, 10.0), 25.0, "outside: the middle");
    }
}
