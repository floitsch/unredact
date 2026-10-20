// Copyright (C) 2026 Toit contributors.
use crate::{
    calibration::Profile,
    mosaic_cells::{self, Integral},
    native::Font,
    raster::{GrayImage, Rect},
    shared_grid,
    solver::Candidate,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HypothesisScore {
    pub fit_index: usize,
    pub grid: crate::pixelation::Grid,
    pub image_cost: f64,
    pub rmse: f64,
    pub surface: String,
    pub ink_offset: [usize; 2],
    pub render_phase: [f64; 2],
}

#[derive(Deserialize, Serialize)]
pub struct Request {
    pub image: PathBuf,
    pub regions: Vec<Rect>,
    pub target_index: usize,
    pub profile: Profile,
    pub candidates: Vec<Candidate>,
    #[serde(default)]
    pub vertical_scales: Vec<f64>,
    #[serde(default = "canonical")]
    pub surface_kind: String,
    #[serde(default = "default_min_block")]
    pub min_block: usize,
    #[serde(default = "default_max_block")]
    pub max_block: usize,
}

fn default_min_block() -> usize {
    4
}
fn default_max_block() -> usize {
    20
}

#[derive(Serialize)]
struct Score {
    id: String,
    cost: Option<f64>,
    rmse: Option<f64>,
    fit_index: Option<usize>,
    grid_index: Option<usize>,
    surface: Option<String>,
    ink_offset: Option<[usize; 2]>,
    render_phase: Option<[f64; 2]>,
    hypotheses: Vec<HypothesisScore>,
}

fn tight(image: &GrayImage) -> Result<Option<GrayImage>> {
    let mut left = image.width;
    let mut top = image.height;
    let mut right = 0;
    let mut bottom = 0;
    for y in 0..image.height {
        for x in 0..image.width {
            if image.pixels[y * image.width + x] < 255 {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
    }
    if right == 0 {
        return Ok(None);
    }
    // Refuse a potentially clipped renderer result; do not fit truncated strings.
    if left == 0 || top == 0 || right == image.width || bottom == image.height {
        return Ok(None);
    }
    Ok(Some(image.crop(Rect {
        x: left as f64,
        y: top as f64,
        w: (right - left) as f64,
        h: (bottom - top) as f64,
    })?))
}

fn canonical() -> String {
    "canonical".into()
}

fn page_identity(image: &GrayImage) -> Result<String> {
    Ok(crate::project::hash(&serde_json::to_vec(&(
        image.width,
        image.height,
        crate::project::hash(&image.pixels),
    ))?))
}

fn profile_identity(profile: &Profile, fonts: &[Font], scales: &[f64]) -> Result<String> {
    let fits = profile
        .fits
        .iter()
        .zip(fonts)
        .zip(scales)
        .map(|((fit, font), scale)| {
            (
                font.sha256(),
                fit.kern,
                fit.size,
                fit.tracking,
                *scale,
                (fit.validation_rmse - profile.fits[0].validation_rmse).max(0.),
            )
        })
        .collect::<Vec<_>>();
    Ok(crate::project::hash(&serde_json::to_vec(&(
        &profile.id,
        &profile.block,
        fits,
    ))?))
}

pub fn rank(mut request: Request) -> Result<serde_json::Value> {
    if request.vertical_scales.is_empty() {
        request.vertical_scales = vec![1.; request.profile.fits.len()];
    }
    ensure!(
        request
            .vertical_scales
            .iter()
            .all(|s| s.is_finite() && (0.25..=4.).contains(s)),
        "Invalid vertical scale"
    );
    let mut ids = BTreeSet::new();
    ensure!(
        request
            .candidates
            .iter()
            .all(|c| !c.id.is_empty() && ids.insert(c.id.clone()) && !c.surfaces.is_empty()),
        "Candidate IDs must be unique and surfaces nonempty"
    );
    ensure!(
        request.candidates.len() <= 1000 && !request.candidates.is_empty(),
        "Candidate count outside1..1000"
    );
    ensure!(
        request.target_index < request.regions.len(),
        "Invalid target"
    );
    ensure!(
        !request.profile.fits.is_empty() && request.profile.fits.len() <= 32,
        "Invalid profile fit count"
    );
    ensure!(
        request.vertical_scales.len() == request.profile.fits.len(),
        "One vertical scale per fit required"
    );
    let image = GrayImage::read(&request.image)?;
    let analysis = shared_grid::analyze(
        &image,
        &request.regions,
        request.min_block,
        request.max_block,
        3,
    )?;
    let models = analysis
        .hypotheses
        .iter()
        .enumerate()
        .filter(|(_, h)| h.supporting_regions.contains(&request.target_index))
        .map(|(i, h)| {
            Ok((
                i,
                mosaic_cells::prepare(&image, &request.regions, request.target_index, h.grid)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(!models.is_empty(), "No supported shared grid for target");
    let mut fonts = vec![];
    for fit in &request.profile.fits {
        ensure!(
            fit.width_metric == crate::calibration::WidthMetric::Ink
                && fit.measurement_mode == crate::native::MeasurementMode::Design
                && fit.spacing.as_ref().is_none_or(|s| s.adjustment == 0.),
            "Pixelation synthesis requires design-metric ink profiles without learned space adjustments"
        );
        let font = Font::open(&fit.font)?;
        fit.verify_font(&font)?;
        ensure!(
            fit.size.is_finite() && fit.size > 0. && fit.tracking.is_finite(),
            "Invalid font fit"
        );
        fonts.push(font);
    }
    let target_width = models[0].1.width;
    let target_height = models[0].1.height;
    let mut scores = vec![];
    for candidate in &request.candidates {
        let mut hypotheses = vec![
            vec![None::<HypothesisScore>; analysis.hypotheses.len()];
            request.profile.fits.len()
        ];
        let mut best = Score {
            id: candidate.id.clone(),
            cost: None,
            rmse: None,
            fit_index: None,
            grid_index: None,
            surface: None,
            ink_offset: None,
            render_phase: None,
            hypotheses: vec![],
        };
        for (fit_index, (fit, font)) in request.profile.fits.iter().zip(&fonts).enumerate() {
            let vertical_scale = request.vertical_scales[fit_index];
            let prior = (fit.validation_rmse - request.profile.fits[0].validation_rmse).max(0.);
            for surface in &candidate.surfaces {
                if request.surface_kind != "any" && surface.kind != request.surface_kind {
                    continue;
                }
                let Ok(metrics) = font.measure(&surface.text, fit.kern) else {
                    continue;
                };
                let width = (metrics.right - metrics.left) * fit.size
                    + (metrics.glyphs - 1).max(0) as f64 * fit.tracking;
                let height = (metrics.bottom - metrics.top) * fit.size * vertical_scale;
                if !width.is_finite()
                    || !height.is_finite()
                    || width <= 0.
                    || height <= 0.
                    || width > target_width as f64 + 4.
                    || height > target_height as f64 + 4.
                {
                    continue;
                }
                for px in [0., 0.5] {
                    for py in [0., 0.5] {
                        let rendered = font.render_vertical(
                            &surface.text,
                            fit.size,
                            fit.kern,
                            fit.tracking,
                            4. - metrics.left * fit.size + px,
                            4. - metrics.top * fit.size * vertical_scale + py,
                            vertical_scale,
                            width.ceil() as usize + 10,
                            height.ceil() as usize + 10,
                        )?;
                        let Some(ink) = tight(&rendered)? else {
                            continue;
                        };
                        if ink.width > target_width || ink.height > target_height {
                            continue;
                        }
                        let integral = Integral::new(&ink);
                        for (grid_index, model) in &models {
                            for y in 0..=target_height - ink.height {
                                for x in 0..=target_width - ink.width {
                                    let mse = model.mse(&integral, x, y)?;
                                    // Fixed legacy sigma8 and profile penalty; not a posterior.
                                    let cost = 0.5 * mse / 64. + prior;
                                    let image_cost = 0.5 * mse / 64.;
                                    let entry = &mut hypotheses[fit_index][*grid_index];
                                    if entry.as_ref().is_none_or(|old| image_cost < old.image_cost)
                                    {
                                        *entry = Some(HypothesisScore {
                                            fit_index,
                                            grid: analysis.hypotheses[*grid_index].grid,
                                            image_cost,
                                            rmse: mse.sqrt(),
                                            surface: surface.text.clone(),
                                            ink_offset: [x, y],
                                            render_phase: [px, py],
                                        });
                                    }
                                    if best.cost.is_none_or(|old| cost < old) {
                                        best = Score {
                                            id: candidate.id.clone(),
                                            cost: Some(cost),
                                            rmse: Some(mse.sqrt()),
                                            fit_index: Some(fit_index),
                                            grid_index: Some(*grid_index),
                                            surface: Some(surface.text.clone()),
                                            ink_offset: Some([x, y]),
                                            render_phase: Some([px, py]),
                                            hypotheses: vec![],
                                        };
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        best.hypotheses = hypotheses.into_iter().flatten().flatten().collect();
        scores.push(best);
    }
    scores.sort_by(|a, b| {
        a.cost
            .unwrap_or(f64::INFINITY)
            .total_cmp(&b.cost.unwrap_or(f64::INFINITY))
            .then_with(|| a.id.cmp(&b.id))
    });
    let page_key = page_identity(&image)?;
    let profile_key = profile_identity(&request.profile, &fonts, &request.vertical_scales)?;
    let mut sorted_regions = request
        .regions
        .iter()
        .map(|r| [r.x as usize, r.y as usize, r.w as usize, r.h as usize])
        .collect::<Vec<_>>();
    sorted_regions.sort();
    let regions_key = crate::project::hash(&serde_json::to_vec(&sorted_regions)?);
    let target = request.regions[request.target_index];
    let observation_key = crate::project::hash(&serde_json::to_vec(&(
        &page_key,
        [
            target.x as usize,
            target.y as usize,
            target.w as usize,
            target.h as usize,
        ],
    ))?);
    let font_penalties = request
        .profile
        .fits
        .iter()
        .map(|fit| (fit.validation_rmse - request.profile.fits[0].validation_rmse).max(0.))
        .collect::<Vec<_>>();
    Ok(serde_json::json!({
        "schema_version": 1, "rendering_vertical_scales": request.vertical_scales, "grid_analysis": analysis, "cell_models": models, "scores": scores,
        "joint_schema_version":1,"page_key":page_key,"profile_key":profile_key,"profile_id":request.profile.id,"regions_key":regions_key,
        "observation_key":observation_key,"font_penalties":font_penalties,
        "interpretation": "Experimental candidate ranking under page-aligned averages copied inside masks; outside pixels assumed unchanged. All declared masks are excluded from known outside contributions. Free integer positioning of complete glyph images with four render phases, shared grid inferred from pixels. Cached visible-text typography. Fixed sigma8/profile cost; no calibrated compatibility or posterior. Unsupported renderings retained with null cost."
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_shape_is_part_of_duplicate_identity() -> Result<()> {
        let a = GrayImage::white(8, 16)?;
        let b = GrayImage::white(16, 8)?;
        assert_eq!(a.pixels, b.pixels);
        assert_ne!(page_identity(&a)?, page_identity(&b)?);
        assert_eq!(page_identity(&a)?, page_identity(&a.clone())?);
        Ok(())
    }

    #[test]
    fn font_file_alias_does_not_split_shared_profile() -> Result<()> {
        let path = &crate::calibration::default_fonts()[0];
        let font = Font::open(path)?;
        let mut profile: Profile = serde_json::from_value(serde_json::json!({
            "id":"body","page":"p","block":null,"training_texts":[],"validation_texts":[],
            "fits":[{"font":path,"kern":true,"size":40.,"tracking":0.,"ink_bias":0.,"corrections":{},
                "validation_rmse":1.,"validation_max_error":2.,"training_words":12,"validation_words":1}]
        }))?;
        let fonts = vec![font];
        let original = profile_identity(&profile, &fonts, &[0.8])?;
        profile.fits[0].font = "another-path-to-identical-font.ttf".into();
        assert_eq!(original, profile_identity(&profile, &fonts, &[0.8])?);
        profile.fits[0].tracking = 1.;
        assert_ne!(original, profile_identity(&profile, &fonts, &[0.8])?);
        Ok(())
    }
}
