//! Runs the Ghostscript plate check over scenes and writes `report.md` and `results.json`.
//!
//! Pending scenes are written as far as the writer can ([`reference::write_partial`]) and are
//! expected to fail; their results show what the writer prints today.
//!
//! [`run_flattened`] also flattens each scene's PDF with a caller's flattener and checks the
//! flattened page the same way, plus [`structure::transparency`]. The flattener comes from the
//! caller so that this crate doesn't depend on it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

use crate::judge::{self, Judgement, Verdict};
use crate::reference::{self, Context};
use crate::scene::{Level, PROCESS, Scene};
use crate::{Error, gs, structure};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Pass,
    Fail,
    /// A pending scene fails, as it should until its task lands.
    ExpectedFail,
    /// A pending scene passes: the feature works, or the scene doesn't catch its absence.
    PendingPasses,
    /// A pending scene needs a feature the writer can't leave out.
    NotWritten,
    /// The harness failed (Ghostscript, TIFF, writer error).
    Error,
}

impl Status {
    /// Whether the scene result is what the scene's `pending` field says it should be.
    pub fn as_expected(self) -> bool {
        matches!(self, Status::Pass | Status::ExpectedFail | Status::PendingPasses | Status::NotWritten)
    }

    fn label(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Fail => "**FAIL**",
            Status::ExpectedFail => "expected fail",
            Status::PendingPasses => "pending, passes",
            Status::NotWritten => "not written",
            Status::Error => "**ERROR**",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SceneResult {
    pub id: String,
    pub title: String,
    pub pending: Option<String>,
    pub status: Status,
    /// What the writer left out or skipped.
    pub notes: Vec<String>,
    /// Why the scene was not written or judged.
    pub error: Option<String>,
    pub judgement: Option<Judgement>,
    /// The flattened page's result, from [`run_flattened`].
    pub flattened: Option<FlatResult>,
}

/// One scene's flattened page.
#[derive(Debug, Clone, Serialize)]
pub struct FlatResult {
    /// As for the source page; a pass also needs `transparency` empty.
    pub status: Status,
    /// What [`structure::transparency`] found in the flattened page.
    pub transparency: Vec<String>,
    /// Time spent in the flattener.
    pub millis: f64,
    pub error: Option<String>,
    pub judgement: Option<Judgement>,
}

/// Flattens a scene's PDF: the flattened PDF, or why it failed.
pub type Flatten<'a> = &'a dyn Fn(&[u8]) -> Result<Vec<u8>, String>;

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub ghostscript: String,
    pub resolution: f32,
    pub scenes: Vec<SceneResult>,
}

impl Report {
    /// Scenes whose source or flattened result is not what their `pending` field says.
    pub fn unexpected(&self) -> Vec<String> {
        let mut out = Vec::new();
        for r in &self.scenes {
            if !r.status.as_expected() {
                out.push(format!("{} {:?}{}", r.id, r.status, r.error.as_deref().map(|e| format!(": {e}")).unwrap_or_default()));
            }
            if let Some(f) = r.flattened.as_ref().filter(|f| !f.status.as_expected()) {
                let why = f.error.clone().or_else(|| (!f.transparency.is_empty()).then(|| f.transparency.join(", ")));
                out.push(format!("{} flattened {:?}{}", r.id, f.status, why.map(|e| format!(": {e}")).unwrap_or_default()));
            }
        }
        out
    }
}

/// Write, separate and judge every scene in `ctx.scenes`.
pub fn run(ctx: &Context<'_>, ghostscript: String) -> Report {
    Report { ghostscript, resolution: gs::RESOLUTION, scenes: ctx.scenes.iter().map(|scene| check_scene(scene, ctx)).collect() }
}

/// [`run`], and each scene's page flattened by `flatten`, separated and judged as well.
pub fn run_flattened(ctx: &Context<'_>, ghostscript: String, flatten: Flatten<'_>) -> Report {
    let scenes = ctx.scenes.iter().map(|scene| check_scene_with(scene, ctx, Some(flatten))).collect();
    Report { ghostscript, resolution: gs::RESOLUTION, scenes }
}

/// Write, separate and judge one scene.
pub fn check_scene(scene: &Scene, ctx: &Context<'_>) -> SceneResult {
    check_scene_with(scene, ctx, None)
}

fn check_scene_with(scene: &Scene, ctx: &Context<'_>, flatten: Option<Flatten<'_>>) -> SceneResult {
    let mut result = SceneResult {
        id: scene.id.clone(),
        title: scene.title.clone(),
        pending: scene.pending.clone(),
        status: Status::Error,
        notes: Vec::new(),
        error: None,
        judgement: None,
        flattened: None,
    };
    let written = if scene.pending.is_some() { reference::write_partial(scene, ctx) } else { reference::write(scene, ctx) };
    let output = match written {
        Ok(output) => output,
        Err(Error::Unsupported(feature)) if scene.pending.is_some() => {
            result.status = Status::NotWritten;
            result.error = Some(format!("needs {feature}"));
            return result;
        }
        Err(e) => {
            result.error = Some(e.to_string());
            return result;
        }
    };
    result.notes = output.notes;
    result.flattened = flatten.map(|f| check_flattened(scene, ctx, &output.pdf, f));
    match separate_and_judge(scene, ctx, &output.pdf, &scene.id) {
        Ok(judgement) => {
            result.status = status(scene, judgement.passed());
            result.judgement = Some(judgement);
        }
        Err(e) => result.error = Some(e.to_string()),
    }
    result
}

fn check_flattened(scene: &Scene, ctx: &Context<'_>, pdf: &[u8], flatten: Flatten<'_>) -> FlatResult {
    let mut result = FlatResult { status: Status::Error, transparency: Vec::new(), millis: 0.0, error: None, judgement: None };
    let start = Instant::now();
    let flat = flatten(pdf);
    result.millis = start.elapsed().as_secs_f64() * 1000.0;
    let flat = match flat {
        Ok(flat) => flat,
        Err(e) => {
            result.error = Some(format!("flatten: {e}"));
            return result;
        }
    };
    match structure::summarize(&flat) {
        Ok(summary) => result.transparency = structure::transparency(&summary),
        Err(e) => {
            result.error = Some(e.to_string());
            return result;
        }
    }
    match separate_and_judge(scene, ctx, &flat, &format!("{}-flat", scene.id)) {
        Ok(judgement) => {
            result.status = status(scene, judgement.passed() && result.transparency.is_empty());
            result.judgement = Some(judgement);
        }
        Err(e) => result.error = Some(e.to_string()),
    }
    result
}

fn separate_and_judge(scene: &Scene, ctx: &Context<'_>, pdf: &[u8], label: &str) -> Result<Judgement, Error> {
    let plates = gs::separate(pdf, label)?;
    Ok(judge::judge(scene, &plates, gs::RESOLUTION, ctx.font.is_some()))
}

fn status(scene: &Scene, passed: bool) -> Status {
    match (scene.pending.is_some(), passed) {
        (false, true) => Status::Pass,
        (false, false) => Status::Fail,
        (true, false) => Status::ExpectedFail,
        (true, true) => Status::PendingPasses,
    }
}

/// Cargo's target directory: `CARGO_TARGET_DIR` (relative to the workspace root) or `target`.
pub fn target_dir() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().and_then(Path::parent).unwrap_or(manifest);
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => root.join(dir),
        None => root.join("target"),
    }
}

