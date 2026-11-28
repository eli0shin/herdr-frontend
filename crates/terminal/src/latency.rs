//! Input-to-frame latency tracing (`docs/latency.md`): when a key or click reached each stage on
//! its way to the screen, and when each frame was drawn and shown.
//!
//! Off unless the app starts with [`VAR`] naming a file; then every stage costs one lock, and off
//! it costs one atomic load. One interaction is followed at a time, from its input to the first
//! frame that shows the program's answer:
//!
//! | Stage | Where | What |
//! | --- | --- | --- |
//! | `os` | the input handler | the event's own time (`NSEvent` timestamp), when Rune has it |
//! | `handler` | the input handler | the app's handler ran |
//! | `encoded` | the terminal worker | the key or mouse report was encoded |
//! | `sent` | the UI thread | the bytes went to the connection as DATA |
//! | `echo` | the attach-stream reader | the program's first output after them arrived |
//! | `parsed` | the terminal worker | that output was parsed |
//! | `published` | the terminal worker | a snapshot with it was put in the handoff |
//! | `taken` | the UI thread | the UI took that snapshot |
//! | `built` | the UI thread | the view built the pane's rows from it |
//! | `drawn` | the UI thread | the frame that draws it was encoded and submitted |
//! | `presented` | the UI thread | that frame reached the screen (Metal's `presentedTime`) |
//!
//! Each stage counts only after the one before it, for the same terminal, so output another
//! terminal or a timer prints isn't taken for the answer. An interaction a new one starts before
//! it finished is written as far as it got. Each drawn frame is written too, for the frame
//! pacing: when it started and was drawn, the display link's target for it, when it was shown,
//! how long its update and encode took, and the display's refresh rate. A frame that was due but
//! not drawn (the window covered, minimised or occluded) is written as skipped.
//!
//! The lines are JSON, written off the threads that record them. Times are milliseconds: an
//! interaction's from its handler (so `os` is negative), a frame's from the trace's start.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// The variable that turns tracing on: the file the lines are appended to.
pub const VAR: &str = "HERDR_FRONTEND_LATENCY";

/// A stage of one interaction, in the order they happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Os,
    Handler,
    Encoded,
    Sent,
    Echo,
    Parsed,
    Published,
    Taken,
    Built,
    Drawn,
    Presented,
}

impl Stage {
    pub const ALL: [Stage; 11] = [
        Stage::Os,
        Stage::Handler,
        Stage::Encoded,
        Stage::Sent,
        Stage::Echo,
        Stage::Parsed,
        Stage::Published,
        Stage::Taken,
        Stage::Built,
        Stage::Drawn,
        Stage::Presented,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Stage::Os => "os",
            Stage::Handler => "handler",
            Stage::Encoded => "encoded",
            Stage::Sent => "sent",
            Stage::Echo => "echo",
            Stage::Parsed => "parsed",
            Stage::Published => "published",
            Stage::Taken => "taken",
            Stage::Built => "built",
            Stage::Drawn => "drawn",
            Stage::Presented => "presented",
        }
    }

    /// The stage that must have happened first, for this one to count.
    fn after(self) -> Option<Stage> {
        match self {
            Stage::Os | Stage::Handler => None,
            Stage::Encoded => Some(Stage::Handler),
            Stage::Sent => Some(Stage::Encoded),
            Stage::Echo => Some(Stage::Sent),
            Stage::Parsed => Some(Stage::Echo),
            Stage::Published => Some(Stage::Parsed),
            Stage::Taken => Some(Stage::Published),
            Stage::Built => Some(Stage::Taken),
            Stage::Drawn => Some(Stage::Taken),
            Stage::Presented => Some(Stage::Drawn),
        }
    }
}

/// One interaction on its way to the screen.
#[derive(Clone, Debug)]
struct Probe {
    input: &'static str,
    terminal: u64,
    times: [Option<Instant>; Stage::ALL.len()],
    /// The display link's target for the frame that drew it.
    target: Option<Instant>,
    /// The frame that drew it, to match its presentation.
    seq: Option<u64>,
}

impl Probe {
    fn at(&self, stage: Stage) -> Option<Instant> {
        self.times[stage as usize]
    }

