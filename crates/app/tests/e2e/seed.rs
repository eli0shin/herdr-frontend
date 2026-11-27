//! The world every scenario starts from (`docs/e2e.md` "The seed"): three spaces with tabs, a
//! three-pane split and fake agents in each status, written to a fresh isolated core as another
//! client (Herdr app, say) would have left it.
//!
//! Fake agents are `sh` scripts (`FAKE_AGENT`) that report OSC 7501 status the way Pi and Claude
//! do. Each reads its status from a control file every 200 ms and reports it when it changes, so
//! a step can move an agent to another status ([`World::set_status`]) without typing into it.
//!
//! Ids are made from names (`id("sp", "work")`), the same on every core, so a scenario names
//! them as `{{sp:work}}`, `{{tb:work-2}}`, `{{p:pi}}` and `{{t:pi}}` (the terminal's address,
//! which differs per core).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use herdr_proto::methods::{DocGet, DocSet, TerminalGet, TerminalRun};
use serde_json::{Value, json};

use crate::isolated::Core;

/// A fake agent: sets its title once, then reports the control file's status whenever it
/// changes, and prints it.
const FAKE_AGENT: &str = r#"#!/bin/sh
# fake-agent NAME CTL TITLE: herdr-frontend's end-to-end harness (docs/e2e.md).
name=$1; ctl=$2; title=$3
[ -n "$title" ] && printf '\033]2;%s\007' "$title"
printf 'fake agent %s\n' "$name"
last=
while :; do
  s=$(cat "$ctl" 2>/dev/null)
  if [ "$s" != "$last" ]; then
    last=$s
    printf '\033]7501;%s\033\\' "$s"
    printf 'status %s\n' "$s"
  fi
  sleep 0.2
done
"#;

/// A space's id, a tab's or a pane's, from its name: `sp`, `tb`, `p` or `s` and 32 hex digits,
/// as herdr-app makes them.
pub fn id(kind: &str, name: &str) -> String {
    let hash = |seed: u64| {
        let mut h = 0xcbf2_9ce4_8422_2325_u64 ^ seed;
        for b in format!("{kind}:{name}").bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h
    };
    format!("{kind}{:016x}{:016x}", hash(1), hash(2))
}

/// One pane of the seed: a shell, or an agent with its first status.
enum Pane {
    Shell(&'static str),
    Agent {
        name: &'static str,
        status: &'static str,
        title: &'static str,
    },
}

/// The seeded core: its terminals by name, and where the fake agents read their status.
pub struct World {
    names: BTreeMap<String, String>,
    agents: PathBuf,
    script: PathBuf,
    folders: PathBuf,
}

/// The seed's spaces, outside in: `(space, name, folder, tabs)`, each tab `(tab, name, panes)`.
/// A tab with three panes is split `p1 | (p2 / p3)`.
type Tabs = &'static [(&'static str, &'static str, &'static [Pane])];
const SEED: &[(&str, &str, &str, Tabs)] = &[
    (
        "work",
        "work",
        "work",
        &[
            (
                "work-1",
                "",
                &[Pane::Shell("w1"), Pane::Shell("w2"), Pane::Shell("w3")],
            ),
            (
                "work-2",
                "",
                &[Pane::Agent {
                    name: "pi",
                    status: "state=working:app=pi",
                    title: "π - fix-login - work",
                }],
            ),
            (
                "work-3",
                "",
                &[Pane::Agent {
                    name: "done",
                    status: "state=done:app=claude",
                    title: "Write the changelog",
                }],
            ),
        ],
    ),
    (
        "api",
        "",
        "api",
        &[
            ("api-1", "server", &[Pane::Shell("a1")]),
            (
                "api-2",
                "",
                &[Pane::Agent {
                    name: "blocked",
                    status: "state=blocked:kind=permission:app=claude",
                    title: "Migrate the schema",
                }],
            ),
        ],
    ),
    (
        "notes",
        "notes",
        "notes",
        &[
            ("notes-1", "", &[Pane::Shell("n1")]),
            (
                "notes-2",
                "",
                &[Pane::Agent {
                    name: "idle",
                    status: "state=idle:app=pi",
                    title: "π - triage - notes",
                }],
            ),
        ],
    ),
];

