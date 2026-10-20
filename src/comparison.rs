// Copyright (C) 2026 Toit contributors.
//! Matched-input adapter evaluation. Labels never enter the adapter request.
use crate::{
    project::{Evidence, hash},
    provider,
    raster::Rect,
    solver::Candidate,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImagePage {
    pub id: String,
    pub image: PathBuf,
    pub image_sha256: String,
    pub dpi: f64,
    pub redaction_regions: Vec<Rect>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputTarget {
    pub id: String,
    pub page: String,
    pub bounds: Rect,
    pub surface_kind: String,
    pub evidence: Evidence,
    pub left_context: String,
    pub right_context: String,
    pub dependence_key: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterInput {
    pub schema_version: u32,
    pub pages: Vec<ImagePage>,
    pub targets: Vec<InputTarget>,
    pub candidates: Vec<Candidate>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub input: AdapterInput,
    pub truth_id: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    pub schema_version: u32,
    pub track: String,
    pub description: String,
    pub cases: Vec<Case>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Local,
    External,
    Ablation,
    Test,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryPin {
    pub directory: PathBuf,
    pub commit: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Adapter {
    pub id: String,
    pub kind: ToolKind,
    pub executable: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    /// Adapter code and runtime binaries, in addition to the executable itself.
    pub artifacts: Vec<PathBuf>,
    pub repository: Option<RepositoryPin>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub required_external: Vec<String>,
    pub adapters: Vec<Adapter>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdapterStatus {
    Ok,
    Unsupported,
    Error,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterResult {
    pub schema_version: u32,
    pub status: AdapterStatus,
    pub reason: Option<String>,
    /// Each group is an evidence tie, in best-to-worst order. No ordinal tie-breaking.
    pub ranked_groups: Vec<Vec<String>>,
    pub ranking_complete: bool,
    /// None when the tool does not expose a compatibility/acceptance decision.
    pub compatible: Option<Vec<String>>,
}
#[derive(Debug, Serialize)]
pub struct CaseMetrics {
    pub truth_in_vocabulary: bool,
    pub unique_correct_first: bool,
    pub truth_in_best_group: bool,
    pub uniquely_wrong_first: bool,
    pub truth_rank: Option<[usize; 2]>,
    pub ranked_candidates: usize,
    pub unranked_candidates: usize,
    pub truth_compatible: Option<bool>,
    pub compatible_candidates: Option<usize>,
    pub single_wrong_compatible: Option<bool>,
    pub no_compatible: Option<bool>,
}
pub fn assess(case: &Case, result: &AdapterResult) -> Result<Option<CaseMetrics>> {
    ensure!(
        result.schema_version == 1,
        "Unsupported adapter response version"
    );
    let ids: BTreeSet<_> = case
        .input
        .candidates
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    ensure!(
        ids.len() == case.input.candidates.len(),
        "Duplicate input candidate IDs"
    );
    if result.status != AdapterStatus::Ok {
        ensure!(
            result.ranked_groups.is_empty()
                && !result.ranking_complete
                && result.compatible.is_none(),
            "Unsupported/error responses must not contain candidate decisions"
        );
        ensure!(
            result.reason.as_ref().is_some_and(|s| !s.trim().is_empty()),
            "Non-success needs a reason"
        );
        return Ok(None);
    }
    let mut seen = BTreeSet::new();
    let mut rank = None;
    let mut start = 1;
    for group in &result.ranked_groups {
        ensure!(!group.is_empty(), "Empty evidence tie group");
        for id in group {
            ensure!(
                ids.contains(id.as_str()) && seen.insert(id.as_str()),
                "Unknown or repeated ranked candidate: {id}"
            );
        }
        if group.contains(&case.truth_id) {
            rank = Some([start, start + group.len() - 1]);
        }
        start += group.len();
    }
    ensure!(
        !result.ranking_complete || seen == ids,
        "Complete ranking omitted candidates"
    );
    if let Some(compatible) = &result.compatible {
        let unique: BTreeSet<_> = compatible.iter().collect();
        ensure!(
            unique.len() == compatible.len()
                && compatible.iter().all(|id| ids.contains(id.as_str())),
            "Unknown or repeated compatible candidate"
        );
    }
    let best = result.ranked_groups.first();
    let truth_present = ids.contains(case.truth_id.as_str());
    Ok(Some(CaseMetrics {
        truth_in_vocabulary: truth_present,
        unique_correct_first: best.is_some_and(|g| g.len() == 1 && g[0] == case.truth_id),
        truth_in_best_group: best.is_some_and(|g| g.contains(&case.truth_id)),
        uniquely_wrong_first: best.is_some_and(|g| g.len() == 1 && g[0] != case.truth_id),
        truth_rank: rank,
        ranked_candidates: seen.len(),
        unranked_candidates: ids.len() - seen.len(),
        truth_compatible: result
            .compatible
            .as_ref()
            .map(|c| c.contains(&case.truth_id)),
        compatible_candidates: result.compatible.as_ref().map(Vec::len),
        single_wrong_compatible: result
            .compatible
            .as_ref()
            .map(|c| c.len() == 1 && c[0] != case.truth_id),
        no_compatible: result.compatible.as_ref().map(Vec::is_empty),
    }))
}
#[derive(Debug, Serialize)]
pub struct Provenance {
    pub adapter: String,
    pub kind: ToolKind,
    pub files: BTreeMap<PathBuf, String>,
    pub repository_commit: Option<String>,
    pub error: Option<String>,
}
fn resolve(base: &Path, path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
    .canonicalize()
    .with_context(|| format!("Missing path {}", path.display()))
}
fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").arg("-C").arg(dir).args(args).output()?;
    ensure!(
        output.status.success(),
        "Git provenance command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().into())
}
fn provenance(adapter: &mut Adapter, base: &Path) -> Result<Provenance> {
    // Invocation paths can carry semantics: resolving a venv's python symlink
    // before execution silently drops its environment. Hash the resolved file
    // below, but invoke the manifest's absolute path without dereferencing it.
    if !adapter.executable.is_absolute() {
        adapter.executable = base.join(&adapter.executable);
    }
    ensure!(
        adapter.kind != ToolKind::External || adapter.repository.is_some(),
        "External adapters require a pinned repository"
    );
    ensure!(
        !adapter.artifacts.is_empty(),
        "List adapter code/runtime artifacts explicitly"
    );
    let mut files = BTreeMap::new();
    for path in std::iter::once(&adapter.executable).chain(&adapter.artifacts) {
        let path = resolve(base, path)?;
        files.insert(path.clone(), hash(&fs::read(path)?));
    }
    let commit = if let Some(repo) = &adapter.repository {
        ensure!(
            (repo.commit.len() == 40 || repo.commit.len() == 64)
                && repo.commit.bytes().all(|c| c.is_ascii_hexdigit()),
            "Pin a full Git object ID, not a branch/tag"
        );
        let directory = resolve(base, &repo.directory)?;
        let actual = git(&directory, &["rev-parse", "HEAD"])?;
        ensure!(
            actual == repo.commit.to_lowercase(),
            "Repository HEAD differs from pinned commit"
        );
        ensure!(
            git(
                &directory,
                &["status", "--porcelain", "--untracked-files=normal"]
            )?
            .is_empty(),
            "Repository contains uncommitted/untracked source changes"
        );
        Some(actual)
    } else {
        None
    };
    Ok(Provenance {
        adapter: adapter.id.clone(),
        kind: adapter.kind.clone(),
        files,
        repository_commit: commit,
        error: None,
    })
}
#[derive(Debug, Serialize)]
pub struct Execution {
    pub adapter: String,
    pub case: String,
    pub repetition: usize,
    pub elapsed_seconds: f64,
    pub input_sha256: String,
    pub result: Option<AdapterResult>,
    pub metrics: Option<CaseMetrics>,
    pub error: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct Summary {
    pub executions: usize,
    pub successful: usize,
    pub unsupported: usize,
    pub failed: usize,
    pub truth_present_successes: usize,
    pub truth_absent_successes: usize,
    pub unique_correct_first: usize,
    pub truth_in_best_group: usize,
    pub uniquely_wrong_first: usize,
    pub compatible_reported: usize,
    pub truth_compatible: usize,
    pub single_wrong_compatible: usize,
    pub no_compatible_when_truth_absent: usize,
    pub mean_elapsed_seconds: f64,
}
#[derive(Debug, Serialize)]
pub struct ComparisonReport {
    pub schema_version: u32,
    pub suite_sha256: String,
    pub manifest_sha256: String,
    pub track: String,
    pub description: String,
    pub missing_verified_external: Vec<String>,
    pub provenance: Vec<Provenance>,
    pub summary: BTreeMap<String, Summary>,
    pub executions: Vec<Execution>,
    pub interpretation: String,
}
pub fn run(
    suite_path: &Path,
    manifest_path: &Path,
    out: &Path,
    repetitions: usize,
    timeout: Duration,
) -> Result<ComparisonReport> {
    ensure!((1..=20).contains(&repetitions), "Use 1–20 repetitions");
    ensure!(!out.exists(), "Use a new comparison output directory");
    let suite_bytes = fs::read(suite_path)?;
    let manifest_bytes = fs::read(manifest_path)?;
    let mut suite: Suite = serde_json::from_slice(&suite_bytes)?;
    let mut manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;
    ensure!(
        suite.schema_version == 1 && manifest.schema_version == 1,
        "Unsupported manifest/suite version"
    );
    ensure!(
        suite.track == "candidate_ranking_from_images",
        "Unsupported comparison track"
    );
    ensure!(
        !suite.cases.is_empty() && suite.cases.len() <= 256,
        "Use 1–256 cases"
    );
    ensure!(
        !manifest.adapters.is_empty() && manifest.adapters.len() <= 16,
        "Use 1–16 adapters"
    );
    let suite_base = suite_path.canonicalize()?.parent().unwrap().to_path_buf();
    let manifest_base = manifest_path
        .canonicalize()?
        .parent()
        .unwrap()
        .to_path_buf();
    let mut case_ids = BTreeSet::new();
    let mut checked_images: BTreeMap<PathBuf, String> = BTreeMap::new();
    for case in &mut suite.cases {
        ensure!(
            !case.id.is_empty() && case_ids.insert(case.id.clone()),
            "Duplicate/empty case ID"
        );
        ensure!(
            !case.truth_id.is_empty() && case.input.schema_version == 1,
            "Invalid case truth/input version"
        );
        ensure!(
            !case.input.candidates.is_empty() && !case.input.targets.is_empty(),
            "Case needs targets and candidates"
        );
        let ids: BTreeSet<_> = case.input.candidates.iter().map(|c| &c.id).collect();
        ensure!(
            ids.len() == case.input.candidates.len(),
            "Duplicate input candidate IDs"
        );
        let mut pages = BTreeSet::new();
        for page in &mut case.input.pages {
            ensure!(
                pages.insert(page.id.clone()) && page.dpi.is_finite() && page.dpi > 0.0,
                "Invalid page ID/DPI"
            );
            page.image = resolve(&suite_base, &page.image)?;
            let actual = if let Some(h) = checked_images.get(&page.image) {
                h.clone()
            } else {
                let h = hash(&fs::read(&page.image)?);
                checked_images.insert(page.image.clone(), h.clone());
                h
            };
            ensure!(actual == page.image_sha256, "Input image hash mismatch");
        }
        let mut target_ids = BTreeSet::new();
        for target in &case.input.targets {
            ensure!(
                pages.contains(&target.page)
                    && target.bounds.valid()
                    && target_ids.insert(&target.id),
                "Invalid target page/geometry/ID"
            );
        }
    }
    let mut ids = BTreeSet::new();
    for adapter in &manifest.adapters {
        ensure!(
            !adapter.id.is_empty() && ids.insert(&adapter.id),
            "Duplicate/empty adapter ID"
        );
    }
    fs::create_dir_all(out)?;
    let out = out.canonicalize()?;
    let mut provenance_records = vec![];
    for adapter in &mut manifest.adapters {
        let p = provenance(adapter, &manifest_base).unwrap_or_else(|e| Provenance {
            adapter: adapter.id.clone(),
            kind: adapter.kind.clone(),
            files: BTreeMap::new(),
            repository_commit: None,
            error: Some(format!("{e:#}")),
        });
        provenance_records.push(p);
    }
    let mut executions = vec![];
    for (case_index, case) in suite.cases.iter().enumerate() {
        for repetition in 0..repetitions {
            // Rotate adapter order by case and repetition to reduce fixed ordering bias.
            for offset in 0..manifest.adapters.len() {
                let tool_index = (offset + case_index + repetition) % manifest.adapters.len();
                let adapter = &manifest.adapters[tool_index];
                let dir = out.join(format!(
                    "case-{case_index:03}-repeat-{repetition:02}-tool-{tool_index:02}"
                ));
                fs::create_dir(&dir)?;
                let request = serde_json::json!({"schema_version":1,"input":case.input,"work_dir":dir.join("work")});
                let input_hash = hash(&serde_json::to_vec(&case.input)?);
                fs::write(
                    dir.join("request.json"),
                    serde_json::to_vec_pretty(&request)?,
                )?;
                let started = Instant::now();
                eprintln!(
                    "Comparing {} on {} (repetition {})",
                    adapter.id,
                    case.id,
                    repetition + 1
                );
                let evaluated = (|| -> Result<(AdapterResult, Option<CaseMetrics>)> {
                    ensure!(
                        provenance_records[tool_index].error.is_none(),
                        "Unverified adapter provenance: {:?}",
                        provenance_records[tool_index].error
                    );
                    let value =
                        provider::call(&adapter.executable, &adapter.args, &request, timeout)?;
                    fs::write(
                        dir.join("response.json"),
                        serde_json::to_vec_pretty(&value)?,
                    )?;
                    let result: AdapterResult = serde_json::from_value(value)?;
                    let metrics = assess(case, &result)?;
                    Ok((result, metrics))
                })();
                let (result, metrics, error) = match evaluated {
                    Ok((r, m)) => (Some(r), m, None),
                    Err(e) => (None, None, Some(format!("{e:#}"))),
                };
                let execution = Execution {
                    adapter: adapter.id.clone(),
                    case: case.id.clone(),
                    repetition,
                    elapsed_seconds: started.elapsed().as_secs_f64(),
                    input_sha256: input_hash,
                    result,
                    metrics,
                    error,
                };
                fs::write(
                    dir.join("execution.json"),
                    serde_json::to_vec_pretty(&execution)?,
                )?;
                executions.push(execution);
            }
        }
    }
    for (image, expected) in checked_images {
        ensure!(
            hash(&fs::read(&image)?) == expected,
            "Input image changed during comparison: {}",
            image.display()
        );
    }
    // Pins and artifact hashes must remain valid throughout the run, not just at startup.
    for (i, adapter) in manifest.adapters.iter_mut().enumerate() {
        if provenance_records[i].error.is_some() {
            continue;
        }
        let failure = match provenance(adapter, &manifest_base) {
            Ok(after)
                if after.files == provenance_records[i].files
                    && after.repository_commit == provenance_records[i].repository_commit =>
            {
                None
            }
            Ok(_) => Some("Adapter artifacts/source changed during comparison".to_string()),
            Err(e) => Some(format!("Post-run provenance check failed: {e:#}")),
        };
        if let Some(error) = failure {
            provenance_records[i].error = Some(error.clone());
            for execution in executions.iter_mut().filter(|e| e.adapter == adapter.id) {
                execution.metrics = None;
                execution.result = None;
                execution.error = Some(error.clone());
                let case_index = suite
                    .cases
                    .iter()
                    .position(|c| c.id == execution.case)
                    .unwrap();
                let path = out.join(format!(
                    "case-{case_index:03}-repeat-{:02}-tool-{i:02}/execution.json",
                    execution.repetition
                ));
                fs::write(path, serde_json::to_vec_pretty(execution)?)?;
            }
        }
    }
    let missing = manifest
        .required_external
        .iter()
        .filter(|id| {
            !provenance_records.iter().any(|p| {
                p.adapter == **id
                    && p.kind == ToolKind::External
                    && p.error.is_none()
                    && p.repository_commit.is_some()
            })
        })
        .cloned()
        .collect();
    let mut summary = BTreeMap::new();
    for adapter in &manifest.adapters {
        let rows: Vec<_> = executions
            .iter()
            .filter(|e| e.adapter == adapter.id)
            .collect();
        let metrics: Vec<_> = rows.iter().filter_map(|e| e.metrics.as_ref()).collect();
        let successful = metrics.len();
        let unsupported = rows
            .iter()
            .filter(|e| {
                e.result
                    .as_ref()
                    .is_some_and(|r| r.status == AdapterStatus::Unsupported)
            })
            .count();
        summary.insert(
            adapter.id.clone(),
            Summary {
                executions: rows.len(),
                successful,
                unsupported,
                failed: rows.len() - successful - unsupported,
                truth_present_successes: metrics.iter().filter(|m| m.truth_in_vocabulary).count(),
                truth_absent_successes: metrics.iter().filter(|m| !m.truth_in_vocabulary).count(),
                unique_correct_first: metrics.iter().filter(|m| m.unique_correct_first).count(),
                truth_in_best_group: metrics.iter().filter(|m| m.truth_in_best_group).count(),
                uniquely_wrong_first: metrics.iter().filter(|m| m.uniquely_wrong_first).count(),
                compatible_reported: metrics
                    .iter()
                    .filter(|m| m.compatible_candidates.is_some())
                    .count(),
                truth_compatible: metrics
                    .iter()
                    .filter(|m| m.truth_compatible == Some(true))
                    .count(),
                single_wrong_compatible: metrics
                    .iter()
                    .filter(|m| m.single_wrong_compatible == Some(true))
                    .count(),
                no_compatible_when_truth_absent: metrics
                    .iter()
                    .filter(|m| !m.truth_in_vocabulary && m.no_compatible == Some(true))
                    .count(),
                mean_elapsed_seconds: rows.iter().map(|e| e.elapsed_seconds).sum::<f64>()
                    / rows.len() as f64,
            },
        );
    }
    let report=ComparisonReport{schema_version:1,suite_sha256:hash(&suite_bytes),manifest_sha256:hash(&manifest_bytes),
        track:suite.track,description:suite.description,missing_verified_external:missing,provenance:provenance_records,summary,executions,
        interpretation:"Matched-input, scope-limited measurements. Ties, unsupported cases, missing candidates, and failures remain explicit. Repository pins verify source state, not adapter fidelity; adapters must be reviewed to establish that the intended implementation actually ran. No broad superiority claim follows automatically. Visible-image inputs omit labels and precomputed typography, but adapters are not sandboxed against reading other local files. Timing includes adapter startup and processing; cache/order effects remain possible.".into()};
    fs::write(
        out.join("comparison.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}
