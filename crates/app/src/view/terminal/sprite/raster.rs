// SPDX-License-Identifier: MPL-2.0
//   Copyright © 2024-2026 Chris Marchesi
//   Copyright © 2002 University of Southern California
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// https://mozilla.org/MPL/2.0/.
//
// This file is a Rust port, modified for herdr-frontend, of z2d
// (https://github.com/vancluever/z2d at 40af3e84a4e15a9f31d126b3f281f23ce6a3dae5). Portions of
// the z2d code it ports were derived and adapted from the Cairo project
// (https://www.cairographics.org/): `Spline.zig` from cairo-spline.c, `arc.zig` from cairo-arc.c and
// cairo-matrix.c, `Pen.zig` from cairo-pen.c. It stays under the MPL 2.0, not the MIT License of the rest of
// herdr-frontend. See THIRD-PARTY-NOTICES.md and licenses/z2d-LICENSE.txt.

//! The path rasterizer Ghostty's sprite canvas draws with: a port of z2d 0.13's default fill
//! (`z2d:src/painter.zig` `fill`, `internal/tess/fill_plotter.zig`, `internal/tess/Spline.zig`,
//! `internal/tess/Polygon.zig` and `internal/raster/multisample.zig` at `40af3e84`, the revision
//! Ghostty 1.4.0 pins).
//!
//! A fill is plotted into edges at 4× scale, curves flattened by de Casteljau subdivision to
//! 0.1 px, then rasterized with 4 × 4 samples a pixel: each of a pixel's four sub-scanlines is
//! sampled at its middle, edge crossings are rounded to the nearest quarter pixel, and the
//! covered quarters are summed. A pixel's alpha is `16 × coverage − 1`, or 255 at full coverage,
//! composited source-over in z2d's integer arithmetic.
//!
//! Strokes are turned into fills here rather than through z2d's stroke plotter: each segment is
//! a quad, each round join or cap a disc of z2d's pen, each miter join its wedge, all wound the
//! same way and filled non-zero. That's the same region z2d's single outline encloses, so the
//! two differ only where a sample lands within float error of an edge.

/// A point in the canvas's pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

pub const fn pt(x: f64, y: f64) -> Point {
    Point { x, y }
}

/// z2d's default curve tolerance, in pixels.
const TOLERANCE: f64 = 0.1;
/// Samples a pixel along each axis.
const SCALE: f64 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cap {
    Butt,
    Round,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    Miter,
    Round,
}

#[derive(Clone, Copy, Debug)]
enum Node {
    Move(Point),
    Line(Point),
    Curve(Point, Point, Point),
    Close,
}

/// A path in the canvas's pixels: z2d's `Path` nodes.
#[derive(Clone, Debug, Default)]
pub struct Path {
    nodes: Vec<Node>,
}

impl Path {
    pub fn new() -> Path {
        Path::default()
    }

    pub fn move_to(mut self, x: f64, y: f64) -> Path {
        self.nodes.push(Node::Move(pt(x, y)));
        self
    }

    pub fn line_to(mut self, x: f64, y: f64) -> Path {
        self.nodes.push(Node::Line(pt(x, y)));
        self
    }

    pub fn curve_to(mut self, x1: f64, y1: f64, x2: f64, y2: f64, x3: f64, y3: f64) -> Path {
        self.nodes
            .push(Node::Curve(pt(x1, y1), pt(x2, y2), pt(x3, y3)));
        self
    }

    pub fn close(mut self) -> Path {
        self.nodes.push(Node::Close);
        self
    }

    /// The path moved by (`dx`, `dy`), as the canvas's padding transform moves its paths.
    pub fn translated(&self, dx: f64, dy: f64) -> Path {
        let m = |p: Point| pt(p.x + dx, p.y + dy);
        Path {
            nodes: self
                .nodes
                .iter()
                .map(|n| match *n {
                    Node::Move(p) => Node::Move(m(p)),
                    Node::Line(p) => Node::Line(m(p)),
                    Node::Curve(a, b, c) => Node::Curve(m(a), m(b), m(c)),
                    Node::Close => Node::Close,
                })
                .collect(),
        }
    }

