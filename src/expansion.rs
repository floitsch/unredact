// Copyright (C) 2026 Toit contributors.
//! Open candidate proposals are hypotheses, checked by the ordinary joint solver.
use crate::{
    contextual::{self, Context},
    project::{Project, hash},
    solver::{self, Candidate, Report, Surface},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone)]
pub struct Config {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub timeout_seconds: u64,
    pub max_proposals: usize,
    pub surface_kind: String,
    pub paragraph_context: bool,
    pub width_guidance: bool,
}

/// Fixed reference strings, never selected from hidden text or candidate labels.
/// Each is measured in exactly the target's coordinate system, including anchors.
pub fn width_guidance(
    project: &Project,
    dir: &Path,
    group: &str,
) -> Result<BTreeMap<String, Value>> {
    let (targets, _) = solver::independent_targets(project, group)?;
    let mut fonts = BTreeMap::new();
    for t in &targets {
        crate::scan_span::validate_pixels(project.page(&t.page)?, t, dir)?;
        for fit in &project.profile(&t.profile)?.fits {
            if !fonts.contains_key(&fit.font) {
                fonts.insert(fit.font.clone(), crate::native::Font::open(&fit.font)?);
            }
            fit.verify_font(&fonts[&fit.font])?;
        }
    }
    let spacing = crate::line_spacing::prepare(project, &targets, &fonts)?;
    let mut result = BTreeMap::new();
    for t in targets {
        let mut references = vec![];
        let mut allowed = None;
        for text in [
            "a",
            "word",
            "example",
            "information",
            "short phrase",
            "several words together",
        ] {
            let mut predictions = vec![];
            let mut sigmas = vec![];
            for (i, fit) in project.profile(&t.profile)?.fits.iter().enumerate() {
                if let Some(m) = solver::measure_width(
                    t,
                    fit,
                    &fonts[&fit.font],
                    solver::WidthContext {
                        page: project.page(&t.page)?,
                        line_spacing: spacing.get(&t.id).map(|v| &v[i]),
                    },
                    text,
                )? {
                    ensure!(
                        m.predicted.is_finite()
                            && m.sigma.is_finite()
                            && m.allowed.iter().all(|v| v.is_finite()),
                        "Nonfinite width guidance"
                    );
                    allowed = Some(m.allowed);
                    predictions.push(m.predicted);
                    sigmas.push(m.sigma);
                }
            }
            if !predictions.is_empty() {
                predictions.sort_by(f64::total_cmp);
                sigmas.sort_by(f64::total_cmp);
                references.push(json!({"replacement":text,"predicted_span_pixels":[predictions[0],predictions[predictions.len()-1]],
                    "sigma_pixels":[sigmas[0],sigmas[sigmas.len()-1]]}));
            }
        }
        result.insert(t.id.clone(), json!({"observed_span_pixels":allowed,"references":references,
            "interpretation":"Each fixed example replaces the gap in the same measured span. Anchor text is included for anchored evidence. Ranges retain font hypotheses, not posterior confidence. Compare lengths approximately; these examples are not suggested answers. Pixelation provides no scalar width guidance."}));
    }
    Ok(result)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub target: String,
    pub text: String,
}

#[derive(Serialize)]
pub struct ExpansionReport {
    pub schema_version: u32,
    pub interpretation: String,
    /// Also a candidate file accepted by solve/context-rank.
    pub candidates: Vec<Candidate>,
    pub added_candidate_ids: Vec<String>,
    pub compatible_added_candidate_ids: Vec<String>,
    pub initial: Option<Report>,
    pub expanded: Option<Report>,
    pub contexts: BTreeMap<String, Context>,
    pub proposals: Vec<Proposal>,
    pub provider: Value,
}

