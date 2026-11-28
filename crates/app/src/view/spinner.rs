// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend: the path data
// of its shipped mods' spinner icon. See THIRD-PARTY-NOTICES.md.

//! The working spinners, the one motion the app has (`AGENTS.md`): the ring the sidebar, the tab
//! pills and the picker turn (`workspace-chrome/spinner.svg`, a ring at a quarter's strength
//! with a quarter arc on it), and the peek bar's braille glyph. Each goes round once per
//! [`PERIOD`].
//!
//! Every spinner on screen points the same way at the same moment, as Herdr app's do, however
//! long ago each one came: they all read the window's one clock. Rune's rotating loop
//! (`LoopSpec::rotation`) would start each spinner's turn when it mounts, so a sidebar of
//! agents that started working at different times would turn out of step. A [`Cycle`] is
//! sampled from the window's clock instead ("every cycle with the same period stays in
//! phase"), so a spinner is its frames stacked, each shown by a `Cycle` for its slot of the
//! turn: the ring's arc at [`STEPS`] angles, the braille glyph's ten frames.

use std::sync::LazyLock;

use rune::{Color, Cycle, El, Icon, TextStyle, el, icon, text};

/// One turn, in seconds.
pub const PERIOD: f32 = 0.9;

/// The ring's arc's angles: one every 10°, 40 a second.
pub const STEPS: usize = 36;

/// The ring, still under the arc.
static RING: Icon = Icon {
    name: "herdr-spinner-ring",
    view: 24.0,
    body: r#"<circle cx="12" cy="12" r="9.4" fill="none" stroke="currentColor" stroke-width="3.2" stroke-opacity="0.24"/>"#,
};

/// The arc at each of the [`STEPS`] angles, clockwise from twelve o'clock.
static ARCS: LazyLock<Vec<Icon>> = LazyLock::new(|| {
    (0..STEPS)
        .map(|i| {
            let degrees = i as f32 * 360.0 / STEPS as f32;
            let body = format!(
                r#"<path d="M12 2.6a9.4 9.4 0 0 1 9.4 9.4" transform="rotate({degrees} 12 12)" fill="none" stroke="currentColor" stroke-width="3.2" stroke-linecap="round"/>"#
            );
            Icon {
                name: Box::leak(format!("herdr-spinner-arc-{i}").into_boxed_str()),
                view: 24.0,
                body: Box::leak(body.into_boxed_str()),
            }
        })
        .collect()
});

/// The braille glyph's frames, one every 90 ms.
const BRAILLE: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// A frame of `n` that shows for its `1 / n` of the turn, around its phase: from half a slot
/// before it up to, not including, half a slot after, so at a slot's edge only the next frame
/// shows.
const fn slot(n: usize) -> [(f32, f32); 6] {
    let half = 0.5 / n as f32;
    [
        (0.0, 1.0),
        (half - 0.0001, 1.0),
        (half, 0.0),
        (1.0 - half - 0.0001, 0.0),
        (1.0 - half, 1.0),
        (1.0, 1.0),
    ]
}

static ARC_SLOT: [(f32, f32); 6] = slot(STEPS);
static BRAILLE_SLOT: [(f32, f32); 6] = slot(BRAILLE.len());

/// Frame `i` of `n`, shown from `i / n` of the way round the window's clock's turn, `n` frames a
/// turn. The first frame shows when motion is reduced.
fn frame(i: usize, n: usize, stops: &'static [(f32, f32)]) -> Cycle {
    Cycle {
        period: PERIOD,
        phase: i as f32 / n as f32,
        stops,
        still: if i == 0 { 1.0 } else { 0.0 },
        fps: n as f32 / PERIOD,
    }
}

/// The working ring, `size` points square, keyed `key`. Its arcs are its children, keyed
/// `arc-<i>` for the `i`th of [`STEPS`] angles.
pub fn ring(key: impl Into<String>, size: f32, color: Color) -> El {
    let arcs = ARCS.iter().enumerate().map(|(i, arc)| {
        icon(arc, size)
            .key(format!("arc-{i}"))
            .color(color)
            .absolute(0.0, 0.0)
            .cycle(frame(i, STEPS, &ARC_SLOT))
    });
    el().key(key)
        .size(size, size)
        .child(icon(&RING, size).color(color).absolute(0.0, 0.0))
        .children(arcs)
}

/// The peek bar's braille spinner in `font`, keyed `key`.
pub fn braille(key: impl Into<String>, font: TextStyle, color: Color) -> El {
    let n = BRAILLE.len();
    let frames = BRAILLE.iter().enumerate().map(|(i, glyph)| {
        text(*glyph)
            .font(font)
            .color(color)
            .absolute(0.0, 0.0)
            .cycle(frame(i, n, &BRAILLE_SLOT))
    });
    el().key(key).size(12.0, 16.0).children(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Which of `n` frames shows at `t` on the window's clock.
    fn shown(t: f64, n: usize, stops: &'static [(f32, f32)]) -> Vec<usize> {
        (0..n)
            .filter(|&i| frame(i, n, stops).sample(t) > 0.5)
            .collect()
    }

    #[test]
    fn one_frame_shows_at_a_time_and_they_go_round_once_a_period() {
        for k in 0..400 {
            let t = f64::from(k) * 0.0071;
            let arc = shown(t, STEPS, &ARC_SLOT);
            assert_eq!(arc.len(), 1, "t = {t}: {arc:?}");
            // The arc nearest the turn's angle, but for a time on the edge between two.
            let steps = (t / f64::from(PERIOD)).rem_euclid(1.0) * STEPS as f64;
            if (steps.fract() - 0.5).abs() > 0.01 {
                assert_eq!(arc[0], steps.round() as usize % STEPS, "t = {t}");
            }
            assert_eq!(shown(t, BRAILLE.len(), &BRAILLE_SLOT).len(), 1, "t = {t}");
        }
    }

    #[test]
    fn the_arcs_are_the_spinner_turned_a_step_at_a_time() {
        assert_eq!(ARCS.len(), STEPS);
        assert!(ARCS[0].body.contains("rotate(0 12 12)"));
        assert!(ARCS[9].body.contains("rotate(90 12 12)"));
        let names: std::collections::BTreeSet<_> = ARCS.iter().map(|a| a.name).collect();
        assert_eq!(names.len(), STEPS, "each angle is its own icon");
    }
}
