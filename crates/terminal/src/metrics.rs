//! Ghostty's cell metrics: [`Metrics::calc`] and the `adjust-*` [`Modifiers`].
//!
//! A port of `ghostty:src/font/Metrics.zig` at `vt-1.0.2` (`573c4a21`). Every value a renderer
//! lays out with is a whole device pixel; only the face's own sizes stay fractional. See
//! `docs/research/terminal-rendering.md` §2 "Cell metrics identical to Ghostty".

/// The cell metrics of a font, in whole device pixels. Positions are measured from the top of the
/// cell, except `cell_baseline`, which is measured from the bottom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// Cell width; column `k` starts at `k × cell_width`.
    pub cell_width: u32,
    /// Cell height; row `k` starts at `k × cell_height`.
    pub cell_height: u32,
    /// Distance from the bottom of the cell to the text baseline.
    pub cell_baseline: u32,
    /// Distance from the top of the cell to the top of the underline.
    pub underline_position: u32,
    pub underline_thickness: u32,
    /// Distance from the top of the cell to the top of the strikethrough.
    pub strikethrough_position: u32,
    pub strikethrough_thickness: u32,
    /// Distance from the top of the cell to the top of the overline. Negative is above the cell.
    pub overline_position: i32,
    pub overline_thickness: u32,
    /// Line thickness of box drawing sprites.
    pub box_thickness: u32,
    /// Thickness of the bar and hollow cursors. Set by configuration, not by the font.
    pub cursor_thickness: u32,
    pub cursor_height: u32,
    /// Constraint height for Nerd Font icons.
    pub icon_height: f64,
    /// Constraint height for Nerd Font icons limited to one cell width.
    pub icon_height_single: f64,
    /// The unrounded face width, for scaling.
    pub face_width: f64,
    /// The unrounded face height, for scaling.
    pub face_height: f64,
    /// Offset from the bottom of the cell to the bottom of the face's bounding box.
    pub face_y: f64,
}

/// Metrics read from a font face, in pixels at the face's size. Vertical positions are relative to
/// the baseline with +Y up. `None` means the font doesn't say, and the getters estimate it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FaceMetrics {
    /// Pixels per em; dividing the other values by it gives sizes in ems.
    pub px_per_em: f64,
    /// The widest advance among printable ASCII (0x20–0x7E).
    pub cell_width: f64,
    pub ascent: f64,
    /// Below the baseline, so usually negative.
    pub descent: f64,
    pub line_gap: f64,
    /// The top of the underline stroke.
    pub underline_position: Option<f64>,
    pub underline_thickness: Option<f64>,
    /// The top of the strikethrough stroke.
    pub strikethrough_position: Option<f64>,
    pub strikethrough_thickness: Option<f64>,
    pub cap_height: Option<f64>,
    pub ex_height: Option<f64>,
    /// Height of the box around every printable ASCII glyph.
    pub ascii_height: Option<f64>,
    /// Advance of "水" (U+6C34), to normalize CJK fallback fonts.
    pub ic_width: Option<f64>,
}

fn positive(value: Option<f64>) -> Option<f64> {
    value.filter(|v| *v > 0.0)
}

impl FaceMetrics {
    /// `ascent − descent + line_gap`.
    pub fn line_height(&self) -> f64 {
        self.ascent - self.descent + self.line_gap
    }

    /// The cap height, or 75% of the ascent.
    pub fn cap_height(&self) -> f64 {
        positive(self.cap_height).unwrap_or(0.75 * self.ascent)
    }

    /// The ex height, or 75% of the cap height.
    pub fn ex_height(&self) -> f64 {
        positive(self.ex_height).unwrap_or(0.75 * self.cap_height())
    }

    /// The ASCII height, or 1.5 × the cap height.
    pub fn ascii_height(&self) -> f64 {
        positive(self.ascii_height).unwrap_or(1.5 * self.cap_height())
    }

    /// The ideograph width, or the smaller of the ASCII height and two cells.
    pub fn ic_width(&self) -> f64 {
        positive(self.ic_width).unwrap_or(self.ascii_height().min(2.0 * self.cell_width))
    }

    /// The underline thickness, or 15% of the ex height.
    pub fn underline_thickness(&self) -> f64 {
        positive(self.underline_thickness).unwrap_or(0.15 * self.ex_height())
    }

    /// The strikethrough thickness, or the underline thickness.
    pub fn strikethrough_thickness(&self) -> f64 {
        positive(self.strikethrough_thickness).unwrap_or_else(|| self.underline_thickness())
    }

