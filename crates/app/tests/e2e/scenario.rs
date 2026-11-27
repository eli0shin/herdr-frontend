//! Scenarios (`tests/e2e/scenarios/*.json`, `docs/e2e.md` "Scenarios"): steps run on both apps,
//! each followed by a full-window screenshot and dumps of what the app reports.
//!
//! A step can, in this order:
//!
//! - change the core both apps share: `status` (a fake agent's new status), `adopt` (a labeled
//!   terminal the apps must place), `send` (`hc send` a line to a terminal);
//! - drive the app: `do` (with `args`) runs through each app's CLI; `keys` and `type` go to
//!   herdr-frontend's window; `herdr-app` replaces what Herdr app does (`{"do", "args"}`, or
//!   `{"skip": why}`); `restart` quits the app and opens it again on the same home;
//! - wait `wait` more seconds before the capture.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::apps::{App, Which};
use crate::isolated::Core;
use crate::report::{Shot, StepRecord};
use crate::seed::World;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// The file's name without `.json`: the scenario's folder in the run.
    #[serde(skip)]
    pub id: String,
    pub title: String,
    /// The `feature-inventory.md` section it covers (`§3`).
    pub inventory: String,
    #[serde(default)]
    pub notes: String,
    pub steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub name: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub status: Option<AgentStatus>,
    #[serde(default)]
    pub adopt: Option<Adopt>,
    #[serde(default)]
    pub send: Option<Send>,
    #[serde(rename = "do", default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Option<Value>,
    #[serde(default)]
    pub keys: Option<String>,
    #[serde(rename = "type", default)]
    pub text: Option<String>,
    #[serde(rename = "herdr-app", default)]
    pub herdr_app: Option<Value>,
    #[serde(default)]
    pub restart: bool,
    #[serde(default)]
    pub wait: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentStatus {
    pub agent: String,
    pub to: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Adopt {
    pub name: String,
    pub space: String,
    pub tab: String,
    pub status: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Send {
    pub terminal: String,
    pub text: String,
}

/// Every scenario in `dir`, by file name.
pub fn load(dir: &Path) -> Result<Vec<Scenario>, String> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|path| {
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            let mut scenario: Scenario =
                serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            scenario.id = path.file_stem().unwrap().to_string_lossy().into_owned();
            for step in &scenario.steps {
                step.check()
                    .map_err(|e| format!("{}: step {:?}: {e}", path.display(), step.name))?;
            }
            Ok(scenario)
        })
        .collect()
}

impl Step {
    /// What the format can't say: keys that read, a `herdr-app` part that is `do` or `skip`, and
    /// `do` alone or keys alone for herdr-frontend.
    fn check(&self) -> Result<(), String> {
        if let Some(keys) = &self.keys {
            herdr_frontend::e2e::key_events(&json!({ "keys": keys }))?;
        }
        if self.command.is_some() && (self.keys.is_some() || self.text.is_some()) {
            return Err("do and keys or type: herdr-frontend would only get the do".into());
        }
        if self.args.is_some() && self.command.is_none() {
            return Err("args without do".into());
        }
        if let Some(over) = &self.herdr_app {
            let valid = over.get("skip").is_some_and(Value::is_string)
                || over.get("do").is_some_and(Value::is_string);
            if !valid {
                return Err("herdr-app: expected {\"do\", \"args\"} or {\"skip\": why}".into());
            }
        }
        Ok(())
    }
}

/// How one app run goes: where its files land and how long Herdr app gets to settle.
pub struct Run<'a> {
    pub dir: &'a Path,
    pub settle: Duration,
}

/// Runs `scenario`'s steps on `app`, after a first capture of where it starts. Returns one shot
/// per step taken (the start first), and why the run stopped early: the window ended, or didn't
/// open again after a `restart`. A step the app refuses is only marked in its shot.
pub fn run(
    run: &Run,
    scenario: &Scenario,
    app: &mut App,
    core: &Core,
    world: &mut World,
) -> (Vec<Shot>, Option<String>) {
    let start = "opened on the seed".to_owned();
    let mut shots = vec![capture(run, app, core, 0, "start", start, true)];
    for (i, step) in scenario.steps.iter().enumerate() {
        let mut done = Vec::new();
        let mut ok = true;
        match change_core(step, core, world) {
            Ok(changes) => done.extend(changes),
            Err(e) => {
                done.push(e);
                ok = false;
            }
        }
        if step.restart {
            let reopened = app.restart(core).and_then(|()| app.wait_ready(core, None));
            if let Err(e) = reopened {
                return (shots, Some(format!("step {:?}: {e}", step.name)));
            }
            done.push("quit and opened again".into());
        }
        match drive(step, app, core, world) {
            Ok((driven, good)) => {
                done.extend(driven);
                ok &= good;
            }
            Err(e) => {
                done.push(e);
                ok = false;
            }
        }
        if step.wait > 0.0 {
            std::thread::sleep(Duration::from_secs_f64(step.wait));
        }
        if let Err(e) = app.running() {
            return (shots, Some(format!("after step {:?}: {e}", step.name)));
        }
        shots.push(capture(
            run,
            app,
            core,
            i + 1,
            &step.name,
            done.join("\n"),
            ok,
        ));
    }
    (shots, None)
}

