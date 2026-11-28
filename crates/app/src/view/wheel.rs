//! A mouse wheel's notches move Rune's own scroll viewports (the Agents list, the keys help, the
//! tab strip's pills) in the frame they come (no animations, `AGENTS.md`). Rune eases each notch
//! there over 100 to 200 ms, as Chromium does, and the one switch for that,
//! `Runtime::set_wheel_glide`, is out of an `App`'s reach at the pinned rev
//! (`docs/research/rune.md` "No animations"). So each viewport takes its notches itself, before
//! Rune would, and moves by the same distance with an instant scroll, from where it last reported
//! being. A trackpad's precise deltas and their momentum go on to Rune, which follows them 1:1;
//! the terminal panes and the popup lists take the wheel in their own handlers.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rune::{El, ScrollPos, Tx, WheelEvent};

use crate::State;

/// The way a viewport scrolls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

/// Where each viewport is scrolled to, by key, as it last reported it.
#[derive(Clone, Default)]
pub struct Offsets(Rc<RefCell<HashMap<&'static str, ScrollPos>>>);

impl Offsets {
    /// A wheel event over the viewport keyed `key`, which scrolls along `axis`: a notch along
    /// it scrolls it there now, and is taken. Anything else is left to whoever is next.
    pub fn wheel(&self, key: &'static str, axis: Axis, e: &WheelEvent, tx: &mut Tx) -> bool {
        let delta = match axis {
            Axis::X => e.dx,
            Axis::Y => e.dy,
        };
        if !e.notch || e.precise || e.touch || delta == 0.0 {
            return false;
        }
        let mut offsets = self.0.borrow_mut();
        let Some(at) = offsets.get_mut(key) else {
            return false;
        };
        // A positive delta moves the content right or down: towards the start.
        let i = usize::from(axis == Axis::Y);
        at.offset[i] = (at.offset[i] - delta).clamp(0.0, at.max[i].max(0.0));
        let [x, y] = at.offset;
        tx.scroll_to_instant(key, x, y);
        true
    }
}

/// `viewport`, keyed `key` and scrolling along `axis`, moved at once by a wheel's notches.
pub fn notches_at_once(viewport: El, key: &'static str, axis: Axis, s: &State) -> El {
    let seen = s.wheel.clone();
    viewport
        .on_scroll_offset(move |at: ScrollPos| {
            seen.0.borrow_mut().insert(key, at);
        })
        .on_wheel(move |s: &mut State, e: WheelEvent, tx| s.wheel.wheel(key, axis, &e, tx))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notch(dx: f32, dy: f32) -> WheelEvent {
        let mut e = WheelEvent::default();
        (e.dx, e.dy, e.notch) = (dx, dy, true);
        e
    }

    fn at(y: f32, max: f32) -> Offsets {
        let mut pos = ScrollPos::default();
        (pos.offset, pos.max) = ([0.0, y], [0.0, max]);
        let offsets = Offsets::default();
        offsets.0.borrow_mut().insert("list", pos);
        offsets
    }

    #[test]
    fn a_notch_moves_the_viewport_by_its_delta_within_its_ends() {
        let offsets = at(100.0, 300.0);
        let mut tx = Tx::default();
        assert!(offsets.wheel("list", Axis::Y, &notch(0.0, -40.0), &mut tx));
        assert_eq!(offsets.0.borrow()["list"].offset, [0.0, 140.0], "down");
        assert!(offsets.wheel("list", Axis::Y, &notch(0.0, 400.0), &mut tx));
        assert_eq!(
            offsets.0.borrow()["list"].offset,
            [0.0, 0.0],
            "back to the top, no further"
        );
        assert!(offsets.wheel("list", Axis::Y, &notch(0.0, -900.0), &mut tx));
        assert_eq!(
            offsets.0.borrow()["list"].offset,
            [0.0, 300.0],
            "to the end, no further"
        );
    }

    #[test]
    fn a_trackpad_another_axis_or_an_unseen_viewport_is_left_to_rune() {
        let offsets = at(100.0, 300.0);
        let mut tx = Tx::default();
        let mut precise = notch(0.0, -40.0);
        precise.precise = true;
        assert!(
            !offsets.wheel("list", Axis::Y, &precise, &mut tx),
            "trackpad"
        );
        let mut free = notch(0.0, -40.0);
        free.notch = false;
        assert!(
            !offsets.wheel("list", Axis::Y, &free, &mut tx),
            "not a notch"
        );
        assert!(
            !offsets.wheel("list", Axis::X, &notch(0.0, -40.0), &mut tx),
            "vertical, on a row"
        );
        assert!(
            !offsets.wheel("other", Axis::Y, &notch(0.0, -40.0), &mut tx),
            "never laid out"
        );
        assert_eq!(offsets.0.borrow()["list"].offset, [0.0, 100.0], "unmoved");
    }
}
