//! The Agents list (`model::agents`), as `eli-agents` builds it (`feature-inventory.md` §6), on a
//! model connected to a fake core.

mod support;

use std::collections::{BTreeMap, BTreeSet};

use herdr_proto::types::{MachineState, MachineView};
use herdr_proto::{StatusState, Terminal, TerminalState};
use herdr_shared::Displayed;
use model::agents::{
    self, AgentRow, Branch, Empty, Input, List, Program, Row, Sort, SubRow, Tracker, Tree,
};
use model::{Action, Event, Model, TerminalUpdate};
use serde_json::{Value, json};
use support::*;

use StatusState::*;

const NOW: u64 = 1_800_000_000_000;

/// The program table the tests use: an agent's report names it, else its command.
fn program(t: &Terminal) -> Program {
    match t
        .status
        .as_ref()
        .map(|s| s.app.as_str())
        .filter(|a| !a.is_empty())
    {
        Some(app) => Program {
            key: app.into(),
            name: if app == "pi" { "Pi".into() } else { app.into() },
            agent: true,
        },
        None => Program {
            key: t.argv.first().cloned().unwrap_or_else(|| "shell".into()),
            name: t.argv.first().cloned().unwrap_or_else(|| "zsh".into()),
            agent: false,
        },
    }
}

struct Opts {
    sort: Sort,
    hidden: BTreeSet<String>,
    colors: BTreeMap<String, String>,
    now: u64,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts {
            sort: Sort::Priority,
            hidden: BTreeSet::new(),
            colors: BTreeMap::new(),
            now: NOW,
        }
    }
}

fn build(tracker: &mut Tracker, m: &Model, o: &Opts) -> List {
    tracker.build(
        m,
        &Input {
            sort: o.sort,
            sort_value: Some(o.sort.as_str()),
            hidden_machines: &o.hidden,
            space_colors: &o.colors,
            now_ms: o.now,
            program: &program,
            peeked: None,
        },
    )
}

fn agent_of(app: &str, address: &str, state: StatusState) -> Terminal {
    let mut t = agent(address, state);
    t.status.as_mut().unwrap().app = app.into();
    t
}

fn named_tab(id: &str, name: &str, children: Vec<Value>) -> Value {
    let mut t = tab(id, children);
    t["name"] = json!(name);
    t
}

fn named_space(id: &str, name: &str, tabs: Vec<Value>) -> Value {
    let mut s = space(id, tabs);
    s["name"] = json!(name);
    s
}

/// What each row reads, one line each: `# label` for headers, `title [status] label` for
/// agents, `  name` for sub-agents.
fn lines(list: &List) -> Vec<String> {
    list.rows
        .iter()
        .map(|r| match r {
            Row::Header(h) => match h.count {
                Some(n) => format!("# {} {n}", h.label),
                None => format!("# {}", h.label),
            },
            Row::Agent(a) => format!(
                "{}{} [{}] {}",
                match a.line {
                    Some(Branch::Middle) => "├ ",
                    Some(Branch::Last) => "└ ",
                    None => "",
                },
                a.title,
                a.status.as_str(),
                a.label
            ),
            Row::Sub(s) => format!("  {}", s.name),
        })
        .collect()
}

fn agent_rows(list: &List) -> Vec<&AgentRow> {
    list.rows
        .iter()
        .filter_map(|r| match r {
            Row::Agent(a) => Some(a.as_ref()),
            _ => None,
        })
        .collect()
}

fn row<'a>(list: &'a List, address: &str) -> &'a AgentRow {
    agent_rows(list)
        .into_iter()
        .find(|a| a.address == address)
        .unwrap_or_else(|| panic!("no row for {address}"))
}

fn subs(list: &List) -> Vec<&SubRow> {
    list.rows
        .iter()
        .filter_map(|r| match r {
            Row::Sub(s) => Some(s),
            _ => None,
        })
        .collect()
}

fn update(m: &mut Model, core: &mut Core, terminals: Vec<Terminal>) {
    for t in &terminals {
        core.terminals.insert(t.address.clone(), t.clone());
    }
    drive(
        m,
        core,
        Event::Terminals(TerminalUpdate {
            terminals,
            ..Default::default()
        }),
    );
}

