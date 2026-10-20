// Copyright (C) 2026 Toit contributors.
//! Experimental shared page-grid inference from disjoint, explicitly supplied regions.
use crate::{
    pixelation::Grid,
    raster::{GrayImage, Rect},
};
use anyhow::{Result, ensure};
use serde::Serialize;

#[derive(Serialize)]
pub struct Hypothesis {
    pub grid: Grid,
    pub periodicity_score: f64,
    pub within_block_variance_fraction: f64,
    pub supporting_regions: Vec<usize>,
}

#[derive(Serialize)]
pub struct Analysis {
    pub schema_version: u32,
    pub hypotheses: Vec<Hypothesis>,
    pub regions: usize,
    pub interpretation: &'static str,
}

struct Region {
    image: GrayImage,
    x: usize,
    y: usize,
    variance: f64,
}

fn phase(observations: &[(usize, f64)], block: usize) -> Option<(usize, f64)> {
    let n = observations.len();
    if n < block * 3 {
        return None;
    }
    let total = observations.iter().map(|o| o.1).sum::<f64>();
    let mean = total / n as f64;
    let variance = observations
        .iter()
        .map(|o| (o.1 - mean).powi(2))
        .sum::<f64>();
    if variance < 1e-9 {
        return None;
    }
    let mut best = None;
    for p in 0..block {
        let mut count = 0;
        let mut on = 0.;
        for &(coordinate, energy) in observations {
            if (coordinate + p) % block == 0 {
                count += 1;
                on += energy;
            }
        }
        if count < 3 || count == n {
            continue;
        }
        let on_mean = on / count as f64;
        let off_mean = ((total - on) / (n - count) as f64).max(0.);
        let active = observations
            .iter()
            .filter(|&&(coordinate, energy)| {
                (coordinate + p) % block == 0 && energy > off_mean * 4. + 4.
            })
            .count();
        if on_mean < 4. || on_mean / off_mean.max(0.25) < 4. || active < 3 {
            continue;
        }
        let gain =
            (count * (n - count)) as f64 / n as f64 * (on_mean - off_mean).powi(2) / variance;
        if best.is_none_or(|(_, old)| gain > old) {
            best = Some((p, gain));
        }
    }
    best
}