    /// The point the next node starts from: the last one, or a closed subpath's first.
    fn current(&self) -> Option<Point> {
        let mut start = None;
        let mut current = None;
        for node in &self.nodes {
            match *node {
                Node::Move(p) => (start, current) = (Some(p), Some(p)),
                Node::Line(p) | Node::Curve(_, _, p) => current = Some(p),
                Node::Close => current = start,
            }
        }
        current
    }

    /// z2d's `arc(xc, yc, r, a1, a2)`, clockwise (y down) from `a1` to `a2`: a line to its start
    /// (or a move, on an empty path), then cubics no wider than the curve tolerance allows,
    /// halving any turn over π first (`internal/arc.zig` `arc_in_direction`).
    pub fn arc(mut self, xc: f64, yc: f64, r: f64, a1: f64, a2: f64) -> Path {
        let mut a2 = a2;
        while a2 < a1 {
            a2 += std::f64::consts::TAU;
        }
        self.arc_forward(xc, yc, r, a1, a2);
        self
    }

    fn arc_forward(&mut self, xc: f64, yc: f64, r: f64, amin: f64, amax: f64) {
        use std::f64::consts::PI;
        let line_to = |path: &mut Path, x: f64, y: f64| match path.current() {
            Some(p) if p == pt(x, y) => {}
            Some(_) => path.nodes.push(Node::Line(pt(x, y))),
            None => path.nodes.push(Node::Move(pt(x, y))),
        };
        if amax - amin > PI {
            let amid = amin + (amax - amin) / 2.0;
            self.arc_forward(xc, yc, r, amin, amid);
            self.arc_forward(xc, yc, r, amid, amax);
        } else if amax != amin {
            let mut segments = (((amax - amin).abs()) / max_arc_angle(TOLERANCE / r)).ceil() as i32;
            let step = (amax - amin) / f64::from(segments);
            segments -= 1;
            line_to(self, xc + r * amin.cos(), yc + r * amin.sin());
            let mut a = amin;
            for _ in 0..segments.max(0) {
                self.arc_segment(xc, yc, r, a, a + step);
                a += step;
            }
            self.arc_segment(xc, yc, r, a, amax);
        } else {
            line_to(self, xc + r * amin.cos(), yc + r * amin.sin());
        }
    }

    fn arc_segment(&mut self, xc: f64, yc: f64, r: f64, a: f64, b: f64) {
        let (sa, ca, sb, cb) = (r * a.sin(), r * a.cos(), r * b.sin(), r * b.cos());
        let h = 4.0 / 3.0 * ((b - a) / 4.0).tan();
        self.nodes.push(Node::Curve(
            pt(xc + ca - h * sa, yc + sa + h * ca),
            pt(xc + cb + h * sb, yc + sb - h * cb),
            pt(xc + cb, yc + sb),
        ));
    }

    /// The path's subpaths as polylines, curves flattened to z2d's tolerance, each with whether
    /// it was closed.
    fn polylines(&self) -> Vec<(Vec<Point>, bool)> {
        let mut out: Vec<(Vec<Point>, bool)> = Vec::new();
        let mut current: Vec<Point> = Vec::new();
        let flush = |current: &mut Vec<Point>, out: &mut Vec<(Vec<Point>, bool)>, closed| {
            if !current.is_empty() {
                out.push((std::mem::take(current), closed));
            }
        };
        for node in &self.nodes {
            match *node {
                Node::Move(p) => {
                    flush(&mut current, &mut out, false);
                    current.push(p);
                }
                Node::Line(p) => {
                    if current.last() != Some(&p) {
                        current.push(p);
                    }
                }
                Node::Curve(b, c, d) => {
                    let a = *current.last().expect("a curve starts at a point");
                    flatten(a, b, c, d, &mut |p| {
                        if current.last() != Some(&p) {
                            current.push(p);
                        }
                    });
                }
                Node::Close => {
                    let start = current.first().copied();
                    flush(&mut current, &mut out, true);
                    // z2d starts a closed subpath's successor at its first point.
                    if let Some(p) = start {
                        current.push(p);
                    }
                }
            }
        }
        flush(&mut current, &mut out, false);
        // A lone point left by a close isn't a subpath.
        out.retain(|(points, closed)| *closed || points.len() > 1);
        out
    }
}

