//! How soon a trackpad scroll over the scrollback shows, and how closely the screen follows the
//! fingers (ticket 051): the real app on a fake core through Rune's runtime with no window, its
//! terminal on its real worker thread, frames paced at 120 Hz in real time.
//!
//!     just scroll-latency
//!
//! Two runs:
//!
//! - **Latency.** One wheel event of a whole row at a random moment in a frame, then frames until
//!   the screen shows it: the frames counted from the event (the next frame is 1) and the time
//!   from the event to the end of that frame.
//! - **A flick.** A trackpad flick and its momentum, one event per frame at a random moment in
//!   it, as macOS delivers them: each frame, how far the screen is from where the fingers put it
//!   (rows), and the frames the fingers moved and the screen didn't.
//!
//! The screen's place is read the way it's drawn: the snapshot's viewport, less the rows the grid
//! sits below the pane's top. Only public app and Rune APIs are used, so the same file measures
//! an older build.

use std::time::{Duration, Instant};

use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::demo_core;
use herdr_frontend::{SIZE, State, app};
use rune::{Env, Runtime, WheelEvent};

const FRAME: Duration = Duration::from_nanos(8_333_333);

/// A tiny deterministic random number generator, so runs compare.
struct Rng(u64);

impl Rng {
    /// 0..1.
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

struct Bench {
    rt: Runtime<State>,
    /// When the next frame is due.
    tick: Instant,
    line: f32,
    at: (f32, f32),
}

impl Bench {
    fn frame(&mut self) -> Instant {
        let now = Instant::now();
        if self.tick > now {
            std::thread::sleep(self.tick - now);
        }
        self.tick += FRAME;
        self.rt.invalidate();
        self.rt.frame(FRAME.as_secs_f64());
        Instant::now()
    }

    /// The screen's top as drawn, in rows from the top of the scrollback.
    fn top(&self) -> f64 {
        let pane = self.rt.state.viewers["t1"]
            .pane
            .as_ref()
            .expect("t1's pane");
        let elements = self.rt.elements();
        let y = |key: &str, inside: &str| {
            elements
                .iter()
                .find(|e| e.key.as_deref() == Some(key) && e.path.contains(inside))
                .map(|e| e.rect[1])
                .expect("drawn")
        };
        let below = y("screen", "pane-pn-1/") - y("pane-pn-1", "");
        pane.snapshot.scrollbar.offset as f64 - f64::from(below / self.line)
    }

    fn wheel(&mut self, dy: f32) {
        let (x, y) = self.at;
        self.rt
            .wheel_event(WheelEvent::new(0.0, dy).at(x, y).precise(true));
    }

    /// Waits a moment into the frame: `k` of it.
    fn wait_into_frame(&self, k: f64) {
        let start = self.tick - FRAME;
        let at = start + FRAME.mul_f64(k);
        let now = Instant::now();
        if at > now {
            std::thread::sleep(at - now);
        }
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn main() {
    let fake = demo_core();
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.with_settings(Settings::fixture());
    // No `settle`: the terminals run on their worker threads, as in the window.
    rt.state.connect(Box::new(fake.clone()));
    let line = rt.state.face(2.0).cell_pt().1;
    let mut bench = Bench {
        rt,
        tick: Instant::now(),
        line,
        at: (0.0, 0.0),
    };
    for _ in 0..120 {
        bench.frame();
    }
    let lines: String = (1..=5000)
        .map(|i| format!("\x1b[3{}mline {i}\x1b[0m of the scrollback\r\n", i % 8))
        .collect();
    let mut bytes = b"\x1bc".to_vec();
    bytes.extend_from_slice(lines.as_bytes());
    fake.output("t1", &bytes);
    for _ in 0..120 {
        bench.frame();
    }
    let pane = bench
        .rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("pane-pn-1"))
        .expect("t1's pane");
    bench.at = (pane.rect[0] + 40.0, pane.rect[1] + 80.0);
    let mut rng = Rng(0x5eed_0051);

    // Latency: a row up at a random moment, then frames until it shows.
    let (mut frames, mut ms) = (Vec::new(), Vec::new());
    for _ in 0..200 {
        let from = bench.top();
        bench.wait_into_frame(rng.next());
        let sent = Instant::now();
        bench.wheel(line);
        let mut n = 0;
        loop {
            n += 1;
            let drawn = bench.frame();
            if (bench.top() - (from - 1.0)).abs() < 0.01 {
                frames.push(f64::from(n));
                ms.push((drawn - sent).as_secs_f64() * 1000.0);
                break;
            }
            assert!(n < 120, "the scroll never showed");
        }
        bench.frame();
    }
    frames.sort_by(f64::total_cmp);
    ms.sort_by(f64::total_cmp);
    let late = frames.iter().filter(|&&f| f > 1.0).count();
    println!("latency, one row, {} events:", frames.len());
    println!(
        "  frames to show: p50 {:.0}, p95 {:.0}, max {:.0}; later than the next frame: {late}",
        percentile(&frames, 0.5),
        percentile(&frames, 0.95),
        percentile(&frames, 1.0)
    );
    println!(
        "  event to frame: p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms",
        percentile(&ms, 0.5),
        percentile(&ms, 0.95),
        percentile(&ms, 1.0)
    );

    // A flick: speeding up over 10 frames, then momentum easing off.
    for _ in 0..10 {
        bench.frame();
    }
    let mut speeds: Vec<f32> = (1..=10).map(|k| 6.0 * k as f32).collect();
    let mut v = 60.0;
    while v > 0.5 {
        v *= 0.95;
        speeds.push(v);
    }
    let start = bench.top();
    let mut fingers = start;
    let (mut errors, mut stalls, mut steps) = (Vec::new(), 0, Vec::new());
    let mut last = start;
    for dy in &speeds {
        bench.wait_into_frame(rng.next());
        bench.wheel(*dy);
        fingers -= f64::from(*dy / line);
        bench.frame();
        let top = bench.top();
        errors.push((top - fingers).abs());
        if (top - last).abs() < 1e-6 {
            stalls += 1;
        }
        steps.push((top - last).abs());
        last = top;
    }
    for _ in 0..6 {
        bench.frame();
    }
    let settled = (bench.top() - fingers).abs();
    errors.sort_by(f64::total_cmp);
    println!(
        "flick, {} events, {:.0} rows:",
        speeds.len(),
        start - fingers
    );
    println!(
        "  screen off the fingers each frame: p50 {:.2}, p95 {:.2}, max {:.2} rows; at rest {:.2}",
        percentile(&errors, 0.5),
        percentile(&errors, 0.95),
        percentile(&errors, 1.0),
        settled
    );
    let rms = (steps.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f64>()
        / (steps.len() - 1) as f64)
        .sqrt();
    println!(
        "  frames the fingers moved and the screen didn't: {stalls}; frame-to-frame change in step: {rms:.2} rows rms"
    );
}
