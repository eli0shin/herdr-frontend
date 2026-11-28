//! The app's frames, one by one, through the real renderer, headless: the binary's `--scene`,
//! `--script` and `--frames` (Rune's tooling), cropped to one element. Shared by
//! `switching.rs` and `no_animations.rs`: every frame around an input is either the look before
//! it or the look after it, never one between (`AGENTS.md`: no animations).

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// When the script's input comes, and the dump starts just before it.
pub const AT: f64 = 0.5;
pub const START: f64 = AT - 0.05;
/// The frames dumped, at 60 fps: until well after any fade would have settled (Rune's default
/// entrance is 0.3 s, its exit 0.24 s).
pub const FRAMES: usize = 40;
pub const FPS: f64 = 60.0;
/// How long a script's tap holds the button: rows and pills act on the release.
pub const HOLD: f64 = 0.08;

/// An RGB image, cropped to one element.
#[derive(PartialEq)]
pub struct Pixels {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

impl Pixels {
    pub fn count(&self, color: [u8; 3]) -> usize {
        self.rgb.chunks(3).filter(|p| *p == color).count()
    }

    pub fn differing(&self, other: &Pixels) -> usize {
        self.rgb
            .chunks(3)
            .zip(other.rgb.chunks(3))
            .filter(|(p, q)| p != q)
            .count()
    }
}

/// What a run drew of its element from `start`, and the split gaps' border color.
pub struct Run {
    pub frames: Vec<Pixels>,
    pub gap: [u8; 3],
    pub start: f64,
}

impl Run {
    /// The time frame `i` shows.
    pub fn t(&self, i: usize) -> f64 {
        self.start + i as f64 / FPS
    }
}

/// A scratch directory for one run's frames, never under the real home.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "herdr-frontend-frames-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The PNG at `path`, as RGB, cropped to `rect` (device pixels: x, y, w, h), with what's under
/// each of `masks` (the same) black.
fn read(path: &Path, rect: [usize; 4], masks: &[[usize; 4]]) -> Pixels {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let channels = info.color_type.samples();
    let [x, y, w, h] = rect;
    assert!(x + w <= info.width as usize && y + h <= info.height as usize);
    let mut rgb = Vec::with_capacity(w * h * 3);
    let masked = |col: usize, row: usize| {
        masks
            .iter()
            .any(|&[mx, my, mw, mh]| (mx..mx + mw).contains(&col) && (my..my + mh).contains(&row))
    };
    for row in y..y + h {
        for col in x..x + w {
            if masked(col, row) {
                rgb.extend_from_slice(&[0, 0, 0]);
                continue;
            }
            let at = (row * info.width as usize + col) * channels;
            rgb.extend_from_slice(&buf[at..at + 3]);
        }
    }
    Pixels {
        width: w,
        height: h,
        rgb,
    }
}

/// An element of a `--tree` by key.
fn element<'a>(tree: &'a serde_json::Value, key: &str) -> &'a serde_json::Value {
    tree["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["key"] == key)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
}

/// Runs `scene` through `script` and returns each frame of the element keyed `area`, from
/// [`START`]: `main` for the panes, `work` for the tab strip and the panes, `window` for
/// everything.
pub fn run(name: &str, scene: &str, script: serde_json::Value, area: &str) -> Run {
    run_from(name, scene, script, area, START)
}

/// [`run`], dumping from `start`. What loops (a working agent's spinner) is masked out: it
/// turns whatever the input does.
pub fn run_from(name: &str, scene: &str, script: serde_json::Value, area: &str, start: f64) -> Run {
    let dir = scratch(name);
    let script_path = dir.join("script.json");
    std::fs::write(&script_path, script.to_string()).unwrap();
    let (out, tree) = (dir.join("frames"), dir.join("tree.json"));
    let status = Command::new(env!("CARGO_BIN_EXE_herdr-frontend"))
        .args(["--scene", scene, "--scale", "1", "--fps", &FPS.to_string()])
        .arg("--script")
        .arg(&script_path)
        .args([
            "--start",
            &start.to_string(),
            "--frames",
            &FRAMES.to_string(),
        ])
        .arg("--out")
        .arg(&out)
        .arg("--tree")
        .arg(&tree)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "{scene}: {status}");

    let tree: serde_json::Value = serde_json::from_slice(&std::fs::read(&tree).unwrap()).unwrap();
    // The view's points times the device scale and zoom; whole pixels inside the rect.
    let scale = tree["scale"].as_f64().unwrap();
    let points = |e: &serde_json::Value| -> Vec<f64> {
        e["rect"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap() * scale)
            .collect()
    };
    let r = points(element(&tree, area));
    let (x0, y0) = (r[0].ceil(), r[1].ceil());
    let (x1, y1) = ((r[0] + r[2]).floor(), (r[1] + r[3]).floor());
    let rect = [x0, y0, x1 - x0, y1 - y0].map(|v| v as usize);
    // A turning element's corners sweep a circle around its center: every pixel it touches.
    let masks: Vec<[usize; 4]> = tree["elements"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| !e["loopPhase"].is_null())
        .map(|e| {
            let r = points(e);
            let (cx, cy, radius) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2].hypot(r[3]) / 2.0);
            let (x0, y0) = ((cx - radius).floor() - 1.0, (cy - radius).floor() - 1.0);
            let (x1, y1) = ((cx + radius).ceil() + 1.0, (cy + radius).ceil() + 1.0);
            [x0, y0, x1 - x0, y1 - y0].map(|v| v.max(0.0) as usize)
        })
        .collect();
    // The gaps' fill, or the tab area's where no split draws one (none before a core).
    let elements = tree["elements"].as_array().unwrap();
    let gap = elements
        .iter()
        .find(|e| e["key"].as_str().is_some_and(|k| k.starts_with("gap-")))
        .or_else(|| elements.iter().find(|e| e["key"] == "main"))
        .and_then(|e| e["fill"].as_str())
        .map_or([0; 3], |fill| {
            let hex = |i: usize| u8::from_str_radix(&fill[i..i + 2], 16).unwrap();
            [hex(1), hex(3), hex(5)]
        });

    let frames = (0..FRAMES)
        .map(|i| read(&out.join(format!("frame_{i:04}.png")), rect, &masks))
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    Run { frames, gap, start }
}

/// Every frame is the first (before the input) or the last (after it), once it shows the new
/// look it never goes back, and it shows it by the first frame after `acts` (seconds).
pub fn assert_swaps(what: &str, run: &Run, acts: f64) {
    let frames = &run.frames;
    let (before, after) = (&frames[0], &frames[frames.len() - 1]);
    assert!(
        before != after,
        "{what}: the input changed nothing on screen"
    );
    let mut swapped = None;
    for (i, frame) in frames.iter().enumerate() {
        let t = run.t(i);
        if frame == after {
            swapped.get_or_insert(i);
            continue;
        }
        assert!(
            frame == before,
            "{what}: frame {i} (t = {t:.3} s) is neither the look before the input nor after it, \
             {} of {} pixels differ from after",
            frame.differing(after),
            frame.width * frame.height,
        );
        assert!(
            swapped.is_none(),
            "{what}: frame {i} went back to before after the swap"
        );
    }
    let first = swapped.expect("the new look shows");
    assert!(
        run.t(first) <= acts + 1.5 / FPS,
        "{what}: the swap waited until frame {first}"
    );
}