/// `Spline.decompose`: the cubic `a b c d` as line segments, every point but `a`.
fn flatten(a: Point, b: Point, c: Point, d: Point, line_to: &mut impl FnMut(Point)) {
    if a == b && c == d {
        line_to(d);
        return;
    }
    let mut s1 = Knots { a, b, c, d };
    decompose_into(&mut s1, a, TOLERANCE * TOLERANCE, line_to);
    line_to(d);
}

#[derive(Clone, Copy)]
struct Knots {
    a: Point,
    b: Point,
    c: Point,
    d: Point,
}

fn decompose_into(s1: &mut Knots, start: Point, tolerance: f64, line_to: &mut impl FnMut(Point)) {
    if s1.error_sq() < tolerance {
        if s1.a != start {
            line_to(s1.a);
        }
        return;
    }
    let mut s2 = s1.de_casteljau();
    decompose_into(s1, start, tolerance, line_to);
    decompose_into(&mut s2, start, tolerance, line_to);
}

impl Knots {
    fn error_sq(&self) -> f64 {
        let (mut bx, mut by) = (self.b.x - self.a.x, self.b.y - self.a.y);
        let (mut cx, mut cy) = (self.c.x - self.a.x, self.c.y - self.a.y);
        if self.a != self.d {
            let (dx, dy) = (self.d.x - self.a.x, self.d.y - self.a.y);
            let d_sq = dx * dx + dy * dy;
            let bd = bx * dx + by * dy;
            if bd >= d_sq {
                bx -= dx;
                by -= dy;
            } else {
                bx -= bd / d_sq * dx;
                by -= bd / d_sq * dy;
            }
            let cd = cx * dx + cy * dy;
            if cd >= d_sq {
                cx -= dx;
                cy -= dy;
            } else {
                cx -= cd / d_sq * dx;
                cy -= cd / d_sq * dy;
            }
        }
        (bx * bx + by * by).max(cx * cx + cy * cy)
    }

    fn de_casteljau(&mut self) -> Knots {
        let half = |p: Point, q: Point| pt(p.x + (q.x - p.x) / 2.0, p.y + (q.y - p.y) / 2.0);
        let ab = half(self.a, self.b);
        let bc = half(self.b, self.c);
        let cd = half(self.c, self.d);
        let abbc = half(ab, bc);
        let bccd = half(bc, cd);
        let fin = half(abbc, bccd);
        let second = Knots {
            a: fin,
            b: bccd,
            c: cd,
            d: self.d,
        };
        self.b = ab;
        self.c = abbc;
        self.d = fin;
        second
    }
}

/// `arc_max_angle_for_tolerance_normalized`: the widest arc one cubic draws within `tolerance`
/// (in radii).
// z2d's table, digit for digit.
#[allow(clippy::excessive_precision)]
fn max_arc_angle(tolerance: f64) -> f64 {
    use std::f64::consts::PI;
    const TABLE: [(f64, f64); 11] = [
        (PI / 1.0, 0.0185185185185185036127),
        (PI / 2.0, 0.000272567143730179811158),
        (PI / 3.0, 2.38647043651461047433e-05),
        (PI / 4.0, 4.2455377443222443279e-06),
        (PI / 5.0, 1.11281001494389081528e-06),
        (PI / 6.0, 3.72662000942734705475e-07),
        (PI / 7.0, 1.47783685574284411325e-07),
        (PI / 8.0, 6.63240432022601149057e-08),
        (PI / 9.0, 3.2715520137536980553e-08),
        (PI / 10.0, 1.73863223499021216974e-08),
        (PI / 11.0, 9.81410988043554039085e-09),
    ];
    if let Some(&(angle, _)) = TABLE.iter().find(|&&(_, err)| err < tolerance) {
        return angle;
    }
    let mut angle = PI / TABLE.len() as f64;
    for i in TABLE.len()..1000 {
        angle = PI / i as f64;
        let err = 2.0 / 27.0 * (angle / 4.0).sin().powi(6) / (angle / 4.0).cos().powi(2);
        if err <= tolerance {
            break;
        }
    }
    angle
}

