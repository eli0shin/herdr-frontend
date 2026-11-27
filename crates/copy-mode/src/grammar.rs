//! [`Vim`]: copy mode's keys as Vim's normal and visual modes read them. A count, then a
//! motion; `y` and a motion or text object; `g`, `f`/`F`/`t`/`T`, `m`, `'` and `` ` `` waiting
//! for their second key; `/` and `?` typing a query. It needs no text, so it runs where the keys
//! arrive and says at once whether copy mode goes on.
//!
//! Esc cancels a pending command, then drops the selection, then leaves; `q` and Ctrl-C leave;
//! `y` and Enter yank the selection (Enter with nothing selected just leaves), and so do `yy`,
//! `Y`, `y{motion}` and `y{object}` (`feature-inventory.md` §16 and §22). A yank ends copy mode
//! once the terminal has copied something ([`Step::yanks`]).

use crate::command::{Command, FindKind, Motion, Object, Visual};

/// A key as copy mode reads it: the character it typed (`G`, `$`, `{`), or Ctrl with a letter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    /// Ctrl and a lowercase letter.
    Ctrl(char),
    Escape,
    Enter,
    Backspace,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

/// The `/` or `?` prompt and what's typed in it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prompt {
    pub backward: bool,
    pub query: String,
    /// The count typed before the `/`: which match to go to.
    count: Option<usize>,
    /// A `y` before the `/`: the search is its motion, with the counts it takes.
    yank: Option<Option<usize>>,
}

/// A key waiting for the one after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    G,
    Find(FindKind),
    Mark,
    /// `` ` `` (exact) or `'`.
    Jump {
        exact: bool,
    },
    /// `i` or `a`, waiting for the object.
    Object {
        inner: bool,
    },
}

/// What one key did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Step {
    /// For the engine, in order.
    pub commands: Vec<Command>,
    /// Copy mode ends, after the commands. A yank doesn't end it here: see [`Step::yanks`].
    pub leave: bool,
    /// Copy mode used the key. A key it doesn't know goes to the window's own bindings.
    pub taken: bool,
}

impl Step {
    fn taken() -> Step {
        Step {
            taken: true,
            ..Step::default()
        }
    }

    fn run(commands: Vec<Command>) -> Step {
        Step {
            commands,
            taken: true,
            leave: false,
        }
    }

    fn leave(commands: Vec<Command>) -> Step {
        Step {
            commands,
            taken: true,
            leave: true,
        }
    }

    /// A yank. Copy mode ends once the terminal says it copied something; a yank that finds
    /// nothing (`y/nowhere`, `yiw` past a line's text) stays in copy mode and says why, as Vim
    /// does.
    fn yank(commands: Vec<Command>) -> Step {
        Step::run(commands)
    }

    /// Whether the step yanks: copy mode then waits for the terminal's answer to end.
    pub fn yanks(&self) -> bool {
        self.commands.iter().any(Command::yanks)
    }
}

/// The grammar's state between keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vim {
    visual: Option<Visual>,
    count: Option<usize>,
    pending: Option<Pending>,
    /// `y` is waiting for its motion, with the count typed before it.
    yank: Option<Option<usize>>,
    prompt: Option<Prompt>,
    /// The last `f`, `F`, `t` or `T`, for `;` and `,`.
    last_find: Option<(FindKind, char)>,
    /// The keys of the command so far, for the bar.
    typed: String,
}

/// The most a count goes up to.
const MAX_COUNT: usize = 99_999;

impl Vim {
    pub fn visual(&self) -> Option<Visual> {
        self.visual
    }

    /// The `/` or `?` prompt, while one is open.
    pub fn prompt(&self) -> Option<&Prompt> {
        self.prompt.as_ref()
    }

    /// The keys of a command part-way through (`3`, `y`, `2f`, `"`), for the bar.
    pub fn typed(&self) -> &str {
        &self.typed
    }

    /// Reads one key.
    pub fn key(&mut self, key: Key) -> Step {
        if self.prompt.is_some() {
            return self.prompt_key(key);
        }
        if let Some(pending) = self.pending.take() {
            let step = self.pending_key(pending, key);
            if !step.taken {
                self.cancel();
            }
            return step;
        }
        if let Key::Char(c @ '0'..='9') = key
            && (c != '0' || self.count.is_some())
        {
            let digit = c as usize - '0' as usize;
            self.count = Some((self.count.unwrap_or(0) * 10 + digit).min(MAX_COUNT));
            self.typed.push(c);
            return Step::taken();
        }
        let step = self.normal_key(key);
        if !step.taken || step.leave || !step.commands.is_empty() {
            // Any finished command, or a key copy mode doesn't use, ends what was typed.
            if self.pending.is_none() && self.prompt.is_none() && self.yank.is_none() {
                self.count = None;
                self.typed.clear();
            }
        }
        if !step.taken {
            self.cancel();
        }
        step
    }

    /// Drops a command part-way through.
    fn cancel(&mut self) {
        self.count = None;
        self.pending = None;
        self.yank = None;
        self.typed.clear();
    }

