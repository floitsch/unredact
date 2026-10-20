// Copyright (C) 2026 Toit contributors.
use crate::{
    calibration::{self, Word},
    native::{Font, PdfCanvas},
    project::{Evidence, Page, Project, Target, hash},
    raster::{GrayImage, Rect},
    solver::{self, Candidate, Surface},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

const TRAINING: &str = "The quick brown fox jumps over lazy dogs. Typography measures character spacing while document analysis compares several possible explanations. Reliable calibration requires independent observations and careful validation. Public records contain letters reports invoices meeting summaries schedules notes and correspondence. Every measurement carries uncertainty because printed material changes during copying scanning compression rotation scaling and rendering. Distinct examples improve estimates across ordinary vocabulary including water mountains rivers forests buildings cities countries research technology science education music transport communication language history mathematics physics chemistry biology engineering architecture development production distribution administration information knowledge discovery evidence investigation procedure decision policy system method result review observation reference comparison variation position boundary alignment shape outline contrast brightness resolution transformation illustration paragraph sentence punctuation capital lowercase numerical alphabetical regular italic bold narrow wide proportional fixed variable standard unusual ambiguous unknown repeated unique separate combined original duplicate alternative accurate approximate robust consistent uncertain plausible incompatible impossible sufficient limited additional experimental theoretical practical mechanical optical digital visual spatial temporal statistical empirical physical contextual structural geometric horizontal vertical symmetrical asymmetric conservative systematic random deterministic useful informative meaningful available relevant complete partial detailed simple complex common exceptional generally specifically carefully independently precisely approximately";

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub scope: String,
    pub cases: usize,
    pub dictionary_size: usize,
    pub additive_top1: usize,
    pub calibrated_single_top1: usize,
    pub joint_top1: usize,
    pub additive_unique_top1: usize,
    pub calibrated_single_unique_top1: usize,
    pub joint_unique_top1: usize,
    pub additive_best_group_recall: usize,
    pub calibrated_single_best_group_recall: usize,
    pub joint_best_group_recall: usize,
    pub additive_mean_survivors: f64,
    pub calibrated_single_mean_survivors: f64,
    pub joint_mean_survivors: f64,
    pub joint_truth_recall: usize,
    pub duplicate_invariance: bool,
    pub mosaic_truth_rank: usize,
    pub mosaic_evidence_rank: [usize; 2],
    pub truth_absent_no_compatible: usize,
    pub truth_absent_single_compatible: usize,
    pub wrong_link_no_compatible: usize,
    pub wrong_link_single_compatible: usize,
    pub page_validation_rmse: Vec<f64>,
    pub limitations: Vec<String>,
}
fn render_word(font: &Font, text: &str, size: f64, tracking: f64) -> Result<(GrayImage, Rect)> {
    let m = font.measure(text, true)?;
    let w = (m.advance * size + tracking * text.chars().count() as f64 + 40.0).ceil() as usize;
    let image = font.render(
        text,
        size,
        true,
        tracking,
        15.0,
        size + 10.0,
        w,
        (size * 1.8 + 20.0) as usize,
    )?;
    let ink = image.ink_bounds(160).unwrap();
    Ok((image, ink))
}
pub fn run(dir: &Path, fonts: &[PathBuf]) -> Result<BenchmarkReport> {
    ensure!(
        fonts.len() >= 2,
        "Benchmark needs at least two distinct fonts"
    );
    ensure!(
        !dir.join("project.json").exists(),
        "Use a fresh benchmark directory"
    );
    fs::create_dir_all(dir.join("pages"))?;
    let font_paths: Vec<_> = fonts.iter().take(3).cloned().collect();
    let mut project = Project::default();
    let training: Vec<_> = TRAINING
        .split_whitespace()
        .map(|s| s.trim_matches(|c: char| !c.is_alphabetic()))
        .filter(|s| s.len() >= 3)
        .collect();
    for (i, path) in font_paths.iter().enumerate() {
        let font = Font::open(path)?;
        let size = 27.5 + i as f64 * 2.25;
        let tracking = 0.35 + i as f64 * 0.12;
        let mut image = GrayImage::white(1800, 3100)?;
        let mut words = vec![];
        let (mut x, mut y) = (30, 30);
        for text in &training {
            let (render, ink) = render_word(&font, text, size, tracking)?;
            if x + render.width + 30 > image.width {
                x = 30;
                y += 70;
            }
            image.paste(&render, x, y)?;
            words.push(Word {
                text: text.to_string(),
                bounds: Rect {
                    x: x as f64 + ink.x,
                    y: y as f64 + ink.y,
                    w: ink.w,
                    h: ink.h,
                },
                confidence: 100.0,
                block: "body".into(),
                line: format!("{y}"),
            });
            x += render.width + 12;
        }
        let id = format!("bench-{i}");
        let rel = PathBuf::from(format!("pages/{id}.pgm"));
        image.save(&dir.join(&rel))?;
        let profile = calibration::calibrate(
            format!("profile-{i}"),
            id.clone(),
            Some("body".into()),
            &words,
            fonts,
        )?;
        project.profiles.push(profile);
        project.pages.push(Page {
            id,
            source: path.clone(),
            source_sha256: hash(&fs::read(path)?),
            page_number: 1,
            dpi: 200.0,
            width: image.width,
            height: image.height,
            image: rel,
            image_sha256: hash(&image.pixels),
            likely_scan: true,
            image_coverage: 1.0,
            native_chars: vec![],
            words,
            proposed_redactions: vec![],
        });
    }
    let first = [
        "Amber", "Avery", "Blair", "Casey", "Drew", "Ellis", "Finley", "Harper", "Jamie", "Jordan",
        "Morgan", "Parker", "Quinn", "Reese", "Riley", "Robin", "Rowan", "Sage", "Taylor", "Wren",
    ];
    let last = [
        "Vale", "Alden", "Briar", "Cedar", "Dale", "Elm", "Frost", "Glen", "Hart", "Ives", "Lake",
        "Marsh", "North", "Oak", "Pine", "Reed", "Stone", "Valehart", "West", "Wood",
    ];
    let candidates: Vec<_> = first
        .iter()
        .flat_map(|a| last.iter().map(move |b| format!("{a} {b}")))
        .enumerate()
        .map(|(i, text)| Candidate {
            id: format!("synthetic-{i}"),
            canonical_name: text.clone(),
            surfaces: vec![Surface {
                text,
                kind: "canonical".into(),
            }],
            source_ids: vec!["synthetic benchmark vocabulary; no asserted real identities".into()],
            context_score: None,
        })
        .collect();
    fs::write(
        dir.join("candidates.json"),
        serde_json::to_vec_pretty(&solver::Candidates {
            candidates: candidates.clone(),
        })?,
    )?;
    let indices: Vec<usize> = (0..24).map(|i| (i * 37 + 19) % candidates.len()).collect();
    let mut truths = vec![];
    let (mut additive_top1, mut single_top1, mut joint_top1, mut recall) = (0, 0, 0, 0);
    let (mut additive_survivors, mut single_survivors, mut joint_survivors) = (0, 0, 0);
    let mut invariant = true;
    let (mut additive_unique, mut single_unique, mut joint_unique) = (0, 0, 0);
    let (mut additive_best, mut single_best, mut joint_best) = (0, 0, 0);
    let (mut truth_absent_none, mut truth_absent_single) = (0, 0);
    for (case, &index) in indices.iter().enumerate() {
        let truth = &candidates[index];
        let group = format!("case-{case}");
        let mut widths = vec![];
        for (i, path) in font_paths.iter().enumerate() {
            let font = Font::open(path)?;
            let (_, ink) = render_word(
                &font,
                &truth.canonical_name,
                27.5 + i as f64 * 2.25,
                0.35 + i as f64 * 0.12,
            )?;
            widths.push(ink.w);
            let bounds = Rect {
                x: 30.0,
                y: 1600.0 + case as f64 * 50.0,
                w: ink.w + 4.0,
                h: 40.0,
            };
            let mut page_image = GrayImage::read(&dir.join(&project.pages[i].image))?;
            page_image.fill(bounds, 0)?;
            page_image.save(&dir.join(&project.pages[i].image))?;
            project.pages[i].image_sha256 = hash(&page_image.pixels);
            project.targets.push(Target {
                id: format!("target-{case}-{i}"),
                page: format!("bench-{i}"),
                bounds,
                profile: format!("profile-{i}"),
                group: group.clone(),
                surface_kind: "canonical".into(),
                evidence: Evidence::Opaque {
                    padding_min: 4.0,
                    padding_max: 4.0,
                    measurement_sigma: 0.75,
                },
                left_context: String::new(),
                right_context: String::new(),
                dependence_key: None,
            });
        }
        let joint = solver::solve(&project, dir, &group, &candidates, candidates.len())?;
        joint_top1 += usize::from(joint.candidates.first().is_some_and(|r| r.id == truth.id));
        recall += usize::from(
            joint
                .candidates
                .iter()
                .any(|r| r.id == truth.id && r.compatible),
        );
        joint_survivors += joint.compatible_candidates;
        let joint_truth = joint
            .candidates
            .iter()
            .find(|r| r.id == truth.id)
            .context("Truth unexpectedly unsupported")?;
        joint_unique += usize::from(joint_truth.evidence_rank_max == 1);
        joint_best += usize::from(joint_truth.evidence_rank_min == 1);
        let mut single_project = project.clone();
        single_project
            .targets
            .retain(|t| t.group == group && t.page == "bench-0");
        let single = solver::solve(&single_project, dir, &group, &candidates, candidates.len())?;
        single_top1 += usize::from(single.candidates.first().is_some_and(|r| r.id == truth.id));
        single_survivors += single.compatible_candidates;
        let single_truth = single
            .candidates
            .iter()
            .find(|r| r.id == truth.id)
            .context("Truth unexpectedly unsupported")?;
        single_unique += usize::from(single_truth.evidence_rank_max == 1);
        single_best += usize::from(single_truth.evidence_rank_min == 1);
        let without_truth: Vec<_> = candidates
            .iter()
            .filter(|c| c.id != truth.id)
            .cloned()
            .collect();
        let absent = solver::solve(&project, dir, &group, &without_truth, 1)?;
        truth_absent_none += usize::from(absent.compatible_candidates == 0);
        truth_absent_single += usize::from(absent.compatible_candidates == 1);
        // Explicit ablation, not a claim to reproduce either third-party implementation.
        let base = &project.profiles[0].fits[0];
        let base_font = Font::open(&base.font)?;
        let mut baseline = vec![];
        for c in &candidates {
            let mut width = 0.0;
            for ch in c.canonical_name.chars() {
                width += base_font.measure(&ch.to_string(), false)?.advance * base.size;
            }
            baseline.push(((width - widths[0]).abs(), c.id.clone()));
        }
        baseline.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        additive_top1 += usize::from(baseline[0].1 == truth.id);
        let additive_truth_error = baseline.iter().find(|(_, id)| id == &truth.id).unwrap().0;
        let additive_ties = baseline
            .iter()
            .take_while(|(error, _)| (*error - baseline[0].0).abs() <= 1e-9)
            .count();
        let in_best = (additive_truth_error - baseline[0].0).abs() <= 1e-9;
        additive_best += usize::from(in_best);
        additive_unique += usize::from(in_best && additive_ties == 1);
        additive_survivors += baseline
            .iter()
            .filter(|(d, _)| *d <= 3.0 * base.validation_rmse.max(1.0))
            .count();
        let mut duplicated = project.clone();
        let mut t = duplicated
            .targets
            .iter()
            .find(|t| t.group == group)
            .unwrap()
            .clone();
        t.id.push_str("-copy");
        duplicated.targets.push(t);
        let copy = solver::solve(&duplicated, dir, &group, &candidates, 10)?;
        invariant &=
            copy.independent_observations == joint.independent_observations
                && copy.candidates.iter().zip(&joint.candidates).all(|(a, b)| {
                    a.id == b.id && (a.evidence_score - b.evidence_score).abs() < 1e-9
                });
        truths.push(serde_json::json!({"group":group,"candidate_id":truth.id,"text":truth.canonical_name,"measured_widths":widths,"joint_rank":joint.candidates.iter().position(|r|r.id==truth.id).map(|x|x+1),"single_rank":single.candidates.iter().position(|r|r.id==truth.id).map(|x|x+1),"joint_evidence_rank":[joint_truth.evidence_rank_min,joint_truth.evidence_rank_max],"single_evidence_rank":[single_truth.evidence_rank_min,single_truth.evidence_rank_max],"truth_absent_compatible_candidates":absent.compatible_candidates}));
    }
    let (mut wrong_link_none, mut wrong_link_single) = (0, 0);
    for case in 0..indices.len() {
        let group = format!("case-{case}");
        let next = format!("case-{}", (case + 1) % indices.len());
        let mut incorrect = project.clone();
        let replacement = project
            .targets
            .iter()
            .find(|t| t.group == next && t.page == "bench-1")
            .unwrap();
        let target = incorrect
            .targets
            .iter_mut()
            .find(|t| t.group == group && t.page == "bench-1")
            .unwrap();
        target.bounds = replacement.bounds;
        target.evidence = replacement.evidence.clone();
        let result = solver::solve(&incorrect, dir, &group, &candidates, 1)?;
        wrong_link_none += usize::from(result.compatible_candidates == 0);
        wrong_link_single += usize::from(result.compatible_candidates == 1);
    }
    // Independent pixel comparison path, with a fractional render origin.
    let font = Font::open(&font_paths[0])?;
    let truth = &candidates[143];
    let raw = font.render(
        &truth.canonical_name,
        27.5,
        true,
        0.35,
        8.25,
        34.25,
        300,
        55,
    )?;
    let mosaic = raw.mosaic(5, 0, 0)?;
    let mut page_image = GrayImage::read(&dir.join(&project.pages[0].image))?;
    page_image.paste(&mosaic, 500, 1900)?;
    page_image.save(&dir.join(&project.pages[0].image))?;
    project.pages[0].image_sha256 = hash(&page_image.pixels);
    project.targets.push(Target {
        id: "mosaic-0".into(),
        page: "bench-0".into(),
        bounds: Rect {
            x: 500.0,
            y: 1900.0,
            w: 300.0,
            h: 55.0,
        },
        profile: "profile-0".into(),
        group: "mosaic".into(),
        surface_kind: "canonical".into(),
        evidence: Evidence::Mosaic {
            block: 5,
            phase_x: 0,
            phase_y: 0,
            origin_x: 8.0,
            baseline_y: 34.0,
            noise_sigma: 8.0,
        },
        left_context: String::new(),
        right_context: String::new(),
        dependence_key: None,
    });
    let mosaic_report = solver::solve(&project, dir, "mosaic", &candidates, candidates.len())?;
    let mosaic_rank = mosaic_report
        .candidates
        .iter()
        .position(|c| c.id == truth.id)
        .map_or(0, |i| i + 1);
    let mosaic_truth = mosaic_report
        .candidates
        .iter()
        .find(|c| c.id == truth.id)
        .context("Mosaic truth unsupported")?;
    project.save(dir)?;
    fs::write(dir.join("truth.json"), serde_json::to_vec_pretty(&truths)?)?;
    // A real native PDF for end-to-end import testing, independent of project JSON.
    let canvas = PdfCanvas::new(&dir.join("demo.pdf"), 700.0, 900.0)?;
    let (mut x, mut y) = (25.0, 30.0);
    for text in &training {
        let w = font.measure(text, true)?.advance * 12.0 + 5.0;
        if x + w > 670.0 {
            x = 25.0;
            y += 22.0;
        }
        canvas.text(&font, text, 12.0, true, 0.0, x, y)?;
        x += w;
    }
    canvas.rect(25.0, 800.0, 100.0, 15.0);
    canvas.finish()?;
    let report=BenchmarkReport{scope:"Controlled synthetic rendering benchmark with disjoint calibration and target vocabulary. Additive baseline is an ablation, not an execution of Alex-Gilbert/unredact or Unredactron.".into(),cases:indices.len(),dictionary_size:candidates.len(),additive_top1,calibrated_single_top1:single_top1,joint_top1,additive_unique_top1:additive_unique,calibrated_single_unique_top1:single_unique,joint_unique_top1:joint_unique,additive_best_group_recall:additive_best,calibrated_single_best_group_recall:single_best,joint_best_group_recall:joint_best,additive_mean_survivors:additive_survivors as f64/indices.len() as f64,calibrated_single_mean_survivors:single_survivors as f64/indices.len() as f64,joint_mean_survivors:joint_survivors as f64/indices.len() as f64,joint_truth_recall:recall,duplicate_invariance:invariant,mosaic_truth_rank:mosaic_rank,mosaic_evidence_rank:[mosaic_truth.evidence_rank_min,mosaic_truth.evidence_rank_max],truth_absent_no_compatible:truth_absent_none,truth_absent_single_compatible:truth_absent_single,wrong_link_no_compatible:wrong_link_none,wrong_link_single_compatible:wrong_link_single,page_validation_rmse:project.profiles.iter().map(|p|p.fits[0].validation_rmse).collect(),limitations:vec!["Synthetic pages use Cairo/HarfBuzz with installed fonts; external renderers and real scans are not covered by these numbers.".into(),"Training word labels and bounding boxes come from fixture construction; run import on demo.pdf/pages to test OCR separately.".into(),"Opaque cases use known 4-pixel total padding and correct entity links. Unknown padding needs separate evaluation; wrong-link and missing-truth counts are negative controls, not calibrated error rates.".into(),"No direct third-party comparison has run. Superiority over existing projects is unproven.".into()]};
    fs::write(
        dir.join("benchmark.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}
