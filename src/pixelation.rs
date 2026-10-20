// Copyright (C) 2026 Toit contributors.
//! Infer integer square mosaic grids from periodic discontinuities and within-block residuals.
//! This detects block structure, not the identity or contents of hidden text.
use crate::{project::hash, raster::GrayImage};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Grid {
    pub block: usize,
    pub phase_x: usize,
    pub phase_y: usize,
}
impl Grid {
    pub fn interior_mse(self, observed: &GrayImage, rendered: &GrayImage) -> Result<f64> {
        ensure!(
            observed.width == rendered.width && observed.height == rendered.height,
            "Mosaic image dimensions differ"
        );
        ensure!(
            self.block > 0 && self.phase_x < self.block && self.phase_y < self.block,
            "Invalid mosaic grid"
        );
        let left = (self.block - self.phase_x) % self.block;
        let top = (self.block - self.phase_y) % self.block;
        let right = observed
            .width
            .saturating_sub((observed.width + self.phase_x) % self.block);
        let bottom = observed
            .height
            .saturating_sub((observed.height + self.phase_y) % self.block);
        ensure!(
            right > left && bottom > top,
            "Mosaic crop contains no full grid cells"
        );
        let mut sum = 0.0;
        for y in top..bottom {
            for x in left..right {
                let i = y * observed.width + x;
                sum += (observed.pixels[i] as f64 - rendered.pixels[i] as f64).powi(2);
            }
        }
        Ok(sum / ((right - left) * (bottom - top)) as f64)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridHypothesis {
    pub grid: Grid,
    /// Fraction of derivative-profile variance explained by the periodic boundary model.
    pub periodicity_score: f64,
    pub boundary_to_interior_ratio_x: f64,
    pub boundary_to_interior_ratio_y: f64,
    pub active_boundaries_x: usize,
    pub active_boundaries_y: usize,
    pub within_block_rmse: f64,
    pub within_block_variance_fraction: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridAnalysis {
    pub schema_version: u32,
    pub width: usize,
    pub height: usize,
    pub pixel_sha256: String,
    pub status: String,
    pub min_block: usize,
    pub max_block: usize,
    pub hypotheses: Vec<GridHypothesis>,
    pub interpretation: String,
}
struct AxisFit {
    phase: usize,
    gain: f64,
    ratio: f64,
    active: usize,
}

fn best_phase(profile: &[f64], block: usize) -> Option<AxisFit> {
    let n = profile.len().checked_sub(1)?;
    if n < block * 3 {
        return None;
    }
    let total = profile[1..].iter().sum::<f64>();
    let mean = total / n as f64;
    let sst = profile[1..].iter().map(|d| (d - mean).powi(2)).sum::<f64>();
    if sst < 1e-9 {
        return None;
    }
    let mut best = None;
    for phase in 0..block {
        let mut on = 0.0;
        let mut count = 0;
        for (i, &energy) in profile.iter().enumerate().skip(1) {
            if (i + phase) % block == 0 {
                on += energy;
                count += 1;
            }
        }
        if count < 3 || count == n {
            continue;
        }
        let on_mean = on / count as f64;
        let off_mean = (total - on).max(0.0) / (n - count) as f64;
        if on_mean <= off_mean || on_mean < 4.0 {
            continue;
        }
        let gain = (count * (n - count)) as f64 / n as f64 * (on_mean - off_mean).powi(2) / sst;
        let active = profile
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(i, d)| (i + phase) % block == 0 && **d > off_mean * 4.0 + 4.0)
            .count();
        let candidate = AxisFit {
            phase,
            gain,
            ratio: on_mean / off_mean.max(0.25),
            active,
        };
        if best
            .as_ref()
            .is_none_or(|b: &AxisFit| candidate.gain > b.gain)
        {
            best = Some(candidate);
        }
    }
    best
}

pub fn analyze(
    image: &GrayImage,
    min_block: usize,
    max_block: usize,
    keep: usize,
) -> Result<GridAnalysis> {
    ensure!(
        (2..=64).contains(&min_block) && (min_block..=64).contains(&max_block),
        "Automatic block range must be within 2–64 pixels"
    );
    ensure!((1..=8).contains(&keep), "Keep 1–8 grid hypotheses");
    ensure!(
        image.width >= 8 && image.height >= 8,
        "Pixelation crop must be at least 8 by 8 pixels"
    );
    let mut x_energy = vec![0.0; image.width];
    let mut y_energy = vec![0.0; image.height];
    for (y, row) in image.pixels.chunks(image.width).enumerate() {
        for x in 1..image.width {
            x_energy[x] += (row[x] as f64 - row[x - 1] as f64).powi(2) / image.height as f64;
        }
        if y > 0 {
            y_energy[y] = row
                .iter()
                .zip(&image.pixels[(y - 1) * image.width..y * image.width])
                .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
                .sum::<f64>()
                / image.width as f64;
        }
    }
    let mean = image.pixels.iter().map(|&v| v as f64).sum::<f64>() / image.pixels.len() as f64;
    let variance = image
        .pixels
        .iter()
        .map(|&v| (v as f64 - mean).powi(2))
        .sum::<f64>()
        / image.pixels.len() as f64;
    let mut hypotheses = vec![];
    if variance > 4.0 {
        for block in min_block..=max_block {
            let (Some(x), Some(y)) = (best_phase(&x_energy, block), best_phase(&y_energy, block))
            else {
                continue;
            };
            // Require several strong transitions in both axes. A solid black rectangle
            // has only two boundaries and is not evidence of pixelated lettering.
            if x.ratio < 4.0 || y.ratio < 4.0 || x.active < 3 || y.active < 3 {
                continue;
            }
            let mosaic = image.mosaic(block, x.phase, y.phase)?;
            let mse = image.mse(&mosaic)?;
            // Original text and JPEG's block artifacts retain substantial interior
            // variation. A mosaic should be approximately constant inside each cell.
            if mse / variance > 0.12 {
                continue;
            }
            hypotheses.push(GridHypothesis {
                grid: Grid {
                    block,
                    phase_x: x.phase,
                    phase_y: y.phase,
                },
                periodicity_score: (x.gain * y.gain).sqrt(),
                boundary_to_interior_ratio_x: x.ratio,
                boundary_to_interior_ratio_y: y.ratio,
                active_boundaries_x: x.active,
                active_boundaries_y: y.active,
                within_block_rmse: mse.sqrt(),
                within_block_variance_fraction: mse / variance,
            });
        }
    }
    hypotheses.sort_by(|a, b| b.periodicity_score.total_cmp(&a.periodicity_score));
    if let Some(best) = hypotheses.first() {
        let floor = best.periodicity_score * 0.8;
        hypotheses.retain(|h| h.periodicity_score >= floor);
    }
    hypotheses.truncate(keep);
    Ok(GridAnalysis{schema_version:1,width:image.width,height:image.height,pixel_sha256:hash(&image.pixels),
        status:if hypotheses.is_empty(){"no_supported_grid".into()}else if hypotheses.len()==1{"one_supported_grid".into()}else{"ambiguous_grid".into()},
        min_block,max_block,hypotheses,interpretation:"Integer square block structure inferred from pixel discontinuities and within-cell variation. Scores are not probabilities; resampling can also produce blocks. Strong blur, noninteger scaling, and too few visible boundaries may prevent inference.".into()})
}
