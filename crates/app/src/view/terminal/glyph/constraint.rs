//! How a glyph is sized and placed in its cells: a port of `RenderOptions.Constraint` in
//! `ghostty:src/font/Glyph.zig` at `v1.4.0`, with the rules for which glyphs get one
//! (`renderer/cell.zig` `isSymbol` and `constraintWidth`, `font/SharedGrid.zig` for emoji, and
//! `font/nerd_font_attributes.zig` for Nerd Font icons).

use terminal::Metrics;

use super::tables::{CONSTRAINTS, NERD};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    None,
    /// Scale down to fit the bounds, keeping the aspect ratio.
    Fit,
    /// Scale up or down to the bounds, keeping the aspect ratio.
    Cover,
    /// Fit, but scale up to cover at least one cell (a Nerd Font rule).
    FitCover1,
    /// Stretch to the bounds in both directions.
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    None,
    Start,
    End,
    Center,
    /// Centered in the first cell, even for a multi-cell constraint (a Nerd Font rule).
    Center1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Height {
    /// The primary face's line height.
    Cell,
    /// The grid's icon height.
    Icon,
}

/// A glyph's sizing and alignment rules.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Constraint {
    pub size: Size,
    pub align_vertical: Align,
    pub align_horizontal: Align,
    pub pad_top: f64,
    pub pad_left: f64,
    pub pad_right: f64,
    pub pad_bottom: f64,
    pub relative_width: f64,
    pub relative_height: f64,
    pub relative_x: f64,
    pub relative_y: f64,
    pub max_xy_ratio: Option<f64>,
    pub max_constraint_width: u32,
    pub height: Height,
}

/// A glyph's box in pixels: its size, and its left and bottom edges from the cell's left and
/// bottom (+y up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphBox {
    pub width: f64,
    pub height: f64,
    pub x: f64,
    pub y: f64,
}

impl Constraint {
    pub const NONE: Constraint = Constraint {
        size: Size::None,
        align_vertical: Align::None,
        align_horizontal: Align::None,
        pad_top: 0.0,
        pad_left: 0.0,
        pad_right: 0.0,
        pad_bottom: 0.0,
        relative_width: 1.0,
        relative_height: 1.0,
        relative_x: 0.0,
        relative_y: 0.0,
        max_xy_ratio: None,
        max_constraint_width: 2,
        height: Height::Cell,
    };

    /// `SharedGrid.renderGlyph`'s rule for every emoji: as large as fits, centered, with a
    /// little room at the sides.
    pub const EMOJI: Constraint = Constraint {
        size: Size::Cover,
        align_horizontal: Align::Center,
        align_vertical: Align::Center,
        pad_left: 0.025,
        pad_right: 0.025,
        ..Constraint::NONE
    };

    /// A symbol with no Nerd Font rule: scaled down to fit, never moved.
    pub const FIT: Constraint = Constraint {
        size: Size::Fit,
        ..Constraint::NONE
    };

    pub fn does_anything(&self) -> bool {
        self.size != Size::None
            || self.align_horizontal != Align::None
            || self.align_vertical != Align::None
    }

    /// `constrain`: where and how big `glyph` draws, with `constraint_width` cells free.
    pub fn constrain(&self, glyph: GlyphBox, m: &Metrics, constraint_width: u32) -> GlyphBox {
        if !self.does_anything() {
            return glyph;
        }
        if self.size == Size::Stretch {
            let mut m = *m;
            m.face_width = f64::from(m.cell_width);
            m.face_height = f64::from(m.cell_height);
            m.face_y = 0.0;
            let mut c = *self;
            c.pad_bottom = c.pad_bottom.max(0.0);
            c.pad_top = c.pad_top.max(0.0);
            c.pad_left = c.pad_left.max(0.0);
            c.pad_right = c.pad_right.max(0.0);
            return c.constrain_inner(glyph, &m, constraint_width);
        }
        self.constrain_inner(glyph, m, constraint_width)
    }

