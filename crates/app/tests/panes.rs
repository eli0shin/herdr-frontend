//! Rearranging and resizing panes with the mouse, on the demo core through Rune's real runtime
//! with no window: a divider's handle drags the split (and a cancelled drag puts it back), a
//! pane's top-edge grip drops it onto another pane, and a zoomed tab's pill says " Z".

use herdr_frontend::fixtures::demo_core;
use herdr_frontend::link::Fake;
use herdr_frontend::{SIZE, State, app};
use herdr_shared::{Document, Size};
use rune::{ElementInfo, Env, Key, KeyEvent, Mods, Runtime};

fn runtime() -> (Runtime<State>, Fake) {
    let fake = demo_core();
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.connect(Box::new(fake.clone()));
    rt.state.settle();
    frames(&mut rt, 3);
    (rt, fake)
}

fn frames(rt: &mut Runtime<State>, n: usize) {
    for _ in 0..n {
        rt.invalidate();
        rt.frame(1.0 / 60.0);
    }
}

/// An element on screen (not one playing its exit transition).
fn find(rt: &Runtime<State>, key: &str) -> Option<ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn element(rt: &Runtime<State>, key: &str) -> ElementInfo {
    find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}"))
}

/// Equal to within the rounding of drawing at a zoom.
fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.05
}

fn near_rect(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(a, b)| near(*a, b))
}

fn center(e: &ElementInfo) -> (f32, f32) {
    (e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0)
}

fn doc(fake: &Fake) -> Document {
    Document::from_value(fake.core().docs[&None].0.clone()).expect("a document")
}

fn size_of(fake: &Fake, id: &str) -> Option<Size> {
    doc(fake)
        .tab("tb-work")
        .unwrap()
        .root
        .find(id)
        .unwrap()
        .size()
}

