//! The window: Eli's Ghostty frame (`window-decoration = false`, `feature-inventory.md` §1) and
//! where it was last time.
//!
//! The window is borderless: AppKit draws no frame, no title bar and no traffic lights (not
//! parked off the window's edge, as Herdr app does: there are none), and a borderless window's
//! corners are square. It stays resizable. The tab strip and the sidebar's top row drag it
//! (`.window_drag()`), and only they do. A double click there does nothing ([`no_zoom`]).
//!
//! A tiling window manager (AeroSpace) places and sizes it. AeroSpace floats a window with no
//! full-screen button, as a borderless one is, so it tiles this one only with an
//! `on-window-detected` rule for `dev.herdr.frontend` (README "With AeroSpace"). Its size and
//! position are saved in
//! `window.json` when it closes and when the app quits, not while it runs: in a run they are
//! the tiler's, down to the corner AeroSpace parks the windows of a hidden workspace in. It opens
//! there again, unless that place is mostly off every connected display (a display that's gone,
//! or that parking spot); then it opens centered on the main display. It never reopens zoomed
//! or full screen.

use std::cell::RefCell;
use std::rc::Rc;

use rune::{Chrome, Display, WindowHandle, WindowOptions};
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

/// Where the window was, in the units Rune opens a window with. A file from an older version
/// also has `maximized`, `full_screen` and `display`, which are ignored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    /// The frame's top-left (`WindowOptions::position`).
    pub position: (i32, i32),
    /// The content's size in physical pixels (`WindowOptions::physical_size`), which for a
    /// borderless window is the frame's.
    pub size: (u32, u32),
}

impl Placement {
    pub fn from_json(text: &str) -> Option<Placement> {
        serde_json::from_str(text)
            .ok()
            .filter(|p: &Placement| p.size.0 > 0 && p.size.1 > 0)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a placement always serializes")
    }

    /// Whether at least half of the window is on one of `displays`. Not when the display it
    /// was on is gone, nor in the corner AeroSpace parks a hidden workspace's windows in, where
    /// a pixel of the window is left on the display.
    pub fn on(&self, displays: &[Display]) -> bool {
        let area = |w: i64, h: i64| w.max(0) * h.max(0);
        let (x, y) = (i64::from(self.position.0), i64::from(self.position.1));
        let (w, h) = (i64::from(self.size.0), i64::from(self.size.1));
        displays.iter().any(|d| {
            let (dx, dy) = (i64::from(d.position.0), i64::from(d.position.1));
            let (dw, dh) = (i64::from(d.size.0), i64::from(d.size.1));
            let shown = area(
                (x + w).min(dx + dw) - x.max(dx),
                (y + h).min(dy + dh) - y.max(dy),
            );
            2 * shown >= area(w, h)
        })
    }
}

/// Makes a double click on the drag rows do nothing, as Ghostty's borderless window has no
/// title bar to double-click. Rune carries out that double click as System Settings' "Double-click
/// a window's title bar to" says (`AppleActionOnDoubleClick`); unset, it zooms the window to the
/// display's visible frame, which takes no account of a tiling window manager's layout or bar.
/// This sets the app's default in memory only (`NSUserDefaults` `registerDefaults:`), so a
/// value set in System Settings still wins and nothing is written to disk.
#[cfg(target_os = "macos")]
pub fn no_zoom() {
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSDictionary, NSString, NSUserDefaults};

    let key = NSString::from_str("AppleActionOnDoubleClick");
    let none = NSString::from_str("None");
    let defaults = NSDictionary::<NSString, AnyObject>::from_slices(&[&*key], &[none.as_ref()]);
    // SAFETY: the dictionary's keys and values are strings, which property lists hold.
    unsafe { NSUserDefaults::standardUserDefaults().registerDefaults(&defaults) };
}

