//! The run's report: `run.json` with every step's record, and `index.html`, each scenario's
//! steps with herdr-frontend's screenshot beside Herdr app's. Both are rewritten after every
//! scenario, so a run stopped halfway still has a report of what it did.

use std::fmt::Write as _;
use std::path::Path;

use serde::Serialize;

/// One app's capture at one step.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Shot {
    /// What the step did to this app, and how the app answered.
    pub action: String,
    /// Whether the app did what the step asked (a `do` exited 0).
    pub ok: bool,
    /// Whether the window came to rest before the capture.
    pub settled: bool,
    /// Files in the scenario's folder.
    pub png: Option<String>,
    pub state: Option<String>,
    pub tree: Option<String>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct StepRecord {
    pub name: String,
    pub note: String,
    pub frontend: Option<Shot>,
    pub herdr_app: Option<Shot>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ScenarioRecord {
    pub id: String,
    pub title: String,
    pub inventory: String,
    pub notes: String,
    pub steps: Vec<StepRecord>,
    /// Why an app's run of it failed or was left out, by app.
    pub problems: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub run: String,
    pub started: String,
    pub frontend: String,
    pub herdr_app: Option<String>,
    pub scenarios: Vec<ScenarioRecord>,
}

impl Report {
    /// Writes `run.json` and `index.html` into `dir`.
    pub fn write(&self, dir: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("run.json"), json).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("index.html"), self.html()).map_err(|e| e.to_string())
    }

    fn html(&self) -> String {
        let mut h = String::new();
        let _ = write!(
            h,
            "<!doctype html><meta charset=utf-8><title>herdr-frontend e2e {run}</title>\
             <style>{STYLE}</style><h1>herdr-frontend end-to-end run {run}</h1>\
             <p class=meta>Started {started}. herdr-frontend: <code>{frontend}</code>. \
             Herdr app: <code>{app}</code>. Each step shows herdr-frontend on the left and Herdr \
             app on the right, on identical isolated cores. Click a screenshot for full size.</p>",
            run = esc(&self.run),
            started = esc(&self.started),
            frontend = esc(&self.frontend),
            app = esc(self.herdr_app.as_deref().unwrap_or("not run")),
        );
        h.push_str("<nav><ul>");
        for s in &self.scenarios {
            let _ = write!(
                h,
                "<li><a href='#{id}'>{inv} {title}</a>{bad}</li>",
                id = esc(&s.id),
                inv = esc(&s.inventory),
                title = esc(&s.title),
                bad = if s.problems.is_empty() {
                    ""
                } else {
                    " <span class=bad>!</span>"
                },
            );
        }
        h.push_str("</ul></nav>");
        for s in &self.scenarios {
            let _ = write!(
                h,
                "<section id='{id}'><h2>{inv} {title}</h2>",
                id = esc(&s.id),
                inv = esc(&s.inventory),
                title = esc(&s.title),
            );
            if !s.notes.is_empty() {
                let _ = write!(h, "<p class=notes>{}</p>", esc(&s.notes));
            }
            for p in &s.problems {
                let _ = write!(h, "<p class=bad>{}</p>", esc(p));
            }
            h.push_str("<table><tr><th>Step</th><th>herdr-frontend</th><th>Herdr app</th></tr>");
            for (i, step) in s.steps.iter().enumerate() {
                let _ = write!(
                    h,
                    "<tr><td class=step><b>{i}. {name}</b><p>{note}</p></td>{fe}{app}</tr>",
                    name = esc(&step.name),
                    note = esc(&step.note),
                    fe = cell(&s.id, step.frontend.as_ref()),
                    app = cell(&s.id, step.herdr_app.as_ref()),
                );
            }
            h.push_str("</table></section>");
        }
        h
    }
}

/// One app's cell: its screenshot, what the step did, and links to the dumps.
fn cell(scenario: &str, shot: Option<&Shot>) -> String {
    let Some(shot) = shot else {
        return "<td class=none>not run</td>".into();
    };
    let mut h = String::from("<td>");
    if let Some(png) = &shot.png {
        let _ = write!(
            h,
            "<a href='{scenario}/{png}'><img loading=lazy src='{scenario}/{png}'></a>",
            scenario = esc(scenario),
            png = esc(png),
        );
    }
    let class = if shot.ok { "action" } else { "action bad" };
    let _ = write!(h, "<pre class='{class}'>{}</pre>", esc(&shot.action));
    if !shot.settled {
        h.push_str("<p class=warn>did not come to rest before the capture</p>");
    }
    for e in &shot.errors {
        let _ = write!(h, "<p class=bad>{}</p>", esc(e));
    }
    let links: Vec<String> = [("state", &shot.state), ("tree", &shot.tree)]
        .into_iter()
        .filter_map(|(label, file)| {
            file.as_ref()
                .map(|f| format!("<a href='{}/{}'>{label}</a>", esc(scenario), esc(f)))
        })
        .collect();
    if !links.is_empty() {
        let _ = write!(h, "<p class=links>{}</p>", links.join(" · "));
    }
    h.push_str("</td>");
    h
}

fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&#39;")
        .replace('"', "&quot;")
}

const STYLE: &str = "body{background:#161616;color:#d4d4d4;font:14px -apple-system,sans-serif;\
margin:24px}h1{font-size:20px}h2{font-size:17px;margin-top:40px;border-top:1px solid #333;\
padding-top:16px}code,pre{font:12px ui-monospace,monospace}.meta,.notes{color:#8f8f8f;\
max-width:1100px}nav ul{columns:3;list-style:none;padding:0}a{color:#3794ff}table{width:100%;\
border-collapse:collapse;table-layout:fixed}th{text-align:left;color:#8f8f8f;font-weight:500}\
th:first-child,td.step{width:220px}td{vertical-align:top;padding:8px;border-top:1px solid #2a2a2a}\
td img{width:100%;border:1px solid #333}pre.action{white-space:pre-wrap;word-break:break-all;\
color:#a9a9a9;margin:6px 0}.bad{color:#f48771}.warn{color:#cca700}.none{color:#666}\
td.step p{color:#8f8f8f}.links{font-size:12px}";