/// A polygon edge at 4× scale: `Polygon.Edge`.
#[derive(Clone, Copy, Debug)]
struct Edge {
    top: f64,
    bottom: f64,
    /// x at `top`.
    x_start: f64,
    x_inc: f64,
    /// −1 for an edge running down, 1 up.
    dir: i32,
}

/// Edges at 4× scale, with their extents.
#[derive(Default)]
pub struct Polygon {
    edges: Vec<Edge>,
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
}

impl Polygon {
    fn add_edge(&mut self, p0: Point, p1: Point) {
        let (p0, p1) = (
            pt(p0.x * SCALE, p0.y * SCALE),
            pt(p1.x * SCALE, p1.y * SCALE),
        );
        let edge = if p0.y < p1.y {
            Edge {
                top: p0.y,
                bottom: p1.y,
                x_start: p0.x,
                x_inc: (p1.x - p0.x) / (p1.y - p0.y),
                dir: -1,
            }
        } else if p0.y > p1.y {
            Edge {
                top: p1.y,
                bottom: p0.y,
                x_start: p1.x,
                x_inc: (p0.x - p1.x) / (p0.y - p1.y),
                dir: 1,
            }
        } else {
            return;
        };
        let (left, right) = (p0.x.min(p1.x), p0.x.max(p1.x));
        if self.edges.is_empty() {
            (self.left, self.right, self.top, self.bottom) = (left, right, edge.top, edge.bottom);
        } else {
            self.left = self.left.min(left);
            self.right = self.right.max(right);
            self.top = self.top.min(edge.top);
            self.bottom = self.bottom.max(edge.bottom);
        }
        self.edges.push(edge);
    }

    /// A closed contour's edges, last point back to the first.
    pub fn add_contour(&mut self, points: &[Point]) {
        for w in points.windows(2) {
            self.add_edge(w[0], w[1]);
        }
        if let (Some(&first), Some(&last)) = (points.first(), points.last())
            && points.len() > 1
        {
            self.add_edge(last, first);
        }
    }

    /// `fill_plotter.plot`: every subpath's edges, an unclosed one included as z2d closes none
    /// itself (Ghostty's fills all close).
    pub fn from_fill(path: &Path) -> Polygon {
        let mut polygon = Polygon::default();
        for (points, closed) in path.polylines() {
            for w in points.windows(2) {
                polygon.add_edge(w[0], w[1]);
            }
            if closed && points.len() >= 3 && points.first() != points.last() {
                polygon.add_edge(*points.last().unwrap(), points[0]);
            }
        }
        polygon
    }

    /// The outline of `path` stroked `width` wide, as contours wound the same way.
    pub fn from_stroke(path: &Path, width: f64, cap: Cap, join: Join) -> Polygon {
        let mut polygon = Polygon::default();
        let half = width / 2.0;
        let pen = pen(half);
        let disc = |polygon: &mut Polygon, at: Point| {
            let ring: Vec<Point> = pen.iter().map(|v| pt(at.x + v.x, at.y + v.y)).collect();
            polygon.add_wound(&ring);
        };
        for (sub, curve_joins) in path.stroke_polylines() {
            let (points, closed) = sub;
            if points.len() < 2 {
                continue;
            }
            for w in points.windows(2) {
                let (a, b) = (w[0], w[1]);
                let (nx, ny) = normal(a, b, half);
                polygon.add_wound(&[
                    pt(a.x + nx, a.y + ny),
                    pt(b.x + nx, b.y + ny),
                    pt(b.x - nx, b.y - ny),
                    pt(a.x - nx, a.y - ny),
                ]);
            }
            let n = points.len();
            let mut joint = |i_prev: usize, i: usize, i_next: usize, round: bool| {
                let (p0, p1, p2) = (points[i_prev], points[i], points[i_next]);
                if round || join == Join::Round {
                    disc(&mut polygon, p1);
                } else {
                    polygon.miter(p0, p1, p2, half);
                }
            };
            for (i, &round) in curve_joins.iter().enumerate().take(n - 1).skip(1) {
                joint(i - 1, i, i + 1, round);
            }
            if closed && n >= 3 {
                joint(n - 2, n - 1, 0, false);
                joint(n - 1, 0, 1, false);
            } else if cap == Cap::Round {
                disc(&mut polygon, points[0]);
                disc(&mut polygon, points[n - 1]);
            }
        }
        polygon
    }

