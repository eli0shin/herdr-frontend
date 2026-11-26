//! Box Drawing, U+2500 to U+257F: a port of `ghostty:src/font/sprite/draw/box.zig` at `v1.4.0`.
//! `linesChar` draws the 109 line characters, the rest are dashes, arcs and diagonals.

use terminal::Metrics;

use super::PxRect;
use super::canvas::Canvas;
use super::raster::{Cap, Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    None,
    Light,
    Heavy,
    Double,
}

/// The four arms of a box-drawing character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Lines {
    up: Line,
    right: Line,
    down: Line,
    left: Line,
}

const fn arm(weight: u8) -> Line {
    match weight {
        1 => Line::Light,
        2 => Line::Heavy,
        3 => Line::Double,
        _ => Line::None,
    }
}

/// Arms up, right, down, left: 0 none, 1 light, 2 heavy, 3 double.
const fn l(up: u8, right: u8, down: u8, left: u8) -> Lines {
    Lines {
        up: arm(up),
        right: arm(right),
        down: arm(down),
        left: arm(left),
    }
}

/// `box.zig`'s `linesChar` entries, in code point order.
const LINES: &[(u32, Lines)] = &[
    (0x2500, l(0, 1, 0, 1)),
    (0x2501, l(0, 2, 0, 2)),
    (0x2502, l(1, 0, 1, 0)),
    (0x2503, l(2, 0, 2, 0)),
    (0x250C, l(0, 1, 1, 0)),
    (0x250D, l(0, 2, 1, 0)),
    (0x250E, l(0, 1, 2, 0)),
    (0x250F, l(0, 2, 2, 0)),
    (0x2510, l(0, 0, 1, 1)),
    (0x2511, l(0, 0, 1, 2)),
    (0x2512, l(0, 0, 2, 1)),
    (0x2513, l(0, 0, 2, 2)),
    (0x2514, l(1, 1, 0, 0)),
    (0x2515, l(1, 2, 0, 0)),
    (0x2516, l(2, 1, 0, 0)),
    (0x2517, l(2, 2, 0, 0)),
    (0x2518, l(1, 0, 0, 1)),
    (0x2519, l(1, 0, 0, 2)),
    (0x251A, l(2, 0, 0, 1)),
    (0x251B, l(2, 0, 0, 2)),
    (0x251C, l(1, 1, 1, 0)),
    (0x251D, l(1, 2, 1, 0)),
    (0x251E, l(2, 1, 1, 0)),
    (0x251F, l(1, 1, 2, 0)),
    (0x2520, l(2, 1, 2, 0)),
    (0x2521, l(2, 2, 1, 0)),
    (0x2522, l(1, 2, 2, 0)),
    (0x2523, l(2, 2, 2, 0)),
    (0x2524, l(1, 0, 1, 1)),
    (0x2525, l(1, 0, 1, 2)),
    (0x2526, l(2, 0, 1, 1)),
    (0x2527, l(1, 0, 2, 1)),
    (0x2528, l(2, 0, 2, 1)),
    (0x2529, l(2, 0, 1, 2)),
    (0x252A, l(1, 0, 2, 2)),
    (0x252B, l(2, 0, 2, 2)),
    (0x252C, l(0, 1, 1, 1)),
    (0x252D, l(0, 1, 1, 2)),
    (0x252E, l(0, 2, 1, 1)),
    (0x252F, l(0, 2, 1, 2)),
    (0x2530, l(0, 1, 2, 1)),
    (0x2531, l(0, 1, 2, 2)),
    (0x2532, l(0, 2, 2, 1)),
    (0x2533, l(0, 2, 2, 2)),
    (0x2534, l(1, 1, 0, 1)),
    (0x2535, l(1, 1, 0, 2)),
    (0x2536, l(1, 2, 0, 1)),
    (0x2537, l(1, 2, 0, 2)),
    (0x2538, l(2, 1, 0, 1)),
    (0x2539, l(2, 1, 0, 2)),
    (0x253A, l(2, 2, 0, 1)),
    (0x253B, l(2, 2, 0, 2)),
    (0x253C, l(1, 1, 1, 1)),
    (0x253D, l(1, 1, 1, 2)),
    (0x253E, l(1, 2, 1, 1)),
    (0x253F, l(1, 2, 1, 2)),
    (0x2540, l(2, 1, 1, 1)),
    (0x2541, l(1, 1, 2, 1)),
    (0x2542, l(2, 1, 2, 1)),
    (0x2543, l(2, 1, 1, 2)),
    (0x2544, l(2, 2, 1, 1)),
    (0x2545, l(1, 1, 2, 2)),
    (0x2546, l(1, 2, 2, 1)),
    (0x2547, l(2, 2, 1, 2)),
    (0x2548, l(1, 2, 2, 2)),
    (0x2549, l(2, 1, 2, 2)),
    (0x254A, l(2, 2, 2, 1)),
    (0x254B, l(2, 2, 2, 2)),
    (0x2550, l(0, 3, 0, 3)),
    (0x2551, l(3, 0, 3, 0)),
    (0x2552, l(0, 3, 1, 0)),
    (0x2553, l(0, 1, 3, 0)),
    (0x2554, l(0, 3, 3, 0)),
    (0x2555, l(0, 0, 1, 3)),
    (0x2556, l(0, 0, 3, 1)),
    (0x2557, l(0, 0, 3, 3)),
    (0x2558, l(1, 3, 0, 0)),
    (0x2559, l(3, 1, 0, 0)),
    (0x255A, l(3, 3, 0, 0)),
    (0x255B, l(1, 0, 0, 3)),
    (0x255C, l(3, 0, 0, 1)),
    (0x255D, l(3, 0, 0, 3)),
    (0x255E, l(1, 3, 1, 0)),
    (0x255F, l(3, 1, 3, 0)),
    (0x2560, l(3, 3, 3, 0)),
    (0x2561, l(1, 0, 1, 3)),
    (0x2562, l(3, 0, 3, 1)),
    (0x2563, l(3, 0, 3, 3)),
    (0x2564, l(0, 3, 1, 3)),
    (0x2565, l(0, 1, 3, 1)),
    (0x2566, l(0, 3, 3, 3)),
    (0x2567, l(1, 3, 0, 3)),
    (0x2568, l(3, 1, 0, 1)),
    (0x2569, l(3, 3, 0, 3)),
    (0x256A, l(1, 3, 1, 3)),
    (0x256B, l(3, 1, 3, 1)),
    (0x256C, l(3, 3, 3, 3)),
    (0x2574, l(0, 0, 0, 1)),
    (0x2575, l(1, 0, 0, 0)),
    (0x2576, l(0, 1, 0, 0)),
    (0x2577, l(0, 0, 1, 0)),
    (0x2578, l(0, 0, 0, 2)),
    (0x2579, l(2, 0, 0, 0)),
    (0x257A, l(0, 2, 0, 0)),
    (0x257B, l(0, 0, 2, 0)),
    (0x257C, l(0, 2, 0, 1)),
    (0x257D, l(1, 0, 2, 0)),
    (0x257E, l(0, 1, 0, 2)),
    (0x257F, l(2, 0, 1, 0)),
];