pub fn analyze(
    image: &GrayImage,
    bounds: &[Rect],
    min: usize,
    max: usize,
    keep: usize,
) -> Result<Analysis> {
    ensure!(
        (2..=64).contains(&min) && (min..=64).contains(&max),
        "Invalid grid range"
    );
    ensure!((1..=8).contains(&keep), "Keep 1–8 hypotheses");
    ensure!(
        (2..=128).contains(&bounds.len()),
        "Supply 2–128 disjoint regions"
    );
    let mut regions = vec![];
    let mut x_energy = vec![];
    let mut y_energy = vec![];
    for (i, b) in bounds.iter().enumerate() {
        ensure!(b.valid(), "Invalid region");
        for other in &bounds[..i] {
            ensure!(
                b.x.floor() >= (other.x + other.w).ceil()
                    || other.x.floor() >= (b.x + b.w).ceil()
                    || b.y.floor() >= (other.y + other.h).ceil()
                    || other.y.floor() >= (b.y + b.h).ceil(),
                "Overlapping regions duplicate evidence"
            );
        }
        let crop = image.crop(*b)?;
        ensure!(crop.width >= 2 && crop.height >= 2, "Region too small");
        let x0 = b.x.floor() as usize;
        let y0 = b.y.floor() as usize;
        let mean = crop.pixels.iter().map(|&p| p as f64).sum::<f64>() / crop.pixels.len() as f64;
        let variance = crop
            .pixels
            .iter()
            .map(|&p| (p as f64 - mean).powi(2))
            .sum::<f64>()
            / crop.pixels.len() as f64;
        // Blank/opaque patches contribute neither periodicity nor support.
        if variance > 4. {
            for x in 1..crop.width {
                let energy = (0..crop.height)
                    .map(|y| {
                        let at = y * crop.width + x;
                        (crop.pixels[at] as f64 - crop.pixels[at - 1] as f64).powi(2)
                    })
                    .sum::<f64>()
                    / crop.height as f64;
                x_energy.push((x0 + x, energy));
            }
            for y in 1..crop.height {
                let energy = (0..crop.width)
                    .map(|x| {
                        let at = y * crop.width + x;
                        (crop.pixels[at] as f64 - crop.pixels[at - crop.width] as f64).powi(2)
                    })
                    .sum::<f64>()
                    / crop.width as f64;
                y_energy.push((y0 + y, energy));
            }
        }
        regions.push(Region {
            image: crop,
            x: x0,
            y: y0,
            variance,
        });
    }
    let mut hypotheses = vec![];
    for block in min..=max {
        let (Some((px, gx)), Some((py, gy))) = (phase(&x_energy, block), phase(&y_energy, block))
        else {
            continue;
        };
        let mut squared_error = 0.;
        let mut total_variance = 0.;
        let mut supporting_regions = vec![];
        for (i, region) in regions.iter().enumerate() {
            if region.variance <= 4. {
                continue;
            }
            let simulated =
                region
                    .image
                    .mosaic(block, (region.x + px) % block, (region.y + py) % block)?;
            let mse = region.image.mse(&simulated)?;
            let count = region.image.pixels.len() as f64;
            squared_error += mse * count;
            total_variance += region.variance * count;
            if mse / region.variance <= 0.12 {
                supporting_regions.push(i);
            }
        }
        if supporting_regions.len() < 2
            || total_variance <= 0.
            || squared_error / total_variance > 0.12
        {
            continue;
        }
        hypotheses.push(Hypothesis {
            grid: Grid {
                block,
                phase_x: px,
                phase_y: py,
            },
            periodicity_score: (gx * gy).sqrt(),
            within_block_variance_fraction: squared_error / total_variance,
            supporting_regions,
        });
    }
    hypotheses.sort_by(|a, b| b.periodicity_score.total_cmp(&a.periodicity_score));
    if let Some(best) = hypotheses.first() {
        let floor = best.periodicity_score * 0.8;
        hypotheses.retain(|h| h.periodicity_score >= floor);
    }
    hypotheses.truncate(keep);
    Ok(Analysis {
        schema_version: 1,
        hypotheses,
        regions: bounds.len(),
        interpretation: "Experimental common integer square grid across supplied disjoint regions. Phase uses page coordinates. Supporting regions share block structure; neither shared hidden text nor concealment is established. Different grids, noninteger scaling or strong blur can prevent inference.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_disjoint_regions_recover_common_phase() -> Result<()> {
        // Independent piecewise-constant construction; no library mosaic call.
        let mut image = GrayImage::white(220, 230)?;
        for y in 0..image.height {
            for x in 0..image.width {
                let a = (x + 13) / 16;
                let b = (y + 7) / 16;
                image.pixels[y * image.width + x] = ((a * 73 + b * 101 + a * b * 13) % 220) as u8;
            }
        }
        let bounds = (0..6)
            .map(|i| Rect {
                x: 11.,
                y: (9 + 31 * i) as f64,
                w: 180.,
                h: 12.,
            })
            .collect::<Vec<_>>();
        for b in &bounds {
            let single = crate::pixelation::analyze(&image.crop(*b)?, 4, 20, 3)?;
            assert!(!single.hypotheses.iter().any(|h| h.grid.block == 16));
        }
        let shared = analyze(&image, &bounds, 4, 20, 3)?;
        assert_eq!(
            shared.hypotheses[0].grid,
            Grid {
                block: 16,
                phase_x: 13,
                phase_y: 7
            }
        );
        assert_eq!(shared.hypotheses[0].supporting_regions.len(), 6);
        assert!(shared.hypotheses[0].within_block_variance_fraction < 1e-10);
        Ok(())
    }

    #[test]
    fn blank_noise_and_duplicate_evidence() -> Result<()> {
        let mut image = GrayImage::white(200, 160)?;
        let bounds = vec![
            Rect {
                x: 10.,
                y: 10.,
                w: 180.,
                h: 25.,
            },
            Rect {
                x: 10.,
                y: 80.,
                w: 180.,
                h: 25.,
            },
        ];
        assert!(analyze(&image, &bounds, 4, 20, 3)?.hypotheses.is_empty());
        assert!(analyze(&image, &[bounds[0], bounds[0]], 4, 20, 3).is_err());
        let mut random = 8123_u32;
        for p in &mut image.pixels {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            *p = (random >> 24) as u8;
        }
        assert!(analyze(&image, &bounds, 4, 20, 3)?.hypotheses.is_empty());
        Ok(())
    }
}