impl World {
    /// Seeds `core`: terminals, agents and the sub-agent, then the layout document.
    pub fn seed(core: &Core) -> Result<World, String> {
        let agents = core.scratch.join("agents");
        let folders = core.scratch.join("code");
        std::fs::create_dir_all(&agents).map_err(|e| e.to_string())?;
        let script = core.scratch.join("fake-agent");
        std::fs::write(&script, FAKE_AGENT).map_err(|e| e.to_string())?;
        let mut world = World {
            names: BTreeMap::new(),
            agents,
            script,
            folders,
        };
        let mut spaces = Vec::new();
        for (space, name, folder, tabs) in SEED {
            let dir = world.folders.join(folder);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let mut made = Vec::new();
            for (tab, tab_name, panes) in *tabs {
                let mut blocks = Vec::new();
                for pane in *panes {
                    let (key, address) = match pane {
                        Pane::Shell(name) => (*name, world.shell(core, name, &dir)?),
                        Pane::Agent {
                            name,
                            status,
                            title,
                        } => (
                            *name,
                            world.agent(core, name, status, title, &dir, None, None)?,
                        ),
                    };
                    let pane_id = id("p", key);
                    world.names.insert(format!("p:{key}"), pane_id.clone());
                    blocks.push(
                        json!({"id": pane_id, "type": "block", "content": {"terminal": address}}),
                    );
                }
                let tab_id = id("tb", tab);
                world.names.insert(format!("tb:{tab}"), tab_id.clone());
                let children = if blocks.len() == 3 {
                    let right = blocks.split_off(1);
                    vec![
                        blocks.remove(0),
                        json!({"id": id("s", tab), "type": "column", "size": {"weight": 1.0},
                               "children": right}),
                    ]
                } else {
                    blocks
                };
                made.push(json!({"id": tab_id, "name": tab_name,
                                 "root": {"id": tab_id, "type": "row", "children": children}}));
            }
            let space_id = id("sp", space);
            world.names.insert(format!("sp:{space}"), space_id.clone());
            spaces.push(
                json!({"id": space_id, "name": name, "folder": dir.display().to_string(),
                               "tabs": made}),
            );
        }
        // A sub-agent of `pi`, with no pane: it shows under its parent.
        let parent = world.address("pi")?;
        let work = world.folders.join("work");
        world.agent(
            core,
            "explore",
            "state=working:app=claude",
            "",
            &work,
            Some(parent),
            None,
        )?;
        // `done` has to have been working for its turn to count.
        world.wait_status(core, "done", "done")?;
        for agent in ["pi", "blocked", "idle", "explore"] {
            world.wait_reported(core, agent)?;
        }
        write_layout(core, json!({ "spaces": spaces }))?;
        Ok(world)
    }

    /// The space ids the seed made, in order.
    pub fn space_ids(&self) -> Vec<String> {
        SEED.iter().map(|(space, ..)| id("sp", space)).collect()
    }

    /// A seeded terminal's address.
    pub fn address(&self, name: &str) -> Result<String, String> {
        self.names
            .get(&format!("t:{name}"))
            .cloned()
            .ok_or_else(|| format!("no seeded terminal {name:?}"))
    }

    /// A folder under the seed's `code/`.
    pub fn folder(&self, name: &str) -> String {
        self.folders.join(name).display().to_string()
    }

    /// `{{kind:name}}` in `text`, replaced: `t` is a terminal's address, `sp`, `tb`, `p` and `s`
    /// ids (also of things a step makes later), `folder` a folder under the seed's `code/`.
    pub fn resolve(&self, text: &str) -> Result<String, String> {
        let mut out = String::new();
        let mut rest = text;
        while let Some(start) = rest.find("{{") {
            out.push_str(&rest[..start]);
            let end = rest[start..]
                .find("}}")
                .ok_or_else(|| format!("unclosed {{{{ in {text:?}"))?;
            let reference = &rest[start + 2..start + end];
            let (kind, name) = reference
                .split_once(':')
                .ok_or_else(|| format!("{{{{{reference}}}}}: expected kind:name"))?;
            out.push_str(&match kind {
                "t" => self.address(name)?,
                "folder" => self.folder(name),
                "sp" | "tb" | "p" | "s" => id(kind, name),
                _ => return Err(format!("{{{{{reference}}}}}: unknown kind {kind:?}")),
            });
            rest = &rest[start + end + 2..];
        }
        out.push_str(rest);
        Ok(out)
    }