    /// The underline position, or one underline thickness below the baseline.
    pub fn underline_position(&self) -> f64 {
        self.underline_position
            .unwrap_or_else(|| -self.underline_thickness())
    }

    /// The strikethrough position, or centered at half the ex height.
    pub fn strikethrough_position(&self) -> f64 {
        self.strikethrough_position
            .unwrap_or_else(|| (self.ex_height() + self.strikethrough_thickness()) * 0.5)
    }
}

/// `@intFromFloat` for values `calc` has already rounded. Zig traps on a negative value here; the
/// cast saturates to 0 and [`Metrics::clamp`] then applies the minimums.
fn whole(value: f64) -> u32 {
    value as u32
}

impl Metrics {
    /// Ghostty's `Metrics.calc`: the cell metrics for a face, before modifiers. Pass the face's
    /// values unrounded.
    pub fn calc(face: &FaceMetrics) -> Metrics {
        let face_width = face.cell_width;
        let face_height = face.line_height();

        // Round, not ceil, so the cell stays within half a pixel of the font's design.
        let cell_width = face_width.round();
        let cell_height = face_height.round();

        // Half the line gap goes above the text and half below. The baseline (from the bottom)
        // centers the face in the rounded cell height.
        let face_baseline = face.line_gap / 2.0 - face.descent;
        let cell_baseline = (face_baseline - (cell_height - face_height) / 2.0).round();
        let face_y = cell_baseline - face_baseline;
        let top_to_baseline = cell_height - cell_baseline;

        let cap_height = face.cap_height();
        let underline_thickness = face.underline_thickness().ceil().max(1.0);
        let strikethrough_thickness = face.strikethrough_thickness().ceil().max(1.0);
        let underline_position = (top_to_baseline - face.underline_position()).round();
        let strikethrough_position = (top_to_baseline - face.strikethrough_position()).round();

        // The font_patcher script's heuristic.
        let icon_height = face_height;
        let icon_height_single = (2.0 * cap_height + face_height) / 3.0;

        let mut metrics = Metrics {
            cell_width: whole(cell_width),
            cell_height: whole(cell_height),
            cell_baseline: whole(cell_baseline),
            underline_position: whole(underline_position),
            underline_thickness: whole(underline_thickness),
            strikethrough_position: whole(strikethrough_position),
            strikethrough_thickness: whole(strikethrough_thickness),
            overline_position: 0,
            overline_thickness: whole(underline_thickness),
            box_thickness: whole(underline_thickness),
            cursor_thickness: 1,
            cursor_height: whole(cell_height),
            icon_height,
            icon_height_single,
            face_width,
            face_height,
            face_y,
        };
        metrics.clamp();
        metrics
    }

    /// Calc plus modifiers, as Ghostty's `Collection.updateMetrics` does for the primary face.
    pub fn with_modifiers(face: &FaceMetrics, modifiers: &Modifiers) -> Metrics {
        let mut metrics = Metrics::calc(face);
        metrics.apply(modifiers);
        metrics
    }

    /// Apply the `adjust-*` modifiers, then clamp to the minimums.
    ///
    /// Ghostty iterates a hash map, so its order is unspecified; here they apply in field order.
    /// The order only matters when `cell_height` is combined with a percent modifier on a
    /// position it also moves.
    pub fn apply(&mut self, modifiers: &Modifiers) {
        if let Some(m) = modifiers.cell_width {
            self.cell_width = m.apply_u32(self.cell_width).max(1);
        }
        if let Some(m) = modifiers.cell_height {
            let original = self.cell_height;
            let new = m.apply_u32(original).max(1);
            if new != original {
                self.cell_height = new;
                self.recenter_for_height(original, new);
            }
        }
        if let Some(m) = modifiers.cell_baseline {
            self.cell_baseline = m.apply_u32(self.cell_baseline);
        }
        if let Some(m) = modifiers.underline_position {
            self.underline_position = m.apply_u32(self.underline_position);
        }
        if let Some(m) = modifiers.underline_thickness {
            self.underline_thickness = m.apply_u32(self.underline_thickness);
        }
        if let Some(m) = modifiers.strikethrough_position {
            self.strikethrough_position = m.apply_u32(self.strikethrough_position);
        }
        if let Some(m) = modifiers.strikethrough_thickness {
            self.strikethrough_thickness = m.apply_u32(self.strikethrough_thickness);
        }
        if let Some(m) = modifiers.overline_position {
            self.overline_position = m.apply_i32(self.overline_position);
        }
        if let Some(m) = modifiers.overline_thickness {
            self.overline_thickness = m.apply_u32(self.overline_thickness);
        }
        if let Some(m) = modifiers.cursor_thickness {
            self.cursor_thickness = m.apply_u32(self.cursor_thickness);
        }
        if let Some(m) = modifiers.cursor_height {
            self.cursor_height = m.apply_u32(self.cursor_height);
        }
        if let Some(m) = modifiers.box_thickness {
            self.box_thickness = m.apply_u32(self.box_thickness);
        }
        if let Some(m) = modifiers.icon_height {
            self.icon_height = m.apply_f64(self.icon_height);
            self.icon_height_single = m.apply_f64(self.icon_height_single);
        }
        self.clamp();
    }

