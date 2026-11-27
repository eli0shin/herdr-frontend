// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! Where a tab's panes sit, from its tree alone, and which pane lies in a direction.
//!
//! Ported from herdr-app (`app:src/window/pane_size.rs` `shares`/`block_bounds`, and
//! `app:src/window.rs` `nearest`) without frames, borders or padding: Eli's panes are bare, with a
//! gap between split panes (`feature-inventory.md` §1). The renderer lays panes out with the same
//! rules, so a direction picked here matches what is drawn.

use herdr_shared::{Axis, Node, Size};
use serde_json::Value;

use crate::action::{Direction, Edge};

/// How far a keyboard resize moves a divider, in points at zoom 1 (`pane.resize`'s default, as
/// in the TUI).
pub const RESIZE_STEP: f32 = 40.0;

/// The least a divider drag leaves either pane beside it, in points at zoom 1
/// (`app:src/window.rs` `drag_to`).
pub const DRAG_MIN: f32 = 24.0;

/// The top strip of a pane that shows its drag grip on hover, in points at zoom 1
/// (`app:src/window/pane_drag.rs` `STRIP`).
pub const GRIP_ROW: f32 = 18.0;

/// A rectangle in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// How a tab's area is divided.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    /// The tab's area.
    pub area: Rect,
    /// Between split panes (Eli's layout: 1).
    pub gap: f32,
    /// The UI zoom: `px` sizes, `min` and `max` scale with it.
    pub zoom: f32,
}

impl Default for Geometry {
    fn default() -> Self {
        Geometry {
            area: Rect::new(0.0, 0.0, 1200.0, 800.0),
            gap: 1.0,
            zoom: 1.0,
        }
    }
}

fn number(node: &Node, key: &str) -> Option<f32> {
    node.extra
        .get(key)
        .and_then(Value::as_f64)
        .map(|n| n as f32)
}

fn weight(node: &Node) -> f32 {
    match node.size() {
        Some(Size::Px(_)) => 0.0,
        Some(Size::Weight(weight)) => weight,
        None => 1.0,
    }
}

/// Each child's length along the axis: fixed `px` children first, then the rest shared by
/// weight, honoring `min` and `max` (`app:src/window/pane_size.rs` `shares`).
pub fn shares(children: &[&Node], available: f32, zoom: f32) -> Vec<f32> {
    let min = |n: &Node| number(n, "min").unwrap_or(0.0) * zoom;
    let max = |n: &Node| number(n, "max").unwrap_or(f32::INFINITY) * zoom;
    let mut sizes = vec![0.0; children.len()];
    let mut pending = Vec::new();
    let mut left = available;
    for (i, n) in children.iter().enumerate() {
        match n.size() {
            Some(Size::Px(px)) => {
                sizes[i] = (px * zoom).max(min(n)).min(max(n));
                left -= sizes[i];
            }
            _ => pending.push(i),
        }
    }
    let total_weight = pending
        .iter()
        .map(|&i| weight(children[i]))
        .sum::<f32>()
        .max(f32::EPSILON);
    let initial_free = left.max(0.0);
    while !pending.is_empty() {
        let total = pending
            .iter()
            .map(|&i| weight(children[i]))
            .sum::<f32>()
            .max(f32::EPSILON);
        // Grow factors stay normalized to the original siblings after an item freezes.
        let free = left.max(0.0).min(initial_free * (total / total_weight));
        let mut violations = Vec::new();
        for &i in &pending {
            let n = children[i];
            let share = free * weight(n) / total;
            sizes[i] = share.min(max(n)).max(min(n));
            violations.push((i, sizes[i] - share));
        }
        let total_violation = violations.iter().map(|(_, v)| v).sum::<f32>();
        if total_violation == 0.0 {
            break;
        }
        for (i, violation) in violations {
            if (total_violation > 0.0 && violation <= 0.0)
                || (total_violation < 0.0 && violation >= 0.0)
            {
                continue;
            }
            left -= sizes[i];
            pending.retain(|&j| j != i);
        }
    }
    sizes
}

/// Every visible block under `node` and its rectangle, in tree order. Hidden nodes take no
/// room; a stack shows its active child (else its first).
pub fn blocks(node: &Node, geometry: &Geometry) -> Vec<(String, Rect)> {
    let mut out = Vec::new();
    place(node, geometry.area, geometry, &mut out, false);
    out
}