    fn line(&self) -> String {
        let zero = self
            .at(Stage::Handler)
            .expect("a probe starts at its handler");
        let ms = |t: Instant| signed_ms(t, zero);
        let mut line = format!(
            "{{\"input\":\"{}\",\"terminal\":{}",
            self.input, self.terminal
        );
        for stage in Stage::ALL {
            if let Some(t) = self.at(stage) {
                let _ = write!(line, ",\"{}\":{:.3}", stage.name(), ms(t));
            }
        }
        if let Some(t) = self.target {
            let _ = write!(line, ",\"target\":{:.3}", ms(t));
        }
        line.push('}');
        line
    }
}

/// A drawn frame, as the window reported it (Rune's `FrameEncoded`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// The window's frame number, which its presentation names.
    pub seq: u64,
    /// When the frame started, after its drawable came.
    pub started: Option<Instant>,
    /// When its commands were submitted.
    pub drawn: Instant,
    /// The refresh the display link aimed it at.
    pub target: Option<Instant>,
    /// The display's refresh rate.
    pub hz: Option<f64>,
    /// Its update (events, poll, view, layout, paint) and encode, in milliseconds.
    pub update_ms: Option<f64>,
    pub encode_ms: Option<f64>,
}

impl Frame {
    /// A frame drawn at `drawn` with nothing else known: for tests.
    pub fn at(seq: u64, drawn: Instant) -> Frame {
        Frame {
            seq,
            started: None,
            drawn,
            target: None,
            hz: None,
            update_ms: None,
            encode_ms: None,
        }
    }
}

/// How many drawn frames wait for their presentation before the oldest is written without one.
const FRAMES_IN_FLIGHT: usize = 8;

/// The tracing state, with every time given: [`begin`], [`mark`] and the frame calls drive the
/// process's one recorder, and tests drive their own.
#[derive(Debug)]
pub struct Recorder {
    epoch: Instant,
    probe: Option<Probe>,
    frames: VecDeque<Frame>,
}

impl Recorder {
    pub fn new(epoch: Instant) -> Recorder {
        Recorder {
            epoch,
            probe: None,
            frames: VecDeque::new(),
        }
    }

    /// An input reached its handler at `now` (the OS stamped it `os`), for `terminal`. The
    /// interaction before it is written as far as it got.
    pub fn begin(
        &mut self,
        input: &'static str,
        terminal: u64,
        os: Option<Instant>,
        now: Instant,
    ) -> Vec<String> {
        let out = self.probe.take().map(|p| p.line()).into_iter().collect();
        let mut times = [None; Stage::ALL.len()];
        times[Stage::Os as usize] = os;
        times[Stage::Handler as usize] = Some(now);
        self.probe = Some(Probe {
            input,
            terminal,
            times,
            target: None,
            seq: None,
        });
        out
    }

    /// `terminal` reached `stage` at `now`: it counts if the interaction is on that terminal,
    /// the stage before it happened, and this stage hasn't yet.
    pub fn mark(&mut self, stage: Stage, terminal: u64, now: Instant) {
        if let Some(probe) = &mut self.probe
            && probe.terminal == terminal
            && probe.at(stage).is_none()
            && stage
                .after()
                .is_none_or(|before| probe.at(before).is_some())
        {
            probe.times[stage as usize] = Some(now);
        }
    }