    /// Keep the face centered when the cell height changes: the positions measured from the top
    /// move by the top half of the difference, the baseline by the bottom half. An odd pixel goes
    /// to the edge the face sits further from.
    fn recenter_for_height(&mut self, original: u32, new: u32) {
        let original = f64::from(original);
        let half_diff = (f64::from(new) - original) / 2.0;
        let position_with_respect_to_center = self.face_y - (original - self.face_height) / 2.0;
        let (diff_top, diff_bottom) = if position_with_respect_to_center > 0.0 {
            (half_diff.ceil(), half_diff.floor())
        } else {
            (half_diff.floor(), half_diff.ceil())
        };

        add_float_to_int(&mut self.cell_baseline, diff_bottom);
        self.face_y += diff_bottom;
        add_float_to_int(&mut self.underline_position, diff_top);
        add_float_to_int(&mut self.strikethrough_position, diff_top);
        self.overline_position = self.overline_position.saturating_add(diff_top as i32);
    }

    /// Ghostty's `Minimums`: no zero-sized cells, lines or cursors.
    fn clamp(&mut self) {
        self.cell_width = self.cell_width.max(1);
        self.cell_height = self.cell_height.max(1);
        self.underline_thickness = self.underline_thickness.max(1);
        self.strikethrough_thickness = self.strikethrough_thickness.max(1);
        self.overline_thickness = self.overline_thickness.max(1);
        self.box_thickness = self.box_thickness.max(1);
        self.cursor_thickness = self.cursor_thickness.max(1);
        self.cursor_height = self.cursor_height.max(1);
        self.icon_height = self.icon_height.max(1.0);
        self.icon_height_single = self.icon_height_single.max(1.0);
        self.face_height = self.face_height.max(1.0);
        self.face_width = self.face_width.max(1.0);
    }
}

/// Saturating add of a whole-valued float to a `u32`.
fn add_float_to_int(int: &mut u32, float: f64) {
    debug_assert_eq!(float.floor(), float);
    *int = if float >= 0.0 {
        int.saturating_add(float as u32)
    } else {
        int.saturating_sub((-float) as u32)
    };
}

/// The `adjust-*` settings, each keyed by the metric it changes (`adjust-font-baseline` is
/// `cell_baseline`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Modifiers {
    pub cell_width: Option<Modifier>,
    pub cell_height: Option<Modifier>,
    pub cell_baseline: Option<Modifier>,
    pub underline_position: Option<Modifier>,
    pub underline_thickness: Option<Modifier>,
    pub strikethrough_position: Option<Modifier>,
    pub strikethrough_thickness: Option<Modifier>,
    pub overline_position: Option<Modifier>,
    pub overline_thickness: Option<Modifier>,
    pub cursor_thickness: Option<Modifier>,
    pub cursor_height: Option<Modifier>,
    pub box_thickness: Option<Modifier>,
    pub icon_height: Option<Modifier>,
}

/// A change to one metric: `"20%"` makes it 20% larger, `"-1"` one pixel smaller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Modifier {
    /// A factor: 1.2 for `"20%"`.
    Percent(f64),
    /// Pixels to add.
    Absolute(i32),
}

/// A modifier string that is neither a number nor a percentage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidModifier(pub String);

impl std::fmt::Display for InvalidModifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid metric modifier {:?}: expected an integer or a percentage",
            self.0
        )
    }
}

impl std::error::Error for InvalidModifier {}

impl std::str::FromStr for Modifier {
    type Err = InvalidModifier;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Modifier::parse(input)
    }
}

