//! The window: Eli's Ghostty frame (`window-decoration = false`, `feature-inventory.md` §1) and
//! where it was last time.
//!
//! The window is borderless: AppKit draws no frame, no title bar and no traffic lights (not
//! parked off the window's edge, as Herdr app does: there are none), and a borderless window's
//! corners are square. It stays resizable. The tab strip and the sidebar's top row drag it
//! (`.window_drag()`), and only they do.
//!
//! Its size, position, display and maximized or full-screen state are saved in `window.json`
//! when it closes, when the app quits, when it loses focus and when its size changes, and it
//! opens there again. A saved place on a display that's gone opens centered on the main display
//! instead.

use std::cell::RefCell;
use std::rc::Rc;

use rune::{Chrome, Display, WindowHandle, WindowOptions, WindowPlacement};
use serde::{Deserialize, Serialize};

use crate::home::{Home, WINDOW};

/// The frame: borderless and resizable.
pub fn options(saved: Option<&Placement>) -> WindowOptions {
    let mut options = WindowOptions {
        borderless: true,
        resizable: true,
        ..WindowOptions::default()
    };
    if let Some(saved) = saved {
        options.position = Some(saved.position);
        options.physical_size = Some(saved.size);
    }
    options
}

/// No titlebar band and no traffic lights; only `.window_drag()` regions move the window.
pub const CHROME: Chrome = Chrome {
    full_size_content: false,
    traffic_lights: None,
    drag_regions: true,
};

/// Where the window was, in the units Rune opens a window with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    /// The frame's top-left (`WindowOptions::position`).
    pub position: (i32, i32),
    /// The content's size in physical pixels (`WindowOptions::physical_size`).
    pub size: (u32, u32),
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub full_screen: bool,
    /// The display it was on: its position and size.
    #[serde(default)]
    pub display: Option<((i32, i32), (u32, u32))>,
}

impl Placement {
    pub fn from_window(p: &WindowPlacement) -> Placement {
        Placement {
            position: p.position,
            size: p.content_size,
            maximized: p.maximized,
            full_screen: p.full_screen,
            display: p.display,
        }
    }

    pub fn from_json(text: &str) -> Option<Placement> {
        serde_json::from_str(text)
            .ok()
            .filter(|p: &Placement| p.size.0 > 0 && p.size.1 > 0)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a placement always serializes")
    }

    /// Whether its display is still connected, where it was. A placement that names none
    /// counts as on one.
    pub fn on(&self, displays: &[Display]) -> bool {
        self.display
            .is_none_or(|(at, size)| displays.iter().any(|d| d.position == at && d.size == size))
    }
}

/// The open window, once Rune made it, and where to save its place.
#[derive(Clone, Default)]
pub struct Keeper {
    handle: Rc<RefCell<Option<WindowHandle>>>,
    home: Option<Home>,
}

impl Keeper {
    pub fn new(home: Option<Home>) -> Keeper {
        Keeper {
            handle: Rc::default(),
            home,
        }
    }

    /// The saved placement, if there is a readable one.
    pub fn saved(&self) -> Option<Placement> {
        Placement::from_json(&self.home.as_ref()?.read(WINDOW)?)
    }

    /// The window was made (`App::on_window`). A saved place whose display is gone moves to the
    /// middle of the main display.
    pub fn created(&self, handle: &WindowHandle, saved: Option<&Placement>) {
        *self.handle.borrow_mut() = Some(handle.clone());
        if saved.is_some_and(|p| !p.on(&rune::displays())) {
            handle.center();
        }
    }

    /// The window's macOS CGWindowID (`screencapture -l`), once it's open.
    pub fn window_number(&self) -> Option<u32> {
        self.handle.borrow().as_ref()?.window_number()
    }

    /// Saves where the window is: in the background, or at once when `now` (closing, quitting).
    pub fn save(&self, now: bool) {
        let (Some(home), Some(handle)) = (&self.home, self.handle.borrow().clone()) else {
            return;
        };
        let Some(placement) = handle.placement() else {
            return;
        };
        if placement.minimized || placement.content_size.0 == 0 {
            return;
        }
        let text = Placement::from_window(&placement).to_json();
        if now {
            home.save_now(WINDOW, text);
        } else {
            home.save(WINDOW, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_placement_reads_back_and_knows_its_display() {
        let p = Placement {
            position: (100, 50),
            size: (2400, 1600),
            maximized: false,
            full_screen: false,
            display: Some(((0, 0), (3024, 1964))),
        };
        assert_eq!(Placement::from_json(&p.to_json()), Some(p.clone()));
        assert_eq!(Placement::from_json("{}"), None);
        assert_eq!(
            Placement::from_json(r#"{"position":[0,0],"size":[0,0]}"#),
            None
        );
        let display = |position, size| Display {
            id: None,
            name: None,
            position,
            size,
            scale: 2.0,
            primary: true,
        };
        assert!(p.on(&[display((0, 0), (3024, 1964))]));
        assert!(!p.on(&[display((0, 0), (1920, 1080))]));
        let options = options(Some(&p));
        assert!(options.borderless && options.resizable);
        assert_eq!(options.position, Some((100, 50)));
        assert_eq!(options.physical_size, Some((2400, 1600)));
    }
}
