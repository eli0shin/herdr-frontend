//! Where a tab's panes sit, from its tree alone, and which pane lies in a direction.
//!
//! Ported from herdr-app (`app:src/window/pane_size.rs` `shares`/`block_bounds`, and
//! `app:src/window.rs` `nearest`) without frames, borders or padding: Eli's panes are bare, with a
//! gap between split panes (`feature-inventory.md` §1). The renderer lays panes out with the same
//! rules, so a direction picked here matches what is drawn.

use herdr_shared::{Node, Size};
use serde_json::Value;

use crate::action::Direction;

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
    place(node, geometry.area, geometry, &mut out);
    out
}

fn place(node: &Node, rect: Rect, g: &Geometry, out: &mut Vec<(String, Rect)>) {
    if node.hidden() {
        return;
    }
    if node.content().is_some() {
        out.push((node.id.clone(), rect));
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
            place(child, child_rect, g, out);
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
        place(child, rect, g, out);
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
