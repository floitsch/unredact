// Copyright (C) 2026 Toit contributors.
//! A forward model for page-grid averages copied only into declared mask pixels.
//! Border cells use observed, unchanged pixels outside the mask. Cells touching
//! another mask are excluded because their outside contribution is unknown.
use crate::{
    pixelation::Grid,
    raster::{GrayImage, Rect},
};
use anyhow::{Result, ensure};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Cell {
    pub bounds_in_target: [usize; 4],
    pub full_area: usize,
    pub hidden_area: usize,
    pub outside_darkness: f64,
    pub observed_mean: f64,
}

#[derive(Debug, Serialize)]
pub struct Model {
    pub width: usize,
    pub height: usize,
    pub cells: Vec<Cell>,
    pub excluded_cells: usize,
    pub complete_cells: usize,
}

fn integer_bounds(rect: Rect, image: &GrayImage) -> Result<[usize; 4]> {
    ensure!(
        rect.valid()
            && [rect.x, rect.y, rect.w, rect.h]
                .iter()
                .all(|x| x.fract() == 0.),
        "This mosaic process requires integer mask bounds"
    );
    ensure!(
        rect.x + rect.w <= image.width as f64 && rect.y + rect.h <= image.height as f64,
        "Mask outside image"
    );
    Ok([
        rect.x as usize,
        rect.y as usize,
        (rect.x + rect.w) as usize,
        (rect.y + rect.h) as usize,
    ])
}