    /// Every string in `value`, resolved.
    pub fn resolve_json(&self, value: &Value) -> Result<Value, String> {
        Ok(match value {
            Value::String(s) => Value::String(self.resolve(s)?),
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|v| self.resolve_json(v))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| Ok((k.clone(), self.resolve_json(v)?)))
                    .collect::<Result<_, String>>()?,
            ),
            other => other.clone(),
        })
    }

    /// Moves a fake agent to another status (`state=done:app=pi`); it reports it within 200 ms.
    pub fn set_status(&self, agent: &str, status: &str) -> Result<(), String> {
        self.address(agent)?;
        std::fs::write(self.agents.join(agent), status).map_err(|e| e.to_string())
    }

    /// Starts a fake agent labeled for a space and tab, the way `spawn-worker` does
    /// (`hc run --bg --keep --no-parent --label herdr.space=… --label herdr.tab=…`).
    pub fn adopt(
        &mut self,
        core: &Core,
        name: &str,
        space: &str,
        tab: &str,
        status: &str,
    ) -> Result<(), String> {
        let labels = BTreeMap::from([
            ("herdr.space".to_owned(), id("sp", space)),
            ("herdr.tab".to_owned(), id("tb", tab)),
        ]);
        let folder = self.folders.join(space);
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        self.agent(core, name, status, "", &folder, None, Some(labels))?;
        Ok(())
    }

    fn shell(&mut self, core: &Core, name: &str, dir: &Path) -> Result<String, String> {
        let script =
            format!("printf '%s\\n' 'shell {name}'; export PS1='{name} $ '; exec /bin/sh -i");
        let run = TerminalRun {
            argv: vec!["/bin/sh".into(), "-c".into(), script],
            cwd: Some(dir.display().to_string()),
            keep: true,
            no_parent: true,
            ..Default::default()
        };
        self.run(core, name, run)
    }

    #[allow(clippy::too_many_arguments)]
    fn agent(
        &mut self,
        core: &Core,
        name: &str,
        status: &str,
        title: &str,
        dir: &Path,
        parent: Option<String>,
        labels: Option<BTreeMap<String, String>>,
    ) -> Result<String, String> {
        let ctl = self.agents.join(name);
        // A done agent starts working, so its turn goes up when it finishes.
        let first = if status.starts_with("state=done") {
            status.replacen("state=done", "state=working", 1)
        } else {
            status.to_owned()
        };
        std::fs::write(&ctl, &first).map_err(|e| e.to_string())?;
        let run = TerminalRun {
            argv: vec![
                "/bin/sh".into(),
                self.script.display().to_string(),
                name.into(),
                ctl.display().to_string(),
                title.into(),
            ],
            cwd: Some(dir.display().to_string()),
            name: parent.is_some().then(|| name.to_owned()),
            labels: labels.unwrap_or_default(),
            keep: parent.is_none(),
            no_parent: parent.is_none(),
            parent,
            ..Default::default()
        };
        let address = self.run(core, name, run)?;
        if first != status {
            self.wait_status(core, name, "working")?;
            self.set_status(name, status)?;
        }
        Ok(address)
    }

    fn run(&mut self, core: &Core, name: &str, run: TerminalRun) -> Result<String, String> {
        let address = core
            .conn
            .call(run)
            .wait()
            .map_err(|e| format!("starting {name}: {e}"))?
            .terminal
            .address;
        self.names.insert(format!("t:{name}"), address.clone());
        Ok(address)
    }

    fn status_of(&self, core: &Core, agent: &str) -> Result<Option<String>, String> {
        let terminal = core
            .conn
            .call(TerminalGet {
                terminal: self.address(agent)?,
                ..Default::default()
            })
            .wait()
            .map_err(|e| format!("terminal.get {agent}: {e}"))?;
        Ok(terminal.status.map(|s| s.state.as_str().to_owned()))
    }

    fn wait_status(&self, core: &Core, agent: &str, state: &str) -> Result<(), String> {
        until(&format!("{agent} to report {state}"), || {
            Ok(self.status_of(core, agent)?.as_deref() == Some(state))
        })
    }

    fn wait_reported(&self, core: &Core, agent: &str) -> Result<(), String> {
        until(&format!("{agent} to report a status"), || {
            Ok(self.status_of(core, agent)?.is_some())
        })
    }
}

/// Polls `done` for up to 10 s.
fn until(what: &str, mut done: impl FnMut() -> Result<bool, String>) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done()? {
        if Instant::now() > deadline {
            return Err(format!("timed out waiting for {what}"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

/// Writes the layout document at the version it reads.
fn write_layout(core: &Core, doc: Value) -> Result<(), String> {
    let read = core
        .conn
        .call(DocGet {
            name: "herdr-app".into(),
            ..Default::default()
        })
        .wait()
        .map_err(|e| format!("doc.get: {e}"))?;
    core.conn
        .call(DocSet {
            name: "herdr-app".into(),
            value: doc,
            version: read.version,
            ..Default::default()
        })
        .wait()
        .map_err(|e| format!("doc.set: {e}"))?;
    Ok(())
}