/// Every visible node under `node`, containers included, and its rectangle, in tree order.
pub fn nodes(node: &Node, geometry: &Geometry) -> Vec<(String, Rect)> {
    let mut out = Vec::new();
    place(node, geometry.area, geometry, &mut out, true);
    out
}

fn place(node: &Node, rect: Rect, g: &Geometry, out: &mut Vec<(String, Rect)>, all: bool) {
    if node.hidden() {
        return;
    }
    if all || node.content().is_some() {
        out.push((node.id.clone(), rect));
    }
    if node.content().is_some() {
        return;
    }
    if let Some(axis) = node.axis() {
        let row = axis == herdr_shared::Axis::Row;
        let children: Vec<&Node> = node.children().iter().filter(|c| !c.hidden()).collect();
        let along = if row { rect.w } else { rect.h };
        let available = along - g.gap * children.len().saturating_sub(1) as f32;
        let mut at = if row { rect.x } else { rect.y };
        for (child, span) in children.iter().zip(shares(&children, available, g.zoom)) {
            let child_rect = if row {
                Rect::new(at, rect.y, span, rect.h)
            } else {
                Rect::new(rect.x, at, rect.w, span)
            };
            place(child, child_rect, g, out, all);
            at += span + g.gap;
        }
        return;
    }
    let children = node.children();
    let shown = node
        .active()
        .and_then(|id| children.iter().find(|c| c.id == id))
        .or(children.first());
    if let Some(child) = shown {
        place(child, rect, g, out, all);
    }
}

/// A split's divider: the gap between its `index`th and next shown children.
#[derive(Clone, Debug, PartialEq)]
pub struct Divider {
    pub container: String,
    pub index: usize,
    pub axis: Axis,
    /// The gap, in the tab area.
    pub rect: Rect,
}

/// The dividers of every resizable split under `node`, in tree order.
pub fn dividers(node: &Node, geometry: &Geometry) -> Vec<Divider> {
    let rects = nodes(node, geometry);
    let rect_of = |id: &str| rects.iter().find(|(n, _)| n == id).map(|(_, r)| *r);
    let mut out = Vec::new();
    for n in node.walk() {
        let (Some(axis), Some(own)) = (n.axis(), rect_of(&n.id)) else {
            continue;
        };
        if !n.resizable() {
            continue;
        }
        let shown: Vec<&Node> = n.children().iter().filter(|c| !c.hidden()).collect();
        for (index, pair) in shown.windows(2).enumerate() {
            let (Some(a), Some(b)) = (rect_of(&pair[0].id), rect_of(&pair[1].id)) else {
                continue;
            };
            let rect = match axis {
                Axis::Row => Rect::new(a.right(), own.y, b.x - a.right(), own.h),
                Axis::Column => Rect::new(own.x, a.bottom(), own.w, b.y - a.bottom()),
            };
            out.push(Divider {
                container: n.id.clone(),
                index,
                axis,
                rect,
            });
        }
    }
    out
}

/// The sizes a divider drag to `at` (the pointer along the split's axis, in the tab area) gives
/// the two children beside it: either keeps [`DRAG_MIN`] and its own `min` and `max`, a `px`
/// child stays `px`, two weighted ones share their weight (`app:src/window.rs` `drag_to`).
/// `None` when the split can't be dragged, or is too squeezed to.
pub fn drag_sizes(
    root: &Node,
    container: &str,
    index: usize,
    at: f32,
    geometry: &Geometry,
) -> Option<Vec<(String, Size)>> {
    let node = root.find(container)?;
    let axis = node.axis()?;
    if !node.resizable() {
        return None;
    }
    let shown: Vec<&Node> = node.children().iter().filter(|c| !c.hidden()).collect();
    let (a, b) = (shown.get(index)?, shown.get(index + 1)?);
    let rects = nodes(root, geometry);
    let rect_of = |id: &str| rects.iter().find(|(n, _)| n == id).map(|(_, r)| *r);
    let (ra, rb) = (rect_of(&a.id)?, rect_of(&b.id)?);
    let (start, la, lb) = match axis {
        Axis::Row => (ra.x, ra.w, rb.w),
        Axis::Column => (ra.y, ra.h, rb.h),
    };
    // Measured on screen; sizes are stored as designed (unzoomed).
    let zoom = geometry.zoom.max(f32::EPSILON);
    let (la, lb, start, at) = (la / zoom, lb / zoom, start / zoom, at / zoom);
    let total = la + lb;
    if total <= 2.0 * DRAG_MIN {
        return None;
    }
    let mut new_a = (at - start).clamp(DRAG_MIN, total - DRAG_MIN);
    if let Some(min) = number(a, "min") {
        new_a = new_a.max(min);
    }
    if let Some(max) = number(a, "max") {
        new_a = new_a.min(max);
    }
    let new_a = new_a.min(total - DRAG_MIN);
    Some(match (a.size(), b.size()) {
        (Some(Size::Px(_)), _) => vec![(a.id.clone(), Size::Px(new_a.round()))],
        (_, Some(Size::Px(_))) => vec![(b.id.clone(), Size::Px((total - new_a).round()))],
        (wa, wb) => {
            let w = |s: Option<Size>| match s {
                Some(Size::Weight(w)) => w,
                _ => 1.0,
            };
            let sum = w(wa) + w(wb);
            vec![
                (a.id.clone(), Size::Weight(sum * new_a / total)),
                (b.id.clone(), Size::Weight(sum * (total - new_a) / total)),
            ]
        }
    })
}

