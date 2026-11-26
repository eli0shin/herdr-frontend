//! Grid and padding math: how many cells fit a pane, where the leftover pixels go, and the size a
//! terminal reports.
//!
//! A port of `ghostty:src/renderer/size.zig` and the padding parts of `ghostty:src/Surface.zig` at
//! `vt-1.0.2` (`573c4a21`). All values are device pixels. See
//! `docs/research/terminal-rendering.md` §4 "Sizing".

use crate::face::DEFAULT_DPI;

/// One cell, in device pixels (from [`crate::Metrics`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellSize {
    pub width: u32,
    pub height: u32,
}

/// The pixels a terminal is drawn into, padding included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScreenSize {
    pub width: u32,
    pub height: u32,
}

/// A grid in cells. Ghostty's cell count type is `u16`, as are RESIZE's fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GridSize {
    pub columns: u16,
    pub rows: u16,
}

/// Space around the grid, in device pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Padding {
    pub top: u32,
    pub bottom: u32,
    pub right: u32,
    pub left: u32,
}

/// Ghostty's `window-padding-balance`: where the pixels the grid doesn't fill go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaddingBalance {
    /// The grid hugs the top left, after the explicit padding.
    #[default]
    False,
    /// Balanced on all sides, with the top capped at half the horizontal padding plus half a cell;
    /// the rest goes to the bottom.
    True,
    /// Balanced equally on all sides.
    Equal,
}

impl ScreenSize {
    /// The screen less padding, saturating at 0.
    pub fn sub_padding(self, padding: Padding) -> ScreenSize {
        ScreenSize {
            width: self.width.saturating_sub(padding.left + padding.right),
            height: self.height.saturating_sub(padding.top + padding.bottom),
        }
    }

    /// The unfilled space right of and below the grid, when padding isn't balanced.
    pub fn blank_padding(self, padding: Padding, grid: GridSize, cell: CellSize) -> Padding {
        let padded_width = u32::from(grid.columns) * cell.width + padding.left + padding.right;
        let padded_height = u32::from(grid.rows) * cell.height + padding.top + padding.bottom;
        Padding {
            top: 0,
            bottom: self.height.saturating_sub(padded_height),
            right: self.width.saturating_sub(padded_width),
            left: 0,
        }
    }
}

impl GridSize {
    /// The cells that fit: `floor(screen / cell)` each way, at least 1.
    pub fn new(screen: ScreenSize, cell: CellSize) -> GridSize {
        // Ghostty divides in f32; the cast truncates toward zero and saturates.
        let fit = |screen: u32, cell: u32| ((screen as f32 / cell as f32) as u16).max(1);
        GridSize {
            columns: fit(screen.width, cell.width),
            rows: fit(screen.height, cell.height),
        }
    }
}

impl Padding {
    /// Ghostty's `scaledPadding`: `window-padding-x/y` in points, converted to device pixels and
    /// floored.
    pub fn from_points(
        top: u32,
        bottom: u32,
        left: u32,
        right: u32,
        content_scale: f32,
    ) -> Padding {
        let dpi = content_scale * DEFAULT_DPI;
        let scale = |points: u32| (points as f32 * dpi / 72.0).floor() as u32;
        Padding {
            top: scale(top),
            bottom: scale(bottom),
            left: scale(left),
            right: scale(right),
        }
    }

    /// Half the unfilled space on each side, floored, so the grid is centered.
    pub fn balanced(screen: ScreenSize, grid: GridSize, cell: CellSize) -> Padding {
        let space_right = screen.width as f32 - f32::from(grid.columns) * cell.width as f32;
        let space_bottom = screen.height as f32 - f32::from(grid.rows) * cell.height as f32;
        let horizontal = (space_right / 2.0).floor().max(0.0) as u32;
        let vertical = (space_bottom / 2.0).floor().max(0.0) as u32;
        Padding {
            top: vertical,
            bottom: vertical,
            right: horizontal,
            left: horizontal,
        }
    }
}

/// Everything needed to lay out one terminal: its pixels, its cell and its padding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub screen: ScreenSize,
    pub cell: CellSize,
    pub padding: Padding,
}