    /// A contour, reversed if needed so every contour winds the same way and non-zero fills
    /// their union.
    fn add_wound(&mut self, points: &[Point]) {
        let area: f64 = points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .map(|(p, q)| p.x * q.y - q.x * p.y)
            .sum();
        if area > 0.0 {
            self.add_contour(points);
        } else if area < 0.0 {
            let reversed: Vec<Point> = points.iter().rev().copied().collect();
            self.add_contour(&reversed);
        }
    }

    /// The outer wedge of a miter join at `p1`, or a bevel past z2d's miter limit of 10.
    fn miter(&mut self, p0: Point, p1: Point, p2: Point, half: f64) {
        let (ax, ay) = (p1.x - p0.x, p1.y - p0.y);
        let (bx, by) = (p2.x - p1.x, p2.y - p1.y);
        let cross = ax * by - ay * bx;
        if cross.abs() < 1e-12 {
            return;
        }
        // The outer side is left of the turn's direction for a clockwise (y-down) turn.
        let side = if cross > 0.0 { -1.0 } else { 1.0 };
        let (n0x, n0y) = normal(p0, p1, half);
        let (n1x, n1y) = normal(p1, p2, half);
        let o0 = pt(p1.x + side * n0x, p1.y + side * n0y);
        let o1 = pt(p1.x + side * n1x, p1.y + side * n1y);
        let (la, lb) = ((ax * ax + ay * ay).sqrt(), (bx * bx + by * by).sqrt());
        let cos = (ax * bx + ay * by) / (la * lb);
        // z2d's limit: the miter's length over the line's width, 1 / sin(φ/2) for the angle φ
        // between the two lines, which is 1 / cos(α/2) for the turn α.
        let cos_half_turn = ((1.0 + cos) / 2.0).sqrt();
        if cos_half_turn * 10.0 >= 1.0 {
            // The tip: o0 moved along the incoming line until it meets the outgoing offset line.
            let (dx, dy) = (ax / la, ay / la);
            let (ex, ey) = (bx / lb, by / lb);
            let denom = dx * ey - dy * ex;
            let t = ((o1.x - o0.x) * ey - (o1.y - o0.y) * ex) / denom;
            let tip = pt(o0.x + t * dx, o0.y + t * dy);
            self.add_wound(&[p1, o0, tip, o1]);
        } else {
            self.add_wound(&[p1, o0, o1]);
        }
    }

