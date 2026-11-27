// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! Herdr's attention model (`herdr-core-protocol.md` §5; `app:src/model.rs` `status`, `alert`,
//! `alert_for`, `priority`): what status an agent shows, how tabs and spaces roll it up, and when
//! it raises an in-app toast. Attention shows in the app only: never a native notification, never
//! a sound.

use std::collections::BTreeMap;

use herdr_proto::{StatusState, Terminal};

/// The status the UI shows, ordered by Herdr's attention priority: blocked > done > working >
/// idle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Displayed {
    Idle,
    Working,
    /// Finished and not seen yet.
    Done,
    /// Needs the person: blocked or error.
    Blocked,
}

impl Displayed {
    pub fn as_str(self) -> &'static str {
        match self {
            Displayed::Idle => "idle",
            Displayed::Working => "working",
            Displayed::Done => "done",
            Displayed::Blocked => "blocked",
        }
    }
}

/// The most urgent status among some terminals'. `None` when none is an agent.
pub fn rollup(statuses: impl IntoIterator<Item = Option<Displayed>>) -> Option<Displayed> {
    statuses.into_iter().flatten().max()
}

/// Why an agent wants the person.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alert {
    /// It needs them (blocked or error).
    Blocked,
    /// It finished while they weren't looking.
    Done,
}

fn blocked(word: Option<&StatusState>) -> bool {
    matches!(word, Some(StatusState::Blocked | StatusState::Error))
}

fn busy(word: Option<&StatusState>) -> bool {
    matches!(
        word,
        Some(StatusState::Working | StatusState::Blocked | StatusState::Error)
    )
}

/// The rule for a change of status word: entering blocked or error always alerts; going from
/// busy to any other word alerts only when the person isn't looking. Losing the status alerts
/// nothing.
pub fn alert(
    prev: Option<&StatusState>,
    new: Option<&StatusState>,
    looking: bool,
) -> Option<Alert> {
    if blocked(new) && !blocked(prev) {
        Some(Alert::Blocked)
    } else if busy(prev) && new.is_some() && !busy(new) && !looking {
        Some(Alert::Done)
    } else {
        None
    }
}

/// [`alert`] for one terminal: only one in a pane of this screen's layout alerts. A sub-agent
/// (`parent` set) never does; its root agent stands in for it.
pub fn alert_for(
    in_pane: bool,
    sub_agent: bool,
    prev: Option<&StatusState>,
    new: Option<&StatusState>,
    looking: bool,
) -> Option<Alert> {
    if sub_agent || !in_pane {
        return None;
    }
    alert(prev, new, looking)
}

/// Whether an alert shows a toast: only when the person isn't looking at that agent. An alert
/// always brings the root agent's settled tab back, toast or not.
pub fn toast(looking: bool) -> bool {
    !looking
}

/// The top of a terminal's parent chain among the live terminals: the agent a sub-agent belongs
/// to, whose pane its alerts point at. Follows at most 32 parents. `parent` must already be
/// normalized to this core's addresses (`herdr-core-protocol.md` §5).
pub fn root_agent<'a>(address: &'a str, parent: impl Fn(&str) -> Option<&'a str>) -> &'a str {
    let mut current = address;
    for _ in 0..32 {
        match parent(current) {
            Some(up) => current = up,
            None => break,
        }
    }
    current
}

/// What the status and this screen's attention say about one terminal (`app:src/model.rs`
/// `Agent`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    /// The program's status word, `None` when it doesn't report.
    pub word: Option<StatusState>,
    /// Whether this screen saw it since it last stopped being busy: the fallback when no client
    /// shares `herdr.seen`.
    pub seen: bool,
    /// The core's turn: up each time it stops working.
    pub turn: u64,
    /// The shared `herdr.seen`, or this screen's unechoed write ([`crate::SeenTracker`]).
    pub seen_turn: Option<u64>,
}

impl Default for Agent {
    /// A terminal not seen before starts as seen: what it did before is no news.
    fn default() -> Self {
        Agent {
            word: None,
            seen: true,
            turn: 0,
            seen_turn: None,
        }
    }
}