/// The boxes Ghostty fills for `c`, if it's a box-drawing line character.
pub(super) fn lines(c: char, metrics: &Metrics) -> Option<Vec<PxRect>> {
    let at = LINES
        .binary_search_by_key(&(c as u32), |&(cp, _)| cp)
        .ok()?;
    Some(lines_char(metrics, LINES[at].1))
}

/// Draws box-drawing character `cp` (U+2500 to U+257F) on `canvas`.
pub(super) fn draw(cp: u32, canvas: &mut Canvas, m: &Metrics) {
    let light = m.box_thickness;
    let heavy = m.box_thickness * 2;
    let gap = light.max(4);
    match cp {
        0x2504 => dash_horizontal(m, canvas, 3, light, gap),
        0x2505 => dash_horizontal(m, canvas, 3, heavy, gap),
        0x2506 => dash_vertical(m, canvas, 3, light, gap),
        0x2507 => dash_vertical(m, canvas, 3, heavy, gap),
        0x2508 => dash_horizontal(m, canvas, 4, light, gap),
        0x2509 => dash_horizontal(m, canvas, 4, heavy, gap),
        0x250A => dash_vertical(m, canvas, 4, light, gap),
        0x250B => dash_vertical(m, canvas, 4, heavy, gap),
        0x254C => dash_horizontal(m, canvas, 2, light, light),
        0x254D => dash_horizontal(m, canvas, 2, heavy, heavy),
        0x254E => dash_vertical(m, canvas, 2, light, heavy),
        0x254F => dash_vertical(m, canvas, 2, heavy, heavy),
        0x256D => arc(m, canvas, Corner::BottomRight),
        0x256E => arc(m, canvas, Corner::BottomLeft),
        0x256F => arc(m, canvas, Corner::TopLeft),
        0x2570 => arc(m, canvas, Corner::TopRight),
        0x2571 => diagonal_up(m, canvas),
        0x2572 => diagonal_down(m, canvas),
        0x2573 => {
            diagonal_up(m, canvas);
            diagonal_down(m, canvas);
        }
        _ => {
            if let Some(boxes) = char::from_u32(cp).and_then(|c| lines(c, m)) {
                for b in boxes {
                    canvas.box_(b.x0 as i32, b.y0 as i32, b.x1 as i32, b.y1 as i32, 255);
                }
            }
        }
    }
}

