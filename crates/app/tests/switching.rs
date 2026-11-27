//! Showing another terminal is a swap: clicking an agent or a tab, stepping to the next tab, or
//! showing another space (its key or the space picker), every frame Rune draws of the panes,
//! the tab strip (and, for a space, the whole window) is either what they showed before or what
//! they show after, never a fade between them with the split gap's border color flashing
//! through. Frame by frame through the real renderer, headless: the app's `--scene`, `--script`
//! and `--frames` (Rune's tooling).

use std::path::{Path, PathBuf};
use std::process::Command;

/// The frames dumped: from just before the input, at 60 fps, until well after any fade would
/// have settled (Rune's default entrance is 0.3 s).
const START: f64 = 0.45;
const AT: f64 = 0.5;
const FRAMES: usize = 40;
/// How long a script's tap holds the button: rows and pills act on the release.
const HOLD: f64 = 0.08;

/// An RGB image, cropped to one element.
#[derive(PartialEq)]
struct Pixels {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
}

impl Pixels {
    fn count(&self, color: [u8; 3]) -> usize {
        self.rgb.chunks(3).filter(|p| *p == color).count()
    }

    fn differing(&self, other: &Pixels) -> usize {
        self.rgb
            .chunks(3)
            .zip(other.rgb.chunks(3))
            .filter(|(p, q)| p != q)
            .count()
    }
}

/// What a run drew of its element, and the panes' area's fill: the split gaps' border color.
struct Run {
    frames: Vec<Pixels>,
    gap: [u8; 3],
}

/// A scratch directory for one run's frames, never under the real home.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "herdr-frontend-switching-{name}-{}",
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