    fn constrain_inner(&self, glyph: GlyphBox, m: &Metrics, constraint_width: u32) -> GlyphBox {
        let min_width = if self.size == Size::Stretch && m.face_width > 0.9 * m.face_height {
            1
        } else {
            self.max_constraint_width.min(constraint_width)
        };
        let gw = glyph.width / self.relative_width;
        let gh = glyph.height / self.relative_height;
        let mut group = GlyphBox {
            width: gw,
            height: gh,
            x: glyph.x - gw * self.relative_x,
            y: glyph.y - gh * self.relative_y,
        };
        let (wf, hf) = self.scale_factors(&group, m, min_width);
        let cx = group.x + group.width / 2.0;
        let cy = group.y + group.height / 2.0;
        group.width *= wf;
        group.height *= hf;
        group.x = cx - group.width / 2.0;
        group.y = cy - group.height / 2.0;
        group.y = self.aligned_y(&group, m);
        group.x = self.aligned_x(&group, m, min_width);
        GlyphBox {
            width: wf * glyph.width,
            height: hf * glyph.height,
            x: group.x + group.width * self.relative_x,
            y: group.y + group.height * self.relative_y,
        }
    }

    fn scale_factors(&self, group: &GlyphBox, m: &Metrics, min_width: u32) -> (f64, f64) {
        if self.size == Size::None {
            return (1.0, 1.0);
        }
        let multi = min_width > 1;
        let pad_w = f64::from(min_width) - (self.pad_left + self.pad_right);
        let pad_h = 1.0 - (self.pad_bottom + self.pad_top);
        let target_w = pad_w * m.face_width;
        let target_h = pad_h
            * match self.height {
                Height::Cell => m.face_height,
                Height::Icon if multi => m.icon_height,
                Height::Icon => m.icon_height_single,
            };
        let mut wf = target_w / group.width;
        let mut hf = target_h / group.height;
        match self.size {
            Size::None => unreachable!(),
            Size::Fit => {
                hf = 1f64.min(wf).min(hf);
                wf = hf;
            }
            Size::Cover => {
                hf = wf.min(hf);
                wf = hf;
            }
            Size::FitCover1 => {
                hf = wf.min(hf);
                if multi && hf > 1.0 {
                    let (_, single) = self.scale_factors(group, m, 1);
                    hf = single.max(1.0);
                }
                wf = hf;
            }
            Size::Stretch => {}
        }
        if let Some(ratio) = self.max_xy_ratio
            && group.width * wf > group.height * hf * ratio
        {
            wf = group.height * hf * ratio / group.width;
        }
        (wf, hf)
    }

    fn aligned_y(&self, group: &GlyphBox, m: &Metrics) -> f64 {
        if self.size == Size::None && self.align_vertical == Align::None {
            return group.y;
        }
        let start = m.face_y + self.pad_bottom * m.face_height;
        let end = m.face_y + (m.face_height - group.height - self.pad_top * m.face_height);
        let center = (start + end) / 2.0;
        match self.align_vertical {
            Align::None if end < start => center,
            Align::None => start.max(group.y.min(end)),
            Align::Start => start,
            Align::End => end,
            Align::Center | Align::Center1 => center,
        }
    }

    fn aligned_x(&self, group: &GlyphBox, m: &Metrics, min_width: u32) -> f64 {
        if self.size == Size::None && self.align_horizontal == Align::None {
            return group.x;
        }
        let span = m.face_width + f64::from((min_width - 1) * m.cell_width);
        let pad_l = self.pad_left * m.face_width;
        let pad_r = self.pad_right * m.face_width;
        let start = pad_l;
        let end = span - group.width - pad_r;
        match self.align_horizontal {
            Align::None => start.max(group.x.min(end)),
            Align::Start => start,
            Align::End => start.max(end),
            Align::Center => start.max((start + end) / 2.0),
            Align::Center1 => {
                let end1 = m.face_width - group.width - pad_r;
                start.max((start + end1) / 2.0)
            }
        }
    }
}