/// Elsewhere Rune maximizes on a double click; herdr-frontend runs on macOS only.
#[cfg(not(target_os = "macos"))]
pub fn no_zoom() {}

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

    /// The window was made (`App::on_window`). A saved place that's mostly off every display
    /// moves to the middle of the main display.
    pub fn created(&self, handle: &WindowHandle, saved: Option<&Placement>) {
        *self.handle.borrow_mut() = Some(handle.clone());
        if saved.is_some_and(|p| !p.on(&rune::displays())) {
            handle.center();
        }
    }

    /// Brings the window forward and activates the app, restoring it if minimized (a bare
    /// launch while it runs, `control::raise_running`).
    pub fn raise(&self) {
        if let Some(handle) = self.handle.borrow().as_ref() {
            handle.focus();
        }
    }

    /// The window's macOS CGWindowID (`screencapture -l`), once it's open.
    pub fn window_number(&self) -> Option<u32> {
        self.handle.borrow().as_ref()?.window_number()
    }

    /// Saves where the window is, at once: it's closing, or the app is quitting.
    ///
    /// Only then: `WindowHandle::placement` asks winit whether the window is maximized, and for
    /// a borderless window winit finds out by giving it a title bar and taking it away again,
    /// which rebuilds AppKit's title bar views each time (half the UI thread's time while
    /// AeroSpace resized the window, when this ran on every size change).
    pub fn save(&self) {
        let (Some(home), Some(handle)) = (&self.home, self.handle.borrow().clone()) else {
            return;
        };
        let Some(placement) = handle.placement() else {
            return;
        };
        if placement.minimized || placement.full_screen || placement.content_size.0 == 0 {
            return;
        }
        let saved = Placement {
            position: placement.position,
            size: placement.content_size,
        };
        home.save_now(WINDOW, saved.to_json());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(position: (i32, i32), size: (u32, u32)) -> Display {
        Display {
            id: None,
            name: None,
            position,
            size,
            scale: 2.0,
            primary: true,
        }
    }

    #[test]
    fn a_placement_reads_back_and_opens_the_window_there() {
        let p = Placement {
            position: (100, 50),
            size: (2400, 1600),
        };
        assert_eq!(Placement::from_json(&p.to_json()), Some(p.clone()));
        assert_eq!(Placement::from_json("{}"), None);
        assert_eq!(
            Placement::from_json(r#"{"position":[0,0],"size":[0,0]}"#),
            None
        );
        let options = options(Some(&p));
        assert!(options.borderless && options.resizable);
        assert_eq!(options.position, Some((100, 50)));
        assert_eq!(options.physical_size, Some((2400, 1600)));
    }

    #[test]
    fn an_older_file_reopens_neither_zoomed_nor_full_screen() {
        let old = r#"{"position": [0, 0], "size": [5120, 2880], "maximized": true,
            "full_screen": true, "display": [[0, 0], [5120, 2880]]}"#;
        assert_eq!(
            Placement::from_json(old),
            Some(Placement {
                position: (0, 0),
                size: (5120, 2880)
            })
        );
        assert!(
            !Placement::from_json(old)
                .unwrap()
                .to_json()
                .contains("maximized")
        );
    }

    /// What Rune reads for the double click: the app's default, unless System Settings set one.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_double_click_on_the_drag_rows_does_nothing_unless_system_settings_say_otherwise() {
        use objc2_foundation::{NSString, NSUserDefaults};

        let key = NSString::from_str("AppleActionOnDoubleClick");
        let defaults = NSUserDefaults::standardUserDefaults();
        let global = defaults
            .persistentDomainForName(&NSString::from_str("NSGlobalDomain"))
            .and_then(|d| d.objectForKey(&key))
            .and_then(|v| v.downcast::<NSString>().ok())
            .map(|v| v.to_string());
        no_zoom();
        let action = defaults.stringForKey(&key).map(|v| v.to_string());
        assert_eq!(action, Some(global.unwrap_or_else(|| "None".into())));
    }

    #[test]
    fn a_place_mostly_off_every_display_is_not_reopened() {
        // Eli's 5K display, and a smaller one to its right.
        let both = [
            display((0, 0), (5120, 2880)),
            display((5120, 0), (1280, 800)),
        ];
        let five_k = &both[..1];
        let tiled = Placement {
            position: (0, 64),
            size: (5120, 2816),
        };
        assert!(tiled.on(five_k));
        // Eli's window.json after AeroSpace hid its workspace: one pixel row and column left.
        let parked = Placement {
            position: (-5078, 2878),
            size: (5080, 2816),
        };
        assert!(!parked.on(&both));
        // The display it was on is gone.
        let on_side = Placement {
            position: (5200, 40),
            size: (1000, 600),
        };
        assert!(on_side.on(&both));
        assert!(!on_side.on(five_k));
        // Dragged half off an edge still reopens there.
        let half = Placement {
            position: (4120, 0),
            size: (2000, 1000),
        };
        assert!(half.on(five_k));
    }
}
