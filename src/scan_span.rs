// Copyright (C) 2026 Toit contributors.
//! Visible word anchors for horizontal scan text. Mask edges select the gap;
//! they never supply the measured text width.
use crate::{
    project::{Evidence, Page, Target},
    raster::Rect,
};
use anyhow::{Context, Result, ensure};
use std::path::Path;

/// Reject visible ink that OCR omitted between the anchors and the mask.
/// This checks evidence coverage, not the correctness of recognized text.
pub fn validate_pixels(page: &Page, t: &Target, dir: &Path) -> Result<()> {
    let Evidence::ScanSpan {
        left_word,
        right_word,
        ..
    } = t.evidence
    else {
        return Ok(());
    };
    validate(page, t, left_word, right_word)?;
    let image = crate::raster::GrayImage::read(&dir.join(&page.image))?;
    ensure!(
        image.width == page.width
            && image.height == page.height
            && crate::project::hash(&image.pixels) == page.image_sha256,
        "Scan-anchor page image differs from imported evidence"
    );
    let a = page.words[left_word].bounds;
    let b = page.words[right_word].bounds;
    let top = a.y.min(b.y).floor() as usize;
    let bottom = (a.y + a.h).max(b.y + b.h).ceil() as usize;
    for (start, end) in [(a.x + a.w, t.bounds.x), (t.bounds.x + t.bounds.w, b.x)] {
        // Only whole pixels outside anchor and mask bounds belong to the gap.
        let start = start.ceil() as usize;
        let end = end.floor() as usize;
        if end <= start {
            continue;
        }
        let dark = (top..bottom)
            .map(|y| {
                image.pixels[y * image.width + start..y * image.width + end]
                    .iter()
                    .filter(|v| **v < 160)
                    .count()
            })
            .sum::<usize>();
        ensure!(
            dark < 4,
            "Unexplained ink between scan anchor and mask; refresh OCR or review the anchors"
        );
    }
    Ok(())
}

fn same_row(a: Rect, b: Rect) -> bool {
    let overlap = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    overlap >= a.h.min(b.h) * 0.5
}

pub fn validate(page: &Page, t: &Target, left: usize, right: usize) -> Result<()> {
    ensure!(
        t.bounds.valid()
            && t.bounds.x + t.bounds.w <= page.width as f64
            && t.bounds.y + t.bounds.h <= page.height as f64,
        "Scan target outside page"
    );
    let a = page.words.get(left).context("Invalid left scan anchor")?;
    let b = page.words.get(right).context("Invalid right scan anchor")?;
    for w in [a, b] {
        ensure!(
            w.bounds.valid()
                && w.bounds.x + w.bounds.w <= page.width as f64
                && w.bounds.y + w.bounds.h <= page.height as f64
                && w.confidence.is_finite()
                && w.confidence >= 85.0
                && !w.text.is_empty()
                && w.text.len() <= 128
                && !w.text.chars().any(char::is_whitespace)
                && !w.text.chars().any(char::is_control),
            "Scan anchors require reliable, bounded OCR words"
        );
    }
    ensure!(
        left != right
            && a.bounds.x + a.bounds.w <= t.bounds.x
            && b.bounds.x >= t.bounds.x + t.bounds.w
            && same_row(a.bounds, b.bounds)
            && same_row(a.bounds, t.bounds)
            && same_row(b.bounds, t.bounds),
        "Scan anchors must be fully visible on opposite sides of the mask and the same row"
    );
    // Ink height depends on the actual letters: "as" has neither ascenders nor
    // descenders, whereas "identifies" does. Raw height ratios cannot establish
    // whether these words share a font size. Row overlap is checked above;
    // uniform typography remains an explicit model assumption.
    ensure!(
        !page.words.iter().enumerate().any(|(i, w)| {
            i != left
                && i != right
                && !w.bounds.intersects(t.bounds)
                && same_row(w.bounds, a.bounds)
                && w.bounds.x < b.bounds.x
                && w.bounds.x + w.bounds.w > a.bounds.x + a.bounds.w
        }),
        "Visible text between scan anchors would be omitted"
    );
    let span = Rect {
        x: a.bounds.x,
        y: a.bounds.y.min(b.bounds.y),
        w: b.bounds.x + b.bounds.w - a.bounds.x,
        h: (a.bounds.y + a.bounds.h).max(b.bounds.y + b.bounds.h) - a.bounds.y.min(b.bounds.y),
    };
    ensure!(
        !page
            .proposed_redactions
            .iter()
            .any(|r| { r.intersects(span) && !r.intersects(t.bounds) }),
        "Another redaction lies inside the scan span"
    );
    Ok(())
}

/// Select nearest words before checking confidence; never skip an unreliable
/// nearer word and silently omit it from the shaped span.
pub fn attach(page: &Page, t: &mut Target) -> Result<()> {
    let measurement_sigma = match t.evidence {
        Evidence::Width {
            measurement_sigma, ..
        }
        | Evidence::Opaque {
            measurement_sigma, ..
        }
        | Evidence::OpaqueUniform {
            measurement_sigma, ..
        } => measurement_sigma,
        _ => anyhow::bail!("Automatic scan anchors replace width or opaque evidence only"),
    };
    let left_word = page
        .words
        .iter()
        .enumerate()
        .filter(|(_, w)| same_row(w.bounds, t.bounds) && w.bounds.x + w.bounds.w <= t.bounds.x)
        .max_by(|(_, a), (_, b)| (a.bounds.x + a.bounds.w).total_cmp(&(b.bounds.x + b.bounds.w)))
        .map(|(i, _)| i)
        .context("No visible word before mask")?;
    let right_word = page
        .words
        .iter()
        .enumerate()
        .filter(|(_, w)| same_row(w.bounds, t.bounds) && w.bounds.x >= t.bounds.x + t.bounds.w)
        .min_by(|(_, a), (_, b)| a.bounds.x.total_cmp(&b.bounds.x))
        .map(|(i, _)| i)
        .context("No visible word after mask")?;
    validate(page, t, left_word, right_word)?;
    t.evidence = Evidence::ScanSpan {
        left_word,
        right_word,
        // OCR ink endpoints are raster measurements even if the original
        // supplied width was an exact fractional coordinate.
        measurement_sigma: measurement_sigma.max(1.5),
        local_spacing: false,
    };
    Ok(())
}
