//! The many-pane budget (`docs/research/renderer-gates.md`, gate 3): N terminal panes all changing
//! at once, drawn through Rune's real runtime with no window, against the 8.3 ms frame at 120 Hz.
//!
//!     cargo run --release -p herdr-frontend --example pane-budget
//!
//! Each frame, every pane's terminal takes new output and hands over a snapshot, as its worker
//! would, then the runtime builds the frame: the view builds the changed rows, Rune shapes them,
//! lays the window out and paints its instances. That's the UI thread's work per frame. The VT
//! parse and snapshot run on each terminal's worker thread in the app; they're timed apart. GPU
//! time isn't included.

use std::time::{Duration, Instant};

use herdr_frontend::view::terminal::TerminalPane;
use herdr_frontend::view::terminal::look::arctic;
use rune::Env;
use terminal::{Config, Vt};

/// What the panes' programs do each frame.
#[derive(Clone, Copy)]
enum Load {
    /// A character typed at the prompt: one row changes.
    Type,
    /// One new line, as `tail -f` or a build log prints.
    Line,
    /// A screenful of new lines: the whole screen scrolls.
    Flood,
    /// Every row rewritten in place with new text, as a full-screen program repaints.
    Repaint,
}

impl Load {
    fn name(self) -> &'static str {
        match self {
            Load::Type => "one row",
            Load::Line => "one line",
            Load::Flood => "screen scrolls",
            Load::Repaint => "full repaint",
        }
    }

    /// What pane `pane` is sent on frame `frame`. Every line is new text: no pane or frame
    /// repeats another's, so nothing comes from the text system's shaping cache.
    fn output(self, frame: usize, pane: usize, cols: u16, rows: u16) -> Vec<u8> {
        let line = |n: usize| {
            let mut s = format!("\x1b[3{}m{pane:>2}:{n:>8}\x1b[0m ", n % 8);
            while s.len() < usize::from(cols) {
                s.push_str("lorem ipsum dolor sit amet ");
            }
            s.truncate(usize::from(cols) - 1);
            s
        };
        match self {
            Load::Type => vec![b'a' + (frame % 26) as u8],
            Load::Line => format!("\r\n{}", line(frame)).into_bytes(),
            Load::Flood => (0..rows)
                .map(|k| format!("\r\n{}", line(frame * usize::from(rows) + usize::from(k))))
                .collect::<String>()
                .into_bytes(),
            Load::Repaint => (0..rows)
                .map(|k| {
                    let n = frame * usize::from(rows) + usize::from(k);
                    format!("\x1b[{};1H{}", k + 1, line(n))
                })
                .collect::<String>()
                .into_bytes(),
        }
    }
}

fn percentile(sorted: &[Duration], p: f64) -> f64 {
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i].as_secs_f64() * 1000.0
}

/// Runs `frames` frames of `n` panes of `cols` × `rows` under `load` in a window of `size` points.
fn run(n: usize, cols: u16, rows: u16, size: (f32, f32), load: Load, frames: usize) {
    let (theme, _) = arctic();
    let mut rt = herdr_frontend::app().headless_runtime(size, Env::new(true, 2.0));
    let mut vts: Vec<Vt> = (0..n)
        .map(|_| {
            Vt::new(&Config {
                cols,
                rows,
                theme: theme.clone(),
                ..Config::default()
            })
        })
        .collect();
    rt.state.panes = vts
        .iter_mut()
        .enumerate()
        .map(|(i, vt)| {
            vt.write(&Load::Repaint.output(0, i, cols, rows));
            TerminalPane::new(format!("pane-{}", i + 1), vt.snapshot())
        })
        .collect();
    rt.frame(0.0);

    let (mut ui, mut worker, mut built) = (Vec::new(), Vec::new(), 0);
    for frame in 1..=frames {
        let start = Instant::now();
        let snapshots: Vec<_> = vts
            .iter_mut()
            .enumerate()
            .map(|(i, vt)| {
                vt.write(&load.output(frame, i, cols, rows));
                vt.snapshot()
            })
            .collect();
        worker.push(start.elapsed());
        for (pane, snapshot) in rt.state.panes.iter_mut().zip(snapshots) {
            pane.show(snapshot);
        }
        let start = Instant::now();
        rt.invalidate();
        rt.frame(1.0 / 120.0);
        ui.push(start.elapsed());
        built += rt
            .state
            .panes
            .iter()
            .map(TerminalPane::rows_built)
            .sum::<usize>();
    }
    ui.sort();
    worker.sort();
    println!(
        "| {n} × {cols}×{rows} | {} | {:.0} | {:.2} | {:.2} | {:.2} | {:.2} |",
        load.name(),
        built as f64 / frames as f64,
        percentile(&ui, 0.5),
        percentile(&ui, 0.95),
        percentile(&ui, 0.99),
        percentile(&worker, 0.5),
    );
}

fn main() {
    let frames = 240;
    // A 14-inch MacBook Pro's screen in points, as panes split it, and Rune's 200 × 60 reference.
    let screen = (1512.0, 945.0);
    let layouts: [(usize, u16, u16, (f32, f32)); 5] = [
        (1, 200, 60, (1700.0, 1110.0)),
        (1, 177, 51, screen),
        (2, 88, 51, screen),
        (4, 88, 25, screen),
        (9, 58, 17, screen),
    ];
    println!(
        "| panes | load | rows built a frame | UI p50 ms | UI p95 ms | UI p99 ms | parse + snapshot p50 ms (all panes) |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    for load in [Load::Type, Load::Line, Load::Flood, Load::Repaint] {
        for &(n, cols, rows, size) in &layouts {
            run(n, cols, rows, size, load, frames);
        }
    }
}