/// Where the report goes: `plate-test/` in the target directory.
pub fn output_dir() -> PathBuf {
    target_dir().join("plate-test")
}

/// Write `report.md` and `results.json` into `dir`; returns the report's path.
pub fn write(report: &Report, dir: &Path) -> Result<PathBuf, Error> {
    let io = |path: &Path, e: std::io::Error| Error::Scene(format!("{}: {e}", path.display()));
    std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    let json_path = dir.join("results.json");
    let json = serde_json::to_string_pretty(report).map_err(|e| Error::Scene(e.to_string()))?;
    std::fs::write(&json_path, json).map_err(|e| io(&json_path, e))?;
    let md_path = dir.join("report.md");
    std::fs::write(&md_path, markdown(report)).map_err(|e| io(&md_path, e))?;
    Ok(md_path)
}

/// The report as Markdown.
pub fn markdown(report: &Report) -> String {
    let flat = report.scenes.iter().any(|s| s.flattened.is_some());
    let mut md = String::new();
    let _ = writeln!(
        md,
        "# Plate check\n\nGhostscript {}, `tiffsep` at {} dpi with overprint simulated (`/enable`). Ink % per plate \
         (C M Y K, spots by name); plates under 1% are left out. Pending scenes are written without the features \
         they wait for and are expected to fail. Generated by `scripts/plate-check.sh`.\n",
        report.ghostscript, report.resolution
    );
    if flat {
        md.push_str(
            "Each scene's page is also flattened and separated the same way. \"Transparency left\" lists transparency \
             groups, graphics-state soft masks, blend modes and `ca`/`CA` below 1 in the flattened page.\n\n",
        );
        md.push_str(
            "| Scene | Status | Pending | Failing points | Plates | Flattened | Failing points | Plates | Transparency left | Flatten ms |\n\
             |---|---|---|---|---|---|---|---|---|---|\n",
        );
    } else {
        md.push_str("| Scene | Status | Pending | Failing points | Plates |\n|---|---|---|---|---|\n");
    }
    for scene in &report.scenes {
        let (points, plates) = summary_cells(scene.judgement.as_ref());
        let pending = scene.pending.as_deref().unwrap_or("");
        let _ = write!(
            md,
            "| [{id}](#{anchor}) | {} | {pending} | {points} | {plates} |",
            scene.status.label(),
            id = scene.id,
            anchor = scene.id.to_lowercase()
        );
        if let Some(f) = &scene.flattened {
            let (points, plates) = summary_cells(f.judgement.as_ref());
            let left = if f.transparency.is_empty() { "none".to_owned() } else { f.transparency.join(", ") };
            let _ = write!(md, " {} | {points} | {plates} | {left} | {:.0} |", f.status.label(), f.millis);
        } else if flat {
            md.push_str(" – | – | – | – | – |");
        }
        md.push('\n');
    }
    for scene in &report.scenes {
        let _ = writeln!(md, "\n## {}\n\n{}. Status: {}.", scene.id, scene.title, scene.status.label());
        if let Some(task) = &scene.pending {
            let _ = writeln!(md, "Pending {task}.");
        }
        if let Some(error) = &scene.error {
            let _ = writeln!(md, "\n{error}");
        }
        for note in &scene.notes {
            let _ = writeln!(md, "- {note}");
        }
        let flat_judgement = scene.flattened.as_ref().and_then(|f| f.judgement.as_ref());
        if let Some(f) = &scene.flattened {
            let _ = writeln!(md, "\nFlattened: {}, {:.0} ms.", f.status.label(), f.millis);
            if let Some(error) = &f.error {
                let _ = writeln!(md, "\n{error}");
            }
            if !f.transparency.is_empty() {
                let _ = writeln!(md, "Transparency left: {}.", f.transparency.join(", "));
            }
            if let Some(j) = flat_judgement {
                let _ = writeln!(md, "Flattened plates: {}.{}", j.plates.join(", "), plate_problems(j).map(|p| format!(" {p}.")).unwrap_or_default());
            }
        }
        let Some(judgement) = &scene.judgement else { continue };
        let _ = writeln!(md, "\nPlates: {}.{}", judgement.plates.join(", "), plate_problems(judgement).map(|p| format!(" {p}.")).unwrap_or_default());
        if scene.flattened.is_some() {
            md.push_str("\n| # | Point | Expected | Measured | Verdict | Flattened | Verdict |\n|---|---|---|---|---|---|---|\n");
        } else {
            md.push_str("\n| # | Point | Expected | Measured | Verdict |\n|---|---|---|---|---|\n");
        }
        for (i, point) in judgement.points.iter().enumerate() {
            let how = point.how.as_deref().map(|h| format!("; {h}")).unwrap_or_default();
            let note = point.note.as_deref().map(|n| format!("{n} ")).unwrap_or_default();
            let _ = write!(
                md,
                "| {} | {note}({}, {}){how} | {} ±{} | {} | {} |",
                i + 1,
                point.at[0],
                point.at[1],
                expected(&point.expected),
                point.tol,
                measured(&point.measured),
                verdict(point),
            );
            if scene.flattened.is_some() {
                match flat_judgement.and_then(|j| j.points.get(i)) {
                    Some(p) => {
                        let _ = write!(md, " {} | {} |", measured(&p.measured), verdict(p));
                    }
                    None => md.push_str(" – | – |"),
                }
            }
            md.push('\n');
        }
    }
    md
}