impl Agent {
    /// A new status word. Going busy counts as seen; finishing while nobody looks makes it done
    /// until seen. True when the word changed.
    pub fn set_word(&mut self, word: Option<StatusState>, looking: bool) -> bool {
        if self.word == word {
            return false;
        }
        let was_busy = matches!(self.word, Some(StatusState::Working | StatusState::Blocked));
        match &word {
            Some(StatusState::Working | StatusState::Blocked) => self.seen = true,
            Some(_) if was_busy => self.seen = looking,
            _ => {}
        }
        self.word = word;
        true
    }

    /// The core's turn and the shared seen turn. True when either changed.
    pub fn set_turn(&mut self, turn: u64, seen_turn: Option<u64>) -> bool {
        let changed = (self.turn, self.seen_turn) != (turn, seen_turn);
        (self.turn, self.seen_turn) = (turn, seen_turn);
        changed
    }

    /// The person looked at it.
    pub fn mark_seen(&mut self) -> bool {
        !std::mem::replace(&mut self.seen, true)
    }

    /// What the UI shows. `None`: not an agent. Any word other than working, blocked and error
    /// is done when `turn > herdr.seen`, else idle; without the label, this screen's own `seen`
    /// decides.
    pub fn displayed(&self) -> Option<Displayed> {
        Some(match self.word.as_ref()? {
            StatusState::Working => Displayed::Working,
            StatusState::Blocked | StatusState::Error => Displayed::Blocked,
            _ if self.seen_turn.map_or(!self.seen, |seen| self.turn > seen) => Displayed::Done,
            _ => Displayed::Idle,
        })
    }
}

/// One terminal, as the screen sees it this round.
#[derive(Clone, Copy, Debug)]
pub struct Seen<'a> {
    pub terminal: &'a Terminal,
    /// A pane of this screen's layout shows it.
    pub in_pane: bool,
    /// It's drawn on screen and the window is in the foreground.
    pub looking: bool,
    /// [`crate::SeenTracker::seen_turn`].
    pub seen_turn: Option<u64>,
}

/// An alert to raise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raise {
    pub terminal: String,
    pub alert: Alert,
    /// Show an in-app toast (pointing at the root agent's pane).
    pub toast: bool,
}

/// Every live terminal's [`Agent`], and the alerts their changes raise.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attention {
    agents: BTreeMap<String, Agent>,
    listed_once: bool,
}

impl Attention {
    pub fn get(&self, address: &str) -> Option<&Agent> {
        self.agents.get(address)
    }

    pub fn get_mut(&mut self, address: &str) -> Option<&mut Agent> {
        self.agents.get_mut(address)
    }

    pub fn displayed(&self, address: &str) -> Option<Displayed> {
        self.agents.get(address)?.displayed()
    }