    /// A frame was drawn. The interaction whose snapshot was taken is in it.
    pub fn drawn(&mut self, frame: Frame) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(probe) = &mut self.probe
            && probe.at(Stage::Taken).is_some()
            && probe.at(Stage::Drawn).is_none()
        {
            probe.times[Stage::Drawn as usize] = Some(frame.drawn);
            probe.target = frame.target;
            probe.seq = Some(frame.seq);
        }
        if self.frames.len() == FRAMES_IN_FLIGHT
            && let Some(old) = self.frames.pop_front()
        {
            out.push(self.frame_line(old, None));
        }
        self.frames.push_back(frame);
        out
    }

    /// Frame `seq` reached the screen at `presented` (`None`: the system skipped it). The
    /// interaction it drew is done.
    pub fn presented(&mut self, seq: u64, presented: Option<Instant>) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(frame) = self.frames.front().copied() {
            if frame.seq > seq {
                break;
            }
            self.frames.pop_front();
            let at = (frame.seq == seq).then_some(presented).flatten();
            out.push(self.frame_line(frame, at));
        }
        if let Some(probe) = &mut self.probe
            && probe.seq == Some(seq)
        {
            probe.times[Stage::Presented as usize] = presented;
            out.push(probe.line());
            self.probe = None;
        }
        out
    }

    /// Frame `seq` was due at `at` but not drawn: the window is covered, minimised or occluded,
    /// or no drawable came in time.
    pub fn skipped(&self, seq: u64, at: Instant) -> Vec<String> {
        vec![format!(
            "{{\"frame\":{seq},\"skipped\":{:.3}}}",
            signed_ms(at, self.epoch)
        )]
    }

    /// Everything not written yet, as far as it got.
    pub fn finish(&mut self) -> Vec<String> {
        let mut out: Vec<String> = std::mem::take(&mut self.frames)
            .into_iter()
            .map(|f| self.frame_line(f, None))
            .collect();
        out.extend(self.probe.take().map(|p| p.line()));
        out
    }

    fn frame_line(&self, frame: Frame, presented: Option<Instant>) -> String {
        let ms = |t: Instant| signed_ms(t, self.epoch);
        let mut line = format!("{{\"frame\":{}", frame.seq);
        let times = [
            ("started", frame.started),
            ("drawn", Some(frame.drawn)),
            ("target", frame.target),
            ("presented", presented),
        ];
        for (name, t) in times {
            if let Some(t) = t {
                let _ = write!(line, ",\"{name}\":{:.3}", ms(t));
            }
        }
        let values = [
            ("update_ms", frame.update_ms),
            ("encode_ms", frame.encode_ms),
            ("hz", frame.hz),
        ];
        for (name, v) in values {
            if let Some(v) = v.filter(|v| v.is_finite()) {
                let _ = write!(line, ",\"{name}\":{v:.3}");
            }
        }
        line.push('}');
        line
    }
}

fn signed_ms(t: Instant, zero: Instant) -> f64 {
    match t.checked_duration_since(zero) {
        Some(d) => d.as_secs_f64() * 1e3,
        None => -(zero.duration_since(t).as_secs_f64() * 1e3),
    }
}

/// The process's recorder and where its lines go.
struct Tracer {
    recorder: Mutex<Recorder>,
    lines: Mutex<Sender<String>>,
}

static TRACER: OnceLock<Option<Tracer>> = OnceLock::new();

fn tracer() -> Option<&'static Tracer> {
    TRACER
        .get_or_init(|| {
            let path = std::env::var_os(VAR).filter(|p| !p.is_empty())?;
            let file = match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                Ok(file) => file,
                Err(e) => {
                    eprintln!("herdr-frontend: {VAR}: {}: {e}", path.to_string_lossy());
                    return None;
                }
            };
            let (lines, inbox) = mpsc::channel::<String>();
            // Writing is I/O: never on the threads that record.
            std::thread::Builder::new()
                .name("latency".into())
                .spawn(move || {
                    let mut file = std::io::LineWriter::new(file);
                    for line in inbox {
                        let _ = writeln!(file, "{line}");
                    }
                })
                .ok()?;
            Some(Tracer {
                recorder: Mutex::new(Recorder::new(Instant::now())),
                lines: Mutex::new(lines),
            })
        })
        .as_ref()
}

fn with(f: impl FnOnce(&mut Recorder) -> Vec<String>) {
    let Some(tracer) = tracer() else { return };
    let lines = f(&mut tracer.recorder.lock().unwrap_or_else(|e| e.into_inner()));
    if lines.is_empty() {
        return;
    }
    let sender = tracer.lines.lock().unwrap_or_else(|e| e.into_inner());
    for line in lines {
        let _ = sender.send(line);
    }
}

/// Whether this process traces ([`VAR`] names a file it could open).
pub fn enabled() -> bool {
    tracer().is_some()
}

/// An input for `terminal` reached its handler now; the OS stamped it `os`.
pub fn begin(input: &'static str, terminal: u64, os: Option<Instant>) {
    with(|r| r.begin(input, terminal, os, Instant::now()));
}

/// `terminal` reached `stage` now.
pub fn mark(stage: Stage, terminal: u64) {
    with(|r| {
        r.mark(stage, terminal, Instant::now());
        Vec::new()
    });
}

/// A frame was drawn: encoded and submitted.
pub fn drawn(frame: Frame) {
    with(|r| r.drawn(frame));
}

/// Frame `seq` was due at `at` but not drawn.
pub fn skipped(seq: u64, at: Instant) {
    with(|r| r.skipped(seq, at));
}

