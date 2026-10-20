// Copyright (C) 2026 Toit contributors.
use crate::{
    native::{Font, MeasurementMode},
    raster::Rect,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    pub bounds: Rect,
    pub confidence: f64,
    pub block: String,
    pub line: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fit {
    pub font: PathBuf,
    #[serde(default)]
    pub font_sha256: Option<String>,
    pub kern: bool,
    pub size: f64,
    pub tracking: f64,
    pub ink_bias: f64,
    pub corrections: BTreeMap<String, f64>,
    pub validation_rmse: f64,
    pub validation_max_error: f64,
    pub training_words: usize,
    pub validation_words: usize,
    #[serde(default)]
    pub width_metric: WidthMetric,
    #[serde(default)]
    pub measurement_mode: MeasurementMode,
    #[serde(default)]
    pub audit: Option<FitAudit>,
    #[serde(default)]
    pub spacing: Option<crate::spacing::SpacingFit>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitAudit {
    pub words: usize,
    pub rmse: f64,
    pub max_error: f64,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidthMetric {
    #[default]
    Ink,
    Advance,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub page: String,
    pub block: Option<String>,
    pub fits: Vec<Fit>,
    pub training_texts: Vec<String>,
    pub validation_texts: Vec<String>,
    #[serde(default)]
    pub audit_texts: Vec<String>,
    #[serde(default)]
    pub font_search: Option<FontSearch>,
    #[serde(default)]
    pub spacing: Option<crate::spacing::Report>,
    #[serde(default)]
    pub typography_group: Option<crate::typography::SizeGroup>,
    #[serde(default)]
    pub neural_search: Option<crate::neural::Report>,
}
fn features(text: &str) -> BTreeMap<String, f64> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = BTreeMap::new();
    for c in &chars {
        *out.entry(format!("c:{c}")).or_default() += 1.0;
    }
    for p in chars.windows(2) {
        *out.entry(format!("p:{}{}", p[0], p[1])).or_default() += 1.0;
    }
    out
}
pub fn held_out(text: &str) -> bool {
    partition(text) == 0 || partition(text) == 5
}
pub fn audit_held_out(text: &str) -> bool {
    partition(text) == 5
}
fn partition(text: &str) -> u32 {
    text.bytes()
        .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619))
        % 10
}
#[allow(
    clippy::needless_range_loop,
    reason = "Explicit row/column indices make the pivoting algorithm auditable"
)]
fn linear_solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for k in 0..n {
        let pivot = (k..n).max_by(|&i, &j| a[i][k].abs().total_cmp(&a[j][k].abs()))?;
        if a[pivot][k].abs() < 1e-10 {
            return None;
        }
        a.swap(k, pivot);
        b.swap(k, pivot);
        let div = a[k][k];
        for v in &mut a[k][k..] {
            *v /= div;
        }
        b[k] /= div;
        for i in 0..n {
            if i == k {
                continue;
            }
            let mul = a[i][k];
            for j in k..n {
                a[i][j] -= mul * a[k][j];
            }
            b[i] -= mul * b[k];
        }
    }
    b.iter().all(|v| v.is_finite()).then_some(b)
}
fn normal_equations(
    x: &[Vec<f64>],
    y: &[f64],
    ridge: f64,
    weights: &[f64],
) -> Option<(Vec<Vec<f64>>, Vec<f64>)> {
    let n = x.first()?.len();
    let mut a = vec![vec![0.0; n]; n];
    let mut b = vec![0.0; n];
    for (i, row) in a.iter_mut().enumerate() {
        row[i] = ridge;
    }
    for ((row, y), w) in x.iter().zip(y).zip(weights) {
        for i in 0..n {
            b[i] += row[i] * y * w;
            for j in 0..n {
                a[i][j] += row[i] * row[j] * w;
            }
        }
    }
    Some((a, b))
}
fn regression(x: &[Vec<f64>], y: &[f64], ridge: f64, weights: &[f64]) -> Option<Vec<f64>> {
    let (a, b) = normal_equations(x, y, ridge, weights)?;
    linear_solve(a, b)
}