    fn normal_key(&mut self, key: Key) -> Step {
        use Key as K;
        if let Some(motion) = motion_key(key) {
            return self.motion(motion);
        }
        let c = match key {
            K::Escape => {
                if self.yank.is_some() || self.count.is_some() {
                    self.cancel();
                    return Step::taken();
                }
                if self.visual.take().is_some() {
                    return Step::run(vec![Command::Visual(None)]);
                }
                return Step::leave(vec![]);
            }
            K::Ctrl('c') => return Step::leave(vec![]),
            K::Ctrl('v') => return self.select(Visual::Block),
            K::Enter if self.visual.is_some() => return Step::yank(vec![Command::Yank]),
            K::Enter => return Step::leave(vec![]),
            K::Char(c) => c,
            _ => return Step::default(),
        };
        let count = self.count;
        match c {
            'q' => Step::leave(vec![]),
            'v' | ' ' => self.select(Visual::Chars),
            'V' => self.select(Visual::Lines),
            'o' | 'O' if self.visual.is_some() => Step::run(vec![Command::SwapEnds]),
            'y' if self.visual.is_some() => Step::yank(vec![Command::Yank]),
            'Y' if self.visual.is_some() => {
                // Y in a character selection takes its whole rows, as in Vim.
                let mut commands = Vec::new();
                if self.visual == Some(Visual::Chars) {
                    self.visual = Some(Visual::Lines);
                    commands.push(Command::Visual(Some(Visual::Lines)));
                }
                commands.push(Command::Yank);
                Step::yank(commands)
            }
            'y' => match self.yank {
                Some(before) => Step::yank(vec![Command::YankLines {
                    count: times(before, count).unwrap_or(1),
                }]),
                None => {
                    self.yank = Some(count);
                    self.count = None;
                    self.typed.push('y');
                    Step::taken()
                }
            },
            'Y' => Step::yank(vec![Command::YankLines {
                count: times(self.yank.flatten(), count).unwrap_or(1),
            }]),
            'i' | 'a' if self.visual.is_some() || self.yank.is_some() => {
                self.wait(Pending::Object { inner: c == 'i' }, c)
            }
            'g' => self.wait(Pending::G, c),
            'f' => self.wait(Pending::Find(FindKind::Onto), c),
            'F' => self.wait(Pending::Find(FindKind::OntoBack), c),
            't' => self.wait(Pending::Find(FindKind::Till), c),
            'T' => self.wait(Pending::Find(FindKind::TillBack), c),
            'm' if self.yank.is_none() => self.wait(Pending::Mark, c),
            '\'' => self.wait(Pending::Jump { exact: false }, c),
            '`' => self.wait(Pending::Jump { exact: true }, c),
            ';' | ',' => match self.last_find {
                Some((kind, ch)) => self.motion(Motion::Find {
                    kind: if c == ',' { kind.reversed() } else { kind },
                    ch,
                    repeat: true,
                }),
                None => Step::taken(),
            },
            '/' | '?' => {
                let yank = self.yank.take().map(|before| times(before, count));
                self.prompt = Some(Prompt {
                    backward: c == '?',
                    query: String::new(),
                    count: if yank.is_some() { None } else { count },
                    yank,
                });
                self.count = None;
                self.typed.clear();
                Step::taken()
            }
            _ => Step::default(),
        }
    }

    /// `v`, `V` or Ctrl-V: start that selection, switch to it, or (the same key again) drop
    /// it. A `y` waiting for its motion is dropped.
    fn select(&mut self, kind: Visual) -> Step {
        self.yank = None;
        self.visual = (self.visual != Some(kind)).then_some(kind);
        Step::run(vec![Command::Visual(self.visual)])
    }

    fn wait(&mut self, pending: Pending, c: char) -> Step {
        self.pending = Some(pending);
        self.typed.push(c);
        Step::taken()
    }

    /// The key after a prefix.
    fn pending_key(&mut self, pending: Pending, key: Key) -> Step {
        if key == Key::Escape {
            self.cancel();
            return Step::taken();
        }
        let Key::Char(c) = key else {
            return Step::default();
        };
        match pending {
            Pending::G => match c {
                'g' => self.motion(Motion::Top),
                'e' => self.motion(Motion::WordEndBack { big: false }),
                'E' => self.motion(Motion::WordEndBack { big: true }),
                '0' => self.motion(Motion::LineStart),
                '^' => self.motion(Motion::FirstText),
                '$' | '_' => self.motion(Motion::LineEnd),
                _ => Step::default(),
            },
            Pending::Find(kind) => {
                self.last_find = Some((kind, c));
                self.motion(Motion::Find {
                    kind,
                    ch: c,
                    repeat: false,
                })
            }
            Pending::Mark if c.is_ascii_alphabetic() => {
                self.typed.clear();
                self.count = None;
                Step::run(vec![Command::Mark(c)])
            }
            Pending::Mark => Step::default(),
            Pending::Jump { exact } if c.is_ascii_alphabetic() || c == '\'' || c == '`' => {
                self.motion(Motion::Mark { name: c, exact })
            }
            Pending::Jump { .. } => Step::default(),
            Pending::Object { inner } => {
                let Some(object) = object_key(c) else {
                    return Step::default();
                };
                let count = times(self.yank.flatten(), self.count).unwrap_or(1);
                self.count = None;
                self.typed.clear();
                if self.yank.take().is_some() {
                    return Step::yank(vec![Command::YankObject {
                        object,
                        inner,
                        count,
                    }]);
                }
                let mut commands = Vec::new();
                // A paragraph is whole lines: the selection turns into a line selection.
                if object.linewise() && self.visual != Some(Visual::Lines) {
                    self.visual = Some(Visual::Lines);
                    commands.push(Command::Visual(Some(Visual::Lines)));
                }
                commands.push(Command::Select {
                    object,
                    inner,
                    count,
                });
                Step::run(commands)
            }
        }
    }