impl Modifier {
    /// Ghostty's `Modifier.parse`: a trailing `%` is a percentage, anything else an integer. A
    /// percentage of -100% or less is a factor of 0.
    pub fn parse(input: &str) -> Result<Modifier, InvalidModifier> {
        let invalid = || InvalidModifier(input.to_owned());
        if let Some(percent) = input.strip_suffix('%') {
            let percent = percent.parse::<f64>().map_err(|_| invalid())? / 100.0;
            if percent <= -1.0 {
                return Ok(Modifier::Percent(0.0));
            }
            return Ok(Modifier::Percent(1.0 + percent));
        }
        input
            .parse::<i32>()
            .map(Modifier::Absolute)
            .map_err(|_| invalid())
    }

    /// Apply to an unsigned metric, saturating at 0 and `u32::MAX`.
    pub fn apply_u32(self, v: u32) -> u32 {
        match self {
            Modifier::Percent(p) => (f64::from(v) * p.max(0.0)).round() as u32,
            Modifier::Absolute(abs) => {
                (i64::from(v) + i64::from(abs)).clamp(0, i64::from(u32::MAX)) as u32
            }
        }
    }

    /// Apply to a signed metric, saturating at the `i32` range.
    pub fn apply_i32(self, v: i32) -> i32 {
        match self {
            Modifier::Percent(p) => (f64::from(v) * p.max(0.0)).round() as i32,
            Modifier::Absolute(abs) => v.saturating_add(abs),
        }
    }