/// The pointer moves to `to` in a few steps, with frames between, as a hand does.
fn glide(rt: &mut Runtime<State>, from: (f32, f32), to: (f32, f32)) {
    for i in 1..=4 {
        let t = i as f32 / 4.0;
        rt.pointer_move(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        rt.frame(1.0 / 60.0);
    }
}

#[test]
fn dragging_a_divider_resizes_the_panes_beside_it_and_writes_the_sizes() {
    let (mut rt, fake) = runtime();
    let before = element(&rt, "pane-pn-1").rect;
    let handle = element(&rt, "divider-tb-work-0");
    // A 6 pt handle centered on the 1 pt gap after pn-1.
    assert!(near(handle.rect[2], 6.0), "{:?}", handle.rect);
    assert!(near(handle.rect[0] + 3.0, before[0] + before[2] + 0.5));
    let from = center(&handle);
    rt.pointer_down(from.0, from.1);
    rt.frame(1.0 / 60.0);
    glide(&mut rt, from, (from.0 + 100.0, from.1));
    rt.pointer_up(from.0 + 100.0, from.1);
    frames(&mut rt, 3);
    rt.state.settle();
    frames(&mut rt, 2);

    let after = element(&rt, "pane-pn-1").rect;
    assert!(
        (after[2] - (before[2] + 100.0)).abs() <= 1.0,
        "{before:?} → {after:?}"
    );
    let (Some(Size::Weight(a)), Some(Size::Weight(b))) =
        (size_of(&fake, "pn-1"), size_of(&fake, "c-1"))
    else {
        panic!("weights written");
    };
    assert!(a > b, "{a} {b}");
    assert!(((a + b) - 2.0).abs() < 1e-4, "the two share their weight");
}

#[test]
fn a_cancelled_divider_drag_puts_the_sizes_back() {
    let (mut rt, fake) = runtime();
    let before = element(&rt, "pane-pn-2").rect;
    let from = center(&element(&rt, "divider-c-1-0"));
    rt.pointer_down(from.0, from.1);
    rt.frame(1.0 / 60.0);
    glide(&mut rt, from, (from.0, from.1 + 120.0));
    rt.state.settle();
    assert!(element(&rt, "pane-pn-2").rect[3] > before[3] + 100.0);
    rt.cancel_pointer();
    frames(&mut rt, 2);
    rt.state.settle();
    frames(&mut rt, 2);
    // Where it goes back to (Rune eases a cancelled drag back into place).
    let back = element(&rt, "pane-pn-2").target;
    assert!(near_rect(back, before), "{back:?} {before:?}");
    assert_eq!(size_of(&fake, "pn-2"), Some(Size::Weight(1.0)));

    // Cancelled before it moved, or a click: nothing was written, so nothing is.
    let writes = fake.core().writes;
    let from = center(&element(&rt, "divider-tb-work-0"));
    rt.pointer_down(from.0, from.1);
    rt.frame(1.0 / 60.0);
    rt.cancel_pointer();
    frames(&mut rt, 2);
    rt.pointer_down(from.0, from.1);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(from.0, from.1);
    frames(&mut rt, 2);
    rt.state.settle();
    assert_eq!(fake.core().writes, writes);
    assert_eq!(size_of(&fake, "pn-1"), None);
}

#[test]
fn a_divider_drag_on_a_layout_that_cant_take_it_says_so_once_and_does_nothing() {
    let (mut rt, fake) = runtime();
    // The connection comes back while the machine's layout can't be read: the last one read
    // stays shown, but it takes no edits.
    fake.core().docs_down = true;
    fake.drop_connection();
    frames(&mut rt, 3);
    rt.state.settle();
    assert!(
        rt.state.model.shown_tab().is_some(),
        "the last layout stays shown"
    );
    let toasts = |rt: &Runtime<State>| {
        rt.state
            .toasts
            .shown
            .iter()
            .filter(|t| t.toast.title == "Layout unavailable")
            .count()
    };
    let before_toasts = toasts(&rt);
    let writes = fake.core().writes;
    let pane = element(&rt, "pane-pn-1").rect;

    let from = center(&element(&rt, "divider-tb-work-0"));
    rt.pointer_down(from.0, from.1);
    rt.frame(1.0 / 60.0);
    glide(&mut rt, from, (from.0 + 100.0, from.1));
    rt.pointer_up(from.0 + 100.0, from.1);
    frames(&mut rt, 2);
    rt.state.settle();

    assert_eq!(toasts(&rt), before_toasts + 1, "one toast, on the press");
    assert_eq!(fake.core().writes, writes);
    assert_eq!(size_of(&fake, "pn-1"), None);
    assert!(near_rect(element(&rt, "pane-pn-1").target, pane));
}

#[test]
fn a_panes_grip_shows_on_its_top_edge_and_drops_it_onto_another_pane() {
    let (mut rt, fake) = runtime();
    let pane = element(&rt, "pane-pn-1").rect;
    assert!(
        find(&rt, "grip-pn-1").is_none(),
        "no grip until the pointer is there"
    );
    // Anywhere along the top edge shows the grip, centered.
    rt.pointer_move(pane[0] + 40.0, pane[1] + 5.0);
    frames(&mut rt, 2);
    let grip = element(&rt, "grip-pn-1");
    assert!(near(grip.rect[2], 28.0));
    assert!(near(center(&grip).0, pane[0] + pane[2] / 2.0));
    // Lower down, it goes.
    rt.pointer_move(pane[0] + 40.0, pane[1] + 60.0);
    frames(&mut rt, 2);
    assert!(find(&rt, "grip-pn-1").is_none());

    // Drag it to the middle of pn-3: the preview covers pn-3, and the drop swaps them.
    rt.pointer_move(pane[0] + 40.0, pane[1] + 5.0);
    frames(&mut rt, 2);
    let from = center(&element(&rt, "grip-pn-1"));
    rt.pointer_down(from.0, from.1);
    rt.frame(1.0 / 60.0);
    let target = element(&rt, "pane-pn-3").rect;
    let to = (target[0] + target[2] / 2.0, target[1] + target[3] / 2.0);
    glide(&mut rt, from, to);
    frames(&mut rt, 1);
    let preview = element(&rt, "drop-preview").rect;
    assert!(near_rect(preview, target), "{preview:?} {target:?}");
    rt.pointer_up(to.0, to.1);
    frames(&mut rt, 2);
    rt.state.settle();
    let tab = doc(&fake).tab("tb-work").unwrap().clone();
    let order: Vec<&str> = tab.root.panes().iter().map(|n| n.id.as_str()).collect();
    assert_eq!(order, ["pn-3", "pn-2", "pn-1"]);
    assert_eq!(rt.state.model.focused_pane().as_deref(), Some("pn-1"));
    assert!(find(&rt, "drop-preview").is_none());
}

#[test]
fn a_press_on_a_panes_top_edge_still_reaches_its_terminal() {
    let (mut rt, _) = runtime();
    assert_eq!(rt.state.model.focused_pane().as_deref(), Some("pn-1"));
    // Inside pn-2's top strip but off its grip: the grip shows, and the press focuses the pane.
    let pane = element(&rt, "pane-pn-2").rect;
    let at = (pane[0] + 10.0, pane[1] + 4.0);
    rt.pointer_move(at.0, at.1);
    frames(&mut rt, 2);
    assert!(find(&rt, "grip-pn-2").is_some());
    rt.pointer_down(at.0, at.1);
    rt.pointer_up(at.0, at.1);
    frames(&mut rt, 2);
    assert_eq!(rt.state.model.focused_pane().as_deref(), Some("pn-2"));
}

#[test]
fn a_grip_dropped_on_an_edge_places_the_pane_beside() {
    let (mut rt, fake) = runtime();
    let pane = element(&rt, "pane-pn-3").rect;
    rt.pointer_move(pane[0] + 40.0, pane[1] + 5.0);
    frames(&mut rt, 2);
    let from = center(&element(&rt, "grip-pn-3"));
    rt.pointer_down(from.0, from.1);
    rt.frame(1.0 / 60.0);
    // The left edge of pn-1.
    let target = element(&rt, "pane-pn-1").rect;
    let to = (target[0] + 10.0, target[1] + target[3] / 2.0);
    glide(&mut rt, from, to);
    frames(&mut rt, 1);
    let preview = element(&rt, "drop-preview").rect;
    assert!(near(preview[2], target[2] / 2.0), "the left half");
    rt.pointer_up(to.0, to.1);
    frames(&mut rt, 2);
    rt.state.settle();
    let tab = doc(&fake).tab("tb-work").unwrap().clone();
    let ids: Vec<&str> = tab.root.children().iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids[0], "pn-3", "pn-3 is first in the row, left of pn-1");
    assert_eq!(ids[1], "pn-1");
    assert_eq!(tab.root.addresses(), ["t3", "t1", "t2"]);
}