pub fn run(
    project: &Project,
    dir: &Path,
    group: &str,
    existing: &[Candidate],
    config: &Config,
) -> Result<ExpansionReport> {
    ensure!(
        (1..=4096).contains(&config.max_proposals),
        "Proposal limit must be 1–4096"
    );
    ensure!(
        existing.len() + config.max_proposals <= 100_000,
        "Expanded candidate limit is 100,000"
    );
    ensure!(
        !config.surface_kind.is_empty() && config.surface_kind != "any",
        "Choose a concrete surface kind"
    );
    let initial = if existing.is_empty() {
        None
    } else {
        Some(solver::solve(
            project,
            dir,
            group,
            existing,
            existing.len(),
        )?)
    };
    let (targets, _) = solver::independent_targets(project, group)?;
    ensure!(
        targets
            .iter()
            .all(|t| t.surface_kind == "any" || t.surface_kind == config.surface_kind),
        "Expansion needs the same surface kind across the group; use solve for entity-specific variants"
    );
    let contexts = targets
        .iter()
        .map(|t| {
            let context = if config.paragraph_context {
                crate::context_layout::paragraph_context(project, t)?
            } else {
                contextual::visible_context(project, t)?
            };
            ensure!(
                !context.left.trim().is_empty() || !context.right.trim().is_empty(),
                "Every occurrence needs visible or supplied context"
            );
            Ok((t.id.clone(), context))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    // Neither labels nor the supplied candidate list anchor the proposal model.
    let mut request = json!({"schema_version":1,"task":"propose_context_candidates","contexts":contexts,
        "limits":{"max_proposals":config.max_proposals,"max_text_bytes":512},"surface_kind":config.surface_kind});
    if config.width_guidance {
        request["width_guidance"] = serde_json::to_value(width_guidance(project, dir, group)?)?;
    }
    let response = crate::provider::call(
        &config.executable,
        &config.args,
        &request,
        Duration::from_secs(config.timeout_seconds),
    )?;
    ensure!(
        response["schema_version"] == 1,
        "Invalid proposal response version"
    );
    let proposals: Vec<Proposal> = serde_json::from_value(response["proposals"].clone())?;
    ensure!(
        proposals.len() <= config.max_proposals,
        "Provider exceeded proposal limit"
    );
    let mut seen = BTreeSet::new();
    for p in &proposals {
        ensure!(
            contexts.contains_key(&p.target),
            "Proposal refers to an unknown or dependent target"
        );
        ensure!(
            !p.text.is_empty()
                && p.text.len() <= 512
                && p.text.trim() == p.text
                && !p.text.chars().any(char::is_control),
            "Invalid proposal text"
        );
        ensure!(
            seen.insert((&p.target, &p.text)),
            "Duplicate target/text proposal"
        );
    }
    let mut candidates = existing.to_vec();
    let existing_text: BTreeSet<_> = existing
        .iter()
        .flat_map(|c| &c.surfaces)
        .filter(|s| s.kind == config.surface_kind)
        .map(|s| s.text.as_str())
        .collect();
    let mut additions: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for p in &proposals {
        if !existing_text.contains(p.text.as_str()) {
            additions.entry(&p.text).or_default().insert(&p.target);
        }
    }
    let source = format!("model-proposal:{}", hash(&serde_json::to_vec(&response)?));
    let mut added_candidate_ids = vec![];
    for text in additions.keys() {
        let id = format!(
            "proposal:{}",
            hash(&serde_json::to_vec(&(&config.surface_kind, text))?)
        );
        ensure!(
            !candidates.iter().any(|c| c.id == id),
            "Proposal ID collides with an existing candidate"
        );
        added_candidate_ids.push(id.clone());
        candidates.push(Candidate {
            id,
            canonical_name: (*text).into(),
            surfaces: vec![Surface {
                text: (*text).into(),
                kind: config.surface_kind.clone(),
            }],
            source_ids: vec![source.clone()],
            context_score: None,
        });
    }
    let expanded = if candidates.is_empty() {
        None
    } else {
        Some(solver::solve(
            project,
            dir,
            group,
            &candidates,
            candidates.len(),
        )?)
    };
    let compatible_added_candidate_ids = expanded
        .as_ref()
        .map(|r| {
            r.candidates
                .iter()
                .filter(|c| c.compatible && added_candidate_ids.contains(&c.id))
                .map(|c| c.id.clone())
                .collect()
        })
        .unwrap_or_default();
    Ok(ExpansionReport { schema_version: 1,
        interpretation: "Model proposals enlarge the tested candidate list and are checked against every independent observation. Existing identities and aliases are preserved. A proposal is not evidence that text was present; failure to propose an alternative does not verify a sole survivor. Generation is bounded, not exhaustive, and cannot certify an absent vocabulary.".into(),
        candidates, added_candidate_ids, compatible_added_candidate_ids, initial, expanded, contexts, proposals,
        provider: json!({"executable":config.executable,"args":config.args,"request":request,"response":response,
            "request_sha256":hash(&serde_json::to_vec(&request)?),"response_sha256":hash(&serde_json::to_vec(&response)?),
            "project_sha256":hash(&serde_json::to_vec(project)?)}), })
}