/// Two spaces of agents: `work` (a Claude in tab "api", an unnamed Codex tab) and `notes` (a
/// shell tab, then an agent).
fn two_spaces() -> Core {
    let doc = doc(vec![
        named_space(
            "sp1",
            "work",
            vec![
                tab("tb0", vec![pane("p0", "t0")]),
                named_tab("tb1", "api", vec![pane("p1", "t1")]),
                tab("tb2", vec![pane("p2", "t2")]),
            ],
        ),
        named_space(
            "sp2",
            "notes",
            vec![
                tab("tb3", vec![pane("p3", "t3")]),
                tab("tb4", vec![pane("p4", "t4")]),
            ],
        ),
    ]);
    Core::new(
        doc,
        vec![
            shell("t0", "/home/me/code/x"),
            agent_of("claude", "t1", Working),
            agent_of("codex", "t2", Blocked),
            shell("t3", "/home/me/code/x"),
            agent_of("claude", "t4", Idle),
        ],
    )
}

#[test]
fn priority_sorts_by_rank_into_sections_with_counts_and_numbers_every_row() {
    let mut core = two_spaces();
    let mut m = connected(&mut core);
    let mut tracker = Tracker::default();
    let list = build(&mut tracker, &m, &Opts::default());
    assert_eq!(
        lines(&list),
        [
            "# Needs you 1",
            "codex [blocked] 1",
            "# Working 1",
            "api [working] 2",
            "# Idle 1",
            "claude [idle] 3",
        ]
    );
    let headers: Vec<_> = list
        .rows
        .iter()
        .filter_map(|r| match r {
            Row::Header(h) => Some((h.first, h.status)),
            _ => None,
        })
        .collect();
    assert_eq!(
        headers,
        [
            (true, Some(Displayed::Blocked)),
            (false, Some(Displayed::Working)),
            (false, None),
        ]
    );
    assert_eq!(list.rows_order, ["t2", "t1", "t4"]);
    assert_eq!(list.nav, list.rows_order);
    // Without a space header above, line 2 names the space; under a priority header blocked
    // is the compact "!".
    let codex = row(&list, "t2");
    assert_eq!(codex.space.as_deref(), Some("work"));
    assert!(codex.compact);
    assert_eq!(codex.machine, None);
    // A space color is assigned to each and kept.
    let colors = list.space_colors.clone().expect("assigned");
    assert_eq!(colors["sp1"], "ansi6");
    assert_eq!(colors["sp2"], "ansi3");
    assert_eq!(codex.space_color.as_deref(), Some("#59bbb0"));
    assert_eq!(list.empty, None);

    // Ties break by the last state change, then the change sequence: the newest first.
    update(&mut m, &mut core, vec![agent_of("claude", "t1", Blocked)]);
    let list = build(
        &mut tracker,
        &m,
        &Opts {
            now: NOW + 5_000,
            colors,
            ..Opts::default()
        },
    );
    assert_eq!(list.rows_order, ["t1", "t2", "t4"]);
    assert_eq!(list.space_colors, None, "nothing new to store");
}

#[test]
fn grouped_view_heads_each_space_in_model_order_and_leaves_the_space_out_of_rows() {
    let mut core = two_spaces();
    let m = connected(&mut core);
    let list = build(
        &mut Tracker::default(),
        &m,
        &Opts {
            sort: Sort::Grouped,
            ..Opts::default()
        },
    );
    assert_eq!(
        lines(&list),
        [
            "# work",
            "api [working] 1",
            "codex [blocked] 2",
            "# notes",
            "claude [idle] 3"
        ]
    );
    let api = row(&list, "t1");
    assert_eq!(api.space, None, "the header names it");
    assert!(!api.compact, "the full Needs you pill outside priority");
}