    /// The tab's or space's status: the most urgent of its terminals'.
    pub fn rollup<'a>(&self, addresses: impl IntoIterator<Item = &'a str>) -> Option<Displayed> {
        rollup(addresses.into_iter().map(|a| self.displayed(a)))
    }

    /// Takes the live terminals after a listing (or a batch of events) and returns the alerts to
    /// raise. What changed before the first listing isn't news, so it raises nothing.
    pub fn update<'a>(&mut self, live: impl IntoIterator<Item = Seen<'a>>) -> Vec<Raise> {
        let mut raises = Vec::new();
        let mut present = Vec::new();
        for seen in live {
            let terminal = seen.terminal;
            present.push(terminal.address.clone());
            let word = terminal.status.as_ref().map(|s| s.state.clone());
            let agent = self.agents.entry(terminal.address.clone()).or_default();
            let sub_agent = terminal.parent.as_deref().is_some_and(|p| !p.is_empty());
            let raised = alert_for(
                seen.in_pane,
                sub_agent,
                agent.word.as_ref(),
                word.as_ref(),
                seen.looking,
            );
            agent.set_word(word, seen.looking);
            agent.set_turn(terminal.turn, seen.seen_turn);
            if let Some(alert) = raised.filter(|_| self.listed_once) {
                raises.push(Raise {
                    terminal: terminal.address.clone(),
                    alert,
                    toast: toast(seen.looking),
                });
            }
        }
        self.agents.retain(|address, _| present.contains(address));
        self.listed_once = true;
        raises
    }

    /// The person looks at these: their done ones are no longer news.
    pub fn mark_seen<'a>(&mut self, addresses: impl IntoIterator<Item = &'a str>) -> bool {
        let mut changed = false;
        for address in addresses {
            if let Some(agent) = self.agents.get_mut(address) {
                changed |= agent.mark_seen();
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use StatusState::*;
    use herdr_proto::Status;

    fn w(word: StatusState) -> Option<StatusState> {
        Some(word)
    }

    #[test]
    fn alerts_follow_herdr() {
        let a = |prev: Option<StatusState>, new: Option<StatusState>, looking| {
            alert(prev.as_ref(), new.as_ref(), looking)
        };
        assert_eq!(a(w(Working), w(Blocked), true), Some(Alert::Blocked));
        assert_eq!(a(None, w(Error), false), Some(Alert::Blocked));
        assert_eq!(a(w(Blocked), w(Error), false), None);
        assert_eq!(a(w(Working), w(Idle), false), Some(Alert::Done));
        assert_eq!(a(w(Blocked), w(Done), false), Some(Alert::Done));
        assert_eq!(
            a(w(Error), w(Other("thinking".into())), false),
            Some(Alert::Done)
        );
        assert_eq!(a(w(Working), w(Idle), true), None);
        assert_eq!(a(None, w(Idle), false), None);
        assert_eq!(a(w(Working), None, false), None);
    }

    #[test]
    fn only_this_screens_root_agents_alert() {
        let (prev, new) = (w(Working), w(Done));
        assert_eq!(
            alert_for(true, false, prev.as_ref(), new.as_ref(), false),
            Some(Alert::Done)
        );
        assert_eq!(
            alert_for(false, false, prev.as_ref(), new.as_ref(), false),
            None
        );
        assert_eq!(
            alert_for(true, true, prev.as_ref(), w(Blocked).as_ref(), false),
            None
        );
    }

    #[test]
    fn a_toast_shows_only_when_the_person_is_not_looking() {
        assert!(toast(false));
        assert!(!toast(true));
    }

    #[test]
    fn finishes_you_did_not_watch_are_done_until_seen() {
        let mut a = Agent::default();
        a.set_word(w(Working), false);
        assert_eq!(a.displayed(), Some(Displayed::Working));
        a.set_word(w(Idle), false);
        assert_eq!(a.displayed(), Some(Displayed::Done));
        assert!(a.mark_seen());
        assert!(!a.mark_seen());
        assert_eq!(a.displayed(), Some(Displayed::Idle));
        // Watching it finish: straight to idle.
        a.set_word(w(Working), true);
        a.set_word(w(Done), true);
        assert_eq!(a.displayed(), Some(Displayed::Idle));
        a.set_word(w(Error), true);
        assert_eq!(a.displayed(), Some(Displayed::Blocked));
        assert!(!a.set_word(w(Error), true));
        a.set_word(None, false);
        assert_eq!(a.displayed(), None, "not an agent");
    }

    #[test]
    fn a_shared_seen_turn_decides_over_this_screen() {
        let mut a = Agent::default();
        a.set_word(w(Working), true);
        a.set_word(w(Idle), true);
        assert_eq!(a.displayed(), Some(Displayed::Idle));
        // Another client hasn't seen turn 3 (marked unread): done, though we watched.
        assert!(a.set_turn(3, Some(2)));
        assert_eq!(a.displayed(), Some(Displayed::Done));
        assert!(!a.set_turn(3, Some(2)));
        // Seen elsewhere: idle, though we didn't watch.
        a.set_word(w(Working), false);
        a.set_word(w(Idle), false);
        a.set_turn(4, Some(4));
        assert_eq!(a.displayed(), Some(Displayed::Idle));
        // Needing you or working beats seen.
        a.set_word(w(Blocked), false);
        assert_eq!(a.displayed(), Some(Displayed::Blocked));
        // No label: this screen's own rule.
        a.set_word(w(Idle), false);
        a.set_turn(5, None);
        assert_eq!(a.displayed(), Some(Displayed::Done));
    }

    #[test]
    fn rollups_take_the_most_urgent() {
        use Displayed::*;
        assert_eq!(rollup([Some(Idle), Some(Done), Some(Working)]), Some(Done));
        assert_eq!(
            rollup([Some(Working), Some(Blocked), Some(Done)]),
            Some(Blocked)
        );
        assert_eq!(rollup([Some(Idle), Some(Working)]), Some(Working));
        assert_eq!(rollup([None, Some(Idle)]), Some(Idle));
        assert_eq!(rollup([None, None]), None);
        assert_eq!(Blocked.as_str(), "blocked");
    }

    #[test]
    fn root_agents_follow_parents() {
        let parents = [("t3", "t2"), ("t2", "t1")];
        let parent = |a: &str| parents.iter().find(|(c, _)| *c == a).map(|(_, p)| *p);
        assert_eq!(root_agent("t3", parent), "t1");
        assert_eq!(root_agent("t1", parent), "t1");
        let cycle = |a: &str| Some(if a == "a" { "b" } else { "a" });
        assert!(
            ["a", "b"].contains(&root_agent("a", cycle)),
            "a cycle stops"
        );
    }

    fn terminal(address: &str, word: Option<StatusState>, parent: Option<&str>) -> Terminal {
        Terminal {
            address: address.into(),
            status: word.map(|state| Status {
                state,
                ..Default::default()
            }),
            parent: parent.map(str::to_owned),
            turn: 1,
            ..Default::default()
        }
    }

    fn seen(terminal: &Terminal, looking: bool) -> Seen<'_> {
        Seen {
            terminal,
            in_pane: true,
            looking,
            seen_turn: None,
        }
    }

    #[test]
    fn the_first_listing_raises_nothing_and_later_changes_do() {
        let mut at = Attention::default();
        let blocked = terminal("t1", w(Blocked), None);
        assert_eq!(at.update([seen(&blocked, false)]), []);
        let working = terminal("t1", w(Working), None);
        assert_eq!(at.update([seen(&working, false)]), []);
        let idle = terminal("t1", w(Idle), None);
        assert_eq!(
            at.update([seen(&idle, false)]),
            [Raise {
                terminal: "t1".into(),
                alert: Alert::Done,
                toast: true
            }]
        );
        assert_eq!(at.displayed("t1"), Some(Displayed::Done));
        assert_eq!(at.rollup(["t1", "t9"]), Some(Displayed::Done));
        assert!(at.mark_seen(["t1"]));
        assert_eq!(at.displayed("t1"), Some(Displayed::Idle));
        // Blocked while looking: an alert, but no toast.
        assert_eq!(
            at.update([seen(&blocked, true)]),
            [Raise {
                terminal: "t1".into(),
                alert: Alert::Blocked,
                toast: false
            }]
        );
        // Gone from the list: forgotten.
        at.update([]);
        assert!(at.get("t1").is_none());
    }

    #[test]
    fn sub_agents_and_terminals_outside_the_layout_stay_quiet() {
        let mut at = Attention::default();
        let child = terminal("t2", w(Working), Some("t1"));
        let outside = terminal("t3", w(Working), None);
        let outside_seen = Seen {
            in_pane: false,
            ..seen(&outside, false)
        };
        at.update([seen(&child, false), outside_seen]);
        let child = terminal("t2", w(Blocked), Some("t1"));
        let outside = terminal("t3", w(Blocked), None);
        let outside_seen = Seen {
            in_pane: false,
            ..seen(&outside, false)
        };
        assert_eq!(at.update([seen(&child, false), outside_seen]), []);
        assert_eq!(at.displayed("t2"), Some(Displayed::Blocked), "still shown");
    }
}