    /// `multisample.run` into an alpha8 canvas `width` × `height`, compositing source-over.
    pub fn rasterize(&self, rule: FillRule, buf: &mut [u8], width: i32, height: i32) {
        if self.edges.is_empty() || !self.in_box(width, height) {
            return;
        }
        const FULL: u32 = 16;
        let start_y = ((self.top / SCALE).floor() as i32).clamp(0, height - 1);
        let end_y = ((self.bottom / SCALE).ceil() as i32).clamp(start_y, height - 1);
        let start_x = ((self.left / SCALE).floor() as i32).clamp(0, width - 1);
        let end_x = ((self.right / SCALE).ceil() as i32).clamp(start_x, width);
        let draw_w = end_x - start_x;
        if draw_w < 1 {
            return;
        }
        let start_x_scaled = start_x * SCALE as i32;
        let draw_w_scaled = draw_w * SCALE as i32;
        let mut coverage = vec![0u32; draw_w as usize];
        let mut xs: Vec<(i32, i32)> = Vec::with_capacity(self.edges.len());
        for y in start_y..=end_y {
            coverage.iter_mut().for_each(|c| *c = 0);
            for sub in 0..SCALE as i32 {
                let ys = y * SCALE as i32 + sub;
                let mid = f64::from(ys) + 0.5;
                xs.clear();
                for e in &self.edges {
                    if e.top < mid && e.bottom >= mid {
                        let x = (e.x_start + e.x_inc * (mid - e.top)).round() as i32;
                        xs.push((x, e.dir));
                    }
                }
                xs.sort_unstable_by_key(|&(x, _)| x);
                let crossings: Vec<i32> = match rule {
                    FillRule::EvenOdd => xs.iter().map(|&(x, _)| x).collect(),
                    FillRule::NonZero => {
                        let mut kept = Vec::with_capacity(xs.len());
                        let mut winding = 0;
                        for &(x, dir) in &xs {
                            if winding == 0 {
                                winding += dir;
                                kept.push(x);
                            } else {
                                winding += dir;
                                if winding == 0 {
                                    kept.push(x);
                                }
                            }
                        }
                        kept
                    }
                };
                let mut x_min = 0;
                for pair in crossings.as_chunks::<2>().0 {
                    let s = x_min.max(pair[0] - start_x_scaled);
                    if s >= draw_w_scaled {
                        break;
                    }
                    let e = (pair[1] - start_x_scaled).clamp(s, draw_w_scaled);
                    add_span(&mut coverage, s as u32, (e - s) as u32);
                    x_min = e;
                }
            }
            let row = (y * width) as usize;
            for (i, &c) in coverage.iter().enumerate() {
                let x = start_x as usize + i;
                let dst = &mut buf[row + x];
                match c.min(FULL) {
                    0 => {}
                    FULL => *dst = 255,
                    c => {
                        let sa = (c * 16 - 1).min(255);
                        let da = u32::from(*dst);
                        *dst = (sa + da - sa * da / 255) as u8;
                    }
                }
            }
        }
    }

    /// `Polygon.inBox`: whether any of the polygon falls on the canvas.
    fn in_box(&self, width: i32, height: i32) -> bool {
        if self.right <= 0.0 || self.bottom <= 0.0 {
            return false;
        }
        let x0 = (self.left / SCALE).floor() as i32;
        let y0 = (self.top / SCALE).floor() as i32;
        let w = (self.right / SCALE).ceil() as i32 - x0;
        let h = (self.bottom / SCALE).ceil() as i32 - y0;
        w > 0 && h > 0 && x0 + w >= 0 && y0 + h >= 0 && x0 < width && y0 < height
    }
}

/// Adds coverage for sub-pixels `x..x + len` (4× scale) of one sub-scanline.
fn add_span(coverage: &mut [u32], x: u32, len: u32) {
    let scale = SCALE as u32;
    let (mut at, end) = (x, x + len);
    while at < end {
        let px = at / scale;
        let next = ((px + 1) * scale).min(end);
        if let Some(c) = coverage.get_mut(px as usize) {
            *c += next - at;
        }
        at = next;
    }
}

/// `normal`: the offset `half` to the right of `a → b` (y down).
fn normal(a: Point, b: Point, half: f64) -> (f64, f64) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt();
    if len == 0.0 {
        return (0.0, 0.0);
    }
    (-dy / len * half, dx / len * half)
}

/// z2d's pen (`Pen.init`): a polygon around the origin for a round join or cap of radius
/// `radius`, with as many vertices as the curve tolerance needs.
fn pen(radius: f64) -> Vec<Point> {
    let n = if TOLERANCE >= radius * 4.0 {
        1
    } else if TOLERANCE >= radius {
        4
    } else {
        let delta = (1.0 - TOLERANCE / radius).acos();
        if delta == 0.0 {
            4
        } else {
            let n = (2.0 * std::f64::consts::PI / delta).ceil() as usize;
            if n < 4 {
                4
            } else if !n.is_multiple_of(2) {
                n + 1
            } else {
                n
            }
        }
    };
    (0..n)
        .map(|i| {
            let theta = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
            pt(radius * theta.cos(), radius * theta.sin())
        })
        .collect()
}

