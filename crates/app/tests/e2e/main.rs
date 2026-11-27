//! The end-to-end harness (`docs/e2e.md`): the full herdr-frontend app and Herdr app, each in a
//! real window on its own isolated herdr-core seeded the same way, driven through the scenarios in
//! `tests/e2e/scenarios/`, with full-window screenshots and dumps at every step and an HTML index
//! of the pairs in `target/e2e/<run>/`.
//!
//! It opens windows on the person's display, so plain `cargo test` only builds it: it runs when
//! `HERDR_E2E=1` (`just e2e`). It never touches the person's core or running Herdr app, and it
//! sends no OS-level input.
//!
//! ```text
//! just e2e [--only <part of a scenario name>]... [--app frontend|herdr-app|both]
//!          [--settle <seconds Herdr app gets per step>] [--list]
//! ```

mod apps;
mod isolated;
mod report;
mod scenario;
mod seed;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use apps::{App, Programs, Which};
use report::{Report, ScenarioRecord};
use scenario::Scenario;

/// Runs the harness; without it the binary only says it skipped.
const RUN_VAR: &str = "HERDR_E2E";

struct Options {
    only: Vec<String>,
    apps: Vec<Which>,
    settle: Duration,
    list: bool,
}

fn main() {
    if std::env::var_os(RUN_VAR).is_none() {
        // Still read every scenario, so a broken one fails `cargo test`.
        match scenario::load(&scenarios_dir()) {
            Ok(all) => println!(
                "e2e: {} scenarios read; skipped running them: they open windows on your \
                 display (`just e2e` runs them)",
                all.len()
            ),
            Err(e) => {
                eprintln!("e2e: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    match run() {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("e2e: {e}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<i32, String> {
    let options = parse(std::env::args().skip(1))?;
    let scenarios: Vec<Scenario> = scenario::load(&scenarios_dir())?
        .into_iter()
        .filter(|s| {
            options.only.is_empty() || options.only.iter().any(|o| s.id.contains(o.as_str()))
        })
        .collect();
    if options.list {
        for s in &scenarios {
            println!("{}\t{} {}", s.id, s.inventory, s.title);
        }
        return Ok(0);
    }
    if scenarios.is_empty() {
        return Err("no scenario matches --only".into());
    }

    let hc = std::env::var_os("HERDR_TEST_HC")
        .map(PathBuf::from)
        .or_else(|| herdr_client::find_hc(None))
        .ok_or("no hc: install herdr-core or set HERDR_TEST_HC")?;
    let programs = Programs {
        frontend: PathBuf::from(env!("CARGO_BIN_EXE_herdr-frontend")),
        herdr_app: herdr_app(),
    };
    let mut apps = options.apps.clone();
    if programs.herdr_app.is_none() && apps.contains(&Which::HerdrApp) {
        if apps == [Which::HerdrApp] {
            return Err("no herdr-app, or no config to copy (see docs/e2e.md)".into());
        }
        eprintln!("e2e: no herdr-app or no config to copy: running herdr-frontend only");
        apps.retain(|w| *w == Which::Frontend);
    }

    let run = output("date", &["+%Y%m%d-%H%M%S"])?;
    let out = target_dir()?.join("e2e").join(&run);
    std::fs::create_dir_all(out.join("logs")).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut report = Report {
        run: run.clone(),
        started: output("date", &[])?,
        frontend: programs.frontend.display().to_string(),
        herdr_app: apps
            .contains(&Which::HerdrApp)
            .then(|| {
                programs
                    .herdr_app
                    .as_ref()
                    .map(|(p, _)| p.display().to_string())
            })
            .flatten(),
        scenarios: Vec::new(),
    };
    eprintln!("e2e: {} scenarios into {}", scenarios.len(), out.display());

    let mut problems = 0;
    for s in &scenarios {
        let folder = out.join(&s.id);
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let mut record = ScenarioRecord {
            id: s.id.clone(),
            title: s.title.clone(),
            inventory: s.inventory.clone(),
            notes: s.notes.clone(),
            steps: scenario::records(s),
            problems: Vec::new(),
        };
        for &which in &apps {
            eprintln!("e2e: {} on {}", s.id, which.title());
            let log = out
                .join("logs")
                .join(format!("{}.{}.log", s.id, which.slug()));
            let settings = scenario::Run {
                dir: &folder,
                settle: match which {
                    Which::Frontend => Duration::from_secs(6),
                    Which::HerdrApp => options.settle,
                },
            };
            let ran =
                run_one(&settings, s, which, &programs, &hc, &log).and_then(|(shots, stopped)| {
                    for (step, shot) in record.steps.iter_mut().zip(shots) {
                        match which {
                            Which::Frontend => step.frontend = Some(shot),
                            Which::HerdrApp => step.herdr_app = Some(shot),
                        }
                    }
                    stopped.map_or(Ok(()), Err)
                });
            match ran {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("e2e: {} on {}: {e}", s.id, which.title());
                    record.problems.push(format!("{}: {e}", which.title()));
                    problems += 1;
                }
            }
        }
        report.scenarios.push(record);
        report.write(&out)?;
    }
    eprintln!("e2e: report at {}", out.join("index.html").display());
    Ok(if problems == 0 { 0 } else { 1 })
}

/// One app through one scenario, on a fresh core seeded the same as every other run: its shots,
/// and why it stopped early if it did (`scenario::run`).
fn run_one(
    settings: &scenario::Run,
    s: &Scenario,
    which: Which,
    programs: &Programs,
    hc: &Path,
    log: &Path,
) -> Result<(Vec<report::Shot>, Option<String>), String> {
    let core = isolated::Core::start(hc)?;
    assert!(core.session().starts_with("test-"));
    let mut world = seed::World::seed(&core)?;
    let mut app = App::launch(which, programs, &core, log)?;
    app.wait_ready(&core, Some(&world.space_ids()))?;
    let ran = scenario::run(settings, s, &mut app, &core, &mut world);
    app.quit(&core);
    Ok(ran)
}

fn parse(args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        only: Vec::new(),
        apps: vec![Which::Frontend, Which::HerdrApp],
        settle: Duration::from_secs_f64(1.5),
        list: false,
    };
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--only" => options.only.push(value()?),
            "--app" => {
                options.apps = match value()?.as_str() {
                    "frontend" => vec![Which::Frontend],
                    "herdr-app" => vec![Which::HerdrApp],
                    "both" => vec![Which::Frontend, Which::HerdrApp],
                    other => {
                        return Err(format!(
                            "--app is frontend, herdr-app or both, not {other:?}"
                        ));
                    }
                }
            }
            "--settle" => {
                let secs: f64 = value()?.parse().map_err(|_| "--settle takes seconds")?;
                options.settle = Duration::from_secs_f64(secs.max(0.0));
            }
            "--list" => options.list = true,
            other => return Err(format!("unknown argument {other:?} (see docs/e2e.md)")),
        }
    }
    Ok(options)
}

/// `herdr-app` (`HERDR_E2E_HERDR_APP`, else on `PATH`, else `~/.local/bin`) and the config its
/// home copies (`HERDR_E2E_APP_CONFIG`, else `~/.dotfiles/home/.config/herdr-app`), when both
/// are there.
fn herdr_app() -> Option<(PathBuf, PathBuf)> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let program = std::env::var_os("HERDR_E2E_HERDR_APP")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::split_paths(&std::env::var_os("PATH")?)
                .map(|dir| dir.join("herdr-app"))
                .find(|p| p.is_file())
        })
        .unwrap_or_else(|| home.join(".local/bin/herdr-app"));
    let config = std::env::var_os("HERDR_E2E_APP_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".dotfiles/home/.config/herdr-app"));
    (program.is_file() && config.is_dir()).then_some((program, config))
}

fn scenarios_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/scenarios")
}

/// The Cargo target folder this binary was built into (`target/debug/deps/e2e-…`).
fn target_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    exe.ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("no target folder above {}", exe.display()))
}

fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