#[test]
fn pinned_agents_come_first_under_their_own_header() {
    let mut core = two_spaces();
    let mut m = connected(&mut core);
    let mut pinned = agent_of("claude", "t4", Idle);
    pinned.labels.insert("herdr.pinned".into(), "1".into());
    update(&mut m, &mut core, vec![pinned]);
    let list = build(&mut Tracker::default(), &m, &Opts::default());
    assert_eq!(
        lines(&list),
        [
            "# Pinned 1",
            "claude [idle] 1",
            "# Needs you 1",
            "codex [blocked] 2",
            "# Working 1",
            "api [working] 3",
        ]
    );
    let row = row(&list, "t4");
    assert!(row.pinned && !row.compact);
    assert_eq!(row.space.as_deref(), Some("notes"));
}

#[test]
fn hidden_machines_leave_the_list_and_say_so_when_nothing_is_left() {
    let mut core = two_spaces();
    let m = connected(&mut core);
    let hidden: BTreeSet<String> = [String::new()].into();
    let list = build(
        &mut Tracker::default(),
        &m,
        &Opts {
            hidden,
            ..Opts::default()
        },
    );
    assert!(list.rows.is_empty() && list.rows_order.is_empty());
    assert_eq!(list.empty, Some(Empty::AllHidden));
    let empty = Core::new(doc(vec![]), vec![]);
    let mut core = empty;
    let m = connected(&mut core);
    let list = build(&mut Tracker::default(), &m, &Opts::default());
    assert_eq!(list.empty, Some(Empty::NoAgents));
    assert!(
        Empty::NoAgents
            .message()
            .starts_with("No agent status yet.")
    );
}

#[test]
fn a_viewed_done_agent_stays_in_done_until_you_leave_it() {
    let mut core = two_spaces();
    let mut m = connected(&mut core);
    let mut tracker = Tracker::default();
    // t1 finishes while nobody looks: done.
    update(&mut m, &mut core, vec![agent_of("claude", "t1", Idle)]);
    let list = build(&mut tracker, &m, &Opts::default());
    assert_eq!(row(&list, "t1").status, Displayed::Done);
    assert_eq!(list.rows_order, ["t2", "t1", "t4"]);
    // Looking at it makes it idle, but it keeps its place under Done.
    drive(&mut m, &mut core, Action::FocusAgent("t1".into()));
    update(&mut m, &mut core, vec![agent_of("claude", "t1", Idle)]);
    let list = build(&mut tracker, &m, &Opts::default());
    let t1 = row(&list, "t1");
    assert_eq!(t1.status, Displayed::Idle);
    assert!(t1.focused);
    assert!(lines(&list).contains(&"# Done 1".to_owned()));
    assert_eq!(list.rows_order, ["t2", "t1", "t4"]);
    // Leaving it lets it go.
    drive(&mut m, &mut core, Action::FocusAgent("t2".into()));
    let list = build(&mut tracker, &m, &Opts::default());
    assert_eq!(
        list.rows_order,
        ["t2", "t4", "t1"],
        "idle now: ties go by change sequence"
    );
    assert!(!lines(&list).iter().any(|l| l.starts_with("# Done")));
}

#[test]
fn sub_agents_nest_with_tree_lines_lift_their_parent_and_fold() {
    let mut core = two_spaces();
    let mut m = connected(&mut core);
    let kid = |address: &str, parent: &str, state: StatusState, created: u64| Terminal {
        parent: Some(parent.into()),
        created_ms: created,
        name: Some(format!("{address}-name")),
        ..agent(address, state)
    };
    // t4 (idle) has two sub-agents; the first has one of its own, which needs the person.
    update(
        &mut m,
        &mut core,
        vec![
            kid("t5", "t4", Working, 1),
            kid("t6", "t4", Done, 2),
            kid("t7", "t5", Error, 3),
            Terminal {
                state: TerminalState::Finished,
                ..kid("t8", "t4", Working, 4)
            },
        ],
    );
    let mut tracker = Tracker::default();
    let list = build(&mut tracker, &m, &Opts::default());
    assert_eq!(
        lines(&list),
        [
            "# Needs you 2",
            "claude [idle] 1",
            "  t5-name",
            "  t7-name",
            "  t6-name",
            "codex [blocked] 2",
            "# Working 1",
            "api [working] 3",
        ],
        "t7's error lifts t4 to Needs you (its later change first); the finished one is left out"
    );
    let kids = subs(&list);
    assert_eq!(kids[0].lines, [Tree::Middle]);
    assert_eq!(kids[1].lines, [Tree::Pipe, Tree::Last]);
    assert_eq!(kids[2].lines, [Tree::Last]);
    assert_eq!(kids[0].status, Some(Displayed::Working));
    assert_eq!(
        kids[1].status,
        Some(Displayed::Blocked),
        "error counts as blocked"
    );
    assert_eq!(
        kids[2].status,
        Some(Displayed::Idle),
        "a sub-agent's done is idle"
    );
    assert!(kids.iter().all(|k| k.compact && !k.on));
    let fold = row(&list, "t4").fold.clone().expect("a fold control");
    assert!(fold.open);
    assert_eq!(fold.count, 3);
    assert_eq!(fold.top, Some(Displayed::Blocked));
    // Folding hides them; the row's number stays.
    tracker.toggle_fold("t4");
    let list = build(&mut tracker, &m, &Opts::default());
    assert!(subs(&list).is_empty());
    assert!(!row(&list, "t4").fold.as_ref().unwrap().open);
    assert_eq!(list.rows_order, ["t4", "t2", "t1"]);
    tracker.toggle_fold("t4");
    assert_eq!(subs(&build(&mut tracker, &m, &Opts::default())).len(), 3);
}

