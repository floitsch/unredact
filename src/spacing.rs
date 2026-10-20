// Copyright (C) 2026 Toit contributors.
//! Optional scan spacing calibration. OCR provides labels and line membership;
//! measured ink endpoints provide widths. Audit word identities never fit or
//! select the adjustment. This estimates a local average, not line justification.
use crate::{
    calibration::{Fit, Profile, WidthMetric, Word, audit_held_out, held_out},
    native::Font,
    raster::Rect,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Split {
    Train,
    Validation,
    Audit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    pub text: String,
    pub bounds: Rect,
    pub split: Split,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub samples: Vec<Sample>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpacingFit {
    pub adjustment: f64,
    pub proposed_adjustment: f64,
    pub training_pairs: usize,
    pub validation_pairs: usize,
    pub validation_before: f64,
    pub validation_after: f64,
    pub audit_pairs: usize,
    pub audit_before: Option<f64>,
    pub audit_after: Option<f64>,
}

impl SpacingFit {
    pub fn valid(&self) -> bool {
        self.adjustment.is_finite()
            && self.proposed_adjustment.is_finite()
            && self.training_pairs >= 12
            && self.validation_pairs >= 4
            && [self.validation_before, self.validation_after]
                .iter()
                .all(|v| v.is_finite() && *v >= 0.0)
            && self.audit_before.is_some() == (self.audit_pairs >= 4)
            && self.audit_after.is_some() == (self.audit_pairs >= 4)
            && [self.audit_before, self.audit_after]
                .into_iter()
                .flatten()
                .all(|v| v.is_finite() && v >= 0.0)
    }
}

pub fn space_count(text: &str) -> usize {
    text.trim_matches(' ')
        .bytes()
        .filter(|b| *b == b' ')
        .count()
}

pub fn samples(words: &[Word], block: Option<&str>, excluded: &[Rect]) -> Vec<Sample> {
    let mut lines: BTreeMap<(&str, &str), Vec<&Word>> = BTreeMap::new();
    for w in words {
        if block.is_none_or(|b| b == w.block) {
            lines.entry((&w.block, &w.line)).or_default().push(w);
        }
    }
    let mut seen = BTreeSet::new();
    let mut out = vec![];
    for line in lines.values_mut() {
        line.sort_by(|a, b| a.bounds.x.total_cmp(&b.bounds.x));
        // Keep unreliable words in the adjacency list, so filtering a bad OCR
        // token cannot manufacture a pair across that token.
        for pair in line.windows(2) {
            let [a, b] = [pair[0], pair[1]];
            if [a, b].iter().any(|w| {
                w.confidence < 85.0
                    || !w.confidence.is_finite()
                    || !w.bounds.valid()
                    || w.text.chars().count() < 3
                    || w.text.len() > 128
                    || w.text.chars().any(char::is_whitespace)
            }) {
                continue;
            }
            let gap = b.bounds.x - (a.bounds.x + a.bounds.w);
            let h = a.bounds.h.min(b.bounds.h);
            let overlap =
                (a.bounds.y + a.bounds.h).min(b.bounds.y + b.bounds.h) - a.bounds.y.max(b.bounds.y);
            if gap <= 0.0
                || gap > h * 1.5
                || overlap < h * 0.5
                || a.bounds.h.max(b.bounds.h) > h * 1.8
            {
                continue;
            }
            let bounds = Rect {
                x: a.bounds.x,
                y: a.bounds.y.min(b.bounds.y),
                w: b.bounds.x + b.bounds.w - a.bounds.x,
                h: (a.bounds.y + a.bounds.h).max(b.bounds.y + b.bounds.h)
                    - a.bounds.y.min(b.bounds.y),
            };
            if excluded.iter().any(|r| bounds.intersects(*r)) {
                continue;
            }
            let text = format!("{} {}", a.text, b.text);
            if !seen.insert(text.clone()) {
                continue;
            }
            let split = if [a, b].iter().any(|w| audit_held_out(&w.text)) {
                Split::Audit
            } else if [a, b].iter().any(|w| held_out(&w.text)) {
                Split::Validation
            } else {
                Split::Train
            };
            out.push(Sample {
                text,
                bounds,
                split,
            });
        }
    }
    out
}

fn errors(fit: &Fit, font: &Font, samples: &[&Sample]) -> Result<Vec<f64>> {
    samples
        .iter()
        .map(|s| Ok(s.bounds.w - fit.predict(font, &s.text)?))
        .collect()
}

fn rmse(errors: &[f64], adjustment: f64) -> f64 {
    (errors.iter().map(|e| (e - adjustment).powi(2)).sum::<f64>() / errors.len() as f64).sqrt()
}

pub fn calibrate(profile: &mut Profile, words: &[Word], excluded: &[Rect]) -> Result<()> {
    ensure!(
        !profile.fits.is_empty()
            && profile
                .fits
                .iter()
                .all(|f| f.width_metric == WidthMetric::Ink),
        "Space calibration requires scan/ink measurements"
    );
    let all = samples(words, profile.block.as_deref(), excluded);
    let train: Vec<_> = all.iter().filter(|s| s.split == Split::Train).collect();
    let valid: Vec<_> = all
        .iter()
        .filter(|s| s.split == Split::Validation)
        .collect();
    let audit: Vec<_> = all.iter().filter(|s| s.split == Split::Audit).collect();
    let reason = if train.len() < 12 || valid.len() < 4 {
        Some(format!(
            "Need 12 training and 4 validation pairs; got {} and {}",
            train.len(),
            valid.len()
        ))
    } else {
        None
    };
    // Work on clones: a font failure must not leave a partially updated profile.
    let mut fits = profile.fits.clone();
    for fit in &mut fits {
        fit.spacing = None;
        if reason.is_some() {
            continue;
        }
        let font = Font::open(&fit.font)?;
        let mut residuals = errors(fit, &font, &train)?;
        residuals.sort_by(f64::total_cmp);
        let n = residuals.len();
        let proposed = (residuals[(n - 1) / 2] + residuals[n / 2]) / 2.0;
        let validation = errors(fit, &font, &valid)?;
        let before = rmse(&validation, 0.0);
        // Reject excessive corrections and require an actual held-out gain.
        // Variable justification remains model error, not a fitted line offset.
        let space_advance = font
            .measure_mode(" ", fit.kern, fit.measurement_mode)?
            .advance
            * fit.size
            + fit.tracking;
        let adjustment = if proposed.abs() <= fit.size * 0.25
            && space_advance + proposed > 0.0
            && rmse(&validation, proposed) < before * 0.95
        {
            proposed
        } else {
            0.0
        };
        let audit_errors = errors(fit, &font, &audit)?;
        fit.spacing = Some(SpacingFit {
            adjustment,
            proposed_adjustment: proposed,
            training_pairs: train.len(),
            validation_pairs: valid.len(),
            validation_before: before,
            validation_after: rmse(&validation, adjustment),
            audit_pairs: audit.len(),
            audit_before: (audit.len() >= 4).then(|| rmse(&audit_errors, 0.0)),
            audit_after: (audit.len() >= 4).then(|| rmse(&audit_errors, adjustment)),
        });
    }
    profile.fits = fits;
    profile.spacing = Some(Report {
        samples: all,
        reason,
    });
    Ok(())
}