fn summary_cells(judgement: Option<&Judgement>) -> (String, String) {
    match judgement {
        Some(j) => (format!("{}/{}", j.failed_points(), j.judged_points()), plate_problems(j).unwrap_or_else(|| "ok".into())),
        None => ("–".into(), "–".into()),
    }
}

fn verdict(point: &judge::PointResult) -> String {
    match point.verdict {
        Verdict::Pass => "pass".to_owned(),
        Verdict::Info => "info".to_owned(),
        Verdict::Skipped => "skipped".to_owned(),
        Verdict::Fail if point.off.is_empty() => "**fail**".to_owned(),
        Verdict::Fail => format!("**fail** ({})", point.off.iter().map(|p| short(p)).collect::<Vec<_>>().join(" ")),
    }
}

fn plate_problems(judgement: &Judgement) -> Option<String> {
    let mut parts = Vec::new();
    if !judgement.missing_plates.is_empty() {
        parts.push(format!("missing {}", judgement.missing_plates.join(", ")));
    }
    if !judgement.extra_plates.is_empty() {
        parts.push(format!("extra {}", judgement.extra_plates.join(", ")));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// Process plates as `C M Y K`; spots keep their names.
fn short(plate: &str) -> &str {
    match PROCESS.iter().position(|p| *p == plate) {
        Some(i) => ["C", "M", "Y", "K"].get(i).copied().unwrap_or(plate),
        None => plate,
    }
}

fn expected(inks: &std::collections::BTreeMap<String, Level>) -> String {
    let mut names: Vec<String> = inks.keys().cloned().collect();
    judge::sort_plates(&mut names);
    let parts: Vec<String> = names
        .iter()
        .filter_map(|name| match inks.get(name)? {
            Level::Value(v) if *v >= 1.0 => Some(format!("{}{v}", short(name))),
            Level::Value(_) => None,
            Level::Range([lo, hi]) => Some(format!("{}{lo}–{hi}", short(name))),
        })
        .collect();
    if parts.is_empty() { "paper".into() } else { parts.join(" ") }
}

fn measured(inks: &std::collections::BTreeMap<String, f32>) -> String {
    let mut names: Vec<String> = inks.iter().filter(|(_, v)| **v >= 1.0).map(|(k, _)| k.clone()).collect();
    judge::sort_plates(&mut names);
    let parts: Vec<String> = names.iter().filter_map(|name| Some(format!("{}{:.0}", short(name), inks.get(name)?))).collect();
    if parts.is_empty() { "paper".into() } else { parts.join(" ") }
}
