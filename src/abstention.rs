// Copyright (C) 2026 Toit contributors.
//! Opt-in empirical answer selection, separate from candidate ranking.
use crate::{
    contextual::{ContextRank, ContextReport},
    project::hash,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreMetric {
    CombinedScore,
    MeanSequenceLogScore,
    MeanWordLogScore,
    MeanTokenLogScore,
}
impl ScoreMetric {
    fn score(self, rank: &ContextRank) -> Result<f64> {
        let value = match self {
            Self::CombinedScore => rank.combined_score,
            Self::MeanSequenceLogScore => rank.mean_sequence_log_score,
            Self::MeanWordLogScore => rank.mean_word_log_score,
            Self::MeanTokenLogScore => Some(rank.mean_token_log_score),
        }
        .context("Selected score metric is absent")?;
        ensure!(value.is_finite(), "Nonfinite ranking score");
        Ok(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReportSignature {
    pub normalization: Option<String>,
    pub physical_weight: Option<f64>,
    /// Binds archived policies to their report generation, including old engines.
    pub physical_interpretation: String,
    pub language: serde_json::Value,
}
impl ReportSignature {
    fn accepts(&self, actual: &Self) -> bool {
        if self.normalization != actual.normalization
            || self.physical_weight != actual.physical_weight
            || self.physical_interpretation != actual.physical_interpretation
        {
            return false;
        }
        let mut expected_language = self.language.clone();
        let mut actual_language = actual.language.clone();
        let Some(allowed) = expected_language
            .get("context_sources")
            .and_then(serde_json::Value::as_array)
            .cloned()
        else {
            return false;
        };
        let Some(observed) = actual_language
            .get("context_sources")
            .and_then(serde_json::Value::as_array)
            .cloned()
        else {
            return false;
        };
        if observed.is_empty() || observed.iter().any(|source| !allowed.contains(source)) {
            return false;
        }
        expected_language
            .as_object_mut()
            .unwrap()
            .remove("context_sources");
        actual_language
            .as_object_mut()
            .unwrap()
            .remove("context_sources");
        expected_language == actual_language
    }
}

/// Bind the reported model, scorer, options and adapter. Known replay reports
/// additionally require the hash-verified original cache, with matching scores.
pub fn language_signature(report: &ContextReport, base: &Path) -> Result<serde_json::Value> {
    let provider = report
        .provider
        .as_ref()
        .context("Language provider provenance is absent")?;
    let traces: Vec<_> = provider
        .get("batches")
        .and_then(serde_json::Value::as_array)
        .map(|v| v.iter().collect())
        .unwrap_or_else(|| vec![provider]);
    let mut common = None;
    for trace in traces {
        let response = trace
            .get("response")
            .context("Provider response provenance is absent")?;
        let method = response
            .get("method")
            .and_then(serde_json::Value::as_str)
            .context("Provider scoring method is absent")?;
        let identity = if method == "exact_replay_of_common_language_control" {
            let args = provider
                .get("args")
                .and_then(serde_json::Value::as_array)
                .context("Replay arguments are absent")?;
            let path = args
                .windows(2)
                .find(|pair| pair[0].as_str() == Some("--replay"))
                .and_then(|pair| pair[1].as_str())
                .context("Replay cache path is absent")?;
            let bytes =
                fs::read(base.join(path)).context("Cannot read archived language-score cache")?;
            ensure!(
                response["reference_sha256"].as_str() == Some(hash(&bytes).as_str()),
                "Replay cache hash mismatch"
            );
            let cache: serde_json::Value = serde_json::from_slice(&bytes)?;
            let request = trace.get("request").context("Replay request is absent")?;
            ensure!(
                cache["request"]["contexts"] == request["contexts"],
                "Replay contexts differ from original scores"
            );
            let originals = cache["request"]["items"]
                .as_array()
                .context("Cached items are absent")?;
            let cached_scores = cache["scores"]
                .as_array()
                .context("Cached scores are absent")?;
            let request_items = request["items"]
                .as_array()
                .context("Replay items are absent")?;
            let response_scores = response["scores"]
                .as_array()
                .context("Replay scores are absent")?;
            ensure!(
                request_items.len() == response_scores.len(),
                "Replay item/score counts differ"
            );
            for item in request_items {
                ensure!(
                    originals.iter().any(|o| o == item),
                    "Replay surface differs from original scores"
                );
                let original = cached_scores
                    .iter()
                    .find(|s| s["id"] == item["id"] && s["target"] == item["target"])
                    .context("Replay score is absent from original cache")?;
                let score = response_scores
                    .iter()
                    .find(|s| s["id"] == item["id"] && s["target"] == item["target"])
                    .context("Replay response score is absent")?;
                let original_sum = original["token_log_score_sum"]
                    .as_f64()
                    .context("Invalid cached score")?;
                let replay_sum = score["token_log_score_sum"]
                    .as_f64()
                    .context("Invalid replay score")?;
                // Historical Rust/Python JSON round trips differ by a few ULPs.
                ensure!(
                    original["token_count"] == score["token_count"]
                        && original_sum.is_finite()
                        && replay_sum.is_finite()
                        && (original_sum - replay_sum).abs() <= 1e-9,
                    "Replay score differs from original cache"
                );
            }
            let model = cache
                .get("model")
                .filter(|m| m.as_object().is_some_and(|o| !o.is_empty()))
                .context("Original cached model identity is absent")?;
            serde_json::json!({"method":method,"provenance":model,"source_hashes":null,"domain_adapter":null,"options":null})
        } else {
            let model = response
                .get("provenance")
                .filter(|m| m.as_object().is_some_and(|o| !o.is_empty()))
                .context("Language model identity is absent")?;
            serde_json::json!({"method":method,"provenance":model,"source_hashes":response.get("source_hashes"),
                "domain_adapter":response.get("domain_adapter"),"options":response.get("options")})
        };
        ensure!(
            common.as_ref().is_none_or(|previous| *previous == identity),
            "Language identity differs between batches"
        );
        common = Some(identity);
    }
    let mut identity = common.context("No language provider batches")?;
    let sources: BTreeSet<_> = report
        .contexts
        .values()
        .flat_map(|context| [&context.left_source, &context.right_source])
        .collect();
    identity["context_sources"] = serde_json::to_value(sources)?;
    Ok(identity)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema_version: u32,
    pub id: String,
    pub answer_enabled: bool,
    pub metric: ScoreMetric,
    pub min_score_gap: f64,
    pub min_top_score: f64,
    pub signature: ReportSignature,
    pub require_font_check: bool,
    pub require_link_check: bool,
    pub development_documents: Vec<String>,
    pub development_source_groups: Vec<String>,
    /// Threshold-selection data, objective, report hashes, and model provenance.
    pub provenance: serde_json::Value,
}
impl Policy {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "Unsupported abstention policy version"
        );
        ensure!(!self.id.is_empty(), "Policy ID must not be empty");
        ensure!(
            self.min_top_score.is_finite()
                && self.min_score_gap.is_finite()
                && self.min_score_gap >= 0.,
            "Invalid abstention thresholds"
        );
        ensure!(
            self.signature
                .physical_weight
                .is_none_or(|w| w.is_finite() && w >= 0.),
            "Invalid policy physical weight"
        );
        ensure!(
            !self.signature.physical_interpretation.is_empty(),
            "Policy needs report-generation provenance"
        );
        ensure!(
            self.signature
                .language
                .as_object()
                .is_some_and(|o| !o.is_empty()),
            "Policy needs language-model/scorer identity"
        );
        ensure!(
            !self.development_documents.is_empty() && !self.development_source_groups.is_empty(),
            "Policy must identify development documents and source groups"
        );
        ensure!(
            self.provenance.is_object(),
            "Policy provenance must be an object"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceCheck {
    pub status: CheckStatus,
    /// A user assertion or an independently recorded audit; never inferred here.
    pub source: String,
    pub description: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceChecks {
    pub font_coverage: EvidenceCheck,
    pub link_validity: EvidenceCheck,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Features {
    pub top_candidate: String,
    pub top_score: f64,
    pub score_gap: f64,
    pub eligible_candidates: usize,
}

fn provider_scores(
    report: &ContextReport,
) -> Result<BTreeMap<(String, String), crate::contextual::Score>> {
    let provider = report
        .provider
        .as_ref()
        .context("Provider trace is absent")?;
    let traces: Vec<_> = provider
        .get("batches")
        .and_then(serde_json::Value::as_array)
        .map(|v| v.iter().collect())
        .unwrap_or_else(|| vec![provider]);
    let expected_contexts = serde_json::to_value(&report.contexts)?;
    let mut scores = BTreeMap::new();
    for trace in traces {
        let request = trace.get("request").context("Provider request is absent")?;
        let trace_contexts: BTreeMap<String, crate::contextual::Context> =
            serde_json::from_value(request["contexts"].clone())?;
        ensure!(
            serde_json::to_value(trace_contexts)? == expected_contexts,
            "Report contexts differ from provider request"
        );
        let items = request["items"]
            .as_array()
            .context("Provider request items are absent")?;
        let response: Vec<crate::contextual::Score> =
            serde_json::from_value(trace["response"]["scores"].clone())?;
        ensure!(
            items.len() == response.len(),
            "Provider request/response counts differ"
        );
        let mut item_keys = BTreeSet::new();
        for item in items {
            let id = item["id"].as_str().context("Provider item ID is absent")?;
            let target = item["target"]
                .as_str()
                .context("Provider item target is absent")?;
            ensure!(
                item_keys.insert((id, target)),
                "Duplicate provider request item"
            );
            let physical = report
                .physical
                .candidates
                .iter()
                .find(|c| c.id == id && c.compatible)
                .context("Provider item is not a compatible candidate")?;
            let observed = physical
                .compatible_observations()
                .iter()
                .find(|o| o.target == target)
                .context("Provider item has no physical occurrence")?;
            ensure!(
                item["text"].as_str() == Some(observed.surface.as_str()),
                "Provider text differs from compatible physical surface"
            );
            let matches: Vec<_> = response
                .iter()
                .filter(|s| s.id == id && s.target == target)
                .collect();
            ensure!(
                matches.len() == 1,
                "Provider response item is missing or duplicated"
            );
        }
        for score in response {
            ensure!(
                item_keys.contains(&(score.id.as_str(), score.target.as_str())),
                "Unsolicited provider score"
            );
            ensure!(
                scores
                    .insert((score.id.clone(), score.target.clone()), score)
                    .is_none(),
                "Duplicate provider score across batches"
            );
        }
    }
    Ok(scores)
}

fn same_score(actual: f64, expected: f64) -> bool {
    actual.is_finite() && expected.is_finite() && (actual - expected).abs() <= 1e-8
}

/// Validate a complete ranked report before extracting a top-two score gap.
/// A singleton carries no measured gap and cannot satisfy this policy family.
pub fn features(report: &ContextReport, metric: ScoreMetric) -> Result<Features> {
    ensure!(
        report.schema_version == 1,
        "Unsupported context report version"
    );
    let physical = &report.physical;
    ensure!(
        physical.independent_observations > 0,
        "No independent observations"
    );
    ensure!(
        physical.candidates.len() == physical.tested_candidates,
        "Physical report is incomplete or truncated"
    );
    ensure!(
        physical.compatible_candidates == report.ranked.len(),
        "Context report omits compatible candidates"
    );
    ensure!(
        report.ranked.len() >= 2,
        "At least two eligible candidates are required for a score gap"
    );
    let contexts: BTreeSet<_> = report.contexts.keys().map(String::as_str).collect();
    ensure!(
        contexts.len() == physical.independent_observations
            && report
                .contexts
                .values()
                .all(|c| !c.left.trim().is_empty() || !c.right.trim().is_empty()),
        "Missing occurrence context"
    );
    let mut physical_ids = BTreeSet::new();
    let mut compatible_ids = BTreeSet::new();
    for candidate in &physical.candidates {
        ensure!(
            physical_ids.insert(candidate.id.as_str()) && !candidate.id.is_empty(),
            "Duplicate or empty physical candidate ID"
        );
        ensure!(
            candidate.evidence_score.is_finite(),
            "Nonfinite physical score"
        );
        if candidate.compatible {
            compatible_ids.insert(candidate.id.as_str());
            let observations = candidate.compatible_observations();
            let targets: BTreeSet<_> = observations.iter().map(|o| o.target.as_str()).collect();
            ensure!(
                observations.len() == physical.independent_observations && targets == contexts,
                "Incomplete physical observations"
            );
            ensure!(
                observations.iter().all(|o| o.score.is_finite()
                    && o.standardized_error.is_finite()
                    && o.standardized_error <= 3.),
                "Invalid physical compatibility witness"
            );
        }
    }
    ensure!(
        compatible_ids.len() == physical.compatible_candidates,
        "Physical compatible count is inconsistent"
    );
    let traced_scores = provider_scores(report)?;
    ensure!(
        traced_scores.len() == report.ranked.len() * physical.independent_observations,
        "Provider trace is incomplete"
    );
    let mut ranked_ids = BTreeSet::new();
    let mut scores = Vec::new();
    for rank in &report.ranked {
        ensure!(
            ranked_ids.insert(rank.id.as_str()),
            "Duplicate contextual candidate ID"
        );
        let targets: BTreeSet<_> = rank
            .observations
            .iter()
            .map(|o| o.target.as_str())
            .collect();
        ensure!(
            rank.observations.len() == physical.independent_observations && targets == contexts,
            "Incomplete contextual observations"
        );
        ensure!(
            rank.observations.iter().all(|o| o.id == rank.id
                && o.token_count > 0
                && o.token_log_score_sum.is_finite()
                && o.token_log_score_sum <= 0.),
            "Invalid contextual observations"
        );
        let candidate = physical
            .candidates
            .iter()
            .find(|c| c.id == rank.id && c.compatible)
            .context("Ranked candidate is not physically compatible")?;
        for observation in &rank.observations {
            let traced = traced_scores
                .get(&(rank.id.clone(), observation.target.clone()))
                .context("Ranked observation is absent from provider trace")?;
            ensure!(
                observation.token_count == traced.token_count
                    && same_score(observation.token_log_score_sum, traced.token_log_score_sum),
                "Ranked observation differs from provider response"
            );
        }
        let count = rank.observations.len() as f64;
        let sequence = rank
            .observations
            .iter()
            .map(|o| o.token_log_score_sum / count)
            .sum::<f64>();
        let token = rank
            .observations
            .iter()
            .map(|o| o.token_log_score_sum / o.token_count as f64 / count)
            .sum::<f64>();
        ensure!(
            same_score(rank.mean_token_log_score, token),
            "Token score aggregate is inconsistent"
        );
        if let Some(score) = rank.mean_sequence_log_score {
            ensure!(
                same_score(score, sequence),
                "Sequence score aggregate is inconsistent"
            );
        }
        if let Some(score) = rank.mean_word_log_score {
            let mut expected = 0.;
            for observation in &rank.observations {
                let surface = candidate
                    .compatible_observations()
                    .iter()
                    .find(|o| o.target == observation.target)
                    .context("Missing compatible surface for word score")?;
                let words = surface.surface.split_whitespace().count();
                ensure!(words > 0, "Empty word-normalized surface");
                expected += observation.token_log_score_sum / words as f64 / count;
            }
            ensure!(
                same_score(score, expected),
                "Word score aggregate is inconsistent"
            );
        }
        let physical_cost = candidate.evidence_score / count;
        if let Some(score) = rank.mean_physical_cost {
            ensure!(
                same_score(score, physical_cost),
                "Physical score aggregate is inconsistent"
            );
        }
        if let Some(score) = rank.combined_score {
            let weight = report
                .physical_weight
                .filter(|w| w.is_finite() && *w > 0.)
                .context("Combined score has no positive physical weight")?;
            ensure!(
                rank.mean_sequence_log_score.is_some()
                    && rank.mean_physical_cost.is_some()
                    && same_score(score, sequence - weight * physical_cost),
                "Combined score aggregate is inconsistent"
            );
        }
        match report.normalization.as_deref() {
            Some("sequence_sum_mean_over_occurrences") => ensure!(
                rank.mean_sequence_log_score.is_some(),
                "Missing sequence score"
            ),
            Some("whitespace_word_mean") => {
                ensure!(rank.mean_word_log_score.is_some(), "Missing word score")
            }
            None => {}
            _ => anyhow::bail!("Unknown context score normalization"),
        }
        scores.push((rank.id.as_str(), metric.score(rank)?));
    }
    ensure!(
        ranked_ids == compatible_ids,
        "Context/physical candidate membership differs"
    );
    let grouped_ids: Vec<_> = report
        .ranked_groups
        .iter()
        .flatten()
        .map(String::as_str)
        .collect();
    ensure!(
        grouped_ids.len() == scores.len()
            && grouped_ids.iter().copied().collect::<BTreeSet<_>>() == ranked_ids,
        "Ranking groups are incomplete"
    );
    ensure!(
        report.ranked_groups.first().is_some_and(|g| g.len() == 1),
        "Top rank is tied"
    );
    ensure!(
        scores.windows(2).all(|s| s[0].1 >= s[1].1),
        "Selected metric differs from report ranking order"
    );
    ensure!(
        grouped_ids == scores.iter().map(|s| s.0).collect::<Vec<_>>(),
        "Ranking groups and rows disagree"
    );
    let gap = scores[0].1 - scores[1].1;
    ensure!(
        gap.is_finite() && gap > 1e-9,
        "Top rank is tied or score gap is invalid"
    );
    Ok(Features {
        top_candidate: scores[0].0.into(),
        top_score: scores[0].1,
        score_gap: gap,
        eligible_candidates: scores.len(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Answer,
    InsufficientEvidence,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Assessment {
    pub schema_version: u32,
    pub decision: Decision,
    pub answer_candidate: Option<String>,
    pub reasons: Vec<String>,
    pub features: Option<Features>,
    pub policy: Policy,
    pub policy_sha256: String,
    pub report_sha256: String,
    pub checks: EvidenceChecks,
    pub checks_sha256: Option<String>,
    pub interpretation: String,
}

pub fn assess(
    report: &ContextReport,
    policy: &Policy,
    checks: EvidenceChecks,
) -> Result<Assessment> {
    assess_with_base(report, policy, checks, Path::new("."))
}

fn assess_with_base(
    report: &ContextReport,
    policy: &Policy,
    checks: EvidenceChecks,
    base: &Path,
) -> Result<Assessment> {
    policy.validate()?;
    let mut reasons = Vec::new();
    if !policy.answer_enabled {
        reasons.push("development_constraints_unsatisfied".into());
    }
    let language = match language_signature(report, base) {
        Ok(identity) => identity,
        Err(error) => {
            reasons.push(format!("unusable_language_provenance: {error}"));
            serde_json::Value::Null
        }
    };
    let signature = ReportSignature {
        normalization: report.normalization.clone(),
        physical_weight: report.physical_weight,
        physical_interpretation: report.physical.interpretation.clone(),
        language,
    };
    if !policy.signature.accepts(&signature) {
        reasons.push("report_signature_mismatch".into());
    }
    for (name, check, required) in [
        (
            "font_coverage",
            &checks.font_coverage,
            policy.require_font_check,
        ),
        (
            "link_validity",
            &checks.link_validity,
            policy.require_link_check,
        ),
    ] {
        match check.status {
            CheckStatus::Fail => reasons.push(format!("{name}_failed")),
            CheckStatus::Unknown if required => reasons.push(format!("{name}_unassessed")),
            CheckStatus::Pass if check.source.trim().is_empty() => {
                reasons.push(format!("{name}_check_has_no_source"))
            }
            _ => {}
        }
    }
    let features = match features(report, policy.metric) {
        Ok(f) => {
            if f.top_score < policy.min_top_score {
                reasons.push("top_score_below_policy_threshold".into());
            }
            if f.score_gap < policy.min_score_gap {
                reasons.push("score_gap_below_policy_threshold".into());
            }
            Some(f)
        }
        Err(e) => {
            reasons.push(format!("unusable_report: {e}"));
            None
        }
    };
    let decision = if reasons.is_empty() {
        Decision::Answer
    } else {
        Decision::InsufficientEvidence
    };
    let answer_candidate =
        (decision == Decision::Answer).then(|| features.as_ref().unwrap().top_candidate.clone());
    Ok(Assessment {
        schema_version: 1, decision, answer_candidate, reasons, features,
        policy: policy.clone(), policy_sha256: hash(&serde_json::to_vec(policy)?),
        report_sha256: hash(&serde_json::to_vec(report)?), checks, checks_sha256: None,
        interpretation: "Opt-in empirical answer/abstain decision, separate from ranking. Thresholds are development-set rules, not calibrated probabilities or verified identities. Font/link checks are externally supplied assertions; the assessor does not verify them. Unknown checks remain unknown even if policy permits them.".into(),
    })
}

/// File API preserves exact input hashes, including archived JSON formatting.
pub fn assess_files(
    report_path: &Path,
    policy_path: &Path,
    checks_path: Option<&Path>,
) -> Result<Assessment> {
    let report_bytes = fs::read(report_path)?;
    let policy_bytes = fs::read(policy_path)?;
    let checks_bytes = checks_path.map(fs::read).transpose()?;
    let checks = checks_bytes
        .as_deref()
        .map(serde_json::from_slice)
        .transpose()?
        .unwrap_or_default();
    let mut result = assess_with_base(
        &serde_json::from_slice(&report_bytes)?,
        &serde_json::from_slice(&policy_bytes)?,
        checks,
        report_path.parent().unwrap_or_else(|| Path::new(".")),
    )?;
    result.report_sha256 = hash(&report_bytes);
    result.policy_sha256 = hash(&policy_bytes);
    result.checks_sha256 = checks_bytes.as_deref().map(hash);
    Ok(result)
}
