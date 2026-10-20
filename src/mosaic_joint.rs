// Copyright (C) 2026 Toit contributors.
//! Combine explicitly linked pixelation observations with shared nuisance models.
use crate::{mosaic_rank::HypothesisScore, pixelation::Grid};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Deserialize, Serialize)]
pub struct CandidateScores {
    pub id: String,
    pub cost: Option<f64>,
    pub hypotheses: Vec<HypothesisScore>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct GridOption {
    pub grid: Grid,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct GridOptions {
    pub hypotheses: Vec<GridOption>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct PixelReport {
    pub joint_schema_version: u32,
    pub page_key: String,
    pub profile_key: String,
    pub profile_id: String,
    pub regions_key: String,
    pub observation_key: String,
    pub font_penalties: Vec<f64>,
    pub grid_analysis: GridOptions,
    pub scores: Vec<CandidateScores>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Occurrence {
    pub id: String,
    #[serde(default)]
    pub dependence_key: Option<String>,
    pub report: PixelReport,
}

#[derive(Deserialize, Serialize)]
pub struct Request {
    pub group: String,
    pub occurrences: Vec<Occurrence>,
}

#[derive(Debug, Serialize)]
pub struct Selected {
    pub occurrence: String,
    pub hypothesis: HypothesisScore,
}

#[derive(Debug, Serialize)]
pub struct Penalty {
    pub page: String,
    pub profile: String,
    pub fit_index: usize,
    pub cost: f64,
}

#[derive(Debug, Serialize)]
pub struct Candidate {
    pub id: String,
    pub cost: Option<f64>,
    pub rank_min: Option<usize>,
    pub rank_max: Option<usize>,
    pub observations: Vec<Selected>,
    pub profile_penalties: Vec<Penalty>,
}

#[derive(Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub group: String,
    pub independent_observations: usize,
    pub ignored_occurrences: Vec<String>,
    pub scores: Vec<Candidate>,
    pub ranked_groups: Vec<Vec<String>>,
    pub interpretation: &'static str,
}

fn grid_key(g: Grid) -> (usize, usize, usize) {
    (g.block, g.phase_x, g.phase_y)
}

fn validate(report: &PixelReport) -> Result<BTreeSet<&str>> {
    ensure!(
        report.joint_schema_version == 1
            && !report.page_key.is_empty()
            && !report.profile_key.is_empty()
            && !report.profile_id.is_empty()
            && !report.regions_key.is_empty()
            && !report.observation_key.is_empty(),
        "Missing joint-score provenance"
    );
    ensure!(
        (1..=32).contains(&report.font_penalties.len())
            && report
                .font_penalties
                .iter()
                .all(|p| p.is_finite() && *p >= 0.),
        "Invalid font penalties"
    );
    let mut grids = BTreeSet::new();
    ensure!(
        (1..=8).contains(&report.grid_analysis.hypotheses.len()),
        "Supply1..8 supported grids"
    );
    for h in &report.grid_analysis.hypotheses {
        ensure!(
            (2..=64).contains(&h.grid.block)
                && h.grid.phase_x < h.grid.block
                && h.grid.phase_y < h.grid.block,
            "Invalid page grid"
        );
        ensure!(grids.insert(grid_key(h.grid)), "Duplicate page grid");
    }
    ensure!(!grids.is_empty(), "No supported page grids");
    let mut ids = BTreeSet::new();
    ensure!(
        (1..=1000).contains(&report.scores.len()),
        "Candidate count outside1..1000"
    );
    for score in &report.scores {
        ensure!(
            !score.id.is_empty() && ids.insert(score.id.as_str()),
            "Empty or duplicate candidate ID"
        );
        let mut seen = BTreeSet::new();
        let mut best = f64::INFINITY;
        for h in &score.hypotheses {
            ensure!(
                h.fit_index < report.font_penalties.len()
                    && grids.contains(&grid_key(h.grid))
                    && h.image_cost.is_finite()
                    && h.image_cost >= 0.
                    && h.rmse.is_finite()
                    && h.rmse >= 0.
                    && !h.surface.is_empty()
                    && h.render_phase.iter().all(|p| p.is_finite()),
                "Invalid candidate hypothesis"
            );
            ensure!(
                seen.insert((h.fit_index, grid_key(h.grid))),
                "Duplicate candidate nuisance hypothesis"
            );
            best = best.min(h.image_cost + report.font_penalties[h.fit_index]);
        }
        match score.cost {
            Some(cost) => ensure!(
                cost.is_finite() && (cost - best).abs() <= 1e-8,
                "Solo cost and nuisance hypotheses disagree"
            ),
            None => ensure!(
                !best.is_finite(),
                "Missing solo cost despite renderable hypothesis"
            ),
        }
    }
    Ok(ids)
}

pub fn rank(request: &Request) -> Result<Report> {
    ensure!(
        !request.group.is_empty() && (1..=128).contains(&request.occurrences.len()),
        "Supply a group and1..128 occurrences"
    );
    let mut names = BTreeSet::new();
    let mut exact = BTreeSet::new();
    let mut dependencies = BTreeSet::new();
    let mut active = vec![];
    let mut ignored = vec![];
    let mut candidate_ids = None;
    for occurrence in &request.occurrences {
        ensure!(
            !occurrence.id.is_empty() && names.insert(&occurrence.id),
            "Occurrence IDs must be nonempty and unique"
        );
        let ids = validate(&occurrence.report)?;
        if let Some(expected) = &candidate_ids {
            ensure!(
                expected == &ids,
                "Linked occurrences need identical candidate IDs; alias surfaces may differ"
            );
        } else {
            candidate_ids = Some(ids);
        }
        let duplicate = !exact.insert(&occurrence.report.observation_key);
        let dependent = if let Some(key) = &occurrence.dependence_key {
            ensure!(!key.is_empty(), "Empty dependence key");
            !dependencies.insert(key)
        } else {
            false
        };
        if duplicate || dependent {
            ignored.push(occurrence.id.clone());
        } else {
            active.push(occurrence);
        }
    }
    let mut pages: BTreeMap<&str, Vec<&Occurrence>> = BTreeMap::new();
    let mut indexed = BTreeMap::new();
    for o in &active {
        pages.entry(&o.report.page_key).or_default().push(*o);
        indexed.insert(
            o.id.as_str(),
            o.report
                .scores
                .iter()
                .map(|c| (c.id.as_str(), c))
                .collect::<BTreeMap<_, _>>(),
        );
    }
    for page in pages.values() {
        ensure!(
            page.iter()
                .all(|o| o.report.regions_key == page[0].report.regions_key),
            "Same-page reports must declare the same complete mask set; rerun them with the union of known masks"
        );
        let mut catalogs = BTreeMap::new();
        let mut profile_versions = BTreeMap::new();
        let mut common_grids = page[0]
            .report
            .grid_analysis
            .hypotheses
            .iter()
            .map(|h| grid_key(h.grid))
            .collect::<BTreeSet<_>>();
        for o in page {
            common_grids.retain(|g| {
                o.report
                    .grid_analysis
                    .hypotheses
                    .iter()
                    .any(|h| grid_key(h.grid) == *g)
            });
            if let Some(previous) =
                profile_versions.insert(&o.report.profile_id, &o.report.profile_key)
            {
                ensure!(
                    previous == &o.report.profile_key,
                    "Same page/profile has different calibration; regenerate reports with one shared profile and rendering settings"
                );
            }
            if let Some(previous) = catalogs.insert(&o.report.profile_key, &o.report.font_penalties)
            {
                ensure!(
                    previous == &o.report.font_penalties,
                    "Shared profile has conflicting font hypotheses"
                );
            }
        }
        ensure!(
            !common_grids.is_empty(),
            "Same-page reports have no common grid hypothesis; this is not evidence against the candidate vocabulary"
        );
    }
    let mut scores = vec![];
    for identity in candidate_ids.unwrap() {
        let mut total = 0.;
        let mut observations = vec![];
        let mut penalties = vec![];
        let mut supported = true;
        for (page_key, page) in &pages {
            let mut options = page[0]
                .report
                .grid_analysis
                .hypotheses
                .iter()
                .map(|h| grid_key(h.grid))
                .collect::<BTreeSet<_>>();
            for o in &page[1..] {
                options.retain(|g| {
                    o.report
                        .grid_analysis
                        .hypotheses
                        .iter()
                        .any(|h| grid_key(h.grid) == *g)
                });
            }
            let mut profiles: BTreeMap<&str, Vec<&Occurrence>> = BTreeMap::new();
            for o in page {
                profiles.entry(&o.report.profile_key).or_default().push(o);
            }
            let mut best_page = None;
            for grid in options {
                let mut page_cost = 0.;
                let mut chosen = vec![];
                let mut page_penalties = vec![];
                let mut valid = true;
                for (profile_key, occurrences) in &profiles {
                    let mut best_fit = None;
                    for (fit, penalty) in occurrences[0].report.font_penalties.iter().enumerate() {
                        let mut cost = *penalty;
                        let mut chosen_fit = vec![];
                        let mut valid_fit = true;
                        for o in occurrences {
                            let h = indexed[o.id.as_str()][identity]
                                .hypotheses
                                .iter()
                                .find(|h| h.fit_index == fit && grid_key(h.grid) == grid);
                            if let Some(h) = h {
                                cost += h.image_cost;
                                ensure!(cost.is_finite(), "Joint cost overflow");
                                chosen_fit.push(Selected {
                                    occurrence: o.id.clone(),
                                    hypothesis: h.clone(),
                                });
                            } else {
                                valid_fit = false;
                                break;
                            }
                        }
                        if valid_fit && best_fit.as_ref().is_none_or(|(old, _, _)| cost < *old) {
                            best_fit = Some((
                                cost,
                                chosen_fit,
                                Penalty {
                                    page: (*page_key).into(),
                                    profile: (*profile_key).into(),
                                    fit_index: fit,
                                    cost: *penalty,
                                },
                            ));
                        }
                    }
                    if let Some((cost, mut selected, penalty)) = best_fit {
                        page_cost += cost;
                        chosen.append(&mut selected);
                        page_penalties.push(penalty);
                    } else {
                        valid = false;
                        break;
                    }
                }
                if valid
                    && best_page
                        .as_ref()
                        .is_none_or(|(old, _, _)| page_cost < *old)
                {
                    best_page = Some((page_cost, chosen, page_penalties));
                }
            }
            if let Some((cost, mut selected, mut selected_penalties)) = best_page {
                total += cost;
                ensure!(total.is_finite(), "Joint cost overflow");
                observations.append(&mut selected);
                penalties.append(&mut selected_penalties);
            } else {
                supported = false;
                break;
            }
        }
        scores.push(Candidate {
            id: identity.into(),
            cost: supported.then_some(total),
            rank_min: None,
            rank_max: None,
            observations: if supported { observations } else { vec![] },
            profile_penalties: if supported { penalties } else { vec![] },
        });
    }
    scores.sort_by(|a, b| {
        a.cost
            .unwrap_or(f64::INFINITY)
            .total_cmp(&b.cost.unwrap_or(f64::INFINITY))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut groups = vec![];
    let mut first = 0;
    while first < scores.len() && scores[first].cost.is_some() {
        let value = scores[first].cost.unwrap();
        let mut last = first + 1;
        while last < scores.len() && scores[last].cost.is_some_and(|c| (c - value).abs() <= 1e-9) {
            last += 1;
        }
        groups.push(scores[first..last].iter().map(|s| s.id.clone()).collect());
        for score in &mut scores[first..last] {
            score.rank_min = Some(first + 1);
            score.rank_max = Some(last);
        }
        first = last;
    }
    Ok(Report {
        schema_version: 1,
        group: request.group.clone(),
        independent_observations: active.len(),
        ignored_occurrences: ignored,
        scores,
        ranked_groups: groups,
        interpretation: "Conditional on the supplied occurrences sharing one canonical identity, with declared alias surfaces allowed. Each page shares a grid and each page/profile shares a font choice; profile penalties count once. Identical or explicitly dependent observations count once. Summed model-fit costs are not text probabilities. Rescaled copies need explicit dependence keys.",
    })
}