#[test]
fn a_short_press_on_the_grip_moves_nothing_and_a_zoomed_tab_has_no_grip() {
    let (mut rt, fake) = runtime();
    let writes = fake.core().writes;
    let pane = element(&rt, "pane-pn-2").rect;
    rt.pointer_move(pane[0] + 40.0, pane[1] + 5.0);
    frames(&mut rt, 2);
    let at = center(&element(&rt, "grip-pn-2"));
    rt.pointer_down(at.0, at.1);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(at.0 + 2.0, at.1 + 2.0);
    rt.pointer_up(at.0 + 2.0, at.1 + 2.0);
    frames(&mut rt, 2);
    rt.state.settle();
    assert_eq!(fake.core().writes, writes);

    // Zoomed: one pane drawn, no dividers, no grip, and the pill says so.
    rt.key(KeyEvent {
        key: Key::Enter,
        mods: Mods {
            cmd: true,
            shift: true,
            ..Default::default()
        },
    });
    frames(&mut rt, 3);
    assert!(find(&rt, "pane-pn-3").is_none());
    assert!(find(&rt, "divider-tb-work-0").is_none());
    let zoomed = rt.state.model.focused_pane().unwrap();
    let pane = element(&rt, &format!("pane-{zoomed}")).rect;
    rt.pointer_move(pane[0] + 40.0, pane[1] + 5.0);
    frames(&mut rt, 2);
    assert!(find(&rt, &format!("grip-{zoomed}")).is_none());
    let pill = rt
        .elements()
        .into_iter()
        .filter_map(|e| e.text)
        .find(|t| t.starts_with("work"));
    assert_eq!(pill.as_deref(), Some("work Z"));
}