fn intersects(a: [usize; 4], b: [usize; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

pub fn prepare(image: &GrayImage, regions: &[Rect], target: usize, grid: Grid) -> Result<Model> {
    ensure!(
        target < regions.len() && regions.len() <= 128,
        "Invalid target region"
    );
    ensure!(
        (2..=64).contains(&grid.block) && grid.phase_x < grid.block && grid.phase_y < grid.block,
        "Invalid shared page grid"
    );
    let boxes = regions
        .iter()
        .map(|r| integer_bounds(*r, image))
        .collect::<Result<Vec<_>>>()?;
    for (i, a) in boxes.iter().enumerate() {
        ensure!(
            !boxes[..i].iter().any(|b| intersects(*a, *b)),
            "Overlapping masks"
        );
    }
    let mask = boxes[target];
    let mut model = Model {
        width: mask[2] - mask[0],
        height: mask[3] - mask[1],
        cells: vec![],
        excluded_cells: 0,
        complete_cells: 0,
    };
    let block = grid.block as isize;
    let first_x =
        ((mask[0] + grid.phase_x) / grid.block * grid.block) as isize - grid.phase_x as isize;
    let first_y =
        ((mask[1] + grid.phase_y) / grid.block * grid.block) as isize - grid.phase_y as isize;
    let mut y = first_y;
    while y < mask[3] as isize {
        let mut x = first_x;
        while x < mask[2] as isize {
            let cell = [
                x.max(0) as usize,
                y.max(0) as usize,
                (x + block).min(image.width as isize) as usize,
                (y + block).min(image.height as isize) as usize,
            ];
            if boxes
                .iter()
                .enumerate()
                .any(|(i, b)| i != target && intersects(cell, *b))
            {
                model.excluded_cells += 1;
                x += block;
                continue;
            }
            let hidden = [
                cell[0].max(mask[0]),
                cell[1].max(mask[1]),
                cell[2].min(mask[2]),
                cell[3].min(mask[3]),
            ];
            let hidden_area = (hidden[2] - hidden[0]) * (hidden[3] - hidden[1]);
            let full_area = (cell[2] - cell[0]) * (cell[3] - cell[1]);
            let mut outside_darkness = 0.;
            let mut observed = 0.;
            for cy in cell[1]..cell[3] {
                for cx in cell[0]..cell[2] {
                    let value = image.pixels[cy * image.width + cx] as f64;
                    if cx >= mask[0] && cx < mask[2] && cy >= mask[1] && cy < mask[3] {
                        observed += value;
                    } else {
                        outside_darkness += 255. - value;
                    }
                }
            }
            model.complete_cells += usize::from(hidden_area == full_area);
            model.cells.push(Cell {
                bounds_in_target: [
                    hidden[0] - mask[0],
                    hidden[1] - mask[1],
                    hidden[2] - mask[0],
                    hidden[3] - mask[1],
                ],
                hidden_area,
                full_area,
                outside_darkness,
                observed_mean: observed / hidden_area as f64,
            });
            x += block;
        }
        y += block;
    }
    ensure!(
        !model.cells.is_empty(),
        "No cells with known outside contribution"
    );
    Ok(model)
}

/// Integral darkness supports translated candidate queries without rerendering.
pub struct Integral {
    pub width: usize,
    pub height: usize,
    sums: Vec<u64>,
}
impl Integral {
    pub fn new(image: &GrayImage) -> Self {
        let stride = image.width + 1;
        let mut sums = vec![0; stride * (image.height + 1)];
        for y in 0..image.height {
            let mut row = 0_u64;
            for x in 0..image.width {
                row += (255 - image.pixels[y * image.width + x]) as u64;
                sums[(y + 1) * stride + x + 1] = sums[y * stride + x + 1] + row;
            }
        }
        Self {
            width: image.width,
            height: image.height,
            sums,
        }
    }
    fn darkness(&self, bounds: [usize; 4], x: usize, y: usize) -> f64 {
        let left = bounds[0].saturating_sub(x).min(self.width);
        let top = bounds[1].saturating_sub(y).min(self.height);
        let right = bounds[2].saturating_sub(x).min(self.width);
        let bottom = bounds[3].saturating_sub(y).min(self.height);
        let stride = self.width + 1;
        (self.sums[bottom * stride + right] + self.sums[top * stride + left]
            - self.sums[top * stride + right]
            - self.sums[bottom * stride + left]) as f64
    }
}

impl Model {
    pub fn mse(&self, rendered: &Integral, x: usize, y: usize) -> Result<f64> {
        ensure!(
            x.checked_add(rendered.width)
                .is_some_and(|r| r <= self.width)
                && y.checked_add(rendered.height)
                    .is_some_and(|b| b <= self.height),
            "Candidate must fit completely inside mask"
        );
        let mut sum = 0.;
        for cell in &self.cells {
            let dark = rendered.darkness(cell.bounds_in_target, x, y) + cell.outside_darkness;
            let predicted = 255. - dark / cell.full_area as f64;
            sum += (predicted - cell.observed_mean).powi(2);
        }
        Ok(sum / self.cells.len() as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nonzero_phase_and_page_edge_cells_match_reference() -> Result<()> {
        let mut original = GrayImage::white(37, 23)?;
        for (i, p) in original.pixels.iter_mut().enumerate() {
            *p = ((i * 73 + i / 37 * 19) % 256) as u8;
        }
        for block in [3, 8, 16] {
            let phase_x = block - 1;
            let phase_y = block / 2;
            for b in [[0, 0, 8, 5], [28, 18, 9, 5], [5, 6, 14, 5]] {
                let mask = Rect {
                    x: b[0] as f64,
                    y: b[1] as f64,
                    w: b[2] as f64,
                    h: b[3] as f64,
                };
                let mut observed = original.clone();
                for y in b[1]..b[1] + b[3] {
                    for x in b[0]..b[0] + b[2] {
                        let x0 = ((x + phase_x) / block * block) as isize - phase_x as isize;
                        let y0 = ((y + phase_y) / block * block) as isize - phase_y as isize;
                        let mut total = 0_u64;
                        let mut count = 0;
                        for cy in y0.max(0)..(y0 + block as isize).min(23) {
                            for cx in x0.max(0)..(x0 + block as isize).min(37) {
                                total += original.pixels[cy as usize * 37 + cx as usize] as u64;
                                count += 1;
                            }
                        }
                        observed.pixels[y * 37 + x] = (total as f64 / count as f64).round() as u8;
                    }
                }
                let model = prepare(
                    &observed,
                    &[mask],
                    0,
                    Grid {
                        block,
                        phase_x,
                        phase_y,
                    },
                )?;
                assert!(model.mse(&Integral::new(&original.crop(mask)?), 0, 0)? <= 0.25);
            }
        }
        Ok(())
    }
    #[test]
    fn partial_cells_match_independent_forward_process() -> Result<()> {
        let mut original = GrayImage::white(37, 23)?;
        for y in 0..23 {
            for x in 0..37 {
                original.pixels[y * 37 + x] = ((x * 73 + y * 89 + 13) % 256) as u8;
            }
        }
        let mask = Rect {
            x: 5.,
            y: 6.,
            w: 14.,
            h: 5.,
        };
        let mut observed = original.clone();
        // Independent reference: average the full page cell for each hidden pixel.
        for y in 6..11 {
            for x in 5..19 {
                let x0 = x / 8 * 8;
                let y0 = y / 8 * 8;
                let mut total = 0_u64;
                for cy in y0..(y0 + 8).min(23) {
                    for cx in x0..(x0 + 8).min(37) {
                        total += original.pixels[cy * 37 + cx] as u64;
                    }
                }
                let area = ((x0 + 8).min(37) - x0) * ((y0 + 8).min(23) - y0);
                observed.pixels[y * 37 + x] = (total as f64 / area as f64).round() as u8;
            }
        }
        let model = prepare(
            &observed,
            &[mask],
            0,
            Grid {
                block: 8,
                phase_x: 0,
                phase_y: 0,
            },
        )?;
        assert_eq!(model.complete_cells, 0);
        let candidate = Integral::new(&original.crop(mask)?);
        assert!(model.mse(&candidate, 0, 0)? <= 0.25);
        let white = Integral::new(&GrayImage::white(14, 5)?);
        assert!(model.mse(&white, 0, 0)? > 20.);
        assert!(model.mse(&candidate, 1, 0).is_err());
        Ok(())
    }

    #[test]
    fn other_masks_are_unknown_and_translations_use_integral_sum() -> Result<()> {
        let image = GrayImage::white(40, 30)?;
        let regions = [
            Rect {
                x: 4.,
                y: 4.,
                w: 20.,
                h: 8.,
            },
            Rect {
                x: 25.,
                y: 4.,
                w: 2.,
                h: 8.,
            },
        ];
        let model = prepare(
            &image,
            &regions,
            0,
            Grid {
                block: 8,
                phase_x: 1,
                phase_y: 2,
            },
        )?;
        assert!(model.excluded_cells > 0);
        let small = GrayImage {
            width: 2,
            height: 2,
            pixels: vec![0, 64, 128, 255],
        };
        let integral = Integral::new(&small);
        assert_eq!(integral.darkness([0, 0, 20, 8], 5, 3), 255. + 191. + 127.);
        assert_eq!(integral.darkness([0, 0, 5, 8], 5, 3), 0.);
        assert_eq!(integral.darkness([6, 3, 7, 5], 5, 3), 191.);
        Ok(())
    }
}