fn typography_regression(x: &[Vec<f64>], y: &[f64], weights: &[f64]) -> Option<Vec<f64>> {
    const RIDGE: f64 = 0.001;
    // C * [size, tracking, ink_bias] <= d. These are the existing physical
    // limits, now enforced while fitting rather than after rejecting the fit.
    const BOUNDS: [([f64; 3], f64); 6] = [
        ([-1.0, 0.0, 0.0], -2.0),
        ([1.0, 0.0, 0.0], 1000.0),
        ([-0.25, 1.0, 0.0], 0.0),
        ([-0.25, -1.0, 0.0], 0.0),
        ([-0.5, 0.0, 1.0], 0.0),
        ([-0.5, 0.0, -1.0], 0.0),
    ];
    let feasible = |c: &[f64], tolerance: f64| {
        c.len() == 3
            && c.iter().all(|v| v.is_finite())
            && BOUNDS.iter().all(|(row, limit)| {
                row.iter().zip(c).map(|(a, b)| a * b).sum::<f64>() <= limit + tolerance
            })
    };
    let (a, b) = normal_equations(x, y, RIDGE, weights)?;
    if a.len() != 3 {
        return None;
    }
    let unconstrained = linear_solve(a.clone(), b.clone())?;
    if feasible(&unconstrained, 0.0) {
        return Some(unconstrained);
    }
    // Positive ridge makes this a strictly convex quadratic. Its constrained
    // optimum is interior or on a face with at most three independent active
    // boundaries. Enumerate those small equality-constrained problems; retain
    // the feasible solution with the lowest actual weighted residual objective.
    let mut best: Option<(f64, Vec<f64>)> = None;
    for mask in 1u32..(1 << BOUNDS.len()) {
        let count = mask.count_ones() as usize;
        if count > 3 {
            continue;
        }
        let active: Vec<_> = BOUNDS
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, bound)| bound)
            .collect();
        let mut system = vec![vec![0.0; 3 + count]; 3 + count];
        let mut rhs = vec![0.0; 3 + count];
        for i in 0..3 {
            system[i][..3].copy_from_slice(&a[i]);
            rhs[i] = b[i];
        }
        for (i, (row, limit)) in active.into_iter().enumerate() {
            for j in 0..3 {
                system[j][3 + i] = row[j];
                system[3 + i][j] = row[j];
            }
            rhs[3 + i] = *limit;
        }
        let Some(solution) = linear_solve(system, rhs) else {
            continue;
        };
        let mut c = solution[..3].to_vec();
        if !feasible(&c, 1e-7) {
            continue;
        }
        // Remove only boundary roundoff, so serialized fits obey exact bounds.
        c[0] = c[0].clamp(2.0, 1000.0);
        c[1] = c[1].clamp(-0.25 * c[0], 0.25 * c[0]);
        c[2] = c[2].clamp(-0.5 * c[0], 0.5 * c[0]);
        let objective = x
            .iter()
            .zip(y)
            .zip(weights)
            .map(|((row, y), w)| {
                w * (row.iter().zip(&c).map(|(a, b)| a * b).sum::<f64>() - y).powi(2)
            })
            .sum::<f64>()
            + RIDGE * c.iter().map(|v| v * v).sum::<f64>();
        if objective.is_finite() && best.as_ref().is_none_or(|(cost, _)| objective < *cost) {
            best = Some((objective, c));
        }
    }
    best.map(|(_, coefficients)| coefficients)
}

#[cfg(test)]
mod regression_tests {
    use super::{regression, typography_regression};

    #[test]
    fn bounded_fit_matches_analytic_optima_on_faces_edges_and_vertices() {
        let x = vec![
            vec![1.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.0, 0.0, 1.0],
        ];
        let ridge = 1.001;
        let s = 17.5 / (1.3125 * ridge);
        for (y, expected) in [
            ([0.0, 0.0, 0.0], [2.0, 0.0, 0.0]),
            ([1500.0, 0.0, 0.0], [1000.0, 0.0, 0.0]),
            (
                [10.0, 10.0, 0.0],
                [12.5 / (1.0625 * ridge), 3.125 / (1.0625 * ridge), 0.0],
            ),
            (
                [10.0, -10.0, 0.0],
                [12.5 / (1.0625 * ridge), -3.125 / (1.0625 * ridge), 0.0],
            ),
            ([10.0, 0.0, 10.0], [12.0 / ridge, 0.0, 6.0 / ridge]),
            ([10.0, 0.0, -10.0], [12.0 / ridge, 0.0, -6.0 / ridge]),
            ([10.0, 10.0, 10.0], [s, 0.25 * s, 0.5 * s]),
            ([2000.0, 1000.0, 1000.0], [1000.0, 250.0, 500.0]),
        ] {
            let actual = typography_regression(&x, &y, &[1.0; 3]).unwrap();
            for (a, b) in actual.iter().zip(expected) {
                assert!((a - b).abs() < 1e-8, "{y:?}: {actual:?} != {expected:?}");
            }
        }
    }