fn pi(address: &str, state: StatusState, watch: Value) -> Terminal {
    let mut t = agent_of("pi", address, state);
    t.labels
        .insert(agents::WATCH_LABEL.into(), watch.to_string());
    t
}

/// An orchestrator (t1) with two workers (t2, t4), and a worker of an orchestration nobody runs.
fn orchestration() -> Core {
    let doc = doc(vec![
        named_space(
            "sp1",
            "main",
            vec![
                named_tab("tb1", "orchestrate", vec![pane("p1", "t1")]),
                named_tab("tb2", "worker a", vec![pane("p2", "t2")]),
            ],
        ),
        named_space(
            "sp2",
            "other",
            vec![
                named_tab("tb4", "worker b", vec![pane("p4", "t4")]),
                named_tab("tb5", "stray", vec![pane("p5", "t5")]),
            ],
        ),
    ]);
    Core::new(
        doc,
        vec![
            pi(
                "t1",
                Idle,
                json!({"role": "orchestrator", "orchestration": "o1", "ticket": "FCC-9", "prs": ["#12"]}),
            ),
            pi(
                "t2",
                Working,
                json!({"role": "worker", "orchestration": "o1", "prs": ["#12", "#13"], "paused": true}),
            ),
            pi("t4", Idle, json!({"role": "worker", "orchestration": "o1"})),
            pi(
                "t5",
                Idle,
                json!({"role": "worker", "orchestration": "gone"}),
            ),
        ],
    )
}

#[test]
fn pi_workers_nest_under_their_orchestrator_while_it_or_a_worker_is_focused() {
    let mut core = orchestration();
    let mut m = connected(&mut core);
    let mut tracker = Tracker::default();
    drive(&mut m, &mut core, Action::FocusAgent("t5".into()));
    let list = build(&mut tracker, &m, &Opts::default());
    // A working worker lifts its idle orchestrator; folded, the workers are left out.
    assert_eq!(
        lines(&list),
        [
            "# Working 1",
            "orchestrate [idle] 1",
            "# Idle 1",
            "stray [idle] 2"
        ]
    );
    assert_eq!(list.rows_order, ["t1", "t5"]);
    assert_eq!(list.nav, ["t1", "t2", "t4", "t5"], "navigation keeps them");
    let o = row(&list, "t1");
    assert_eq!(o.workers, 2);
    assert_eq!(o.ticket.as_deref(), Some("FCC-9"));
    assert_eq!(o.prs.as_ref().map(|p| p.first.as_str()), Some("#12"));
    assert_eq!(
        o.fold, None,
        "an orchestrator opens with focus, not a fold control"
    );
    assert_eq!(row(&list, "t5").workers, 0, "its orchestrator isn't listed");

    // Focus a worker: they show, with elbows and no space.
    drive(&mut m, &mut core, Action::FocusAgent("t4".into()));
    let list = build(&mut tracker, &m, &Opts::default());
    assert_eq!(
        lines(&list),
        [
            "# Working 1",
            "orchestrate [idle] 1",
            "├ worker a [working] 2",
            "└ worker b [idle] 3",
            "# Idle 1",
            "stray [idle] 4",
        ]
    );
    let a = row(&list, "t2");
    assert_eq!(a.space, None);
    assert_eq!(a.ticket, None);
    let prs = a.prs.clone().unwrap();
    assert_eq!((prs.first.as_str(), prs.more), ("#12", 1));
    assert_eq!(prs.tooltip, "#12, #13 · paused");
    assert!(prs.paused);
    assert_eq!(list.rows_order, ["t1", "t2", "t4", "t5"]);

    // The label counts only on Pi: another program's label is ignored.
    let mut not_pi = pi(
        "t2",
        Working,
        json!({"role": "worker", "orchestration": "o1"}),
    );
    not_pi.status.as_mut().unwrap().app = "claude".into();
    update(&mut m, &mut core, vec![not_pi]);
    let list = build(&mut tracker, &m, &Opts::default());
    assert_eq!(row(&list, "t1").workers, 1);
}