/// The corner an arc bends toward: `╭` goes from the bottom to the right.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// `arc`: a light line from the cell's edge to the middle, turning with a cubic of radius half the
/// cell's smaller side, stroked with butt caps.
fn arc(m: &Metrics, canvas: &mut Canvas, corner: Corner) {
    let thick = m.box_thickness;
    let (w, h, t) = (
        f64::from(m.cell_width),
        f64::from(m.cell_height),
        f64::from(thick),
    );
    let cx = f64::from(m.cell_width.saturating_sub(thick) / 2) + t / 2.0;
    let cy = f64::from(m.cell_height.saturating_sub(thick) / 2) + t / 2.0;
    let r = w.min(h) / 2.0;
    let s = 0.25;
    let (edge_y, toward_y) = match corner {
        Corner::TopLeft | Corner::TopRight => (0.0, -1.0),
        Corner::BottomLeft | Corner::BottomRight => (h, 1.0),
    };
    let (edge_x, toward_x) = match corner {
        Corner::TopLeft | Corner::BottomLeft => (0.0, -1.0),
        Corner::TopRight | Corner::BottomRight => (w, 1.0),
    };
    let path = Path::new()
        .move_to(cx, edge_y)
        .line_to(cx, cy + toward_y * r)
        .curve_to(
            cx,
            cy + toward_y * s * r,
            cx + toward_x * s * r,
            cy,
            cx + toward_x * r,
            cy,
        )
        .line_to(edge_x, cy);
    canvas.stroke(&path, t, Cap::Butt);
}

/// `╱`: a light line from the top right to the bottom left, overshooting the corners a little
/// along its slope.
fn diagonal_up(m: &Metrics, canvas: &mut Canvas) {
    let (w, h) = (f64::from(m.cell_width), f64::from(m.cell_height));
    let (sx, sy) = ((w / h).min(1.0), (h / w).min(1.0));
    canvas.line(
        (w + 0.5 * sx, -0.5 * sy),
        (-0.5 * sx, h + 0.5 * sy),
        f64::from(m.box_thickness),
    );
}

/// `╲`: a light line from the top left to the bottom right.
fn diagonal_down(m: &Metrics, canvas: &mut Canvas) {
    let (w, h) = (f64::from(m.cell_width), f64::from(m.cell_height));
    let (sx, sy) = ((w / h).min(1.0), (h / w).min(1.0));
    canvas.line(
        (-0.5 * sx, -0.5 * sy),
        (w + 0.5 * sx, h + 0.5 * sy),
        f64::from(m.box_thickness),
    );
}

/// `dashHorizontal`: `count` dashes that tile seamlessly, with half a gap at each end and any
/// spare pixels added to the dashes.
fn dash_horizontal(m: &Metrics, canvas: &mut Canvas, count: u32, thick: u32, desired_gap: u32) {
    if m.cell_width < 2 * count {
        let t = m.box_thickness;
        canvas.box_(
            0,
            (m.cell_height.saturating_sub(t) / 2) as i32,
            m.cell_width as i32,
            (m.cell_height.saturating_sub(t) / 2 + t) as i32,
            255,
        );
        return;
    }
    let gap = desired_gap.min(m.cell_width / (2 * count)) as i32;
    let total_dash = m.cell_width as i32 - count as i32 * gap;
    let dash = total_dash.div_euclid(count as i32);
    let mut extra = total_dash.rem_euclid(count as i32);
    let y = (m.cell_height.saturating_sub(thick) / 2) as i32;
    let mut x = gap.div_euclid(2);
    for _ in 0..count {
        let mut x1 = x + dash;
        if extra > 0 {
            extra -= 1;
            x1 += 1;
        }
        canvas.box_(x, y, x1, y + thick as i32, 255);
        x = x1 + gap;
    }
}