/// The divider a keyboard resize of `pane` toward `direction` moves: the nearest split along
/// that axis above the pane, its divider after the pane (before it, for the last child). The
/// divider moves in `direction` whichever side of it the pane is on, as tmux does
/// (`app:src/window.rs` `resize_pane`). `None` when nothing is split that way.
pub fn resize_divider(root: &Node, pane: &str, direction: Direction) -> Option<(String, usize)> {
    let (axis, forward) = match direction {
        Direction::Left => (Axis::Row, false),
        Direction::Right => (Axis::Row, true),
        Direction::Up => (Axis::Column, false),
        Direction::Down => (Axis::Column, true),
    };
    let path = root.path_to(pane)?;
    for i in (0..path.len().saturating_sub(1)).rev() {
        let Some(node) = root.find(&path[i]) else {
            continue;
        };
        if node.axis() != Some(axis) || !node.resizable() {
            continue;
        }
        let shown: Vec<&Node> = node.children().iter().filter(|c| !c.hidden()).collect();
        if shown.len() < 2 {
            continue;
        }
        let Some(at) = shown.iter().position(|c| c.id == path[i + 1]) else {
            continue;
        };
        let index = if forward {
            if at + 1 < shown.len() { at } else { at - 1 }
        } else {
            at.saturating_sub(1)
        };
        return Some((node.id.clone(), index));
    }
    None
}

/// Where a pane dragged to `(x, y)` lands on a pane at `rect`: the outer quarter nearest an edge
/// places it beside, the middle swaps. `None` outside (`app:src/window/pane_drag.rs`
/// `drop_zone`).
pub fn drop_zone(rect: Rect, x: f32, y: f32) -> Option<Edge> {
    if !rect.contains(x, y) || rect.w <= 0.0 || rect.h <= 0.0 {
        return None;
    }
    let fx = (x - rect.x) / rect.w;
    let fy = (y - rect.y) / rect.h;
    let (distance, edge) = [
        (fx, Edge::Left),
        (1.0 - fx, Edge::Right),
        (fy, Edge::Top),
        (1.0 - fy, Edge::Bottom),
    ]
    .into_iter()
    .min_by(|a, b| a.0.total_cmp(&b.0))
    .expect("four edges");
    Some(if distance < 0.25 { edge } else { Edge::Center })
}

/// What a drop on `rect` at `edge` covers: the half beside, or the whole pane for a swap. For
/// the preview drawn under the pointer.
pub fn drop_rect(rect: Rect, edge: Edge) -> Rect {
    let (hw, hh) = (rect.w / 2.0, rect.h / 2.0);
    match edge {
        Edge::Center => rect,
        Edge::Left => Rect::new(rect.x, rect.y, hw, rect.h),
        Edge::Right => Rect::new(rect.x + hw, rect.y, hw, rect.h),
        Edge::Top => Rect::new(rect.x, rect.y, rect.w, hh),
        Edge::Bottom => Rect::new(rect.x, rect.y + hh, rect.w, hh),
    }
}

