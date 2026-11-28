//! Input-to-frame latency on this Mac (`docs/latency.md`): herdr-frontend in a real window on an
//! isolated, seeded herdr-core, with its latency trace on (`terminal::latency`), keys typed into a
//! shell through its control socket, and a report of where each key's time went on its way to
//! the screen, plus the frame pacing while a program floods the terminal.
//!
//! It opens a window on the person's display, so plain `cargo test` only builds it: it runs when
//! `HERDR_LATENCY=1` (`just latency`). Like the end-to-end harness it uses only its own core and
//! home, and sends no OS-level input. `--summarize <trace>` reports a trace recorded elsewhere
//! (`HERDR_FRONTEND_LATENCY=<file> herdr-frontend`, typing and clicking by hand) and opens nothing.
//!
//! ```text
//! just latency [--keys <n>] [--gap <ms>] [--no-flood] [--run <command>] [--app <herdr-frontend>]
//! just latency --summarize <trace.jsonl>
//! ```

#[allow(dead_code)]
#[path = "../e2e/apps.rs"]
mod apps;
#[allow(dead_code)]
#[path = "../e2e/isolated.rs"]
mod isolated;
#[allow(dead_code)]
#[path = "../e2e/seed.rs"]
mod seed;

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use apps::{App, Programs, Which};

/// Runs it; without it the binary only says it skipped.
const RUN_VAR: &str = "HERDR_LATENCY";

/// Where `just latency` ran: relative paths in the arguments are from there, since Cargo runs
/// this from the package's folder.
const CWD_VAR: &str = "HERDR_LATENCY_CWD";

/// The stages of one interaction, in order (`terminal::latency::Stage`).
const STAGES: [&str; 11] = [
    "os",
    "handler",
    "encoded",
    "sent",
    "echo",
    "parsed",
    "published",
    "taken",
    "built",
    "drawn",
    "presented",
];

/// What each step from the stage before means, for the report.
const STEPS: [(&str, &str); 10] = [
    ("handler", "OS event to the app's handler"),
    ("encoded", "handler to the worker's encoding"),
    ("sent", "encoded to DATA on the connection"),
    ("echo", "DATA to the program's echo arriving"),
    ("parsed", "echo to parsed by the worker"),
    ("published", "parsed to the snapshot published"),
    ("taken", "published to the UI taking it"),
    ("built", "taken to the pane's rows built"),
    ("drawn", "built to the frame encoded"),
    ("presented", "encoded to on screen"),
];

struct Options {
    keys: usize,
    gap: Duration,
    flood: bool,
    run: Option<String>,
    app: Option<PathBuf>,
    summarize: Option<PathBuf>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `cargo test` hands every test binary its filter and flags: only a run or a summary reads them.
    if std::env::var_os(RUN_VAR).is_none() && !args.iter().any(|a| a == "--summarize") {
        println!("latency: skipped: it opens a window on your display (`just latency` runs it)");
        return;
    }
    let options = match parse(args.into_iter()) {
        Ok(options) => options,
        Err(e) => {
            eprintln!("latency: {e}");
            std::process::exit(2);
        }
    };
    if let Some(trace) = &options.summarize {
        match std::fs::read_to_string(trace) {
            Ok(text) => print!("{}", summarize(&text)),
            Err(e) => {
                eprintln!("latency: {}: {e}", trace.display());
                std::process::exit(1);
            }
        }
        return;
    }
    if let Err(e) = run(&options) {
        eprintln!("latency: {e}");
        std::process::exit(1);
    }
}

fn run(options: &Options) -> Result<(), String> {
    let hc = std::env::var_os("HERDR_TEST_HC")
        .map(PathBuf::from)
        .or_else(|| herdr_client::find_hc(None))
        .ok_or("no hc: install herdr-core or set HERDR_TEST_HC")?;
    let programs = Programs {
        frontend: options
            .app
            .clone()
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_herdr-frontend"))),
        herdr_app: None,
    };
    if !programs.frontend.is_file() {
        return Err(format!("no app at {}", programs.frontend.display()));
    }
    let stamp = output("date", &["+%Y%m%d-%H%M%S"])?;
    let out = target_dir()?.join("latency");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let trace = out.join(format!("{stamp}.jsonl"));
    let log = out.join(format!("{stamp}.log"));
    // The window inherits it; nothing else in this process has started yet.
    // SAFETY: no other thread is running to read the environment concurrently.
    unsafe { std::env::set_var(terminal::latency::VAR, &trace) };
    eprintln!(
        "latency: {} on an isolated core; trace {}",
        programs.frontend.display(),
        trace.display()
    );

    let core = isolated::Core::start(&hc)?;
    assert!(core.session().starts_with("test-"));
    let world = seed::World::seed(&core)?;
    let mut app = App::launch(Which::Frontend, &programs, &core, &log)?;
    app.wait_ready(&core, Some(&world.space_ids()))?;
    let measured = measure(options, &app, &core);
    // Time for the last frames to be shown and written.
    std::thread::sleep(Duration::from_millis(500));
    app.quit(&core);
    measured?;
    std::thread::sleep(Duration::from_millis(200));
    let text = std::fs::read_to_string(&trace).map_err(|e| format!("{}: {e}", trace.display()))?;
    print!("{}", summarize(&text));
    println!("trace: {}", trace.display());
    Ok(())
}