    #[test]
    fn feasible_unconstrained_fit_is_preserved_exactly() {
        let x = vec![
            vec![2.0, 3.0, 1.0],
            vec![4.0, 4.0, 1.0],
            vec![6.0, 8.0, 1.0],
            vec![5.0, 9.0, 1.0],
        ];
        let y: Vec<_> = x.iter().map(|r| 24.0 * r[0] + 0.8 * r[1] + 0.4).collect();
        let weights = [1.0, 0.5, 1.0, 0.1];
        assert_eq!(
            typography_regression(&x, &y, &weights),
            regression(&x, &y, 0.001, &weights)
        );
    }

    #[test]
    fn collinear_fixed_pitch_measurements_predict_unseen_lengths() {
        let x: Vec<_> = (3..14)
            .map(|n| vec![0.6 * n as f64, (n - 1) as f64, 1.0])
            .collect();
        let y: Vec<_> = x.iter().map(|r| 24.0 * r[0] + 0.3 * r[1] + 0.5).collect();
        let weights = vec![1.0; x.len()];
        let old = regression(&x, &y, 0.001, &weights).unwrap();
        assert!(old[1].abs() > old[0] * 0.25 || old[2].abs() > old[0] * 0.5);
        let c = typography_regression(&x, &y, &weights).unwrap();
        assert!(
            (2.0..=1000.0).contains(&c[0]) && c[1].abs() <= c[0] * 0.25 && c[2].abs() <= c[0] * 0.5
        );
        // Individual coefficients are not identifiable; held-out widths are.
        for n in [2.0, 15.0, 21.0] {
            let predicted = 0.6 * n * c[0] + (n - 1.0) * c[1] + c[2];
            let expected = 0.6 * n * 24.0 + (n - 1.0) * 0.3 + 0.5;
            assert!((predicted - expected).abs() < 0.01);
        }
    }
}
impl Fit {
    pub fn verify_font(&self, font: &Font) -> Result<()> {
        ensure!(
            self.font_sha256
                .as_deref()
                .is_none_or(|h| h == font.sha256()),
            "Font hash differs from calibration: {}; restore the original font or recalibrate",
            self.font.display()
        );
        Ok(())
    }

