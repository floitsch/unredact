// Copyright (C) 2026 Toit contributors.
use crate::{
    native::Font,
    pixelation::{self, Grid, GridAnalysis},
    project::{Evidence, Project, Target, hash},
    raster::GrayImage,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    time::Duration,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Surface {
    pub text: String,
    #[serde(default = "canonical")]
    pub kind: String,
}
fn canonical() -> String {
    "canonical".into()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    #[serde(default)]
    pub canonical_name: String,
    pub surfaces: Vec<Surface>,
    #[serde(default)]
    pub source_ids: Vec<String>,
    #[serde(default)]
    pub context_score: Option<f64>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Candidates {
    pub candidates: Vec<Candidate>,
}
pub fn read_candidates(path: &Path) -> Result<Vec<Candidate>> {
    let bytes = fs::read(path)?;
    let mut items = if path.extension().is_some_and(|e| e == "json") {
        serde_json::from_slice::<Candidates>(&bytes)?.candidates
    } else {
        String::from_utf8(bytes)?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|text| {
                let text = text.trim().to_string();
                Candidate {
                    id: hash(text.as_bytes())[..16].into(),
                    canonical_name: text.clone(),
                    surfaces: vec![Surface {
                        text,
                        kind: canonical(),
                    }],
                    source_ids: vec![path.display().to_string()],
                    context_score: None,
                }
            })
            .collect()
    };
    validate_candidates(&mut items)?;
    Ok(items)
}
fn validate_candidates(items: &mut [Candidate]) -> Result<()> {
    ensure!(items.len() <= 100_000, "Candidate limit is 100,000");
    let mut seen = BTreeSet::new();
    for c in items.iter_mut() {
        ensure!(
            !c.id.is_empty() && seen.insert(c.id.clone()),
            "Duplicate or empty candidate ID: {}",
            c.id
        );
        ensure!(
            !c.surfaces.is_empty() && c.surfaces.len() <= 100,
            "Each candidate needs 1–100 surface forms"
        );
        for s in &c.surfaces {
            ensure!(
                !s.text.is_empty() && s.text.len() <= 512 && !s.text.chars().any(char::is_control),
                "Invalid candidate surface"
            );
        }
        if c.canonical_name.is_empty() {
            c.canonical_name = c.surfaces[0].text.clone();
        }
        ensure!(
            c.context_score.is_none_or(f64::is_finite),
            "Invalid contextual score"
        );
    }
    Ok(())
}
/// Explicit executable hook, without a shell. Model credentials belong to the provider.
/// Providers get both contexts and all grouped targets, and can supply names or phrases.
pub fn provider(
    executable: &Path,
    args: &[String],
    project: &Project,
    group: &str,
    existing: &[Candidate],
    timeout: Duration,
) -> Result<Vec<Candidate>> {
    let request = serde_json::json!({"schema_version":1,"task":"suggest_candidates","group":group,
        "targets":project.targets.iter().filter(|t|t.group==group).collect::<Vec<_>>(),"existing_candidates":existing});
    let response = crate::provider::call(executable, args, &request, timeout)?;
    let mut reply: Candidates = serde_json::from_value(response)
        .context("Provider must return a JSON object with candidates")?;
    validate_candidates(&mut reply.candidates)?;
    Ok(reply.candidates)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationScore {
    pub target: String,
    pub surface: String,
    pub predicted_width: Option<f64>,
    pub allowed_width: Option<[f64; 2]>,
    pub standardized_error: f64,
    pub image_rmse: Option<f64>,
    pub score: f64,
    pub fit_index: usize,
    #[serde(default)]
    pub mosaic_fit: Option<MosaicFit>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MosaicFit {
    pub grid: Grid,
    pub offset_x: f64,
    pub offset_y: f64,
    pub inferred_grid: bool,
}
struct MosaicData {
    image: GrayImage,
    grids: Vec<Grid>,
    origin_x: f64,
    baseline_y: f64,
    noise_sigma: f64,
    inferred: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ranked {
    pub id: String,
    pub name: String,
    pub evidence_score: f64,
    pub context_score: Option<f64>,
    pub compatible: bool,
    pub observations: Vec<ObservationScore>,
    pub source_ids: Vec<String>,
    pub evidence_rank_min: usize,
    pub evidence_rank_max: usize,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    pub group: String,
    pub status: String,
    pub interpretation: String,
    pub tested_candidates: usize,
    pub independent_observations: usize,
    pub ignored_dependent_targets: Vec<String>,
    pub compatible_candidates: usize,
    pub best_evidence_ties: usize,
    pub candidates: Vec<Ranked>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub pixelation_analysis: BTreeMap<String, GridAnalysis>,
    #[serde(default)]
    pub calibration_audits: BTreeMap<String, Vec<Option<crate::calibration::FitAudit>>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub line_spacing: BTreeMap<String, Vec<crate::line_spacing::Estimate>>,
}
#[derive(Clone, Copy)]
pub(crate) struct WidthContext<'a> {
    pub page: &'a crate::project::Page,
    pub line_spacing: Option<&'a crate::line_spacing::Estimate>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WidthMeasurement {
    pub predicted: f64,
    pub allowed: [f64; 2],
    pub sigma: f64,
}

pub(crate) fn measure_width(
    t: &Target,
    fit: &crate::calibration::Fit,
    font: &Font,
    context: WidthContext<'_>,
    text: &str,
) -> Result<Option<WidthMeasurement>> {
    let page = context.page;
    let (predicted, allowed, sigma) = match &t.evidence {
        Evidence::Opaque {
            padding_min,
            padding_max,
            measurement_sigma,
        }
        | Evidence::OpaqueUniform {
            padding_min,
            padding_max,
            measurement_sigma,
        } => (
            fit.predict(font, text)?,
            [t.bounds.w - padding_max, t.bounds.w - padding_min],
            measurement_sigma.hypot(fit.residual_scale_for_text(text).max(0.5)),
        ),
        Evidence::Width {
            width,
            measurement_sigma,
        } => (
            fit.predict(font, text)?,
            [*width, *width],
            measurement_sigma.hypot(fit.residual_scale_for_text(text).max(0.5)),
        ),
        Evidence::NativeSpan {
            left_anchor,
            right_anchor,
            prefix,
            suffix,
            measurement_sigma_points,
        } => {
            let left = &page.native_chars[*left_anchor];
            let right = &page.native_chars[*right_anchor];
            let scale = page.dpi / 72.0;
            let observed = (right.x1 - left.x1) * scale;
            let span = format!("{prefix}{text}{suffix}{}", right.text);
            // Shape both boundaries, then subtract the right anchor's advance.
            // Validation requires whitespace to prevent a cross-boundary ligature.
            let predicted =
                fit.predict(font, &span)? - fit.predict(font, &right.text.to_string())?;
            (
                predicted,
                [observed, observed],
                (measurement_sigma_points * scale).hypot(fit.residual_scale().max(0.01)),
            )
        }
        Evidence::ScanSpan {
            left_word,
            right_word,
            measurement_sigma,
            local_spacing,
        } => {
            let left = &page.words[*left_word];
            let right = &page.words[*right_word];
            let span = format!("{} {text} {}", left.text, right.text);
            let observed = right.bounds.x + right.bounds.w - left.bounds.x;
            let estimate = if *local_spacing {
                Some(
                    context
                        .line_spacing
                        .context("Missing line-spacing estimate")?,
                )
            } else {
                None
            };
            let spaces = crate::spacing::space_count(&span) as f64;
            let adjustment = estimate.map_or(0.0, |e| e.adjustment) * spaces;
            let local_error = estimate.map_or(0.0, |e| e.scatter) * spaces;
            (
                fit.predict(font, &span)? + adjustment,
                [observed, observed],
                measurement_sigma
                    .hypot(fit.residual_scale_for_text(&span).max(0.5))
                    .hypot(local_error),
            )
        }
        Evidence::Mosaic { .. } | Evidence::MosaicAuto { .. } => return Ok(None),
    };
    Ok(Some(WidthMeasurement {
        predicted,
        allowed,
        sigma,
    }))
}
fn score_one(
    t: &Target,
    c: &Candidate,
    fit: &crate::calibration::Fit,
    font: &Font,
    mosaic: Option<&MosaicData>,
    fit_index: usize,
    context: WidthContext<'_>,
) -> Result<Option<ObservationScore>> {
    let mut best: Option<ObservationScore> = None;
    for s in c
        .surfaces
        .iter()
        .filter(|s| t.surface_kind == "any" || s.kind == t.surface_kind)
    {
        let Ok(prediction) = fit.predict(font, &s.text) else {
            continue;
        };
        let result = if let Some(m) = measure_width(t, fit, font, context, &s.text)? {
            let mut observation = width_score(
                &t.id,
                &s.text,
                m.predicted,
                m.allowed[0],
                m.allowed[1],
                m.sigma,
                fit_index,
            );
            if matches!(t.evidence, Evidence::OpaqueUniform { .. }) {
                observation.score =
                    crate::padding::uniform_cost(m.predicted, m.allowed[0], m.allowed[1], m.sigma)?;
            }
            observation
        } else {
            let mosaic = mosaic.context("Missing mosaic image")?;
            let crop = &mosaic.image;
            let mut error = f64::INFINITY;
            let mut selected = None;
            let outline = font.measure(&s.text, fit.kern)?;
            for dx in [-0.5, 0.0, 0.5] {
                for dy in [-0.5, 0.0, 0.5] {
                    let x = mosaic.origin_x + dx;
                    let y = mosaic.baseline_y + dy;
                    let right = x
                        + outline.right * fit.size
                        + (outline.glyphs - 1).max(0) as f64 * fit.tracking;
                    // The target represents the complete concealed string. Otherwise
                    // arbitrarily long completions can match a cropped prefix perfectly.
                    if x + outline.left * fit.size < -1.0
                        || right > crop.width as f64 + 1.0
                        || y + outline.top * fit.size < -1.0
                        || y + outline.bottom * fit.size > crop.height as f64 + 1.0
                    {
                        continue;
                    }
                    let render = font.render(
                        &s.text,
                        fit.size,
                        fit.kern,
                        fit.tracking,
                        mosaic.origin_x + dx,
                        mosaic.baseline_y + dy,
                        crop.width,
                        crop.height,
                    )?;
                    for grid in &mosaic.grids {
                        let simulated = render.mosaic(grid.block, grid.phase_x, grid.phase_y)?;
                        // Cropping may cut blocks whose average included unseen outside
                        // pixels. Those incomplete border cells carry no faithful match.
                        let e = grid.interior_mse(crop, &simulated)?;
                        if e < error {
                            error = e;
                            selected = Some(MosaicFit {
                                grid: *grid,
                                offset_x: dx,
                                offset_y: dy,
                                inferred_grid: mosaic.inferred,
                            });
                        }
                    }
                }
            }
            if selected.is_none() {
                continue;
            }
            let z = error.sqrt() / mosaic.noise_sigma;
            ObservationScore {
                target: t.id.clone(),
                surface: s.text.clone(),
                predicted_width: Some(prediction),
                allowed_width: None,
                standardized_error: z,
                image_rmse: Some(error.sqrt()),
                score: 0.5 * z * z,
                fit_index,
                mosaic_fit: selected,
            }
        };
        if best.as_ref().is_none_or(|b| result.score < b.score) {
            best = Some(result);
        }
    }
    Ok(best)
}

fn width_score(
    id: &str,
    text: &str,
    prediction: f64,
    low: f64,
    high: f64,
    sigma: f64,
    fit_index: usize,
) -> ObservationScore {
    let error = if prediction < low {
        low - prediction
    } else if prediction > high {
        prediction - high
    } else {
        0.0
    };
    let z = error / sigma;
    ObservationScore {
        target: id.into(),
        surface: text.into(),
        predicted_width: Some(prediction),
        allowed_width: Some([low, high]),
        standardized_error: z,
        image_rmse: None,
        // Gaussian scale normalization prevents a poorly calibrated model from
        // winning merely by inflating sigma. Padding is still a fitted interval,
        // so this is a profile-fit cost, not a marginal likelihood or posterior.
        score: 0.5 * z * z + sigma.ln(),
        fit_index,
        mosaic_fit: None,
    }
}
pub(crate) fn independent_targets<'a>(
    project: &'a Project,
    group: &str,
) -> Result<(Vec<&'a Target>, Vec<String>)> {
    let mut target_ids = BTreeSet::new();
    ensure!(
        project
            .targets
            .iter()
            .filter(|t| t.group == group)
            .all(|t| target_ids.insert(&t.id)),
        "Target IDs must be unique within a group"
    );
    let mut targets = vec![];
    let mut ignored = vec![];
    let mut dependencies = BTreeSet::new();
    let mut exact = BTreeSet::new();
    for t in project.targets.iter().filter(|t| t.group == group) {
        project.validate_target(t)?;
        let p = project.page(&t.page)?;
        let identity = format!(
            "{}:{:.3}:{:.3}:{:.3}:{:.3}",
            p.image_sha256, t.bounds.x, t.bounds.y, t.bounds.w, t.bounds.h
        );
        let is_copy = !exact.insert(identity);
        let is_dependent = t
            .dependence_key
            .as_ref()
            .is_some_and(|k| !dependencies.insert(k.clone()));
        if is_copy || is_dependent {
            ignored.push(t.id.clone());
        } else {
            targets.push(t);
        }
    }
    ensure!(!targets.is_empty(), "No targets in group {group}");
    Ok((targets, ignored))
}
pub fn solve(
    project: &Project,
    dir: &Path,
    group: &str,
    candidates: &[Candidate],
    limit: usize,
) -> Result<Report> {
    let mut candidate_ids = BTreeSet::new();
    ensure!(
        candidates
            .iter()
            .all(|c| !c.id.is_empty() && candidate_ids.insert(&c.id)),
        "Candidate IDs must be nonempty and unique across all sources"
    );
    let (targets, ignored) = independent_targets(project, group)?;
    for t in &targets {
        crate::scan_span::validate_pixels(project.page(&t.page)?, t, dir)?;
    }
    ensure!(!candidates.is_empty(), "No candidates provided");
    let mut by_profile: BTreeMap<&str, Vec<&Target>> = BTreeMap::new();
    for t in &targets {
        by_profile.entry(&t.profile).or_default().push(t);
    }
    let mut crops = BTreeMap::new();
    let mut pixelation_analysis = BTreeMap::new();
    for t in &targets {
        if matches!(
            t.evidence,
            Evidence::Mosaic { .. } | Evidence::MosaicAuto { .. }
        ) {
            let p = project.page(&t.page)?;
            let image = GrayImage::read(&dir.join(&p.image))?.crop(t.bounds)?;
            let data = match t.evidence {
                Evidence::Mosaic {
                    block,
                    phase_x,
                    phase_y,
                    origin_x,
                    baseline_y,
                    noise_sigma,
                } => MosaicData {
                    image,
                    grids: vec![Grid {
                        block,
                        phase_x,
                        phase_y,
                    }],
                    origin_x,
                    baseline_y,
                    noise_sigma,
                    inferred: false,
                },
                Evidence::MosaicAuto {
                    min_block,
                    max_block,
                    origin_x,
                    baseline_y,
                    noise_sigma,
                } => {
                    let analysis = pixelation::analyze(&image, min_block, max_block, 3)?;
                    ensure!(
                        !analysis.hypotheses.is_empty(),
                        "Target {} has no supported pixelation grid; do not treat this as evidence against the candidate vocabulary",
                        t.id
                    );
                    let grids = analysis.hypotheses.iter().map(|h| h.grid).collect();
                    pixelation_analysis.insert(t.id.clone(), analysis);
                    MosaicData {
                        image,
                        grids,
                        origin_x,
                        baseline_y,
                        noise_sigma,
                        inferred: true,
                    }
                }
                _ => unreachable!(),
            };
            crops.insert(t.id.clone(), data);
        }
    }
    let mut fonts = BTreeMap::new();
    for id in by_profile.keys() {
        for fit in &project.profile(id)?.fits {
            if !fonts.contains_key(&fit.font) {
                fonts.insert(fit.font.clone(), Font::open(&fit.font)?);
            }
            fit.verify_font(&fonts[&fit.font])?;
        }
    }
    let line_spacing = crate::line_spacing::prepare(project, &targets, &fonts)?;
    let mut ranked = vec![];
    for candidate in candidates {
        let mut observations = vec![];
        let mut total = 0.0;
        let mut supported = true;
        for (profile_id, occurrences) in &by_profile {
            let profile = project.profile(profile_id)?;
            let mut best: Option<(f64, Vec<ObservationScore>)> = None;
            // One typography hypothesis per profile, shared by all its observations.
            for (i, fit) in profile.fits.iter().enumerate() {
                let mut results = vec![];
                let mut valid = true;
                for t in occurrences {
                    if let Some(s) = score_one(
                        t,
                        candidate,
                        fit,
                        &fonts[&fit.font],
                        crops.get(&t.id),
                        i,
                        WidthContext {
                            page: project.page(&t.page)?,
                            line_spacing: line_spacing.get(&t.id).map(|fits| &fits[i]),
                        },
                    )? {
                        results.push(s);
                    } else {
                        valid = false;
                        break;
                    }
                }
                if !valid {
                    continue;
                }
                let prior = (fit.validation_rmse - profile.fits[0].validation_rmse).max(0.0);
                let score = results.iter().map(|s| s.score).sum::<f64>() + prior;
                if best.as_ref().is_none_or(|(b, _)| score < *b) {
                    best = Some((score, results));
                }
            }
            if let Some((score, mut result)) = best {
                total += score;
                observations.append(&mut result);
            } else {
                supported = false;
                break;
            }
        }
        if supported {
            ranked.push(Ranked {
                id: candidate.id.clone(),
                name: candidate.canonical_name.clone(),
                evidence_score: total,
                context_score: candidate.context_score,
                compatible: observations.iter().all(|s| s.standardized_error <= 3.0),
                observations,
                source_ids: candidate.source_ids.clone(),
                evidence_rank_min: 0,
                evidence_rank_max: 0,
            });
        }
    }
    // Context may break numerical ties, but never overrides contradictory physical evidence.
    ranked.sort_by(|a, b| {
        a.evidence_score
            .total_cmp(&b.evidence_score)
            .then_with(|| {
                b.context_score
                    .unwrap_or(0.0)
                    .total_cmp(&a.context_score.unwrap_or(0.0))
            })
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut start = 0;
    while start < ranked.len() {
        let mut end = start + 1;
        while end < ranked.len()
            && (ranked[end].evidence_score - ranked[start].evidence_score).abs() <= 1e-9
        {
            end += 1;
        }
        for candidate in &mut ranked[start..end] {
            candidate.evidence_rank_min = start + 1;
            candidate.evidence_rank_max = end;
        }
        start = end;
    }
    let best_evidence_ties = ranked.first().map_or(0, |r| r.evidence_rank_max);
    let compatible = ranked.iter().filter(|r| r.compatible).count();
    let status = match compatible {
        0 => "no_compatible_candidate",
        1 => "one_compatible_candidate",
        _ => "ambiguous",
    }
    .into();
    let mut warnings=vec!["Scores are model-fit costs, not probabilities of identity. The true text may be absent from the candidates.".into()];
    let mut calibration_audits = BTreeMap::new();
    for id in by_profile.keys() {
        let profile = project.profile(id)?;
        calibration_audits.insert(
            (*id).to_string(),
            profile.fits.iter().map(|f| f.audit.clone()).collect(),
        );
        if profile.fits.iter().any(|f| f.audit.is_none()) {
            warnings.push(format!("Profile {id} has models without an independent word audit; error scales may be optimistic after model selection."));
        }
        if profile.fits.iter().any(|f| f.font_sha256.is_none()) {
            warnings.push(format!("Profile {id} predates font fingerprints; font-file identity cannot be checked against calibration."));
        }
        if profile
            .fits
            .iter()
            .any(|f| f.spacing.as_ref().is_some_and(|s| s.audit_after.is_none()))
        {
            warnings.push(format!("Profile {id} has spacing models without enough independent audit pairs; phrase error scales rely on model-selection data."));
        }
    }
    if !ignored.is_empty() {
        warnings.push("Dependent or identical observations were counted once. Rescaled/recompressed duplicates need an explicit shared dependence_key.".into());
    }
    if targets.len() > 1 {
        warnings.push("Group membership is an explicit hypothesis that these occurrences refer to the same candidate entity. Shared scan/calibration errors can remain correlated.".into());
    }
    if targets
        .iter()
        .any(|t| matches!(t.evidence, Evidence::ScanSpan { .. }))
    {
        warnings.push(if line_spacing.is_empty() {
            "Scan spans assume correct OCR anchors, uniform typography and one ordinary space on each side of the candidate; justification or mixed styles require a different model."
        } else {
            "Scan spans assume correct OCR anchors, uniform typography and one ordinary space on each side of the candidate. Local spacing assumes equal adjustment per space; mixed styles, tabs and nonuniform justification remain unsupported."
        }.into());
    }
    if targets.iter().any(|t| {
        matches!(
            t.evidence,
            Evidence::Opaque { .. } | Evidence::OpaqueUniform { .. }
        )
    }) {
        warnings.push("Opaque-mask comparisons assume the supplied padding interval; mask edges are not text boundaries.".into());
    }
    if targets
        .iter()
        .any(|t| matches!(t.evidence, Evidence::OpaqueUniform { .. }))
    {
        warnings.push("Opaque-uniform scores integrate Gaussian width error over an explicit uniform total-padding prior. This prior is an assumption, not recovered evidence; typography selection still uses profile-fit costs and scores are not calibrated candidate probabilities.".into());
    }
    if !line_spacing.is_empty() {
        warnings.push("Line-spacing estimates use visible training pairs on the target row; existing profile uncertainty is retained. Missing or implausible estimates fall back to the profile's spacing and record a reason.".into());
    }
    ranked.truncate(limit);
    Ok(Report{group:group.into(),status,interpretation:"Compatibility with measured evidence under the selected models; not a recovered or verified identity.".into(),tested_candidates:candidates.len(),independent_observations:targets.len(),ignored_dependent_targets:ignored,compatible_candidates:compatible,best_evidence_ties,candidates:ranked,warnings,pixelation_analysis,calibration_audits,line_spacing})
}