/// Types into the split tab's shell: keys one at a time with a gap, then more while a program
/// floods the terminal.
fn measure(options: &Options, app: &App, core: &isolated::Core) -> Result<(), String> {
    // A covered window draws nothing, so nothing would reach the screen: bring it forward, as
    // a second launch does. A tiling window manager may still leave it under others; the
    // report then counts the frames macOS skipped.
    app.probe(&json!({ "raise": true }))?;
    let (code, reply) = app.cli(core, &["do", "space.select", r#"{"index":1}"#])?;
    if code != 0 {
        return Err(format!("space.select: {reply}"));
    }
    app.settle(Duration::from_secs(3));
    if let Some(command) = &options.run {
        app.probe(&json!({ "type": format!("{command}\n") }))?;
        app.settle(Duration::from_secs(3));
    }
    let letters = "abcdefghijklmnopqrstuvwxyz";
    let mut typed = 0;
    for i in 0..options.keys {
        // A fresh line every 40 keys, so the line never wraps.
        let key = if i % 40 == 39 {
            "ctrl-u".to_owned()
        } else {
            letters[i % 26..i % 26 + 1].to_owned()
        };
        app.probe(&json!({ "keys": key }))?;
        typed += 1;
        // Spread the keys across the display's refresh, as a person's are.
        let jitter = Duration::from_micros((i as u64 * 7919) % 16_667);
        std::thread::sleep(options.gap + jitter);
    }
    app.probe(&json!({ "keys": "ctrl-u" }))?;
    std::thread::sleep(Duration::from_millis(300));
    if options.flood {
        app.probe(
            &json!({ "type": "yes 0123456789abcdefghijklmnopqrstuvwxyz | head -n 400000\n" }),
        )?;
        std::thread::sleep(Duration::from_millis(200));
        for i in 0..10 {
            app.probe(&json!({ "keys": &letters[i..i + 1] }))?;
            std::thread::sleep(Duration::from_millis(150));
        }
        app.settle(Duration::from_secs(10));
        app.probe(&json!({ "keys": "ctrl-u" }))?;
    }
    eprintln!("latency: typed {typed} keys");
    Ok(())
}

/// The report: each stage's share of an interaction's time, the totals, and the frame pacing.
fn summarize(text: &str) -> String {
    let lines: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let probes: Vec<&Value> = lines.iter().filter(|l| l.get("input").is_some()).collect();
    let frames: Vec<&Value> = lines.iter().filter(|l| l.get("frame").is_some()).collect();
    let mut out = String::new();
    let complete: Vec<&&Value> = probes
        .iter()
        .filter(|p| p.get("presented").is_some())
        .collect();
    let skipped = frames.iter().filter(|f| f.get("skipped").is_some()).count();
    out.push_str(&format!(
        "{} interactions, {} reached the screen; {} frames drawn, {skipped} skipped \
         (the window was covered)\n\n",
        probes.len(),
        complete.len(),
        frames.len() - skipped,
    ));
    // Each step over every interaction that got that far; the totals over those that reached
    // the screen.
    for input in ["key", "click", "text"] {
        let these: Vec<&Value> = probes
            .iter()
            .filter(|p| p["input"] == input)
            .copied()
            .collect();
        if these.is_empty() {
            continue;
        }
        out.push_str(&header(&format!("{input}s ({}), ms", these.len())));
        for (i, (stage, what)) in STEPS.iter().enumerate() {
            let before = STAGES[i];
            let gaps: Vec<f64> = these
                .iter()
                .filter_map(|p| Some(p[stage].as_f64()? - p[before].as_f64()?))
                .collect();
            if gaps.is_empty() {
                continue;
            }
            out.push_str(&row(what, &gaps));
        }
        // A frame that drew without building rows (or a trace from before `built`) still has
        // a frame: the UI's whole part of it, from the snapshot taken.
        let frame: Vec<f64> = these
            .iter()
            .filter_map(|p| Some(p["drawn"].as_f64()? - p["taken"].as_f64()?))
            .collect();
        if !frame.is_empty() {
            out.push_str(&row("taken to the frame encoded, in all", &frame));
        }
        let total = |from: &str| -> Vec<f64> {
            complete
                .iter()
                .filter(|p| p["input"] == input)
                .filter_map(|p| Some(p["presented"].as_f64()? - p[from].as_f64()?))
                .collect()
        };
        let ours: Vec<f64> = complete
            .iter()
            .filter(|p| p["input"] == input)
            .filter_map(|p| {
                let all = p["presented"].as_f64()? - p["handler"].as_f64()?;
                let theirs = p["echo"].as_f64()? - p["sent"].as_f64()?;
                Some(all - theirs)
            })
            .collect();
        let handler = total("handler");
        if !handler.is_empty() {
            out.push_str(&row("handler to on screen", &handler));
            out.push_str(&row("  of which ours (not DATA to echo)", &ours));
        }
        let os = total("os");
        if !os.is_empty() {
            out.push_str(&row("OS event to on screen", &os));
        }
        out.push('\n');
    }
    out.push_str(&pacing(&frames));
    out
}

/// The frame pacing: the display's refresh, the gaps between frames shown back to back, how
/// many refreshes were missed while drawing continuously, and the frames' CPU time.
fn pacing(frames: &[&Value]) -> String {
    let mut out = String::new();
    let hz: Vec<f64> = frames.iter().filter_map(|f| f["hz"].as_f64()).collect();
    let refresh = hz.last().copied();
    match refresh {
        Some(hz) => out.push_str(&format!("display refresh: {hz:.1} Hz\n")),
        None => out.push_str("display refresh: not reported\n"),
    }
    let period = 1000.0 / refresh.unwrap_or(60.0);
    let presented: Vec<f64> = frames
        .iter()
        .filter_map(|f| f["presented"].as_f64())
        .collect();
    // Frames shown back to back: a gap under three refreshes is the window drawing continuously.
    let gaps: Vec<f64> = presented
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|g| *g < 3.0 * period)
        .collect();
    if !gaps.is_empty() {
        out.push_str(&header("frames, ms"));
        out.push_str(&row("between frames shown in a run", &gaps));
        let missed = gaps.iter().filter(|g| **g > 1.5 * period).count();
        out.push_str(&format!(
            "  {missed} of {} gaps in a run missed a refresh\n",
            gaps.len()
        ));
    }
    let lag: Vec<f64> = frames
        .iter()
        .filter_map(|f| Some(f["presented"].as_f64()? - f["drawn"].as_f64()?))
        .collect();
    if !lag.is_empty() {
        out.push_str(&row("encoded to on screen", &lag));
    }
    for (name, what) in [
        ("update_ms", "update (poll, view, paint)"),
        ("encode_ms", "encode"),
    ] {
        let values: Vec<f64> = frames.iter().filter_map(|f| f[name].as_f64()).collect();
        if !values.is_empty() {
            out.push_str(&row(what, &values));
        }
    }
    out
}

/// A table's first line, its columns over [`row`]'s.
fn header(title: &str) -> String {
    format!("{title:<45}p50     p90     max      n\n")
}

fn row(what: &str, values: &[f64]) -> String {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    format!(
        "  {what:<38} {:>7.1} {:>7.1} {:>7.1} {:>6}\n",
        at(0.5),
        at(0.9),
        sorted[sorted.len() - 1],
        sorted.len()
    )
}

fn parse(args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        keys: 60,
        gap: Duration::from_millis(150),
        flood: true,
        run: None,
        app: None,
        summarize: None,
    };
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--keys" => options.keys = value()?.parse().map_err(|_| "--keys takes a count")?,
            "--gap" => {
                let ms: u64 = value()?.parse().map_err(|_| "--gap takes milliseconds")?;
                options.gap = Duration::from_millis(ms);
            }
            "--no-flood" => options.flood = false,
            "--run" => options.run = Some(value()?),
            "--app" => options.app = Some(from_cwd(value()?)),
            "--summarize" => options.summarize = Some(from_cwd(value()?)),
            other => return Err(format!("unknown argument {other:?} (see docs/latency.md)")),
        }
    }
    Ok(options)
}

/// A path from the arguments, relative to where `just latency` ran ([`CWD_VAR`]).
fn from_cwd(path: String) -> PathBuf {
    match std::env::var_os(CWD_VAR) {
        Some(cwd) => Path::new(&cwd).join(path),
        None => PathBuf::from(path),
    }
}

/// The Cargo target folder this binary was built into (`target/debug/deps/latency-…`).
fn target_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    exe.ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("no target folder above {}", exe.display()))
}

fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