/// `dashVertical`: `count` dashes from the top, with a whole gap at the bottom.
fn dash_vertical(m: &Metrics, canvas: &mut Canvas, count: u32, thick: u32, desired_gap: u32) {
    if m.cell_height < 2 * count {
        let t = m.box_thickness;
        let x = (m.cell_width.saturating_sub(t) / 2) as i32;
        canvas.box_(x, 0, x + t as i32, m.cell_height as i32, 255);
        return;
    }
    let gap = desired_gap.min(m.cell_height / (2 * count)) as i32;
    let total_dash = m.cell_height as i32 - count as i32 * gap;
    let dash = total_dash.div_euclid(count as i32);
    let mut extra = total_dash.rem_euclid(count as i32);
    let x = (m.cell_width.saturating_sub(thick) / 2) as i32;
    let mut y = 0;
    for _ in 0..count {
        let mut y1 = y + dash;
        if extra > 0 {
            extra -= 1;
            y1 += 1;
        }
        canvas.box_(x, y, x + thick as i32, y1, 255);
        y = y1 + gap;
    }
}

/// `linesChar`: each arm as a box from the cell's edge to where the arms meet, with Ghostty's
/// whole-pixel thicknesses and joins.
fn lines_char(metrics: &Metrics, lines: Lines) -> Vec<PxRect> {
    use Line::*;

    let (width, height) = (metrics.cell_width, metrics.cell_height);
    let light_px = metrics.box_thickness;
    let heavy_px = metrics.box_thickness * 2;

    let h_light_top = height.saturating_sub(light_px) / 2;
    let h_light_bottom = h_light_top.saturating_add(light_px);
    let h_heavy_top = height.saturating_sub(heavy_px) / 2;
    let h_heavy_bottom = h_heavy_top.saturating_add(heavy_px);
    let h_double_top = h_light_top.saturating_sub(light_px);
    let h_double_bottom = h_light_bottom.saturating_add(light_px);

    let v_light_left = width.saturating_sub(light_px) / 2;
    let v_light_right = v_light_left.saturating_add(light_px);
    let v_heavy_left = width.saturating_sub(heavy_px) / 2;
    let v_heavy_right = v_heavy_left.saturating_add(heavy_px);
    let v_double_left = v_light_left.saturating_sub(light_px);
    let v_double_right = v_light_right.saturating_add(light_px);

    let any = |a: Line, b: Line, line: Line| a == line || b == line;

    let up_bottom = if any(lines.left, lines.right, Heavy) {
        h_heavy_bottom
    } else if lines.left != lines.right || lines.down == lines.up {
        if any(lines.left, lines.right, Double) {
            h_double_bottom
        } else {
            h_light_bottom
        }
    } else if lines.left == None && lines.right == None {
        h_light_bottom
    } else {
        h_light_top
    };

    let down_top = if any(lines.left, lines.right, Heavy) {
        h_heavy_top
    } else if lines.left != lines.right || lines.up == lines.down {
        if any(lines.left, lines.right, Double) {
            h_double_top
        } else {
            h_light_top
        }
    } else if lines.left == None && lines.right == None {
        h_light_top
    } else {
        h_light_bottom
    };

    let left_right = if any(lines.up, lines.down, Heavy) {
        v_heavy_right
    } else if lines.up != lines.down || lines.left == lines.right {
        if any(lines.up, lines.down, Double) {
            v_double_right
        } else {
            v_light_right
        }
    } else if lines.up == None && lines.down == None {
        v_light_right
    } else {
        v_light_left
    };

    let right_left = if any(lines.up, lines.down, Heavy) {
        v_heavy_left
    } else if lines.up != lines.down || lines.right == lines.left {
        if any(lines.up, lines.down, Double) {
            v_double_left
        } else {
            v_light_left
        }
    } else if lines.up == None && lines.down == None {
        v_light_left
    } else {
        v_light_right
    };

    let mut boxes = Vec::new();
    let mut rect = |x0, y0, x1, y1| {
        // `Canvas.box` fills nothing for an empty box.
        if x1 > x0 && y1 > y0 {
            boxes.push(PxRect { x0, y0, x1, y1 });
        }
    };

    match lines.up {
        None => {}
        Light => rect(v_light_left, 0, v_light_right, up_bottom),
        Heavy => rect(v_heavy_left, 0, v_heavy_right, up_bottom),
        Double => {
            let left_bottom = if lines.left == Double {
                h_light_top
            } else {
                up_bottom
            };
            let right_bottom = if lines.right == Double {
                h_light_top
            } else {
                up_bottom
            };
            rect(v_double_left, 0, v_light_left, left_bottom);
            rect(v_light_right, 0, v_double_right, right_bottom);
        }
    }

    match lines.right {
        None => {}
        Light => rect(right_left, h_light_top, width, h_light_bottom),
        Heavy => rect(right_left, h_heavy_top, width, h_heavy_bottom),
        Double => {
            let top_left = if lines.up == Double {
                v_light_right
            } else {
                right_left
            };
            let bottom_left = if lines.down == Double {
                v_light_right
            } else {
                right_left
            };
            rect(top_left, h_double_top, width, h_light_top);
            rect(bottom_left, h_light_bottom, width, h_double_bottom);
        }
    }

    match lines.down {
        None => {}
        Light => rect(v_light_left, down_top, v_light_right, height),
        Heavy => rect(v_heavy_left, down_top, v_heavy_right, height),
        Double => {
            let left_top = if lines.left == Double {
                h_light_bottom
            } else {
                down_top
            };
            let right_top = if lines.right == Double {
                h_light_bottom
            } else {
                down_top
            };
            rect(v_double_left, left_top, v_light_left, height);
            rect(v_light_right, right_top, v_double_right, height);
        }
    }

    match lines.left {
        None => {}
        Light => rect(0, h_light_top, left_right, h_light_bottom),
        Heavy => rect(0, h_heavy_top, left_right, h_heavy_bottom),
        Double => {
            let top_right = if lines.up == Double {
                v_light_left
            } else {
                left_right
            };
            let bottom_right = if lines.down == Double {
                v_light_left
            } else {
                left_right
            };
            rect(0, h_double_top, top_right, h_light_top);
            rect(0, h_light_bottom, bottom_right, h_double_bottom);
        }
    }

    boxes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// JetBrains Mono at 14 pt on a 2x display: a 17 × 37 cell with 2 px box lines.
    fn cell(width: u32, height: u32, box_thickness: u32) -> Metrics {
        let mut m = Metrics::calc(&terminal::FaceMetrics {
            px_per_em: 28.0,
            cell_width: 16.8,
            ascent: 28.56,
            descent: -8.4,
            ..Default::default()
        });
        (m.cell_width, m.cell_height, m.box_thickness) = (width, height, box_thickness);
        m
    }

    #[test]
    fn the_table_is_sorted_and_complete() {
        assert_eq!(LINES.len(), 109);
        assert!(LINES.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(lines('a', &cell(17, 37, 2)).is_none());
        assert!(lines('╭', &cell(17, 37, 2)).is_none(), "arcs aren't lines");
    }

    #[test]
    fn a_light_horizontal_runs_edge_to_edge_through_the_middle() {
        let m = cell(17, 37, 2);
        // Ghostty: top = (37 - 2) / 2 = 17; the left arm meets the right one at the light line.
        assert_eq!(
            lines('─', &m).unwrap(),
            [
                PxRect {
                    x0: 7,
                    y0: 17,
                    x1: 17,
                    y1: 19
                },
                PxRect {
                    x0: 0,
                    y0: 17,
                    x1: 9,
                    y1: 19
                },
            ]
        );
    }

    #[test]
    fn a_vertical_is_centered_in_whole_pixels() {
        let m = cell(10, 22, 2);
        assert_eq!(
            lines('│', &m).unwrap(),
            [
                PxRect {
                    x0: 4,
                    y0: 0,
                    x1: 6,
                    y1: 12
                },
                PxRect {
                    x0: 4,
                    y0: 10,
                    x1: 6,
                    y1: 22
                },
            ]
        );
    }

    #[test]
    fn a_heavy_cross_and_a_double_corner() {
        let m = cell(17, 37, 2);
        let cross = lines('╋', &m).unwrap();
        assert_eq!(cross.len(), 4);
        assert!(cross.iter().all(|r| r.x1 - r.x0 == 4 || r.y1 - r.y0 == 4));
        // ╔: the outer stroke of each arm turns the corner, the inner ones stop short of it.
        assert_eq!(
            lines('╔', &m).unwrap(),
            [
                PxRect {
                    x0: 5,
                    y0: 15,
                    x1: 17,
                    y1: 17
                },
                PxRect {
                    x0: 9,
                    y0: 19,
                    x1: 17,
                    y1: 21
                },
                PxRect {
                    x0: 5,
                    y0: 15,
                    x1: 7,
                    y1: 37
                },
                PxRect {
                    x0: 9,
                    y0: 19,
                    x1: 11,
                    y1: 37
                },
            ]
        );
    }
}
