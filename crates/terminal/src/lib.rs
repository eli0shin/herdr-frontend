//! The libghostty-vt wrapper: the per-terminal worker, snapshots, input encoders and cell metrics.
//!
//! Built so far: the cell metrics, ported from Ghostty so a cell here is the same whole-pixel
//! cell Ghostty draws ([`face`], [`metrics`]), and the grid and padding math ([`size`]).

pub mod face;
pub mod metrics;
pub mod size;

pub use face::{DesiredSize, FaceError};
pub use metrics::{FaceMetrics, Metrics, Modifier, Modifiers};
pub use size::{CellSize, GridSize, Padding, PaddingBalance, ScreenSize, Size, SizeReport};

impl Metrics {
    /// The cell to lay the grid out with.
    pub fn cell_size(&self) -> CellSize {
        CellSize {
            width: self.cell_width,
            height: self.cell_height,
        }
    }
}
