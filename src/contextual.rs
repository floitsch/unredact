// Copyright (C) 2026 Toit contributors.
//! Language-model ranking is a separate view of physically compatible text.
use crate::{
    calibration::Word,
    project::{Project, Target, hash},
    raster::Rect,
    solver::{self, Candidate, Report},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    pub left: String,
    pub right: String,
    pub left_source: String,
    pub right_source: String,
    pub left_word_indices: Vec<usize>,
    pub right_word_indices: Vec<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layout_notes: Vec<String>,
}

/// Keep punctuation; stop at missing/unreliable text rather than joining across
/// another mask. Same-row context avoids assuming paragraph/column order.
pub fn visible_context(project: &Project, target: &Target) -> Result<Context> {
    let page = project.page(&target.page)?;
    let mut words: Vec<(usize, &Word)> = page
        .words
        .iter()
        .enumerate()
        .filter(|(_, w)| {
            w.bounds.valid()
                && w.bounds.x + w.bounds.w <= page.width as f64
                && w.bounds.y + w.bounds.h <= page.height as f64
                && (w.bounds.y + w.bounds.h).min(target.bounds.y + target.bounds.h)
                    - w.bounds.y.max(target.bounds.y)
                    >= w.bounds.h.min(target.bounds.h) * 0.5
        })
        .collect();
    words.sort_by(|a, b| a.1.bounds.x.total_cmp(&b.1.bounds.x));
    let masks: Vec<_> = page
        .proposed_redactions
        .iter()
        .copied()
        .chain(
            project
                .targets
                .iter()
                .filter(|t| t.page == target.page)
                .map(|t| t.bounds),
        )
        .collect();
    let walk = |left: bool| {
        let mut ordered = words.clone();
        if left {
            ordered.reverse();
        }
        let mut boundary = if left {
            target.bounds.x
        } else {
            target.bounds.x + target.bounds.w
        };
        let mut out = vec![];
        for (index, word) in ordered {
            let b = word.bounds;
            if (left && b.x + b.w > target.bounds.x)
                || (!left && b.x < target.bounds.x + target.bounds.w)
            {
                continue;
            }
            let gap = if left {
                boundary - b.x - b.w
            } else {
                b.x - boundary
            };
            let span = Rect {
                x: if left { b.x } else { boundary },
                y: b.y,
                w: if left {
                    boundary - b.x
                } else {
                    b.x + b.w - boundary
                },
                h: b.h,
            };
            if !word.confidence.is_finite()
                || word.confidence < 50.0
                || gap > 4.0 * target.bounds.h
                || word.text.is_empty()
                || word.text.chars().any(char::is_control)
                || masks.iter().any(|m| m.intersects(span))
            {
                break;
            }
            out.push((index, word.text.clone()));
            boundary = if left { b.x } else { b.x + b.w };
            if out.len() == 12 {
                break;
            }
        }
        if left {
            out.reverse();
        }
        out
    };
    let mut left = walk(true);
    let mut right = walk(false);
    let supplied_left = !target.left_context.trim().is_empty();
    let supplied_right = !target.right_context.trim().is_empty();
    let left_text = if supplied_left {
        left.clear();
        target.left_context.clone()
    } else {
        left.iter()
            .map(|(_, s)| s.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let right_text = if supplied_right {
        right.clear();
        target.right_context.clone()
    } else {
        right
            .iter()
            .map(|(_, s)| s.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    ensure!(
        left_text.len() <= 4096 && right_text.len() <= 4096,
        "Context exceeds 4096 bytes per side"
    );
    Ok(Context {
        left: left_text,
        right: right_text,
        left_source: if supplied_left {
            "supplied"
        } else {
            "ocr_same_row"
        }
        .into(),
        right_source: if supplied_right {
            "supplied"
        } else {
            "ocr_same_row"
        }
        .into(),
        left_word_indices: left.into_iter().map(|(i, _)| i).collect(),
        right_word_indices: right.into_iter().map(|(i, _)| i).collect(),
        layout_notes: vec![],
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Score {
    pub id: String,
    pub target: String,
    pub token_log_score_sum: f64,
    pub token_count: usize,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ContextRank {
    pub id: String,
    pub mean_token_log_score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_word_log_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_sequence_log_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_physical_cost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combined_score: Option<f64>,
    pub observations: Vec<Score>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ContextReport {
    pub schema_version: u32,
    pub interpretation: String,
    pub physical: Report,
    pub contexts: BTreeMap<String, Context>,
    pub ranked_groups: Vec<Vec<String>>,
    pub ranked: Vec<ContextRank>,
    pub provider: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalization: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_weight: Option<f64>,
}
pub struct Config {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub timeout_seconds: u64,
    pub paragraph_context: bool,
    pub word_normalization: bool,
    pub sequence_score: bool,
    pub max_compatible: usize,
    pub batch_items: usize,
    /// Experimental sequence log score minus weighted physical cost per occurrence.
    /// Zero retains contextual-only ordering and the original report schema.
    pub physical_weight: f64,
}

pub fn run(
    project: &Project,
    dir: &Path,
    group: &str,
    candidates: &[Candidate],
    config: &Config,
) -> Result<ContextReport> {
    ensure!(
        config.physical_weight.is_finite() && config.physical_weight >= 0.0,
        "Physical weight must be finite and nonnegative"
    );
    ensure!(
        config.physical_weight == 0.0 || config.sequence_score,
        "A positive physical weight requires sequence scoring"
    );
    ensure!(
        (1..=100_000).contains(&config.max_compatible) && (1..=2048).contains(&config.batch_items),
        "Invalid context candidate or batch limit"
    );
    ensure!(
        !(config.word_normalization && config.sequence_score),
        "Sequence scoring and word normalization are mutually exclusive"
    );
    let physical = solver::solve(project, dir, group, candidates, candidates.len())?;
    let (targets, _) = solver::independent_targets(project, group)?;
    let contexts: BTreeMap<_, _> = targets
        .iter()
        .map(|t| {
            Ok((
                t.id.clone(),
                if config.paragraph_context {
                    crate::context_layout::paragraph_context(project, t)?
                } else {
                    visible_context(project, t)?
                },
            ))
        })
        .collect::<Result<_>>()?;
    let mut report = ContextReport { schema_version: 1,
        interpretation: "Context ranks physically compatible candidates by mean per-token model log score, averaged over independent observations. This is a language-model preference, not a probability of the hidden text. Physical scores, ranks and ambiguity are unchanged; the truth can be absent.".into(),
        physical, contexts, ranked_groups: vec![], ranked: vec![], provider: None, normalization: None,
        physical_weight: (config.physical_weight > 0.0).then_some(config.physical_weight) };
    if config.word_normalization {
        report.normalization = Some("whitespace_word_mean".into());
        report.interpretation = "Context ranks physically compatible candidates by summed model token log score per whitespace-delimited word, averaged over independent observations. This is a language-model preference, not a probability of the hidden text. Physical scores, ranks and ambiguity are unchanged; the truth can be absent.".into();
    }
    if config.sequence_score {
        report.normalization = Some("sequence_sum_mean_over_occurrences".into());
        report.interpretation = "Context ranks physically compatible candidates by summed model log score for the complete replacement, averaged over independent observations without token or word normalization. This is a language-model preference, not a probability of the hidden text. Physical scores, ranks and ambiguity are unchanged; the truth can be absent.".into();
    }
    if config.physical_weight > 0.0 {
        report.interpretation = "Ranks physically compatible candidates by mean complete-replacement model log score minus physical_weight times physical model-fit cost per independent occurrence. This experimental combination is not a probability: physical costs profile over uncertain fonts, surfaces and padding. Original physical and language scores remain separate; the true text can be absent.".into();
    }
    let eligible: Vec<_> = report
        .physical
        .candidates
        .iter()
        .filter(|c| c.compatible)
        .collect();
    if eligible.is_empty() {
        return Ok(report);
    }
    ensure!(
        eligible.len() <= config.max_compatible,
        "Compatible candidate count {} exceeds --max-compatible {}; raise the limit or restrict the vocabulary",
        eligible.len(),
        config.max_compatible
    );
    ensure!(
        report
            .contexts
            .values()
            .all(|c| !c.left.trim().is_empty() || !c.right.trim().is_empty()),
        "Every occurrence needs visible or supplied context"
    );
    ensure!(
        eligible.iter().map(|c| c.observations.len()).sum::<usize>() <= 100_000,
        "Context ranking supports at most 100,000 candidate/occurrence items"
    );
    let items: Vec<_> = eligible
        .iter()
        .flat_map(|c| {
            c.observations
                .iter()
                .map(|o| json!({"id":c.id,"target":o.target,"text":o.surface}))
        })
        .collect();
    let (scores, provider_trace) = crate::context_batches::score(
        &report.contexts,
        &items,
        &hash(&serde_json::to_vec(project)?),
        config,
    )?;
    let expected: BTreeSet<_> = eligible
        .iter()
        .flat_map(|c| {
            c.observations
                .iter()
                .map(|o| (c.id.clone(), o.target.clone()))
        })
        .collect();
    let physical_costs: BTreeMap<_, _> = eligible
        .iter()
        .map(|c| (c.id.clone(), (c.evidence_score, c.observations.len())))
        .collect();
    let word_counts: BTreeMap<_, _> = eligible
        .iter()
        .flat_map(|c| {
            c.observations.iter().map(|o| {
                (
                    (c.id.clone(), o.target.clone()),
                    o.surface.split_whitespace().count(),
                )
            })
        })
        .collect();
    ensure!(
        !config.word_normalization || word_counts.values().all(|&n| n > 0),
        "Word normalization needs nonempty words"
    );
    let mut seen = BTreeSet::new();
    let mut grouped: BTreeMap<String, Vec<Score>> = BTreeMap::new();
    for score in scores {
        let key = (score.id.clone(), score.target.clone());
        ensure!(
            expected.contains(&key) && seen.insert(key),
            "Unknown or duplicate context-score candidate/target"
        );
        ensure!(
            score.token_log_score_sum.is_finite()
                && score.token_log_score_sum <= 0.0
                && (1..=128).contains(&score.token_count),
            "Invalid context token log score"
        );
        grouped.entry(score.id.clone()).or_default().push(score);
    }
    ensure!(
        seen == expected,
        "Context provider omitted candidate/target scores"
    );
    for (id, mut observations) in grouped {
        observations.sort_by(|a, b| a.target.cmp(&b.target));
        let mean = observations
            .iter()
            .map(|s| s.token_log_score_sum / s.token_count as f64 / observations.len() as f64)
            .sum::<f64>();
        ensure!(mean.is_finite(), "Context score aggregation overflow");
        let word_mean = config.word_normalization.then(|| {
            observations
                .iter()
                .map(|s| {
                    s.token_log_score_sum
                        / word_counts[&(s.id.clone(), s.target.clone())] as f64
                        / observations.len() as f64
                })
                .sum::<f64>()
        });
        ensure!(
            word_mean.is_none_or(f64::is_finite),
            "Context word score aggregation overflow"
        );
        let sequence_mean = config.sequence_score.then(|| {
            observations
                .iter()
                .map(|s| s.token_log_score_sum / observations.len() as f64)
                .sum::<f64>()
        });
        ensure!(
            sequence_mean.is_none_or(f64::is_finite),
            "Context sequence score aggregation overflow"
        );
        let (mean_physical_cost, combined_score) = if config.physical_weight > 0.0 {
            let (cost, count) = physical_costs[&id];
            ensure!(
                count == observations.len() && count > 0 && cost.is_finite(),
                "Physical and context occurrence accounting differs"
            );
            let cost = cost / count as f64;
            let combined = sequence_mean.unwrap() - config.physical_weight * cost;
            ensure!(combined.is_finite(), "Combined score overflow");
            (Some(cost), Some(combined))
        } else {
            (None, None)
        };
        report.ranked.push(ContextRank {
            id,
            mean_token_log_score: mean,
            mean_word_log_score: word_mean,
            mean_sequence_log_score: sequence_mean,
            mean_physical_cost,
            combined_score,
            observations,
        });
    }
    let rank_score = |r: &ContextRank| {
        r.combined_score
            .or(r.mean_sequence_log_score)
            .or(r.mean_word_log_score)
            .unwrap_or(r.mean_token_log_score)
    };
    report.ranked.sort_by(|a, b| {
        rank_score(b)
            .total_cmp(&rank_score(a))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut i = 0;
    while i < report.ranked.len() {
        let mut j = i + 1;
        while j < report.ranked.len()
            && (rank_score(&report.ranked[j]) - rank_score(&report.ranked[i])).abs() <= 1e-9
        {
            j += 1;
        }
        report
            .ranked_groups
            .push(report.ranked[i..j].iter().map(|r| r.id.clone()).collect());
        i = j;
    }
    report.provider = Some(provider_trace);
    Ok(report)
}