impl Path {
    /// Subpaths for stroking, with which points join two pieces of a flattened curve: z2d
    /// joins those round whatever the line join is.
    #[allow(clippy::type_complexity)]
    fn stroke_polylines(&self) -> Vec<((Vec<Point>, bool), Vec<bool>)> {
        let mut out = Vec::new();
        let mut points: Vec<Point> = Vec::new();
        let mut round: Vec<bool> = Vec::new();
        let flush = |points: &mut Vec<Point>, round: &mut Vec<bool>, out: &mut Vec<_>, closed| {
            if points.len() > 1 {
                out.push(((std::mem::take(points), closed), std::mem::take(round)));
            } else {
                points.clear();
                round.clear();
            }
        };
        for node in &self.nodes {
            match *node {
                Node::Move(p) => {
                    flush(&mut points, &mut round, &mut out, false);
                    points.push(p);
                    round.push(false);
                }
                Node::Line(p) => {
                    if points.last() != Some(&p) {
                        points.push(p);
                        round.push(false);
                    }
                }
                Node::Curve(b, c, d) => {
                    let a = *points.last().expect("a curve starts at a point");
                    // The join where the curve starts is made as its first piece is added,
                    // so it's round too; the one where it ends takes the next line's join.
                    if let Some(last) = round.last_mut() {
                        *last = true;
                    }
                    flatten(a, b, c, d, &mut |p| {
                        if points.last() != Some(&p) {
                            points.push(p);
                            round.push(true);
                        }
                    });
                    if let Some(last) = round.last_mut() {
                        *last = false;
                    }
                }
                Node::Close => {
                    let start = points.first().copied();
                    flush(&mut points, &mut round, &mut out, true);
                    if let Some(p) = start {
                        points.push(p);
                        round.push(false);
                    }
                }
            }
        }
        flush(&mut points, &mut round, &mut out, false);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raster(polygon: &Polygon, w: i32, h: i32) -> Vec<u8> {
        let mut buf = vec![0; (w * h) as usize];
        polygon.rasterize(FillRule::NonZero, &mut buf, w, h);
        buf
    }

    #[test]
    fn a_pixel_aligned_box_is_solid_with_hard_edges() {
        let path = Path::new()
            .move_to(1.0, 1.0)
            .line_to(3.0, 1.0)
            .line_to(3.0, 3.0)
            .line_to(1.0, 3.0)
            .close();
        let buf = raster(&Polygon::from_fill(&path), 4, 4);
        #[rustfmt::skip]
        assert_eq!(buf, [
            0, 0, 0, 0,
            0, 255, 255, 0,
            0, 255, 255, 0,
            0, 0, 0, 0,
        ]);
    }

    #[test]
    fn half_covered_pixels_get_z2ds_alpha() {
        // A box from x = 0.5: the first column is half covered, 8 of 16 samples.
        let path = Path::new()
            .move_to(0.5, 0.0)
            .line_to(2.0, 0.0)
            .line_to(2.0, 1.0)
            .line_to(0.5, 1.0)
            .close();
        let buf = raster(&Polygon::from_fill(&path), 2, 1);
        assert_eq!(buf, [8 * 16 - 1, 255]);
    }

    #[test]
    fn overlapping_strokes_union_without_double_coverage() {
        // Two strokes over the same pixels fill them once, at full coverage.
        let path = Path::new().move_to(0.0, 1.0).line_to(4.0, 1.0);
        let mut polygon = Polygon::from_stroke(&path, 2.0, Cap::Butt, Join::Miter);
        for e in Polygon::from_stroke(&path, 2.0, Cap::Butt, Join::Miter).edges {
            polygon.edges.push(e);
        }
        let buf = raster(&polygon, 4, 2);
        assert!(buf.iter().all(|&a| a == 255), "{buf:?}");
    }

    #[test]
    fn a_curve_flattens_within_tolerance_and_ends_on_its_end() {
        let mut points = Vec::new();
        flatten(
            pt(0.0, 0.0),
            pt(0.0, 10.0),
            pt(10.0, 10.0),
            pt(10.0, 0.0),
            &mut |p| points.push(p),
        );
        assert!(points.len() > 4);
        assert_eq!(*points.last().unwrap(), pt(10.0, 0.0));
    }

    #[test]
    fn the_pen_for_a_2px_line_has_14_vertices() {
        assert_eq!(pen(1.0).len(), 14);
    }
}
