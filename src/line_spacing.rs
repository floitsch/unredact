// Copyright (C) 2026 Toit contributors.
//! Candidate-independent spacing estimates from visible training words on a
//! target's row. Separate from the profile's document-wide space adjustment.
use crate::{
    calibration::{Fit, Profile},
    native::Font,
    project::{Evidence, Page, Target},
    raster::Rect,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    pub left_word: usize,
    pub right_word: usize,
    pub left_text: String,
    pub right_text: String,
    pub bounds: Rect,
    pub observed_gap: f64,
    pub predicted_gap: f64,
    pub residual: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Estimate {
    pub adjustment: f64,
    pub scatter: f64,
    pub reason: Option<String>,
    pub samples: Vec<Sample>,
}

pub fn prepare(
    project: &crate::project::Project,
    targets: &[&Target],
    fonts: &BTreeMap<std::path::PathBuf, Font>,
) -> Result<BTreeMap<String, Vec<Estimate>>> {
    let mut out = BTreeMap::new();
    for target in targets {
        if !matches!(
            target.evidence,
            Evidence::ScanSpan {
                local_spacing: true,
                ..
            }
        ) {
            continue;
        }
        let profile = project.profile(&target.profile)?;
        // A short second mask can resemble an ordinary inter-word gap. Exclude
        // every registered target, including masks absent from auto-detection.
        let mut page = project.page(&target.page)?.clone();
        page.proposed_redactions.extend(
            project
                .targets
                .iter()
                .filter(|t| t.page == target.page)
                .map(|t| t.bounds),
        );
        let fits = profile
            .fits
            .iter()
            .map(|fit| estimate(&page, profile, target, fit, &fonts[&fit.font]))
            .collect::<Result<_>>()?;
        out.insert(target.id.clone(), fits);
    }
    Ok(out)
}

pub fn estimate(
    page: &Page,
    profile: &Profile,
    target: &Target,
    fit: &Fit,
    font: &Font,
) -> Result<Estimate> {
    let Evidence::ScanSpan {
        left_word,
        right_word,
        ..
    } = target.evidence
    else {
        anyhow::bail!("Line spacing requires scan anchors");
    };
    crate::scan_span::validate(page, target, left_word, right_word)?;
    let anchor = page.words[left_word].bounds;
    let mut row: Vec<_> = page
        .words
        .iter()
        .enumerate()
        .filter(|(_, w)| {
            let b = w.bounds;
            b.valid()
                && b.x + b.w <= page.width as f64
                && b.y + b.h <= page.height as f64
                && (b.y + b.h).min(anchor.y + anchor.h) - b.y.max(anchor.y)
                    >= b.h.min(anchor.h) * 0.5
        })
        .collect();
    row.sort_by(|a, b| a.1.bounds.x.total_cmp(&b.1.bounds.x));
    let mut seen = BTreeSet::new();
    let mut samples = vec![];
    // Retain unreliable and withheld words in the adjacency list. Skipping them
    // first would invent a single-space pair across an omitted word.
    for pair in row.windows(2) {
        let ((ia, a), (ib, b)) = (pair[0], pair[1]);
        if [a, b].iter().any(|w| {
            !w.confidence.is_finite()
                || w.confidence < 85.0
                || !profile.training_texts.contains(&w.text)
                || crate::calibration::held_out(&w.text)
                || w.text.is_empty()
                || w.text.len() > 128
                || w.text.chars().any(char::is_whitespace)
                || w.text.chars().any(char::is_control)
        }) {
            continue;
        }
        let gap = b.bounds.x - (a.bounds.x + a.bounds.w);
        if gap <= 0.0 || gap > a.bounds.h.min(b.bounds.h) * 1.5 {
            continue;
        }
        let bounds = Rect {
            x: a.bounds.x,
            y: a.bounds.y.min(b.bounds.y),
            w: b.bounds.x + b.bounds.w - a.bounds.x,
            h: (a.bounds.y + a.bounds.h).max(b.bounds.y + b.bounds.h) - a.bounds.y.min(b.bounds.y),
        };
        if bounds.intersects(target.bounds)
            || page
                .proposed_redactions
                .iter()
                .any(|r| r.intersects(bounds))
            || !seen.insert((a.text.clone(), b.text.clone()))
        {
            continue;
        }
        let predicted = fit.predict(font, &format!("{} {}", a.text, b.text))?
            - fit.predict(font, &a.text)?
            - fit.predict(font, &b.text)?;
        samples.push(Sample {
            left_word: ia,
            right_word: ib,
            left_text: a.text.clone(),
            right_text: b.text.clone(),
            bounds,
            observed_gap: gap,
            predicted_gap: predicted,
            residual: gap - predicted,
        });
    }
    let mut result = Estimate {
        adjustment: 0.0,
        scatter: 0.0,
        reason: None,
        samples,
    };
    if result.samples.len() < 3 {
        result.reason = Some(format!(
            "Need three distinct visible training pairs on this row; found {}",
            result.samples.len()
        ));
        return Ok(result);
    }
    let mut residuals: Vec<_> = result.samples.iter().map(|s| s.residual).collect();
    residuals.sort_by(f64::total_cmp);
    let n = residuals.len();
    let adjustment = (residuals[(n - 1) / 2] + residuals[n / 2]) * 0.5;
    let space = font
        .measure_mode(" ", fit.kern, fit.measurement_mode)?
        .advance
        * fit.size
        + fit.tracking
        + fit.spacing.as_ref().map_or(0.0, |s| s.adjustment);
    if !adjustment.is_finite() || adjustment.abs() > fit.size * 0.5 || space + adjustment <= 0.0 {
        result.reason = Some("Local adjustment is outside the physical space-width bounds".into());
        return Ok(result);
    }
    result.adjustment = adjustment;
    // Do not divide by sqrt(n): adjacent pairs and OCR errors are correlated.
    result.scatter = (residuals
        .iter()
        .map(|r| (r - adjustment).powi(2))
        .sum::<f64>()
        / n as f64)
        .sqrt()
        .max(0.5);
    Ok(result)
}