/// The Nerd Font constraint for `cp`, if it has one (`getConstraint`).
pub fn nerd(cp: u32) -> Option<Constraint> {
    let at = NERD.partition_point(|&(_, hi, _)| hi < cp);
    NERD.get(at)
        .filter(|&&(lo, _, _)| lo <= cp)
        .map(|&(_, _, i)| CONSTRAINTS[usize::from(i)])
}

/// Whether `cp` is "symbol-like" (`isSymbol`): private use, or in the Arrows, Dingbats,
/// Emoticons, Miscellaneous Symbols, Enclosed Alphanumerics (and Supplement), Miscellaneous
/// Symbols and Pictographs, or Transport and Map Symbols blocks.
pub fn is_symbol(cp: u32) -> bool {
    matches!(cp,
        0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD
        | 0x2190..=0x21FF | 0x2700..=0x27BF | 0x1F600..=0x1F64F | 0x2600..=0x26FF
        | 0x2460..=0x24FF | 0x1F100..=0x1F1FF | 0x1F300..=0x1F5FF | 0x1F680..=0x1F6FF)
}

/// `isGraphicsElement`: box drawing, blocks, legacy computing and Powerline.
fn is_graphics(cp: u32) -> bool {
    matches!(cp, 0x2500..=0x259F | 0x1FB00..=0x1FBFF | 0x1CC00..=0x1CEBF | 0xE0B0..=0xE0D7)
}

/// `constraintWidth`: how many cells the glyph at column `x` may take. A wide character takes
/// its two; a symbol may spread into the next cell when that's blank, unless the one before it
/// is a symbol too (so a run of icons lines up) or it's the last column.
pub fn constraint_width(cps: &[u32], wide: bool, x: usize) -> u32 {
    if wide {
        return 2;
    }
    let cp = cps[x];
    if !is_symbol(cp) || x + 1 >= cps.len() {
        return 1;
    }
    if x > 0 && is_symbol(cps[x - 1]) && !is_graphics(cps[x - 1]) {
        return 1;
    }
    match cps[x + 1] {
        0 | 0x20 | 0x2002 => 2,
        _ => 1,
    }
}

/// The constraint a non-emoji glyph for `cp` draws with: its Nerd Font rule, `fit` for another
/// symbol, none otherwise.
pub fn for_text(cp: u32) -> Constraint {
    nerd(cp).unwrap_or(if is_symbol(cp) {
        Constraint::FIT
    } else {
        Constraint::NONE
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nerd_lookups_match_ghosttys_tests() {
        assert!(nerd('A' as u32).is_none());
        assert!(nerd(0x30A2).is_none());
        assert!(nerd(0xFF66).is_none());
        assert!(nerd(0x2630).is_some());
        assert!(nerd(0x110000).is_none());
        // A Powerline separator is a sprite, but Nerd Fonts constrain the symbol too.
        assert!(nerd(0xF015).is_some(), "nf-fa-home");
    }

    #[test]
    fn a_symbol_spreads_into_a_blank_next_cell_only() {
        let row = |s: &str| s.chars().map(|c| c as u32).collect::<Vec<_>>();
        assert_eq!(constraint_width(&row("\u{f015} x"), false, 0), 2);
        assert_eq!(constraint_width(&row("\u{f015}x"), false, 0), 1);
        assert_eq!(constraint_width(&row("\u{f015}\u{f015} "), false, 1), 1);
        assert_eq!(constraint_width(&row("\u{2588}\u{f015} "), false, 1), 2);
        assert_eq!(constraint_width(&row("a "), false, 0), 1);
        assert_eq!(
            constraint_width(&row("\u{f015}"), false, 0),
            1,
            "last column"
        );
    }
}