/// What a terminal of this size tells the core and the program: the grid for RESIZE and
/// `ghostty_terminal_resize`, the cell for FACTS and the `SIZE` callback, and a pixel size that is
/// exactly `cols × cell` by `rows × cell` (§4 rule 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeReport {
    pub grid: GridSize,
    pub cell: CellSize,
    pub width_px: u32,
    pub height_px: u32,
}

impl Size {
    /// A size for a screen, with Ghostty's padding: the explicit padding, then the leftover
    /// pixels balanced if `balance` says so.
    pub fn new(
        screen: ScreenSize,
        cell: CellSize,
        explicit: Padding,
        balance: PaddingBalance,
    ) -> Size {
        let mut size = Size {
            screen,
            cell,
            padding: explicit,
        };
        size.balance_padding(explicit, balance);
        size
    }

    /// The grid that fits the screen less padding.
    pub fn grid(&self) -> GridSize {
        GridSize::new(self.terminal(), self.cell)
    }

    /// The screen less padding.
    pub fn terminal(&self) -> ScreenSize {
        self.screen.sub_padding(self.padding)
    }

    /// Recompute the padding from the explicit padding, as Ghostty does on every resize.
    pub fn balance_padding(&mut self, explicit: Padding, mode: PaddingBalance) {
        self.padding = explicit;
        if mode == PaddingBalance::False {
            return;
        }
        self.padding = Padding::balanced(self.screen, self.grid(), self.cell);
        if mode == PaddingBalance::True {
            let max_top = (explicit.left + explicit.right + self.cell.width) / 2;
            let shift = self.padding.top.saturating_sub(max_top);
            self.padding.top -= shift;
            self.padding.bottom += shift;
        }
    }

    /// The grid, cell and whole-cell pixel size to report.
    pub fn report(&self) -> SizeReport {
        let grid = self.grid();
        SizeReport {
            grid,
            cell: self.cell,
            width_px: u32::from(grid.columns) * self.cell.width,
            height_px: u32::from(grid.rows) * self.cell.height,
        }
    }

    /// The cell under a point on the surface (padding included), clamped to the grid.
    pub fn surface_to_grid(&self, x: f64, y: f64) -> (u16, u16) {
        let grid = self.grid();
        let column =
            ((x - f64::from(self.padding.left)).max(0.0) / f64::from(self.cell.width)) as u16;
        let row = ((y - f64::from(self.padding.top)).max(0.0) / f64::from(self.cell.height)) as u16;
        (column.min(grid.columns - 1), row.min(grid.rows - 1))
    }