    pub fn residual_scale_for_text(&self, text: &str) -> f64 {
        let spaces = crate::spacing::space_count(text) as f64;
        self.residual_scale()
            .max(self.spacing.as_ref().map_or(0.0, |s| {
                // Conservatively allow correlated spacing errors across a phrase.
                spaces * s.validation_after.max(s.audit_after.unwrap_or(0.0))
            }))
    }
    pub fn residual_scale(&self) -> f64 {
        self.validation_rmse
            .max(self.audit.as_ref().map_or(0.0, |a| a.rmse))
    }
    pub fn predict(&self, font: &Font, text: &str) -> Result<f64> {
        self.verify_font(font)?;
        let m = font.measure_mode(text, self.kern, self.measurement_mode)?;
        let width = match self.width_metric {
            WidthMetric::Ink => m.ink_width(),
            WidthMetric::Advance => m.advance,
        };
        Ok(width * self.size
            + (m.glyphs - 1).max(0) as f64 * self.tracking
            + self.ink_bias
            + self.spacing.as_ref().map_or(0.0, |s| s.adjustment)
                * crate::spacing::space_count(text) as f64
            + features(text)
                .iter()
                .map(|(key, count)| self.corrections.get(key).copied().unwrap_or(0.0) * count)
                .sum::<f64>())
    }
}
pub fn calibrate(
    id: String,
    page: String,
    block: Option<String>,
    words: &[Word],
    fonts: &[PathBuf],
) -> Result<Profile> {
    calibrate_for_metric(id, page, block, words, fonts, WidthMetric::Ink)
}
pub fn calibrate_for_metric(
    id: String,
    page: String,
    block: Option<String>,
    words: &[Word],
    fonts: &[PathBuf],
    metric: WidthMetric,
) -> Result<Profile> {
    calibrate_with_modes(
        id,
        page,
        block,
        words,
        fonts,
        metric,
        &[MeasurementMode::Design],
    )
}
pub fn calibrate_with_modes(
    id: String,
    page: String,
    block: Option<String>,
    words: &[Word],
    fonts: &[PathBuf],
    metric: WidthMetric,
    modes: &[MeasurementMode],
) -> Result<Profile> {
    ensure!(
        !modes.is_empty() && modes.len() <= 17 && modes.iter().all(|m| m.valid()),
        "Supply 1–17 valid measurement modes"
    );
    ensure!(!fonts.is_empty(), "At least one font is required");
    // Split by word identity, not occurrence, so repeated words cannot leak across validation.
    let mut unique = BTreeMap::new();
    for word in words.iter().filter(|w| {
        w.confidence >= 85.0
            && w.bounds.valid()
            && w.text.chars().count() >= 3
            && w.text.len() <= 128
            && block.as_ref().is_none_or(|b| &w.block == b)
    }) {
        unique.entry(word.text.clone()).or_insert(word);
    }
    let train: Vec<&Word> = unique
        .values()
        .copied()
        .filter(|w| !held_out(&w.text))
        .collect();
    let valid: Vec<&Word> = unique
        .values()
        .copied()
        .filter(|w| held_out(&w.text) && !audit_held_out(&w.text))
        .collect();
    let audit: Vec<&Word> = unique
        .values()
        .copied()
        .filter(|w| audit_held_out(&w.text))
        .collect();
    ensure!(
        train.len() >= 12 && valid.len() >= 4,
        "Need at least 12 distinct training words and 4 validation words; got {} and {}. Add visible text or select a larger typography block.",
        train.len(),
        valid.len()
    );
    let mut fits = vec![];
    for path in fonts {
        let font = Font::open(path)?;
        for &measurement_mode in modes {
            for kern in [false, true] {
                let rows: Result<Vec<Vec<f64>>> = train
                    .iter()
                    .map(|w| {
                        let m = font.measure_mode(&w.text, kern, measurement_mode)?;
                        let width = match metric {
                            WidthMetric::Ink => m.ink_width(),
                            WidthMetric::Advance => m.advance,
                        };
                        Ok(vec![width, (m.glyphs - 1).max(0) as f64, 1.0])
                    })
                    .collect();
                let Ok(rows) = rows else {
                    continue;
                };
                let widths: Vec<f64> = train.iter().map(|w| w.bounds.w).collect();
                let mut weights = vec![1.0; train.len()];
                let mut coeff = vec![];
                for _ in 0..5 {
                    let Some(c) = typography_regression(&rows, &widths, &weights) else {
                        break;
                    };
                    coeff = c;
                    let mut errors: Vec<f64> = rows
                        .iter()
                        .zip(&widths)
                        .map(|(r, y)| {
                            (r.iter().zip(&coeff).map(|(a, b)| a * b).sum::<f64>() - y).abs()
                        })
                        .collect();
                    errors.sort_by(f64::total_cmp);
                    let cutoff = (errors[errors.len() / 2] * 2.5).max(0.75);
                    for ((w, row), y) in weights.iter_mut().zip(&rows).zip(&widths) {
                        let e = (row.iter().zip(&coeff).map(|(a, b)| a * b).sum::<f64>() - y).abs();
                        *w = (cutoff / e.max(cutoff)).powi(2);
                    }
                }
                if coeff.len() != 3
                    || !(2.0..=1000.0).contains(&coeff[0])
                    || coeff[1].abs() > coeff[0] * 0.25
                    || coeff[2].abs() > coeff[0] * 0.5
                {
                    continue;
                }
                let mut base = Fit {
                    font: path.canonicalize()?,
                    font_sha256: Some(font.sha256().into()),
                    kern,
                    size: coeff[0],
                    tracking: coeff[1],
                    ink_bias: coeff[2],
                    corrections: BTreeMap::new(),
                    validation_rmse: 0.0,
                    validation_max_error: 0.0,
                    training_words: train.len(),
                    validation_words: valid.len(),
                    width_metric: metric,
                    measurement_mode,
                    audit: None,
                    spacing: None,
                };
                if evaluate(&mut base, &font, &valid).is_err() {
                    continue;
                }
                // Learn only repeatedly observed character/pair residuals, with shrinkage.
                // Select the correction model only if it improves held-out prediction.
                if train.len() >= 30 {
                    let maps: Vec<_> = train.iter().map(|w| features(&w.text)).collect();
                    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
                    for m in &maps {
                        for key in m.keys() {
                            *counts.entry(key.clone()).or_default() += 1;
                        }
                    }
                    let keys: Vec<_> = counts
                        .into_iter()
                        .filter(|(_, n)| *n >= 5)
                        .map(|(k, _)| k)
                        .collect();
                    if !keys.is_empty() {
                        let residual: Result<Vec<f64>> = train
                            .iter()
                            .map(|w| Ok(w.bounds.w - base.predict(&font, &w.text)?))
                            .collect();
                        let x: Vec<Vec<f64>> = maps
                            .iter()
                            .map(|m| {
                                keys.iter()
                                    .map(|k| m.get(k).copied().unwrap_or(0.0))
                                    .collect()
                            })
                            .collect();
                        if let Some(c) = regression(&x, &residual?, 25.0, &weights) {
                            let mut learned = base.clone();
                            learned.corrections = keys.into_iter().zip(c).collect();
                            evaluate(&mut learned, &font, &valid)?;
                            if learned.validation_rmse < base.validation_rmse * 0.95 {
                                base = learned;
                            }
                        }
                    }
                }
                fits.push(base);
            }
        }
    }
    fits.sort_by(|a, b| a.validation_rmse.total_cmp(&b.validation_rmse));
    ensure!(
        !fits.is_empty(),
        "None of the fonts explains the visible text; check OCR, typography groups, and font coverage"
    );
    let cutoff = fits[0].validation_rmse + 0.75;
    fits.retain(|f| f.validation_rmse <= cutoff);
    fits.truncate(4);
    // Audit is untouched by fitting, correction selection, ranking, and truncation.
    // Small sets remain visible but are not used as an error-scale estimate.
    if audit.len() >= 4 {
        for fit in &mut fits {
            let font = Font::open(&fit.font)?;
            let mut errors = Vec::new();
            for word in &audit {
                errors.push((fit.predict(&font, &word.text)? - word.bounds.w).abs());
            }
            fit.audit = Some(FitAudit {
                words: errors.len(),
                rmse: (errors.iter().map(|e| e * e).sum::<f64>() / errors.len() as f64).sqrt(),
                max_error: errors.into_iter().fold(0.0, f64::max),
            });
        }
    }
    Ok(Profile {
        id,
        page,
        block,
        fits,
        training_texts: train
            .iter()
            .map(|w| w.text.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        validation_texts: valid.iter().map(|w| w.text.clone()).collect(),
        audit_texts: audit.iter().map(|w| w.text.clone()).collect(),
        font_search: None,
        spacing: None,
        typography_group: None,
        neural_search: None,
    })
}
fn evaluate(fit: &mut Fit, font: &Font, words: &[&Word]) -> Result<()> {
    let errors: Result<Vec<f64>> = words
        .iter()
        .map(|w| Ok((fit.predict(font, &w.text)? - w.bounds.w).abs()))
        .collect();
    let errors = errors?;
    fit.validation_rmse = (errors.iter().map(|e| e * e).sum::<f64>() / errors.len() as f64).sqrt();
    fit.validation_max_error = errors.iter().copied().fold(0.0, f64::max);
    Ok(())
}
pub fn default_fonts() -> Vec<PathBuf> {
    let mut paths = BTreeSet::new();
    for family in [
        "Liberation Serif",
        "Liberation Sans",
        "Liberation Mono",
        "DejaVu Serif",
        "DejaVu Sans",
    ] {
        if let Ok(out) = std::process::Command::new("fc-match")
            .args(["-f", "%{file}", family])
            .output()
            && out.status.success()
        {
            let p = Path::new(String::from_utf8_lossy(&out.stdout).trim()).to_path_buf();
            if p.is_file() {
                paths.insert(p);
            }
        }
    }
    paths.into_iter().collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredFont {
    pub font: PathBuf,
    pub training_rmse: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontSearch {
    pub examined: usize,
    pub unusable: usize,
    pub training_texts: Vec<String>,
    pub shortlist: Vec<ScoredFont>,
}
pub fn installed_fonts() -> Result<Vec<PathBuf>> {
    let result = std::process::Command::new("fc-list")
        .args(["-f", "%{file}\n"])
        .output()?;
    ensure!(result.status.success(), "Fontconfig enumeration failed");
    let paths: BTreeSet<_> = String::from_utf8(result.stdout)?
        .lines()
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .collect();
    ensure!(
        !paths.is_empty() && paths.len() <= 20_000,
        "Installed font search requires 1–20,000 files"
    );
    Ok(paths.into_iter().collect())
}
/// Screen a large font library using only training identities. Full calibration
/// and held-out selection follow; audit words never enter this screening step.
pub fn shortlist_fonts(
    words: &[Word],
    block: Option<&str>,
    fonts: &[PathBuf],
    metric: WidthMetric,
    keep: usize,
) -> Result<FontSearch> {
    ensure!(
        (4..=128).contains(&keep),
        "Font shortlist size must be 4–128"
    );
    ensure!(
        !fonts.is_empty() && fonts.len() <= 20_000,
        "Font search requires 1–20,000 files"
    );
    let mut unique = BTreeMap::new();
    for word in words.iter().filter(|w| {
        w.confidence >= 85.0
            && w.bounds.valid()
            && w.text.chars().count() >= 3
            && w.text.len() <= 128
            && !held_out(&w.text)
            && block.is_none_or(|b| b == w.block)
    }) {
        unique.entry(&word.text).or_insert(word);
    }
    ensure!(
        unique.len() >= 12,
        "Font screening needs at least 12 distinct training words"
    );
    let all: Vec<_> = unique.values().copied().collect();
    let n = all.len().min(24);
    let sample: Vec<_> = (0..n).map(|i| all[i * all.len() / n]).collect();
    let widths: Vec<_> = sample.iter().map(|w| w.bounds.w).collect();
    let weights = vec![1.0; n];
    let mut scores = vec![];
    let mut unusable = 0;
    for path in fonts {
        let Ok(font) = Font::open(path) else {
            unusable += 1;
            continue;
        };
        let mut best = f64::INFINITY;
        for kern in [false, true] {
            let rows: Result<Vec<_>> = sample
                .iter()
                .map(|w| {
                    let m = font.measure(&w.text, kern)?;
                    Ok(vec![
                        match metric {
                            WidthMetric::Ink => m.ink_width(),
                            WidthMetric::Advance => m.advance,
                        },
                        (m.glyphs - 1).max(0) as f64,
                        1.0,
                    ])
                })
                .collect();
            let Ok(rows) = rows else {
                continue;
            };
            let Some(c) = regression(&rows, &widths, 0.001, &weights) else {
                continue;
            };
            if !(2.0..=1000.0).contains(&c[0])
                || c[1].abs() > c[0] * 0.25
                || c[2].abs() > c[0] * 0.5
            {
                continue;
            }
            let mse = rows
                .iter()
                .zip(&widths)
                .map(|(r, y)| (r.iter().zip(&c).map(|(a, b)| a * b).sum::<f64>() - y).powi(2))
                .sum::<f64>()
                / n as f64;
            best = best.min(mse.sqrt());
        }
        if best.is_finite() {
            scores.push(ScoredFont {
                font: path.clone(),
                training_rmse: best,
            });
        } else {
            unusable += 1;
        }
    }
    scores.sort_by(|a, b| {
        a.training_rmse
            .total_cmp(&b.training_rmse)
            .then(a.font.cmp(&b.font))
    });
    scores.truncate(keep);
    ensure!(!scores.is_empty(), "No usable fonts in search library");
    Ok(FontSearch {
        examined: fonts.len(),
        unusable,
        training_texts: sample.iter().map(|w| w.text.clone()).collect(),
        shortlist: scores,
    })
}