#[test]
fn titles_prefer_names_then_live_titles_and_never_a_bare_tab_number() {
    let doc = doc(vec![named_space(
        "sp1",
        "work",
        vec![
            tab("tb1", vec![pane("p1", "t1")]),
            tab("tb2", vec![pane("p2", "t2")]),
            tab("tb3", vec![pane("p3", "t3")]),
            tab("tb4", vec![pane("p4", "t4")]),
        ],
    )]);
    let titled = |address: &str, app: &str, title: &str| Terminal {
        title: title.into(),
        ..agent_of(app, address, Idle)
    };
    let mut core = Core::new(
        doc,
        vec![
            titled("t1", "claude", "✳ Fix the flaky test"),
            titled("t2", "pi", "π - write docs - x"),
            titled("t3", "codex", "/home/me/code/x"),
            titled("t4", "codex", "me@host"),
        ],
    );
    let m = connected(&mut core);
    let list = build(&mut Tracker::default(), &m, &Opts::default());
    assert_eq!(
        row(&list, "t1").title,
        "Fix the flaky test",
        "leading marks go"
    );
    assert_eq!(
        row(&list, "t2").title,
        "write docs",
        "the Pi session's name"
    );
    // Two rows that would both read as a tab number keep the number apart.
    assert_eq!(row(&list, "t3").title, "3");
    assert_eq!(row(&list, "t4").title, "4");

    // Alone, the program stands in for the number.
    let doc = doc_one();
    let mut core = Core::new(doc, vec![titled("t3", "codex", "/home/me")]);
    let m = connected(&mut core);
    let list = build(&mut Tracker::default(), &m, &Opts::default());
    assert_eq!(row(&list, "t3").title, "codex");
}

fn doc_one() -> Value {
    doc(vec![named_space(
        "sp1",
        "work",
        vec![tab("tb3", vec![pane("p3", "t3")])],
    )])
}

#[test]
fn times_are_stamped_when_the_state_changes_and_survive_in_the_tracker() {
    let mut core = two_spaces();
    let mut m = connected(&mut core);
    let mut saved = BTreeMap::new();
    saved.insert(
        "t4".to_owned(),
        agents::Changed {
            s: "idle".into(),
            at: NOW - 3 * 3_600_000,
        },
    );
    saved.insert(
        "gone-old".to_owned(),
        agents::Changed {
            s: "idle".into(),
            at: NOW - 31 * 86_400_000,
        },
    );
    saved.insert(
        "gone-new".to_owned(),
        agents::Changed {
            s: "idle".into(),
            at: NOW - 86_400_000,
        },
    );
    let mut tracker = Tracker::with_times(saved);
    let list = build(&mut tracker, &m, &Opts::default());
    assert_eq!(
        row(&list, "t4").ago.as_deref(),
        Some("3h"),
        "kept from before"
    );
    assert_eq!(
        row(&list, "t1").ago.as_deref(),
        Some("<1m"),
        "first seen now"
    );
    assert!(tracker.dirty);
    assert!(!tracker.changed.contains_key("gone-old"));
    assert!(tracker.changed.contains_key("gone-new"));

    // Done and idle count as one; working is a change.
    tracker.dirty = false;
    update(&mut m, &mut core, vec![agent_of("claude", "t4", Working)]);
    let list = build(&mut tracker, &m, &Opts::default());
    assert_eq!(row(&list, "t4").ago.as_deref(), Some("<1m"));
    assert!(tracker.dirty);
    update(&mut m, &mut core, vec![agent_of("claude", "t4", Idle)]);
    let later = Opts {
        now: NOW + 10 * 60_000,
        ..Opts::default()
    };
    let list = build(&mut tracker, &m, &later);
    assert_eq!(row(&list, "t4").status, Displayed::Done);
    assert_eq!(row(&list, "t4").ago.as_deref(), Some("<1m"));
    assert_eq!(tracker.changed["t4"].s, "idle");
    assert_eq!(tracker.changed["t4"].at, NOW + 10 * 60_000);
    let list = build(
        &mut tracker,
        &m,
        &Opts {
            now: NOW + 70 * 60_000,
            ..Opts::default()
        },
    );
    assert_eq!(row(&list, "t4").ago.as_deref(), Some("1h"));
}