/// The candidate nearest `from` in `direction`: the smallest gap to the edge (within a pixel
/// counts as equal), then the most recently focused (`history`, oldest first), then the most
/// overlap, then layout order (`app:src/window.rs` `nearest`). Candidates must overlap `from`
/// across the direction.
pub fn nearest<'a>(
    from: Rect,
    direction: Direction,
    candidates: impl IntoIterator<Item = (&'a str, Rect)>,
    history: &[String],
) -> Option<&'a str> {
    let overlap = |a0: f32, a1: f32, b0: f32, b1: f32| (a1.min(b1) - a0.max(b0)).max(0.0);
    let cmp = |a: f32, b: f32| {
        if (a - b).abs() < 1.0 {
            std::cmp::Ordering::Equal
        } else {
            a.total_cmp(&b)
        }
    };
    candidates
        .into_iter()
        .filter_map(|(id, r)| {
            let (gap, ov) = match direction {
                Direction::Left => (
                    from.x - r.right(),
                    overlap(from.y, from.bottom(), r.y, r.bottom()),
                ),
                Direction::Right => (
                    r.x - from.right(),
                    overlap(from.y, from.bottom(), r.y, r.bottom()),
                ),
                Direction::Up => (
                    from.y - r.bottom(),
                    overlap(from.x, from.right(), r.x, r.right()),
                ),
                Direction::Down => (
                    r.y - from.bottom(),
                    overlap(from.x, from.right(), r.x, r.right()),
                ),
            };
            let recent = history.iter().position(|pane| pane == id);
            (gap > -2.0 && ov > 0.0).then_some((gap, recent, ov, id))
        })
        .reduce(|best, next| {
            let better = cmp(next.0, best.0)
                .then(best.1.cmp(&next.1))
                .then(cmp(best.2, next.2))
                .is_lt();
            if better { next } else { best }
        })
        .map(|(_, _, _, id)| id)
}