    /// The top-left corner of a cell on the surface.
    pub fn grid_to_surface(&self, column: u16, row: u16) -> (f64, f64) {
        (
            f64::from(column) * f64::from(self.cell.width) + f64::from(self.padding.left),
            f64::from(row) * f64::from(self.cell.height) + f64::from(self.padding.top),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn screen(width: u32, height: u32) -> ScreenSize {
        ScreenSize { width, height }
    }

    const fn cell(width: u32, height: u32) -> CellSize {
        CellSize { width, height }
    }

    const fn even(px: u32) -> Padding {
        Padding {
            top: px,
            bottom: px,
            right: px,
            left: px,
        }
    }

    // The tests below are Ghostty's own, from renderer/size.zig.

    #[test]
    fn balance_equal_distributes_whitespace_equally() {
        // (1050 − 8) / 10 = 104 columns, (850 − 8) / 20 = 42 rows, 10 px left over each way.
        let size = Size::new(
            screen(1050, 850),
            cell(10, 20),
            even(4),
            PaddingBalance::Equal,
        );
        assert_eq!(size.padding, even(5));
    }

    #[test]
    fn balance_true_shifts_excess_top_to_bottom() {
        // 54 × 26 cells; balanced is 5 and 15; the top cap is (0 + 0 + 20) / 2 = 10.
        let size = Size::new(
            screen(1090, 1070),
            cell(20, 40),
            Padding::default(),
            PaddingBalance::True,
        );
        assert_eq!(
            size.padding,
            Padding {
                top: 10,
                bottom: 20,
                right: 5,
                left: 5
            }
        );
    }

    #[test]
    fn balanced_on_zero() {
        let grid = GridSize {
            columns: 100,
            rows: 37,
        };
        assert_eq!(
            Padding::balanced(screen(0, 0), grid, cell(10, 20)),
            Padding::default()
        );
    }

    #[test]
    fn grid_exact() {
        assert_eq!(
            GridSize::new(screen(100, 40), cell(5, 10)),
            GridSize {
                columns: 20,
                rows: 4
            }
        );
    }

    #[test]
    fn grid_rounding() {
        assert_eq!(
            GridSize::new(screen(20, 40), cell(6, 15)),
            GridSize {
                columns: 3,
                rows: 2
            }
        );
    }

    #[test]
    fn grid_is_at_least_one_cell() {
        assert_eq!(
            GridSize::new(screen(3, 3), cell(10, 22)),
            GridSize {
                columns: 1,
                rows: 1
            }
        );
        let tiny = Size::new(screen(4, 4), cell(10, 22), even(4), PaddingBalance::False);
        assert_eq!(tiny.terminal(), screen(0, 0));
        assert_eq!(
            tiny.grid(),
            GridSize {
                columns: 1,
                rows: 1
            }
        );
    }

    #[test]
    fn coordinate_conversion() {
        let size = Size {
            screen: screen(100, 100),
            cell: cell(5, 10),
            padding: Padding::default(),
        };
        assert_eq!(size.surface_to_grid(0.0, 0.0), (0, 0));
        assert_eq!(size.surface_to_grid(6.0, 0.0), (1, 0));
        assert_eq!(size.surface_to_grid(6.0, 10.0), (1, 1));
        assert_eq!(size.surface_to_grid(-10.0, -10.0), (0, 0));
        assert_eq!(size.surface_to_grid(100_000.0, 100_000.0), (19, 9));
        assert_eq!(size.grid_to_surface(1, 1), (5.0, 10.0));
    }

    #[test]
    fn coordinates_account_for_padding() {
        let size = Size {
            screen: screen(100, 100),
            cell: cell(5, 10),
            padding: even(3),
        };
        assert_eq!(size.grid_to_surface(2, 1), (13.0, 13.0));
        assert_eq!(size.surface_to_grid(13.0, 13.0), (2, 1));
        assert_eq!(size.surface_to_grid(12.9, 12.9), (1, 0));
    }

    #[test]
    fn padding_from_points() {
        // Ghostty's default window-padding is 2 pt each side.
        assert_eq!(
            Padding::from_points(2, 2, 2, 2, 1.0),
            even((2.0 * DEFAULT_DPI / 72.0) as u32)
        );
        assert_eq!(
            Padding::from_points(2, 2, 2, 2, 2.0),
            even((4.0 * DEFAULT_DPI / 72.0) as u32)
        );
        // Fractional scales floor.
        assert_eq!(
            Padding::from_points(3, 0, 0, 0, 1.5).top,
            (4.5 * DEFAULT_DPI / 72.0).floor() as u32
        );
    }

    #[test]
    fn blank_padding_is_the_unfilled_space() {
        let size = Size::new(
            screen(1005, 503),
            cell(10, 22),
            Padding::default(),
            PaddingBalance::False,
        );
        assert_eq!(
            size.grid(),
            GridSize {
                columns: 100,
                rows: 22
            }
        );
        let blank = size
            .screen
            .blank_padding(size.padding, size.grid(), size.cell);
        assert_eq!(
            blank,
            Padding {
                top: 0,
                bottom: 19,
                right: 5,
                left: 0
            }
        );
    }

    #[test]
    fn report_is_whole_cells() {
        // Eli's Ghostty: no padding, unbalanced, Operator Mono's 21×45 cell at 2x.
        let size = Size::new(
            screen(2000, 1000),
            cell(21, 45),
            Padding::default(),
            PaddingBalance::False,
        );
        let report = size.report();
        assert_eq!(
            report.grid,
            GridSize {
                columns: 95,
                rows: 22
            }
        );
        assert_eq!(report.cell, cell(21, 45));
        assert_eq!((report.width_px, report.height_px), (95 * 21, 22 * 45));
    }
}