/// The step's records for the report: name and note.
pub fn records(scenario: &Scenario) -> Vec<StepRecord> {
    let start = StepRecord {
        name: "start".into(),
        note: "The seed, as each app opens on it.".into(),
        ..Default::default()
    };
    std::iter::once(start)
        .chain(scenario.steps.iter().map(|s| StepRecord {
            name: s.name.clone(),
            note: s.note.clone(),
            ..Default::default()
        }))
        .collect()
}

/// A step's changes to the core both apps share. Returns what was done.
fn change_core(step: &Step, core: &Core, world: &mut World) -> Result<Vec<String>, String> {
    let mut done = Vec::new();
    if let Some(s) = &step.status {
        world.set_status(&s.agent, &s.to)?;
        done.push(format!("core: {} reports {}", s.agent, s.to));
    }
    if let Some(a) = &step.adopt {
        world.adopt(core, &a.name, &a.space, &a.tab, &a.status)?;
        done.push(format!(
            "core: {} started for {}/{}",
            a.name, a.space, a.tab
        ));
    }
    if let Some(s) = &step.send {
        let terminal = world.resolve(&s.terminal)?;
        let text = world.resolve(&s.text)?;
        let out = core.hc(&["send", &terminal, &text])?;
        if !out.status.success() {
            return Err(format!(
                "hc send: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        done.push(format!("core: sent {text:?} to {terminal}"));
    }
    Ok(done)
}

/// A step's part for the app. Returns what was done and whether the app did it.
fn drive(
    step: &Step,
    app: &App,
    core: &Core,
    world: &World,
) -> Result<(Vec<String>, bool), String> {
    let mut done = Vec::new();
    let mut ok = true;
    let (command, args) = match (app.which, &step.herdr_app) {
        (Which::HerdrApp, Some(over)) => {
            if let Some(why) = over.get("skip") {
                done.push(format!("skipped: {}", why.as_str().unwrap_or("")));
                return Ok((done, true));
            }
            let command = over["do"]
                .as_str()
                .ok_or("herdr-app: expected do or skip")?;
            (Some(command.to_owned()), over.get("args").cloned())
        }
        _ => (step.command.clone(), step.args.clone()),
    };
    if let Some(command) = command {
        let args = world.resolve_json(&args.unwrap_or(Value::Null))?;
        let mut cli = vec!["do".to_owned(), command.clone()];
        if !args.is_null() {
            cli.push(args.to_string());
        }
        let cli: Vec<&str> = cli.iter().map(String::as_str).collect();
        let (code, reply) = app.cli(core, &cli)?;
        ok &= code == 0;
        done.push(format!("do {command} {args} → exit {code}: {reply}"));
    } else if app.which == Which::Frontend {
        if let Some(keys) = &step.keys {
            app.probe(&json!({ "keys": keys }))?;
            done.push(format!("keys {keys}"));
        }
        if let Some(text) = &step.text {
            let text = world.resolve(text)?;
            app.probe(&json!({ "type": text }))?;
            done.push(format!("typed {text:?}"));
        }
    } else if step.keys.is_some() || step.text.is_some() {
        done.push("keys go to herdr-frontend only; no herdr-app equivalent given".into());
    }
    Ok((done, ok))
}

/// Lets the window settle, then saves its screenshot and what it reports: `ls` and `status`
/// for both, and herdr-frontend's element tree.
fn capture(
    run: &Run,
    app: &App,
    core: &Core,
    i: usize,
    name: &str,
    action: String,
    ok: bool,
) -> Shot {
    let settled = app.settle(run.settle);
    let base = format!("{i:02}-{}.{}", slug(name), app.which.slug());
    let mut shot = Shot {
        action,
        ok,
        settled,
        ..Default::default()
    };
    let png = format!("{base}.png");
    match app.capture(core, &run.dir.join(&png)) {
        Ok(()) => shot.png = Some(png),
        Err(e) => shot.errors.push(e),
    }
    let state = json!({
        "status": app.cli(core, &["status"]).map(|(_, v)| v).unwrap_or_else(|e| json!(e)),
        "ls": app.cli(core, &["ls"]).map(|(_, v)| v).unwrap_or_else(|e| json!(e)),
    });
    let file = format!("{base}.state.json");
    if write_json(&run.dir.join(&file), &state).is_ok() {
        shot.state = Some(file);
    }
    if app.which == Which::Frontend {
        match app.tree() {
            Ok(tree) => {
                let file = format!("{base}.tree.json");
                if write_json(&run.dir.join(&file), &tree).is_ok() {
                    shot.tree = Some(file);
                }
            }
            Err(e) => shot.errors.push(format!("tree: {e}")),
        }
    }
    shot
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// A file-name-safe form of a step name.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}
