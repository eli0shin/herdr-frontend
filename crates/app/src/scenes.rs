//! Named states for `--snap --scene NAME` (`--list-scenes` lists them).

use crate::State;

/// A scene: its name and how it changes the first state.
type Scene = (&'static str, fn(&mut State));

pub(crate) const SCENES: &[Scene] = &[("hello", |_| {})];