    /// Apply to a fractional metric.
    pub fn apply_f64(self, v: f64) -> f64 {
        match self {
            Modifier::Percent(p) => v * p.max(0.0),
            Modifier::Absolute(abs) => v + f64::from(abs),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zeroed() -> Metrics {
        Metrics {
            cell_width: 0,
            cell_height: 0,
            cell_baseline: 0,
            underline_position: 0,
            underline_thickness: 0,
            strikethrough_position: 0,
            strikethrough_thickness: 0,
            overline_position: 0,
            overline_thickness: 0,
            box_thickness: 0,
            cursor_thickness: 1,
            cursor_height: 0,
            icon_height: 0.0,
            icon_height_single: 0.0,
            face_width: 0.0,
            face_height: 0.0,
            face_y: 0.0,
        }
    }

    // The tests below are Ghostty's own, from Metrics.zig.

    #[test]
    fn apply_modifiers() {
        let mut m = zeroed();
        m.cell_width = 100;
        m.apply(&Modifiers {
            cell_width: Some(Modifier::Percent(1.2)),
            ..Default::default()
        });
        assert_eq!(m.cell_width, 120);
    }

    fn height_fixture() -> Metrics {
        let mut m = zeroed();
        m.face_y = 0.33;
        m.cell_baseline = 50;
        m.underline_position = 55;
        m.strikethrough_position = 30;
        m.overline_position = 0;
        m.cell_height = 100;
        m.face_height = 99.67;
        m.cursor_height = 100;
        m
    }

    #[test]
    fn adjust_cell_height_smaller() {
        // 25 px removed: 13 from the bottom and 12 from the top, since the face sits 0.33 px high.
        let mut m = height_fixture();
        m.apply(&Modifiers {
            cell_height: Some(Modifier::Percent(0.75)),
            ..Default::default()
        });
        assert_eq!(m.face_y, 0.33 - 13.0);
        assert_eq!(m.cell_height, 75);
        assert_eq!(m.cell_baseline, 37);
        assert_eq!(m.underline_position, 43);
        assert_eq!(m.strikethrough_position, 18);
        assert_eq!(m.overline_position, -12);
        // The cursor keeps its own height.
        assert_eq!(m.cursor_height, 100);
    }

    #[test]
    fn adjust_cell_height_larger() {
        // 75 px added: 37 at the bottom and 38 at the top.
        let mut m = height_fixture();
        m.apply(&Modifiers {
            cell_height: Some(Modifier::Percent(1.75)),
            ..Default::default()
        });
        assert_eq!(m.face_y, 0.33 + 37.0);
        assert_eq!(m.cell_height, 175);
        assert_eq!(m.cell_baseline, 87);
        assert_eq!(m.underline_position, 93);
        assert_eq!(m.strikethrough_position, 68);
        assert_eq!(m.overline_position, 38);
        assert_eq!(m.cursor_height, 100);
    }

    fn icon_fixture() -> Metrics {
        let mut m = zeroed();
        m.icon_height = 100.0;
        m.icon_height_single = 80.0;
        m.face_height = 100.0;
        m.face_y = 1.0;
        m
    }

    #[test]
    fn adjust_icon_height_by_percentage() {
        let mut m = icon_fixture();
        m.apply(&Modifiers {
            icon_height: Some(Modifier::Percent(0.75)),
            ..Default::default()
        });
        assert_eq!(m.icon_height, 75.0);
        assert_eq!(m.icon_height_single, 60.0);
        assert_eq!(m.face_height, 100.0);
        assert_eq!(m.face_y, 1.0);
    }

    #[test]
    fn adjust_icon_height_by_absolute_pixels() {
        let mut m = icon_fixture();
        m.apply(&Modifiers {
            icon_height: Some(Modifier::Absolute(-5)),
            ..Default::default()
        });
        assert_eq!(m.icon_height, 95.0);
        assert_eq!(m.icon_height_single, 75.0);
        assert_eq!(m.face_height, 100.0);
        assert_eq!(m.face_y, 1.0);
    }

    #[test]
    fn modifier_parse_absolute() {
        assert_eq!(Modifier::parse("100"), Ok(Modifier::Absolute(100)));
        assert_eq!(Modifier::parse("-100"), Ok(Modifier::Absolute(-100)));
    }

    #[test]
    fn modifier_parse_percent() {
        assert_eq!(Modifier::parse("20%"), Ok(Modifier::Percent(1.2)));
        assert_eq!(Modifier::parse("-20%"), Ok(Modifier::Percent(0.8)));
        assert_eq!(Modifier::parse("0%"), Ok(Modifier::Percent(1.0)));
        assert_eq!(Modifier::parse("-150%"), Ok(Modifier::Percent(0.0)));
    }

    #[test]
    fn modifier_parse_invalid() {
        for input in ["", "%", "1.5", "abc", "20 %"] {
            assert!(Modifier::parse(input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn modifier_percent() {
        assert_eq!(Modifier::Percent(0.8).apply_u32(100), 80);
        assert_eq!(Modifier::Percent(1.8).apply_u32(100), 180);
    }

    #[test]
    fn modifier_absolute() {
        assert_eq!(Modifier::Absolute(-100).apply_u32(100), 0);
        assert_eq!(Modifier::Absolute(-120).apply_u32(100), 0);
        assert_eq!(Modifier::Absolute(100).apply_u32(100), 200);
        assert_eq!(Modifier::Absolute(-120).apply_i32(100), -20);
    }

    #[test]
    fn minimums_survive_modifiers() {
        let mut m = Metrics::calc(&FaceMetrics {
            px_per_em: 10.0,
            cell_width: 6.0,
            ascent: 8.0,
            descent: -2.0,
            ..Default::default()
        });
        m.apply(&Modifiers {
            cell_width: Some(Modifier::Absolute(-100)),
            cell_height: Some(Modifier::Percent(0.0)),
            underline_thickness: Some(Modifier::Absolute(-100)),
            box_thickness: Some(Modifier::Absolute(-100)),
            cursor_height: Some(Modifier::Absolute(-100)),
            ..Default::default()
        });
        assert_eq!((m.cell_width, m.cell_height), (1, 1));
        assert_eq!(
            (m.underline_thickness, m.box_thickness, m.cursor_height),
            (1, 1, 1)
        );
    }

    #[test]
    fn calc_estimates_missing_metrics() {
        // No underline, strikethrough, cap or ex height: Ghostty's estimates apply.
        let face = FaceMetrics {
            px_per_em: 20.0,
            cell_width: 12.0,
            ascent: 16.0,
            descent: -4.0,
            line_gap: 0.0,
            ..Default::default()
        };
        assert_eq!(face.cap_height(), 12.0);
        assert_eq!(face.ex_height(), 9.0);
        assert_eq!(face.underline_thickness(), 0.15 * 9.0);
        let m = Metrics::calc(&face);
        assert_eq!((m.cell_width, m.cell_height, m.cell_baseline), (12, 20, 4));
        // top_to_baseline 16, underline top 1.35 below the baseline.
        assert_eq!((m.underline_position, m.underline_thickness), (17, 2));
        // Centered at half the ex height: (9 + 1.35) / 2 above the baseline.
        assert_eq!(
            (m.strikethrough_position, m.strikethrough_thickness),
            (11, 2)
        );
        assert_eq!(
            (m.overline_thickness, m.box_thickness, m.cursor_height),
            (2, 2, 20)
        );
        assert_eq!(m.icon_height_single, (2.0 * 12.0 + 20.0) / 3.0);
    }
}