#[test]
fn the_published_order_signs_its_inputs() {
    let mut core = two_spaces();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::FocusAgent("t1".into()));
    let list = build(&mut Tracker::default(), &m, &Opts::default());
    assert_eq!(
        list.published,
        json!({
            "inputs": "priority\nt1\nt1:working:0\nt2:blocked:0\nt4:idle:0",
            "addresses": ["t2", "t1", "t4"],
        })
    );
    assert!(row(&list, "t1").focused);
}

#[test]
fn rows_past_thirty_six_get_two_letter_labels() {
    let tabs: Vec<Value> = (0..40)
        .map(|i| {
            tab(
                &format!("tb{i}"),
                vec![pane(&format!("p{i}"), &format!("t{i}"))],
            )
        })
        .collect();
    let terminals = (0..40)
        .map(|i| agent_of("claude", &format!("t{i}"), Idle))
        .collect();
    let mut core = Core::new(doc(vec![named_space("sp1", "big", tabs)]), terminals);
    let m = connected(&mut core);
    let list = build(
        &mut Tracker::default(),
        &m,
        &Opts {
            sort: Sort::Grouped,
            ..Opts::default()
        },
    );
    let labels: Vec<&str> = agent_rows(&list).iter().map(|a| a.label.as_str()).collect();
    assert_eq!(
        &labels[..11],
        ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "aa"]
    );
    assert!(list.wide_labels);
}

#[test]
fn remote_agents_name_their_machine_and_hide_with_it() {
    let mut core = two_spaces();
    core.terminals
        .insert("devbox/t9".into(), agent_of("claude", "devbox/t9", Working));
    core.docs.insert(
        Some("devbox".into()),
        (
            doc(vec![named_space(
                "sp9",
                "far",
                vec![tab("tb9", vec![pane("p9", "t9")])],
            )]),
            1,
        ),
    );
    let mut m = connected(&mut core);
    let mut view = MachineView {
        state: MachineState::Connected,
        ..Default::default()
    };
    view.machine.name = "devbox".into();
    drive(&mut m, &mut core, Event::Machines(vec![view]));
    let mut tracker = Tracker::default();
    let list = build(&mut tracker, &m, &Opts::default());
    let far = row(&list, "devbox/t9");
    assert_eq!(far.machine.as_deref(), Some("devbox"));
    assert_eq!(far.space.as_deref(), Some("far"));
    assert_eq!(list.rows_order, ["t2", "devbox/t9", "t1", "t4"]);
    let hidden: BTreeSet<String> = ["devbox".to_owned()].into();
    let list = build(
        &mut tracker,
        &m,
        &Opts {
            hidden,
            ..Opts::default()
        },
    );
    assert_eq!(list.rows_order, ["t2", "t1", "t4"]);
    assert!(tracker.changed.contains_key("devbox/t9"), "still tracked");
    assert_eq!(Sort::parse(Some("grouped")), Sort::Grouped);
    assert_eq!(Sort::parse(Some("other")), Sort::Priority);
    assert_eq!(Sort::parse(None), Sort::Priority);
}
