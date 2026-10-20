// Copyright (C) 2026 Toit contributors.
//! Context-based link hypotheses. Matching passages do not establish shared hidden text.
use crate::{
    project::{Project, Target, hash},
    solver::{self, Candidate, Report},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextWindow {
    pub left: Vec<String>,
    pub right: Vec<String>,
    pub left_source: String,
    pub right_source: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub a: String,
    pub b: String,
    pub left_similarity: f64,
    pub right_similarity: f64,
    pub score: f64,
    pub competing_a: Vec<String>,
    pub competing_b: Vec<String>,
    pub dependence: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct LinkReport {
    pub schema_version: u32,
    pub project_sha256: String,
    pub min_score: f64,
    pub contexts: BTreeMap<String, ContextWindow>,
    pub insufficient_context: Vec<String>,
    pub links: Vec<Link>,
    pub warnings: Vec<String>,
}
fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect()
}
fn truncate(mut words: Vec<String>, left: bool) -> Vec<String> {
    if left && words.len() > 8 {
        words.drain(..words.len() - 8);
    } else {
        words.truncate(8);
    }
    words
}
fn context(project: &Project, t: &Target) -> Result<ContextWindow> {
    let page = project.page(&t.page)?;
    let mut words: Vec<_> = page
        .words
        .iter()
        .filter(|w| {
            let overlap =
                (w.bounds.y + w.bounds.h).min(t.bounds.y + t.bounds.h) - w.bounds.y.max(t.bounds.y);
            w.confidence >= 50.0
                && overlap >= 0.5 * w.bounds.h.min(t.bounds.h)
                && !page
                    .proposed_redactions
                    .iter()
                    .any(|b| b.intersects(w.bounds))
                && !project
                    .targets
                    .iter()
                    .any(|other| other.page == t.page && other.bounds.intersects(w.bounds))
        })
        .collect();
    words.sort_by(|a, b| a.bounds.x.total_cmp(&b.bounds.x));
    let mut left = Vec::new();
    let mut boundary = t.bounds.x;
    for w in words
        .iter()
        .rev()
        .filter(|w| w.bounds.x + w.bounds.w <= t.bounds.x)
    {
        if boundary - (w.bounds.x + w.bounds.w) > 4.0 * t.bounds.h {
            break;
        }
        left.push(w.text.as_str());
        boundary = w.bounds.x;
    }
    left.reverse();
    let mut right = Vec::new();
    boundary = t.bounds.x + t.bounds.w;
    for w in words
        .iter()
        .filter(|w| w.bounds.x >= t.bounds.x + t.bounds.w)
    {
        if w.bounds.x - boundary > 4.0 * t.bounds.h {
            break;
        }
        right.push(w.text.as_str());
        boundary = w.bounds.x + w.bounds.w;
    }
    let supplied_left = !t.left_context.trim().is_empty();
    let supplied_right = !t.right_context.trim().is_empty();
    Ok(ContextWindow {
        left: truncate(
            tokens(&if supplied_left {
                t.left_context.clone()
            } else {
                left.join(" ")
            }),
            true,
        ),
        right: truncate(
            tokens(&if supplied_right {
                t.right_context.clone()
            } else {
                right.join(" ")
            }),
            false,
        ),
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
    })
}
fn word_match(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    let a: Vec<_> = a.chars().collect();
    let b: Vec<_> = b.chars().collect();
    // Short words/numbers are significant anchors: never fuzzy-match them.
    if a.len().min(b.len()) < 4 || a.iter().chain(&b).any(|c| c.is_numeric()) {
        return 0.0;
    }
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut prior = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (row[j] + 1).min(old + 1).min(prior + usize::from(x != y));
            prior = old;
        }
    }
    if row[b.len()] <= if a.len().min(b.len()) >= 8 { 2 } else { 1 } {
        0.75
    } else {
        0.0
    }
}
fn similarity(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut row = vec![0.0_f64; b.len() + 1];
    for x in a {
        let mut prior = 0.0;
        for (j, y) in b.iter().enumerate() {
            let old = row[j + 1];
            row[j + 1] = old.max(row[j]).max(prior + word_match(x, y));
            prior = old;
        }
    }
    2.0 * row[b.len()] / (a.len() + b.len()) as f64
}
fn sufficient(w: &ContextWindow) -> bool {
    w.left.len() >= 3 && w.right.len() >= 3
}
pub fn discover(project: &Project, min_score: f64) -> Result<LinkReport> {
    ensure!(
        min_score.is_finite() && (0.5..=1.0).contains(&min_score),
        "Link threshold must be within 0.5–1"
    );
    // Pairwise search is explicit and bounded. No silently truncated corpus.
    ensure!(
        project.targets.len() <= 2000,
        "Context linking currently supports at most 2,000 targets per project"
    );
    let mut contexts = BTreeMap::new();
    let mut insufficient = vec![];
    for t in &project.targets {
        ensure!(
            !contexts.contains_key(&t.id),
            "Target IDs must be globally unique for linking"
        );
        project.validate_target(t)?;
        let c = context(project, t)?;
        if !sufficient(&c) {
            insufficient.push(t.id.clone());
        }
        contexts.insert(t.id.clone(), c);
    }
    let mut links = vec![];
    for (i, a) in project.targets.iter().enumerate() {
        for b in &project.targets[i + 1..] {
            if a.page == b.page {
                continue;
            }
            let ca = &contexts[&a.id];
            let cb = &contexts[&b.id];
            if !sufficient(ca) || !sufficient(cb) {
                continue;
            }
            let left = similarity(&ca.left, &cb.left);
            let right = similarity(&ca.right, &cb.right);
            // Each side must support a link. One shared boilerplate side is insufficient.
            let score = left.min(right);
            if score < min_score {
                continue;
            }
            let pa = project.page(&a.page)?;
            let pb = project.page(&b.page)?;
            let dependence = if a.dependence_key.is_some() && a.dependence_key == b.dependence_key {
                "explicit_shared_dependence"
            } else if !pa.source_sha256.is_empty()
                && pa.source_sha256 == pb.source_sha256
                && pa.page_number == pb.page_number
            {
                "same_source_page"
            } else if !pa.image_sha256.is_empty() && pa.image_sha256 == pb.image_sha256 {
                "same_page_image"
            } else {
                "unknown"
            };
            links.push(Link {
                a: a.id.clone(),
                b: b.id.clone(),
                left_similarity: left,
                right_similarity: right,
                score,
                competing_a: vec![],
                competing_b: vec![],
                dependence: dependence.into(),
            });
        }
    }
    let pages: BTreeMap<_, _> = project
        .targets
        .iter()
        .map(|t| (t.id.as_str(), t.page.as_str()))
        .collect();
    // Alternatives on the same destination page signal ambiguous passage alignment.
    let mut adjacency: BTreeMap<&str, Vec<(&str, f64)>> = BTreeMap::new();
    for link in &links {
        adjacency
            .entry(&link.a)
            .or_default()
            .push((&link.b, link.score));
        adjacency
            .entry(&link.b)
            .or_default()
            .push((&link.a, link.score));
    }
    let competing: Vec<_> = links
        .iter()
        .map(|l| {
            let alternatives = |from: &str, to: &str| -> Vec<String> {
                adjacency[from]
                    .iter()
                    .filter(|(id, s)| *id != to && pages[id] == pages[to] && *s >= l.score - 0.08)
                    .map(|(id, _)| id.to_string())
                    .collect()
            };
            (alternatives(&l.a, &l.b), alternatives(&l.b, &l.a))
        })
        .collect();
    for (l, (a, b)) in links.iter_mut().zip(competing) {
        l.competing_a = a;
        l.competing_b = b;
    }
    links.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.a.cmp(&b.a))
            .then(a.b.cmp(&b.b))
    });
    Ok(LinkReport{schema_version:1,project_sha256:hash(&serde_json::to_vec(project)?),min_score,
        contexts,insufficient_context:insufficient,links,warnings:vec![
            "Context similarity proposes a repeated passage, not a shared identity or recovered text. Scores are not probabilities.".into(),
            "No transitive merging is performed. Repeated templates can have identical contexts and different concealed values.".into(),
            "Unknown dependence is not proof of independence; rescaled/recompressed duplicates may need explicit dependence keys.".into(),
            "OCR fallback uses only nearby words on the same row; wrapped passages and multi-column layouts may be missed.".into()]})
}
#[derive(Debug, Serialize)]
pub struct LinkScenario {
    pub targets: Vec<String>,
    pub dependence_assumption: String,
    pub result: Option<Report>,
    pub error: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct LinkedSolveReport {
    pub schema_version: u32,
    pub anchor: String,
    pub links: LinkReport,
    pub omitted_link_scenarios: usize,
    pub scenarios: Vec<LinkScenario>,
    pub warnings: Vec<String>,
}
/// Solve each link hypothesis separately, always retaining the unlinked anchor.
/// Unknown dependence is examined both as independent and as completely dependent.
pub fn solve_links(
    project: &Project,
    dir: &Path,
    anchor: &str,
    candidates: &[Candidate],
    min_score: f64,
    max_links: usize,
    limit: usize,
) -> Result<LinkedSolveReport> {
    ensure!(
        (1..=100).contains(&max_links),
        "Evaluate 1–100 link hypotheses"
    );
    let links = discover(project, min_score)?;
    project
        .targets
        .iter()
        .find(|t| t.id == anchor)
        .context("Unknown anchor target")?;
    let relevant: Vec<_> = links
        .links
        .iter()
        .filter(|l| l.a == anchor || l.b == anchor)
        .collect();
    let omitted = relevant.len().saturating_sub(max_links);
    let mut scenarios = vec![];
    let mut evaluate = |ids: Vec<String>, dependent: bool, assumption: &str| {
        let mut copy = project.clone();
        let wanted: BTreeSet<_> = ids.iter().collect();
        copy.targets.retain(|t| wanted.contains(&t.id));
        // Anchor first, so dependent occurrences cannot replace its observation.
        copy.targets.sort_by_key(|t| t.id != anchor);
        for t in &mut copy.targets {
            t.group = "link-scenario".into();
            if dependent {
                t.dependence_key = Some("link-scenario-shared".into());
            }
        }
        let (result, error) = match solver::solve(&copy, dir, "link-scenario", candidates, limit) {
            Ok(r) => (Some(r), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        scenarios.push(LinkScenario {
            targets: ids,
            dependence_assumption: assumption.into(),
            result,
            error,
        });
    };
    evaluate(vec![anchor.into()], false, "anchor_only");
    for link in relevant.into_iter().take(max_links) {
        let other = if link.a == anchor { &link.b } else { &link.a };
        let ids = vec![anchor.into(), other.clone()];
        if link.dependence == "unknown" {
            evaluate(ids.clone(), false, "assumed_independent");
            evaluate(ids, true, "assumed_fully_dependent");
        } else {
            evaluate(ids, true, &link.dependence);
        }
    }
    Ok(LinkedSolveReport{schema_version:1,anchor:anchor.into(),links,omitted_link_scenarios:omitted,scenarios,
        warnings:vec!["Each result is conditional on its link and dependence assumptions. Scenarios are not pooled or ranked against each other; no identity is asserted.".into(),
            "Existing groups are ignored for these pairwise scenarios. The original project is not modified.".into()]})
}
