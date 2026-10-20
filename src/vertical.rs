// Copyright (C) 2026 Toit contributors.
//! Learn a separate rendering height scale using visible training-word boxes.
use crate::{
    calibration::{Profile, Word},
    native::Font,
};
use anyhow::{Result, ensure};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Serialize)]
pub struct Audit {
    pub words: usize,
    pub raw_rmse: Option<f64>,
    pub scaled_rmse: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct Fit {
    pub scale: f64,
    pub training_words: usize,
    pub status: &'static str,
    pub validation: Audit,
    pub audit: Audit,
}

fn audit(values: &[(f64, f64)], scale: f64) -> Audit {
    let rmse = |s: f64| {
        (!values.is_empty()).then(|| {
            (values
                .iter()
                .map(|(predicted, observed)| (predicted * s - observed).powi(2))
                .sum::<f64>()
                / values.len() as f64)
                .sqrt()
        })
    };
    Audit {
        words: values.len(),
        raw_rmse: rmse(1.),
        scaled_rmse: rmse(scale),
    }
}

pub fn learn(profile: &Profile, words: &[Word]) -> Result<Vec<Fit>> {
    let training = profile.training_texts.iter().collect::<BTreeSet<_>>();
    let validation = profile.validation_texts.iter().collect::<BTreeSet<_>>();
    let audit_texts = profile.audit_texts.iter().collect::<BTreeSet<_>>();
    ensure!(
        training.is_disjoint(&validation)
            && training.is_disjoint(&audit_texts)
            && validation.is_disjoint(&audit_texts),
        "Visible height partitions must be disjoint by text"
    );
    let mut result = vec![];
    for fit in &profile.fits {
        let font = Font::open(&fit.font)?;
        fit.verify_font(&font)?;
        let mut ratios = vec![];
        let mut val = vec![];
        let mut held = vec![];
        for word in words {
            if word.confidence < 85.
                || !word.bounds.valid()
                || profile.block.as_ref().is_some_and(|b| b != &word.block)
            {
                continue;
            }
            let Ok(metrics) = font.measure(&word.text, fit.kern) else {
                continue;
            };
            let predicted = (metrics.bottom - metrics.top) * fit.size;
            if !predicted.is_finite() || predicted <= 1. {
                continue;
            }
            let observed = word.bounds.h;
            if training.contains(&word.text) {
                ratios.push(observed / predicted);
            } else if validation.contains(&word.text) {
                val.push((predicted, observed));
            } else if audit_texts.contains(&word.text) {
                held.push((predicted, observed));
            }
        }
        ratios.sort_by(f64::total_cmp);
        let mut scale = 1.;
        let mut status = "insufficient_training_words";
        if ratios.len() >= 12 {
            let m = ratios.len() / 2;
            let median = if ratios.len() % 2 == 0 {
                (ratios[m - 1] + ratios[m]) / 2.
            } else {
                ratios[m]
            };
            if median.is_finite() && (0.25..=4.).contains(&median) {
                scale = median;
                status = "fitted";
            } else {
                status = "median_outside_supported_range";
            }
        }
        result.push(Fit {
            scale,
            training_words: ratios.len(),
            status,
            validation: audit(&val, scale),
            audit: audit(&held, scale),
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audit_keeps_empty_partitions_unknown() {
        assert!(audit(&[], 0.5).scaled_rmse.is_none());
        let report = audit(&[(20., 10.), (24., 12.)], 0.5);
        assert_eq!(report.scaled_rmse, Some(0.));
        assert!(report.raw_rmse.unwrap() > 10.);
    }
}