/// Remembers a focused pane for [`nearest`]: most recent last, each once.
pub fn remember(history: &mut Vec<String>, id: &str) {
    if history.last().is_some_and(|last| last == id) {
        return;
    }
    history.retain(|pane| pane != id);
    history.push(id.to_owned());
    if history.len() > 64 {
        history.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use herdr_shared::Axis;
    use serde_json::json;

    fn pane(id: &str) -> Node {
        Node::pane(id, format!("t-{id}"))
    }

    fn sized(mut node: Node, size: Size) -> Node {
        node.set_size(Some(size));
        node
    }

    fn rects(root: &Node, w: f32, h: f32, gap: f32) -> Vec<(String, Rect)> {
        blocks(
            root,
            &Geometry {
                area: Rect::new(0.0, 0.0, w, h),
                gap,
                zoom: 1.0,
            },
        )
    }

    #[test]
    fn a_row_shares_its_width_by_weight_after_gaps() {
        let root = Node::container(
            "r",
            Axis::Row,
            vec![pane("a"), sized(pane("b"), Size::Weight(3.0))],
        );
        let r = rects(&root, 401.0, 100.0, 1.0);
        assert_eq!(r[0], ("a".into(), Rect::new(0.0, 0.0, 100.0, 100.0)));
        assert_eq!(r[1], ("b".into(), Rect::new(101.0, 0.0, 300.0, 100.0)));
    }

    #[test]
    fn px_children_take_their_size_first_and_scale_with_zoom() {
        let children = [sized(pane("a"), Size::Px(100.0)), pane("b")];
        let refs: Vec<&Node> = children.iter().collect();
        assert_eq!(shares(&refs, 500.0, 1.0), [100.0, 400.0]);
        assert_eq!(shares(&refs, 500.0, 2.0), [200.0, 300.0]);
    }

    #[test]
    fn min_and_max_hold_and_the_rest_reshares() {
        let mut a = pane("a");
        a.extra.insert("max".into(), json!(50));
        let mut c = pane("c");
        c.extra.insert("min".into(), json!(200));
        let children = [a, pane("b"), c];
        let refs: Vec<&Node> = children.iter().collect();
        let sizes = shares(&refs, 300.0, 1.0);
        assert_eq!(sizes[0], 50.0);
        assert_eq!(sizes[2], 200.0);
        assert_eq!(sizes[1], 50.0);
    }

    #[test]
    fn nested_columns_and_hidden_nodes() {
        let mut hidden = pane("h");
        hidden.extra.insert("hidden".into(), json!(true));
        let root = Node::container(
            "r",
            Axis::Row,
            vec![
                pane("a"),
                Node::container("c", Axis::Column, vec![pane("b"), hidden, pane("d")]),
            ],
        );
        let r = rects(&root, 201.0, 101.0, 1.0);
        let ids: Vec<&str> = r.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "d"]);
        assert_eq!(r[1].1, Rect::new(101.0, 0.0, 100.0, 50.0));
        assert_eq!(r[2].1, Rect::new(101.0, 51.0, 100.0, 50.0));
    }

    #[test]
    fn a_stack_shows_its_active_child_else_its_first() {
        let mut stack = Node::new(
            "s",
            herdr_shared::Kind::Stack {
                children: vec![pane("a"), pane("b")],
            },
        );
        assert_eq!(rects(&stack, 10.0, 10.0, 1.0)[0].0, "a");
        stack.extra.insert("active".into(), json!("b"));
        assert_eq!(rects(&stack, 10.0, 10.0, 1.0)[0].0, "b");
    }

    /// a | b
    ///   | c
    fn three() -> Vec<(String, Rect)> {
        let root = Node::container(
            "r",
            Axis::Row,
            vec![
                pane("a"),
                Node::container("col", Axis::Column, vec![pane("b"), pane("c")]),
            ],
        );
        rects(&root, 201.0, 201.0, 1.0)
    }

    fn go(layout: &[(String, Rect)], from: &str, d: Direction, history: &[&str]) -> Option<String> {
        let from = layout.iter().find(|(id, _)| id == from).unwrap().1;
        let history: Vec<String> = history.iter().map(|s| s.to_string()).collect();
        let candidates = layout
            .iter()
            .filter(|(_, r)| *r != from)
            .map(|(id, r)| (id.as_str(), *r));
        nearest(from, d, candidates, &history).map(str::to_owned)
    }

    #[test]
    fn nearest_goes_along_the_direction_and_stops_at_edges() {
        let l = three();
        assert_eq!(go(&l, "b", Direction::Down, &[]).as_deref(), Some("c"));
        assert_eq!(go(&l, "c", Direction::Up, &[]).as_deref(), Some("b"));
        assert_eq!(go(&l, "b", Direction::Left, &[]).as_deref(), Some("a"));
        assert_eq!(go(&l, "a", Direction::Left, &[]), None);
        assert_eq!(go(&l, "a", Direction::Up, &[]), None);
        assert_eq!(go(&l, "c", Direction::Down, &[]), None);
    }

    #[test]
    fn nearest_prefers_the_most_recently_focused_along_the_edge() {
        let l = three();
        // Both b and c border a on the right; neither visited: layout order picks b.
        assert_eq!(go(&l, "a", Direction::Right, &[]).as_deref(), Some("b"));
        assert_eq!(go(&l, "a", Direction::Right, &["c"]).as_deref(), Some("c"));
        assert_eq!(
            go(&l, "a", Direction::Right, &["c", "b"]).as_deref(),
            Some("b")
        );
        assert_eq!(
            go(&l, "a", Direction::Right, &["b", "c"]).as_deref(),
            Some("c")
        );
    }

    #[test]
    fn nearest_prefers_more_overlap_among_unvisited_panes() {
        // a spans the left; on the right, b is short and c tall.
        let layout = vec![
            ("a".to_string(), Rect::new(0.0, 0.0, 100.0, 300.0)),
            ("b".to_string(), Rect::new(101.0, 0.0, 100.0, 50.0)),
            ("c".to_string(), Rect::new(101.0, 51.0, 100.0, 249.0)),
        ];
        assert_eq!(
            go(&layout, "a", Direction::Right, &[]).as_deref(),
            Some("c")
        );
    }

    fn geometry(w: f32, h: f32) -> Geometry {
        Geometry {
            area: Rect::new(0.0, 0.0, w, h),
            gap: 1.0,
            zoom: 1.0,
        }
    }

    /// a | (b / c), 201 × 201: a and the column are 100 wide.
    fn three_tree() -> Node {
        Node::container(
            "r",
            Axis::Row,
            vec![
                pane("a"),
                Node::container("col", Axis::Column, vec![pane("b"), pane("c")]),
            ],
        )
    }

    #[test]
    fn dividers_sit_in_the_gaps_of_resizable_splits() {
        let root = three_tree();
        let d = dividers(&root, &geometry(201.0, 201.0));
        assert_eq!(
            d,
            [
                Divider {
                    container: "r".into(),
                    index: 0,
                    axis: Axis::Row,
                    rect: Rect::new(100.0, 0.0, 1.0, 201.0),
                },
                Divider {
                    container: "col".into(),
                    index: 0,
                    axis: Axis::Column,
                    rect: Rect::new(101.0, 100.0, 100.0, 1.0),
                },
            ]
        );
        let mut fixed = three_tree();
        fixed.extra.insert("resizable".into(), json!(false));
        assert_eq!(dividers(&fixed, &geometry(201.0, 201.0)).len(), 1);
    }

    #[test]
    fn a_divider_drag_shares_weight_and_keeps_px_and_the_minimum() {
        let root = three_tree();
        let g = geometry(201.0, 201.0);
        // To 150: a takes 150 of the 200 the two share.
        assert_eq!(
            drag_sizes(&root, "r", 0, 150.0, &g),
            Some(vec![
                ("a".into(), Size::Weight(1.5)),
                ("col".into(), Size::Weight(0.5)),
            ])
        );
        // Past the end: each keeps 24.
        let sizes = drag_sizes(&root, "r", 0, 500.0, &g).unwrap();
        assert_eq!(sizes[1], ("col".into(), Size::Weight(2.0 * 24.0 / 200.0)));
        // A px side stays px.
        let mut px = three_tree();
        px.find_mut("col").unwrap().set_size(Some(Size::Px(100.0)));
        assert_eq!(
            drag_sizes(&px, "r", 0, 60.0, &g),
            Some(vec![("col".into(), Size::Px(140.0))])
        );
        // a's own max holds (laid out at its max, a and the column share 180).
        let mut max = three_tree();
        max.find_mut("a")
            .unwrap()
            .extra
            .insert("max".into(), json!(80));
        let sizes = drag_sizes(&max, "r", 0, 150.0, &g).unwrap();
        assert_eq!(sizes[0], ("a".into(), Size::Weight(2.0 * 80.0 / 180.0)));
        // Too squeezed, or not a split: nothing.
        assert_eq!(drag_sizes(&root, "r", 0, 20.0, &geometry(40.0, 40.0)), None);
        assert_eq!(drag_sizes(&root, "a", 0, 20.0, &g), None);
        assert_eq!(drag_sizes(&root, "r", 1, 20.0, &g), None);
    }

    #[test]
    fn a_divider_drag_measures_in_unzoomed_points() {
        let root = three_tree();
        let mut g = geometry(402.0, 402.0);
        g.zoom = 2.0;
        g.gap = 2.0;
        let sizes = drag_sizes(&root, "r", 0, 300.0, &g).unwrap();
        assert_eq!(sizes[0], ("a".into(), Size::Weight(1.5)));
    }

    #[test]
    fn a_keyboard_resize_moves_the_nearest_divider_along_its_axis() {
        let root = three_tree();
        let at = |pane, d| resize_divider(&root, pane, d);
        assert_eq!(at("a", Direction::Right), Some(("r".into(), 0)));
        assert_eq!(at("a", Direction::Left), Some(("r".into(), 0)));
        assert_eq!(at("b", Direction::Left), Some(("r".into(), 0)));
        assert_eq!(at("b", Direction::Down), Some(("col".into(), 0)));
        assert_eq!(at("c", Direction::Down), Some(("col".into(), 0)));
        assert_eq!(
            at("a", Direction::Down),
            None,
            "a has nothing above or below"
        );
        assert_eq!(at("gone", Direction::Down), None);
    }

    #[test]
    fn drops_beside_in_the_outer_quarter_and_swap_in_the_middle() {
        let r = Rect::new(100.0, 200.0, 800.0, 400.0);
        for (x, y, edge) in [
            (500.0, 400.0, Some(Edge::Center)),
            (110.0, 400.0, Some(Edge::Left)),
            (890.0, 400.0, Some(Edge::Right)),
            (500.0, 210.0, Some(Edge::Top)),
            (500.0, 590.0, Some(Edge::Bottom)),
            (99.0, 400.0, None),
        ] {
            assert_eq!(drop_zone(r, x, y), edge, "{x}, {y}");
        }
        assert_eq!(
            drop_rect(r, Edge::Right),
            Rect::new(500.0, 200.0, 400.0, 400.0)
        );
        assert_eq!(
            drop_rect(r, Edge::Top),
            Rect::new(100.0, 200.0, 800.0, 200.0)
        );
        assert_eq!(drop_rect(r, Edge::Center), r);
    }

    #[test]
    fn remember_keeps_each_pane_once_most_recent_last() {
        let mut h = Vec::new();
        for id in ["a", "b", "a", "a", "c"] {
            remember(&mut h, id);
        }
        assert_eq!(h, ["b", "a", "c"]);
        for i in 0..100 {
            remember(&mut h, &format!("p{i}"));
        }
        assert_eq!(h.len(), 64);
        assert_eq!(h.last().map(String::as_str), Some("p99"));
    }
}