    /// A motion: a move, or the end of `y`'s command.
    fn motion(&mut self, motion: Motion) -> Step {
        let count = self.count.take();
        self.typed.clear();
        if let Some(before) = self.yank.take()
            && !motion.scrolls()
        {
            return Step::yank(vec![Command::YankMotion {
                motion,
                count: times(before, count),
            }]);
        }
        Step::run(vec![Command::Move { motion, count }])
    }

    /// A key while the `/` or `?` prompt is open.
    fn prompt_key(&mut self, key: Key) -> Step {
        let prompt = self.prompt.as_mut().expect("a prompt");
        match key {
            Key::Char(c) => prompt.query.push(c),
            Key::Backspace if prompt.query.is_empty() => self.prompt = None,
            Key::Backspace => {
                prompt.query.pop();
            }
            Key::Ctrl('u') => prompt.query.clear(),
            Key::Ctrl('w') => {
                let kept = prompt.query.trim_end().rfind(' ').map_or(0, |i| i + 1);
                prompt.query.truncate(kept);
            }
            Key::Escape | Key::Ctrl('c') => self.prompt = None,
            Key::Enter => {
                let prompt = self.prompt.take().expect("a prompt");
                let motion = Motion::Search {
                    query: prompt.query,
                    backward: prompt.backward,
                };
                return match prompt.yank {
                    Some(count) => Step::yank(vec![Command::YankMotion { motion, count }]),
                    None => Step::run(vec![Command::Move {
                        motion,
                        count: prompt.count,
                    }]),
                };
            }
            _ => {}
        }
        Step::taken()
    }
}

/// Two counts multiplied (`2y3w` is six words); `None` when neither was typed.
fn times(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (None, None) => None,
        (a, b) => Some((a.unwrap_or(1) * b.unwrap_or(1)).min(MAX_COUNT)),
    }
}

/// The motion a key is by itself.
fn motion_key(key: Key) -> Option<Motion> {
    use Key as K;
    use Motion as M;
    Some(match key {
        K::Left | K::Backspace | K::Char('h') => M::Left,
        K::Right | K::Char('l') => M::Right,
        K::Up | K::Char('k') | K::Ctrl('p') => M::Up,
        K::Down | K::Char('j') | K::Ctrl('n') => M::Down,
        K::Home | K::Char('0') => M::LineStart,
        K::Char('^') => M::FirstText,
        K::End | K::Char('$') => M::LineEnd,
        K::Char('|') => M::Column,
        K::Char('+') => M::NextLine,
        K::Char('-') => M::PrevLine,
        K::Char('w') => M::WordForward { big: false },
        K::Char('W') => M::WordForward { big: true },
        K::Char('b') => M::WordBack { big: false },
        K::Char('B') => M::WordBack { big: true },
        K::Char('e') => M::WordEnd { big: false },
        K::Char('E') => M::WordEnd { big: true },
        K::Char('G') => M::Bottom,
        K::Char('H') => M::ViewTop,
        K::Char('M') => M::ViewMiddle,
        K::Char('L') => M::ViewBottom,
        K::Char('%') => M::Match,
        K::Char('}') => M::ParagraphForward,
        K::Char('{') => M::ParagraphBack,
        K::Char('n') => M::SearchNext { reverse: false },
        K::Char('N') => M::SearchNext { reverse: true },
        K::Char('*') => M::SearchWord { backward: false },
        K::Char('#') => M::SearchWord { backward: true },
        K::Ctrl('d') => M::HalfPageDown,
        K::Ctrl('u') => M::HalfPageUp,
        K::Ctrl('f') | K::PageDown => M::PageDown,
        K::Ctrl('b') | K::PageUp => M::PageUp,
        K::Ctrl('e') => M::ScrollDown,
        K::Ctrl('y') => M::ScrollUp,
        _ => return None,
    })
}

/// The object a key after `i` or `a` names.
fn object_key(c: char) -> Option<Object> {
    Some(match c {
        'w' => Object::Word { big: false },
        'W' => Object::Word { big: true },
        'p' => Object::Paragraph,
        '"' | '\'' | '`' => Object::Quote(c),
        '(' | ')' | 'b' => Object::Bracket('('),
        '[' | ']' => Object::Bracket('['),
        '{' | '}' | 'B' => Object::Bracket('{'),
        '<' | '>' => Object::Bracket('<'),
        _ => return None,
    })
}