/// Frame `seq` reached the screen at `at`, or the system skipped it (`None`).
pub fn presented(seq: u64, at: Option<Instant>) {
    with(|r| r.presented(seq, at));
}

/// Writes what's left: call when the app quits.
pub fn finish() {
    with(Recorder::finish);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_key_is_followed_to_the_frame_that_shows_its_echo() {
        let t0 = Instant::now();
        let mut r = Recorder::new(t0);
        assert!(r.begin("key", 7, Some(t0), t0 + ms(2)).is_empty());
        r.mark(Stage::Encoded, 7, t0 + ms(3));
        r.mark(Stage::Sent, 7, t0 + ms(10));
        r.mark(Stage::Echo, 7, t0 + ms(12));
        r.mark(Stage::Parsed, 7, t0 + ms(13));
        r.mark(Stage::Published, 7, t0 + ms(14));
        r.mark(Stage::Taken, 7, t0 + ms(20));
        r.mark(Stage::Built, 7, t0 + ms(21));
        let frame = Frame {
            started: Some(t0 + ms(19)),
            target: Some(t0 + ms(30)),
            hz: Some(60.0),
            update_ms: Some(1.5),
            encode_ms: Some(0.25),
            ..Frame::at(5, t0 + ms(22))
        };
        assert!(r.drawn(frame).is_empty());
        let lines = r.presented(5, Some(t0 + ms(31)));
        assert_eq!(
            lines,
            [
                "{\"frame\":5,\"started\":19.000,\"drawn\":22.000,\"target\":30.000,\
                 \"presented\":31.000,\"update_ms\":1.500,\"encode_ms\":0.250,\"hz\":60.000}",
                "{\"input\":\"key\",\"terminal\":7,\"os\":-2.000,\"handler\":0.000,\
                 \"encoded\":1.000,\"sent\":8.000,\"echo\":10.000,\"parsed\":11.000,\
                 \"published\":12.000,\"taken\":18.000,\"built\":19.000,\"drawn\":20.000,\
                 \"presented\":29.000,\"target\":28.000}"
            ]
        );
        assert!(r.finish().is_empty());
    }

    #[test]
    fn a_stage_counts_only_after_the_one_before_it_on_the_same_terminal() {
        let t0 = Instant::now();
        let mut r = Recorder::new(t0);
        r.begin("key", 1, None, t0);
        // Output before the key went out isn't its echo, nor is another terminal's.
        r.mark(Stage::Echo, 1, t0 + ms(1));
        r.mark(Stage::Encoded, 2, t0 + ms(1));
        r.mark(Stage::Encoded, 1, t0 + ms(2));
        r.mark(Stage::Encoded, 1, t0 + ms(3));
        // A frame before the snapshot was taken doesn't draw it.
        r.drawn(Frame::at(1, t0 + ms(4)));
        let lines = r.finish();
        assert_eq!(
            lines,
            [
                "{\"frame\":1,\"drawn\":4.000}",
                "{\"input\":\"key\",\"terminal\":1,\"handler\":0.000,\"encoded\":2.000}"
            ]
        );
    }

    #[test]
    fn a_new_input_writes_the_unfinished_one() {
        let t0 = Instant::now();
        let mut r = Recorder::new(t0);
        r.begin("click", 1, None, t0);
        let lines = r.begin("key", 1, None, t0 + ms(5));
        assert_eq!(
            lines,
            ["{\"input\":\"click\",\"terminal\":1,\"handler\":0.000}"]
        );
    }

    #[test]
    fn frames_the_system_skipped_or_never_reported_are_written_without_a_present_time() {
        let t0 = Instant::now();
        let mut r = Recorder::new(t0);
        r.drawn(Frame::at(1, t0 + ms(1)));
        r.drawn(Frame::at(2, t0 + ms(2)));
        assert_eq!(
            r.presented(2, None),
            [
                "{\"frame\":1,\"drawn\":1.000}",
                "{\"frame\":2,\"drawn\":2.000}"
            ]
        );
        for seq in 0..FRAMES_IN_FLIGHT as u64 + 1 {
            let out = r.drawn(Frame::at(10 + seq, t0 + ms(10 + seq)));
            assert_eq!(out.len(), usize::from(seq == FRAMES_IN_FLIGHT as u64));
        }
    }
}
