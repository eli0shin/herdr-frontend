//! Showing another terminal is a swap: clicking an agent or a tab, stepping to the next tab, or
//! showing another space (its key or the space picker), every frame Rune draws of the panes,
//! the tab strip (and, for a space, the whole window) is either what they showed before or what
//! they show after, never a fade between them with the split gap's border color flashing
//! through. Frame by frame through the real renderer, headless: the app's `--scene`, `--script`
//! and `--frames` (Rune's tooling).

mod frames;

use frames::{AT, FPS, HOLD, Pixels, START, assert_swaps, run};

#[test]
fn clicking_an_agent_swaps_its_terminal_in_at_once() {
    // `shell-agents` shows the Pi worker; the pinned agent's row is another terminal, its notes,
    // in another space. The pointer stays on the sidebar, so the strip is compared too.
    let script = serde_json::json!([{ "t": AT, "type": "tap", "key": "ag-t15" }]);
    let run = run("agent", "shell-agents", script, "work");
    assert_swaps("clicking an agent", &run, AT + HOLD);
}

#[test]
fn clicking_an_agent_in_another_space_swaps_the_sidebar_at_once() {
    // The same click, over the Agents list: the selection, the pinned agent's brighter title
    // and the folded orchestration all land on the release. (Before it, the pressed row shows
    // its hover.)
    let script = serde_json::json!([{ "t": AT, "type": "tap", "key": "ag-t15" }]);
    let run = run("agent-list", "shell-agents", script, "ag-list");
    let released = ((AT + HOLD - START) * FPS).ceil() as usize;
    let after = &run.frames[run.frames.len() - 1];
    assert!(run.frames[0] != *after, "the click moved the selection");
    for (i, frame) in run.frames.iter().enumerate().skip(released + 1) {
        assert!(
            frame == after,
            "frame {i} (t = {:.3} s) is not the list after the click, {} pixels differ",
            run.t(i),
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
    assert_swaps("clicking a tab", &run, AT + HOLD);
}

#[test]
fn the_next_tab_swaps_the_strip_and_the_panes_at_once() {
    // Ctrl-Tab from `work` to `logs`: the pills' colors, the × moving to `logs` and the panes.
    let script = serde_json::json!([{ "t": AT, "type": "key", "name": "Tab", "ctrl": true }]);
    let run = run("next-tab", "shell", script, "work");
    assert_swaps("the next tab", &run, AT);
}

#[test]
fn a_space_key_swaps_the_window_at_once() {
    // `shell-spaces` shows the first space; Ctrl-2 shows `dotfiles`: the pills, the pane, and
    // the agent row it focuses in the sidebar.
    let script = serde_json::json!([{ "t": AT, "type": "key", "name": "2", "ctrl": true }]);
    let run = run("space-key", "shell-spaces", script, "window");
    assert_swaps("a space key", &run, AT);
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
    assert_swaps("picking a space", &run, AT);
}

#[test]
fn closing_the_space_picker_closes_it_at_once() {
    // Escape switches nothing, and the picker is still gone in one frame, with no exit fade.
    let script = serde_json::json!([{ "t": AT, "type": "key", "name": "Escape" }]);
    let run = run(
        "space-picker-closed",
        "shell-space-picker",
        script,
        "window",
    );
    assert_swaps("closing the space picker", &run, AT);
}

#[test]
fn resizing_a_pane_keeps_its_terminal_on_its_pane() {
    // Alt-Shift-L grows the left pane. A terminal that landed at its new width while its pane
    // was still on its way there would leave a strip of the gaps' border color at the pane's
    // edge.
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
