// Copyright (C) 2026 Toit contributors.
use unredact::{
    mosaic_joint::{
        self, CandidateScores, GridOption, GridOptions, Occurrence, PixelReport, Request,
    },
    mosaic_rank::HypothesisScore,
    pixelation::Grid,
};

type HypothesisCost = (usize, usize, f64);
type CandidateCost<'a> = (&'a str, &'a [HypothesisCost]);

fn occurrence(
    id: &str,
    page: &str,
    profile: &str,
    penalties: Vec<f64>,
    costs: &[CandidateCost<'_>],
) -> Occurrence {
    Occurrence {
        id: id.into(),
        dependence_key: None,
        report: PixelReport {
            joint_schema_version: 1,
            page_key: page.into(),
            profile_key: profile.into(),
            profile_id: profile.into(),
            regions_key: "all-masks".into(),
            observation_key: id.into(),
            font_penalties: penalties.clone(),
            grid_analysis: GridOptions {
                hypotheses: [8, 16]
                    .into_iter()
                    .map(|block| GridOption {
                        grid: Grid {
                            block,
                            phase_x: 0,
                            phase_y: 0,
                        },
                    })
                    .collect(),
            },
            scores: costs
                .iter()
                .map(|(name, rows)| {
                    let hypotheses = rows
                        .iter()
                        .map(|&(fit, block, cost)| HypothesisScore {
                            fit_index: fit,
                            grid: Grid {
                                block,
                                phase_x: 0,
                                phase_y: 0,
                            },
                            image_cost: cost,
                            rmse: (128. * cost).sqrt(),
                            surface: (*name).into(),
                            ink_offset: [0, 0],
                            render_phase: [0., 0.],
                        })
                        .collect::<Vec<_>>();
                    let cost = hypotheses
                        .iter()
                        .map(|h| h.image_cost + penalties[h.fit_index])
                        .min_by(f64::total_cmp);
                    CandidateScores {
                        id: (*name).into(),
                        cost,
                        hypotheses,
                    }
                })
                .collect(),
        },
    }
}

fn joint(occurrences: Vec<Occurrence>) -> mosaic_joint::Report {
    mosaic_joint::rank(&Request {
        group: "supplied identity link".into(),
        occurrences,
    })
    .unwrap()
}

#[test]
fn one_font_per_profile_and_penalty_once_change_the_winner() {
    let a = occurrence(
        "a",
        "page",
        "body",
        vec![2., 3.],
        &[
            ("x", &[(0, 8, 0.), (1, 8, 10.)]),
            ("y", &[(0, 8, 4.), (1, 8, 4.)]),
        ],
    );
    let b = occurrence(
        "b",
        "page",
        "body",
        vec![2., 3.],
        &[
            ("x", &[(0, 8, 10.), (1, 8, 0.)]),
            ("y", &[(0, 8, 4.), (1, 8, 4.)]),
        ],
    );
    // Separately minimized costs prefer x, but that requires two different fonts.
    let result = joint(vec![a.clone(), b.clone()]);
    assert_eq!(result.ranked_groups[0], vec!["y"]);
    assert_eq!(result.scores[0].cost, Some(10.));
    assert_eq!(result.scores[0].profile_penalties.len(), 1);
    assert!(
        result.scores[0]
            .observations
            .iter()
            .all(|o| o.hypothesis.fit_index == 0)
    );
    let mut independent = b;
    independent.report.page_key = "another PDF page".into();
    let result = joint(vec![a, independent]);
    assert_eq!(result.ranked_groups[0], vec!["x"]);
    assert_eq!(result.scores[0].cost, Some(5.));
}

#[test]
fn page_grid_is_shared_even_between_different_font_profiles() {
    let a = occurrence(
        "a",
        "page",
        "heading",
        vec![0.],
        &[
            ("x", &[(0, 8, 0.), (0, 16, 20.)]),
            ("y", &[(0, 8, 8.), (0, 16, 8.)]),
        ],
    );
    let b = occurrence(
        "b",
        "page",
        "body",
        vec![0.],
        &[
            ("x", &[(0, 8, 20.), (0, 16, 0.)]),
            ("y", &[(0, 8, 8.), (0, 16, 8.)]),
        ],
    );
    let result = joint(vec![a, b]);
    assert_eq!(result.ranked_groups[0], vec!["y"]);
    assert_eq!(result.scores[0].cost, Some(16.));
    assert!(
        result.scores[0]
            .observations
            .iter()
            .all(|o| o.hypothesis.grid.block == 8)
    );
}

#[test]
fn copies_and_declared_dependencies_do_not_strengthen_evidence() {
    let mut a = occurrence(
        "original",
        "page-a",
        "body",
        vec![2.],
        &[("x", &[(0, 8, 5.)])],
    );
    let mut copy = a.clone();
    copy.id = "same pixels exported twice".into();
    let result = joint(vec![a.clone(), copy]);
    assert_eq!(result.independent_observations, 1);
    assert_eq!(result.ignored_occurrences.len(), 1);
    assert_eq!(result.scores[0].cost, Some(7.));
    a.dependence_key = Some("same underlying scan".into());
    let mut rescaled = occurrence(
        "rescaled",
        "page-b",
        "body",
        vec![2.],
        &[("x", &[(0, 8, 5.)])],
    );
    rescaled.dependence_key = a.dependence_key.clone();
    let c = occurrence(
        "independent",
        "page-c",
        "body",
        vec![2.],
        &[("x", &[(0, 8, 1.)])],
    );
    let result = joint(vec![a, rescaled, c]);
    assert_eq!(result.independent_observations, 2);
    assert_eq!(result.scores[0].cost, Some(10.));
}

#[test]
fn aliases_are_allowed_but_missing_hypotheses_remain_unknown() {
    let mut a = occurrence(
        "a",
        "one PDF",
        "body",
        vec![0.],
        &[("x", &[(0, 8, 0.)]), ("y", &[(0, 8, 1.)])],
    );
    let mut b = occurrence(
        "b",
        "another PDF",
        "body",
        vec![0.],
        &[("x", &[]), ("y", &[(0, 8, 1.)])],
    );
    a.report.scores[1].hypotheses[0].surface = "Amber Vale".into();
    b.report.scores[1].hypotheses[0].surface = "Vale".into();
    let result = joint(vec![a, b]);
    assert_eq!(result.ranked_groups[0], vec!["y"]);
    assert_eq!(result.scores[1].cost, None);
    assert_eq!(result.scores[1].rank_min, None);
    assert_eq!(
        result.scores[0]
            .observations
            .iter()
            .find(|o| o.occurrence == "b")
            .unwrap()
            .hypothesis
            .surface,
        "Vale"
    );
}

#[test]
fn conflicting_masks_catalogues_or_scores_are_rejected() {
    let a = occurrence("a", "page", "body", vec![0.], &[("x", &[(0, 8, 1.)])]);
    for variant in 0..5 {
        let mut b = a.clone();
        b.id = "b".into();
        b.report.observation_key = "different region".into();
        match variant {
            0 => b.report.regions_key = "missing another known mask".into(),
            1 => {
                b.report.font_penalties[0] = 1.;
                b.report.scores[0].cost = Some(2.);
            }
            2 => b.report.scores[0].id = "different candidate ID".into(),
            3 => b.report.scores[0].cost = Some(999.),
            _ => b.report.profile_key = "different calibration of the same body profile".into(),
        }
        assert!(
            mosaic_joint::rank(&Request {
                group: "g".into(),
                occurrences: vec![a.clone(), b]
            })
            .is_err()
        );
    }
}

#[test]
fn incompatible_page_grids_are_configuration_errors() {
    let mut a = occurrence("a", "page", "body", vec![0.], &[("x", &[(0, 8, 1.)])]);
    let mut b = occurrence("b", "page", "body", vec![0.], &[("x", &[(0, 16, 1.)])]);
    a.report
        .grid_analysis
        .hypotheses
        .retain(|h| h.grid.block == 8);
    b.report
        .grid_analysis
        .hypotheses
        .retain(|h| h.grid.block == 16);
    assert!(
        mosaic_joint::rank(&Request {
            group: "g".into(),
            occurrences: vec![a, b]
        })
        .is_err()
    );
}

#[test]
fn ties_and_single_occurrence_costs_are_preserved() {
    let a = occurrence(
        "a",
        "page",
        "body",
        vec![2.],
        &[("x", &[(0, 8, 3.)]), ("y", &[(0, 8, 3.)])],
    );
    let result = joint(vec![a]);
    assert_eq!(result.ranked_groups[0], vec!["x", "y"]);
    for score in result.scores {
        assert_eq!(score.cost, Some(5.));
        assert_eq!(score.rank_min, Some(1));
        assert_eq!(score.rank_max, Some(2));
    }
}