/// Runs `scene` through `script` and returns each frame of the element keyed `area`: `main`
/// for the panes, `work` for the tab strip and the panes, `window` for everything. What loops
/// (a working agent's spinner) is masked out: it turns whatever the input does.
fn run(name: &str, scene: &str, script: serde_json::Value, area: &str) -> Run {
    let dir = scratch(name);
    let script_path = dir.join("script.json");
    std::fs::write(&script_path, script.to_string()).unwrap();
    let (out, tree) = (dir.join("frames"), dir.join("tree.json"));
    let status = Command::new(env!("CARGO_BIN_EXE_herdr-frontend"))
        .args(["--scene", scene, "--scale", "1", "--fps", "60"])
        .arg("--script")
        .arg(&script_path)
        .args([
            "--start",
            &START.to_string(),
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
    let fill = element(&tree, "main")["fill"].as_str().unwrap();
    let hex = |i: usize| u8::from_str_radix(&fill[i..i + 2], 16).unwrap();
    let gap = [hex(1), hex(3), hex(5)];

    let frames = (0..FRAMES)
        .map(|i| read(&out.join(format!("frame_{i:04}.png")), rect, &masks))
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    Run { frames, gap }
}

/// Every frame is the first (before the input) or the last (after it), once it shows the new
/// look it never goes back, and it shows it by the first frame after `acts` (seconds).
fn assert_swaps(what: &str, frames: &[Pixels], acts: f64) {
    let (before, after) = (&frames[0], &frames[frames.len() - 1]);
    assert!(before != after, "{what}: the input showed another terminal");
    let mut swapped = None;
    for (i, frame) in frames.iter().enumerate() {
        let t = START + i as f64 / 60.0;
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
        START + first as f64 / 60.0 <= acts + 1.5 / 60.0,
        "{what}: the swap waited until frame {first}"
    );
}

#[test]
fn clicking_an_agent_swaps_its_terminal_in_at_once() {
    // `shell-agents` shows the Pi worker; the pinned agent's row is another terminal, its notes,
    // in another space. The pointer stays on the sidebar, so the strip is compared too.
    let script = serde_json::json!([{ "t": AT, "type": "tap", "key": "ag-t15" }]);
    let run = run("agent", "shell-agents", script, "work");
    assert_swaps("clicking an agent", &run.frames, AT + HOLD);
}

#[test]
fn clicking_an_agent_in_another_space_swaps_the_sidebar_at_once() {
    // The same click, over the Agents list: the selection, the pinned agent's brighter title
    // and the folded orchestration all land on the release. (Before it, the pressed row shows
    // its hover.)
    let script = serde_json::json!([{ "t": AT, "type": "tap", "key": "ag-t15" }]);
    let run = run("agent-list", "shell-agents", script, "ag-list");
    let released = ((AT + HOLD - START) * 60.0).ceil() as usize;
    let after = &run.frames[run.frames.len() - 1];
    assert!(run.frames[0] != *after, "the click moved the selection");
    for (i, frame) in run.frames.iter().enumerate().skip(released + 1) {
        assert!(
            frame == after,
            "frame {i} (t = {:.3} s) is not the list after the click, {} pixels differ",
            START + i as f64 / 60.0,
            frame.differing(after),
        );
    }
}

#[test]
fn clicking_a_tab_swaps_its_panes_in_at_once() {
    // `shell` shows `work`'s three panes; `logs` is one pane in another layout. After the
    // release the pointer leaves the pill, so its tooltip doesn't open over the panes. (The
    // strip isn't compared: the pill under the pointer shows its hover.)
    let script = serde_json::json!([
        { "t": AT, "type": "tap", "key": "tab-tb-logs" },
        { "t": AT + HOLD + 0.02, "type": "move", "x": 100, "y": 600 },
    ]);
    let run = run("tab", "shell", script, "main");
    assert_swaps("clicking a tab", &run.frames, AT + HOLD);
}

#[test]
fn the_next_tab_swaps_the_strip_and_the_panes_at_once() {
    // Ctrl-Tab from `work` to `logs`: the pills' colors, the × moving to `logs` and the panes.
    let script = serde_json::json!([{ "t": AT, "type": "key", "name": "Tab", "ctrl": true }]);
    let run = run("next-tab", "shell", script, "work");
    assert_swaps("the next tab", &run.frames, AT);
}

#[test]
fn a_space_key_swaps_the_window_at_once() {
    // `shell-spaces` shows the first space; Ctrl-2 shows `dotfiles`: the pills, the pane, and
    // the agent row it focuses in the sidebar.
    let script = serde_json::json!([{ "t": AT, "type": "key", "name": "2", "ctrl": true }]);
    let run = run("space-key", "shell-spaces", script, "window");
    assert_swaps("a space key", &run.frames, AT);
}

#[test]
fn picking_a_space_swaps_the_window_at_once() {
    // The space picker, moved down to `dotfiles` well before; Enter shows it, and the picker
    // is gone in the same frame rather than fading out over the new space.
    let script = serde_json::json!([
        { "t": 0.05, "type": "key", "name": "Down" },
        { "t": AT, "type": "key", "name": "Enter" },
    ]);
    let run = run("space-picker", "shell-space-picker", script, "window");
    assert_swaps("picking a space", &run.frames, AT);
}

#[test]
fn closing_the_space_picker_still_fades_it() {
    // Escape switches nothing: the picker keeps its exit fade.
    let script = serde_json::json!([{ "t": AT, "type": "key", "name": "Escape" }]);
    let run = run(
        "space-picker-closed",
        "shell-space-picker",
        script,
        "window",
    );
    let (before, after) = (&run.frames[0], &run.frames[run.frames.len() - 1]);
    assert!(
        run.frames.iter().any(|f| f != before && f != after),
        "the picker closed at once"
    );
}

#[test]
fn resizing_a_pane_keeps_its_terminal_on_its_pane() {
    // Alt-Shift-L grows the left pane; the panes' frames spring there. A terminal that landed
    // at its new width while its pane still sprang would leave a strip of the gaps' border
    // color at the pane's edge.
    let script = serde_json::json!([
        { "t": AT, "type": "key", "name": "l", "alt": true, "shift": true },
    ]);
    let run = run("resize", "shell", script, "main");
    let count = |p: &Pixels| p.count(run.gap);
    let (before, after) = (&run.frames[0], &run.frames[run.frames.len() - 1]);
    assert!(before != after, "the panes resized");
    // At rest the gaps are 1 px lines; a gap between pixels can take one more column.
    let most = count(before).max(count(after)) + before.height;
    for (i, frame) in run.frames.iter().enumerate() {
        assert!(
            count(frame) <= most,
            "frame {i}: {} pixels of the gaps' color, at most {most} at rest",
            count(frame)
        );
    }
}
