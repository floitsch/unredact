// Copyright (C) 2026 Toit contributors.
//! Candidate-independent diagnostics and bounded retries of masked visible OCR.
use crate::{
    calibration::Word,
    project::{Project, Target, hash},
    raster::{GrayImage, Rect},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SideDiagnostic {
    pub source: String,
    pub word_indices: Vec<usize>,
    pub stop_reason: String,
    pub stop_word_index: Option<usize>,
    pub gap_pixels: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextDiagnostics {
    pub page_word_count: usize,
    pub invalid_word_bounds: usize,
    pub same_row_word_count: usize,
    pub left: SideDiagnostic,
    pub right: SideDiagnostic,
}

/// Explain the same conservative same-row walk used by visible_context.
/// No candidate strings, labels, or language-model scores are consulted.
pub fn diagnose_words(
    words: &[Word],
    width: usize,
    height: usize,
    target: Rect,
    masks: &[Rect],
) -> ContextDiagnostics {
    let valid = |b: Rect| b.valid() && b.x + b.w <= width as f64 && b.y + b.h <= height as f64;
    let mut row: Vec<_> = words
        .iter()
        .enumerate()
        .filter(|(_, w)| {
            valid(w.bounds)
                && (w.bounds.y + w.bounds.h).min(target.y + target.h) - w.bounds.y.max(target.y)
                    >= w.bounds.h.min(target.h) * 0.5
        })
        .collect();
    row.sort_by(|a, b| a.1.bounds.x.total_cmp(&b.1.bounds.x));
    let walk = |left: bool| {
        let mut ordered = row.clone();
        if left {
            ordered.reverse();
        }
        let mut result = SideDiagnostic {
            source: "ocr_same_row".into(),
            word_indices: vec![],
            stop_reason: if row.is_empty() {
                "no_same_row_words"
            } else {
                "no_text_on_side"
            }
            .into(),
            stop_word_index: None,
            gap_pixels: None,
        };
        let mut boundary = if left { target.x } else { target.x + target.w };
        for (index, word) in ordered {
            let b = word.bounds;
            if (left && b.x + b.w > target.x) || (!left && b.x < target.x + target.w) {
                continue;
            }
            let gap = if left {
                boundary - b.x - b.w
            } else {
                b.x - boundary
            };
            let span = Rect {
                x: if left { b.x } else { boundary },
                y: b.y,
                w: if left {
                    boundary - b.x
                } else {
                    b.x + b.w - boundary
                },
                h: b.h,
            };
            let reason = if !word.confidence.is_finite() {
                Some("nonfinite_confidence")
            } else if word.confidence < 50.0 {
                Some("low_confidence")
            } else if gap > 4.0 * target.h {
                Some("gap_exceeds_four_target_heights")
            } else if word.text.is_empty() {
                Some("empty_word")
            } else if word.text.chars().any(char::is_control) {
                Some("control_character")
            } else if masks.iter().any(|m| m.intersects(b)) {
                Some("masked_word")
            } else if masks.iter().any(|m| m.intersects(span)) {
                Some("masked_span")
            } else {
                None
            };
            if let Some(reason) = reason {
                result.stop_reason = reason.into();
                result.stop_word_index = Some(index);
                result.gap_pixels = Some(gap);
                break;
            }
            result.word_indices.push(index);
            result.stop_reason = "page_text_exhausted".into();
            boundary = if left { b.x } else { b.x + b.w };
            if result.word_indices.len() == 12 {
                result.stop_reason = "twelve_word_limit".into();
                break;
            }
        }
        if left {
            result.word_indices.reverse();
        }
        result
    };
    ContextDiagnostics {
        page_word_count: words.len(),
        invalid_word_bounds: words.iter().filter(|w| !valid(w.bounds)).count(),
        same_row_word_count: row.len(),
        left: walk(true),
        right: walk(false),
    }
}

pub fn diagnose_target(project: &Project, target: &Target) -> Result<ContextDiagnostics> {
    let page = project.page(&target.page)?;
    let masks: Vec<_> = page
        .proposed_redactions
        .iter()
        .copied()
        .chain(
            project
                .targets
                .iter()
                .filter(|t| t.page == target.page)
                .map(|t| t.bounds),
        )
        .collect();
    let mut report = diagnose_words(&page.words, page.width, page.height, target.bounds, &masks);
    for (side, supplied) in [
        (&mut report.left, &target.left_context),
        (&mut report.right, &target.right_context),
    ] {
        if !supplied.trim().is_empty() {
            side.source = "supplied".into();
            side.stop_reason = "supplied_context".into();
            side.word_indices.clear();
            side.stop_word_index = None;
            side.gap_pixels = None;
        }
    }
    Ok(report)
}

/// Human-readable extraction diagnostics for Context.layout_notes.
pub fn context_notes(project: &Project, target: &Target) -> Result<Vec<String>> {
    let report = diagnose_target(project, target)?;
    Ok([("left", report.left), ("right", report.right)]
        .into_iter()
        .map(|(name, side)| {
            format!(
                "{name}: {} ({} OCR words; stopped at {:?}; gap {:?} px)",
                side.stop_reason,
                side.word_indices.len(),
                side.stop_word_index,
                side.gap_pixels
            )
        })
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrRecoveryOptions {
    /// Default false: the established PSM 3 path remains unchanged.
    pub retry_missing_context: bool,
    /// Only these many missing-context masks receive two local-strip retries.
    pub max_regions: usize,
}
impl Default for OcrRecoveryOptions {
    fn default() -> Self {
        Self {
            retry_missing_context: false,
            max_regions: 16,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryAttempt {
    pub id: String,
    pub psm: usize,
    pub input: String,
    pub input_pixel_sha256: String,
    pub crop_bounds: Rect,
    pub focus_mask: Rect,
    pub trigger: ContextDiagnostics,
    pub diagnostics_after: Option<ContextDiagnostics>,
    pub accepted: bool,
    pub selection_reason: String,
    pub error: Option<String>,
    /// Page-coordinate OCR words for this attempt, including rejected attempts.
    pub words: Vec<Word>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecoveryReport {
    pub enabled: bool,
    pub selection_policy: String,
    pub max_regions: usize,
    pub missing_context_regions_before: usize,
    pub missing_context_regions_after: usize,
    pub skipped_by_region_limit: usize,
    pub focus_masks: Vec<Rect>,
    pub diagnostics_before: Vec<ContextDiagnostics>,
    pub diagnostics_after: Vec<ContextDiagnostics>,
    pub baseline_words: Vec<Word>,
    pub attempts: Vec<RecoveryAttempt>,
}

fn contains(outer: Rect, inner: Rect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + inner.w <= outer.x + outer.w
        && inner.y + inner.h <= outer.y + outer.h
}

/// Reject masked/outside words, then measure ink only on the original raster.
pub(crate) fn refine_words(words: &mut Vec<Word>, original: &GrayImage, masks: &[Rect]) {
    words.retain(|w| {
        w.bounds.valid()
            && w.bounds.x + w.bounds.w <= original.width as f64
            && w.bounds.y + w.bounds.h <= original.height as f64
            && !masks.iter().any(|r| r.intersects(w.bounds))
    });
    words.retain_mut(|word| {
        if let Ok(crop) = original.crop(word.bounds)
            && let Some(ink) = crop.ink_bounds(160)
        {
            // crop rounds its origin down. Keep page coordinates exact for
            // fractional input bounds as well as integer Tesseract boxes.
            word.bounds = Rect {
                x: word.bounds.x.floor() + ink.x,
                y: word.bounds.y.floor() + ink.y,
                w: ink.w,
                h: ink.h,
            };
            true
        } else {
            false
        }
    });
}

fn diagnostics(
    words: &[Word],
    original: &GrayImage,
    focus: &[Rect],
    masks: &[Rect],
) -> Vec<ContextDiagnostics> {
    focus
        .iter()
        .map(|r| diagnose_words(words, original.width, original.height, *r, masks))
        .collect()
}
fn missing(reports: &[ContextDiagnostics]) -> usize {
    reports
        .iter()
        .filter(|r| r.left.word_indices.is_empty() || r.right.word_indices.is_empty())
        .count()
}
fn quality(reports: &[ContextDiagnostics]) -> (usize, usize) {
    reports
        .iter()
        .flat_map(|r| [&r.left, &r.right])
        .fold((0, 0), |(sides, words), side| {
            (
                sides + usize::from(!side.word_indices.is_empty()),
                words + side.word_indices.len().min(4),
            )
        })
}
fn preserves_sides(before: &[ContextDiagnostics], after: &[ContextDiagnostics]) -> bool {
    before.iter().zip(after).all(|(a, b)| {
        [&a.left, &a.right]
            .into_iter()
            .zip([&b.left, &b.right])
            .all(|(x, y)| x.word_indices.is_empty() || !y.word_indices.is_empty())
    })
}

pub(crate) struct RecoveryInput<'a> {
    pub original: &'a GrayImage,
    pub masked: &'a GrayImage,
    pub focus: &'a [Rect],
    pub masks: &'a [Rect],
    pub exclusions: &'a [Rect],
    pub output_dir: &'a Path,
    pub tessdata: Option<&'a Path>,
}

pub(crate) fn recover(
    input: RecoveryInput<'_>,
    options: &OcrRecoveryOptions,
    words: &mut Vec<Word>,
) -> Result<RecoveryReport> {
    let tessdata = input.tessdata;
    recover_using(input, options, words, |path, psm| {
        crate::project::ocr_with_settings(path, tessdata, psm)
    })
}

fn recover_using(
    input: RecoveryInput<'_>,
    options: &OcrRecoveryOptions,
    words: &mut Vec<Word>,
    mut recognize: impl FnMut(&Path, usize) -> Result<Vec<Word>>,
) -> Result<RecoveryReport> {
    let RecoveryInput {
        original,
        masked,
        focus,
        masks,
        exclusions,
        output_dir,
        ..
    } = input;
    ensure!(
        (1..=64).contains(&options.max_regions),
        "OCR retry region limit must be 1–64"
    );
    let initial = diagnostics(words, original, focus, masks);
    let mut report = RecoveryReport { enabled: options.retry_missing_context,
        selection_policy: "Only missing same-row context triggers PSM 6 and 11 on masked, unscaled horizontal strips. Keep every previously covered side; strictly improve (covered sides, context words capped at four per side) over all supplied masks. Ties retain prior OCR. No labels, candidates or model scores are used. Final ink bounds use original pixels.".into(),
        max_regions: options.max_regions, missing_context_regions_before: missing(&initial),
        missing_context_regions_after: missing(&initial), skipped_by_region_limit: 0,
        focus_masks: focus.to_vec(), diagnostics_before: initial.clone(), diagnostics_after: initial,
        baseline_words: words.clone(),
        attempts: vec![] };
    if !options.retry_missing_context {
        return Ok(report);
    }
    let mut tried = 0;
    for (index, mask) in focus.iter().enumerate() {
        let before = diagnostics(words, original, focus, masks);
        if !before[index].left.word_indices.is_empty()
            && !before[index].right.word_indices.is_empty()
        {
            continue;
        }
        if tried == options.max_regions {
            report.skipped_by_region_limit += 1;
            continue;
        }
        tried += 1;
        let y = (mask.y - mask.h * 2.0).floor().max(0.0);
        let bottom = (mask.y + mask.h * 3.0).ceil().min(masked.height as f64);
        let crop_bounds = Rect {
            x: 0.0,
            y,
            w: masked.width as f64,
            h: bottom - y,
        };
        let crop = masked.crop(crop_bounds)?;
        let input_hash = hash(&crop.pixels);
        let input = output_dir.join(format!("retry-{input_hash}.pgm"));
        crop.save(&input)?;
        for psm in [6, 11] {
            let before = diagnostics(words, original, focus, masks);
            let id = format!("region-{index}-psm-{psm}");
            let mut attempt = RecoveryAttempt {
                id: id.clone(),
                psm,
                input: input.file_name().unwrap().to_string_lossy().into_owned(),
                input_pixel_sha256: input_hash.clone(),
                crop_bounds,
                focus_mask: *mask,
                trigger: before[index].clone(),
                diagnostics_after: None,
                accepted: false,
                selection_reason: String::new(),
                error: None,
                words: vec![],
            };
            match recognize(&input, psm) {
                Err(error) => {
                    attempt.selection_reason = "OCR attempt failed; retained prior words".into();
                    attempt.error = Some(format!("{error:#}"));
                }
                Ok(mut retry) => {
                    // Discard words clipped by strip edges and keep groups distinct
                    // from the page OCR so paragraph walking cannot cross workers.
                    retry.retain(|w| {
                        w.bounds.y > 0.0 && w.bounds.y + w.bounds.h < crop.height as f64
                    });
                    for word in &mut retry {
                        word.bounds.y += y;
                        word.block = format!("{id}:{}", word.block);
                        word.line = format!("{id}:{}", word.line);
                    }
                    refine_words(&mut retry, original, exclusions);
                    let retained: Vec<_> = words
                        .iter()
                        .filter(|w| !contains(crop_bounds, w.bounds))
                        .cloned()
                        .collect();
                    retry.retain(|w| !retained.iter().any(|old| old.bounds.intersects(w.bounds)));
                    attempt.words = retry.clone();
                    let mut candidate = retained;
                    candidate.extend(retry);
                    let after = diagnostics(&candidate, original, focus, masks);
                    attempt.diagnostics_after = Some(after[index].clone());
                    if preserves_sides(&before, &after) && quality(&after) > quality(&before) {
                        attempt.accepted = true;
                        attempt.selection_reason = format!(
                            "Visible context coverage improved {:?} -> {:?}; all covered sides preserved",
                            quality(&before),
                            quality(&after)
                        );
                        *words = candidate;
                    } else {
                        attempt.selection_reason = format!(
                            "No strict safe visible-context improvement: {:?} -> {:?}; retained prior words",
                            quality(&before),
                            quality(&after)
                        );
                    }
                }
            }
            report.attempts.push(attempt);
        }
    }
    report.diagnostics_after = diagnostics(words, original, focus, masks);
    report.missing_context_regions_after = missing(&report.diagnostics_after);
    fs::create_dir_all(output_dir)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, x: f64, y: f64) -> Word {
        Word {
            text: text.into(),
            bounds: Rect {
                x,
                y,
                w: 30.0,
                h: 10.0,
            },
            confidence: 99.0,
            block: "1".into(),
            line: "1:1".into(),
        }
    }

    #[test]
    fn retries_map_original_coordinates_exclude_masks_and_keep_ties() {
        let dir = tempfile::tempdir().unwrap();
        let focus = [Rect {
            x: 100.0,
            y: 98.0,
            w: 50.0,
            h: 14.0,
        }];
        let mut original = GrayImage::white(320, 220).unwrap();
        for (x0, x1) in [(40, 70), (100, 150), (180, 210)] {
            for y in 100..110 {
                original.pixels[y * 320 + x0..y * 320 + x1].fill(0);
            }
        }
        let mut masked = original.clone();
        for y in 98..112 {
            masked.pixels[y * 320 + 100..y * 320 + 150].fill(255);
        }
        let mut words = vec![word("Before", 40.0, 100.0)];
        let before_pixels = original.pixels.clone();
        let report = recover_using(
            RecoveryInput {
                original: &original,
                masked: &masked,
                focus: &focus,
                masks: &focus,
                exclusions: &focus,
                output_dir: dir.path(),
                tessdata: None,
            },
            &OcrRecoveryOptions {
                retry_missing_context: true,
                max_regions: 1,
            },
            &mut words,
            |input, psm| {
                let crop = GrayImage::read(input)?;
                // y=70 crop origin, so the protected y=100 row is y=30 here.
                assert_eq!(crop.pixels[30 * 320 + 110], 255);
                Ok(vec![
                    word("Before", 40.0, 30.0),
                    word("hidden", 110.0, 30.0),
                    word(
                        if psm == 6 { "after" } else { "tie replacement" },
                        180.0,
                        30.0,
                    ),
                ])
            },
        )
        .unwrap();
        assert_eq!(report.attempts.len(), 2);
        assert!(report.attempts[0].accepted);
        assert!(!report.attempts[1].accepted);
        assert_eq!(report.missing_context_regions_before, 1);
        assert_eq!(report.missing_context_regions_after, 0);
        assert_eq!(
            words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(),
            ["Before", "after"]
        );
        assert!(words.iter().all(|w| w.bounds.y == 100.0));
        assert!(
            report
                .attempts
                .iter()
                .all(|a| a.words.iter().all(|w| !w.bounds.intersects(focus[0])))
        );
        assert_eq!(original.pixels, before_pixels);
    }

    #[test]
    fn reject_a_retry_that_loses_an_existing_context_side() {
        let dir = tempfile::tempdir().unwrap();
        let image = GrayImage::white(320, 220).unwrap();
        let focus = [Rect {
            x: 100.0,
            y: 98.0,
            w: 50.0,
            h: 14.0,
        }];
        let mut words = vec![word("Before", 40.0, 100.0)];
        let report = recover_using(
            RecoveryInput {
                original: &image,
                masked: &image,
                focus: &focus,
                masks: &focus,
                exclusions: &focus,
                output_dir: dir.path(),
                tessdata: None,
            },
            &OcrRecoveryOptions {
                retry_missing_context: true,
                max_regions: 1,
            },
            &mut words,
            |_, psm| {
                if psm == 11 {
                    anyhow::bail!("model unavailable");
                }
                Ok(vec![
                    word("long", 160.0, 30.0),
                    word("right", 200.0, 30.0),
                    word("context", 240.0, 30.0),
                ])
            },
        )
        .unwrap();
        assert_eq!(words[0].text, "Before");
        assert_eq!(words.len(), 1);
        assert!(!report.attempts[0].accepted);
        assert!(!report.attempts[1].accepted);
        assert_eq!(
            report.attempts[1].error.as_deref(),
            Some("model unavailable")
        );
    }

    #[test]
    fn retries_obey_region_budget_and_remain_opt_in() {
        let dir = tempfile::tempdir().unwrap();
        let image = GrayImage::white(320, 220).unwrap();
        let focus = [
            Rect {
                x: 100.0,
                y: 60.0,
                w: 50.0,
                h: 14.0,
            },
            Rect {
                x: 100.0,
                y: 140.0,
                w: 50.0,
                h: 14.0,
            },
        ];
        let input = || RecoveryInput {
            original: &image,
            masked: &image,
            focus: &focus,
            masks: &focus,
            exclusions: &focus,
            output_dir: dir.path(),
            tessdata: None,
        };
        let mut words = vec![];
        let disabled = recover_using(
            input(),
            &OcrRecoveryOptions::default(),
            &mut words,
            |_, _| panic!("Retries are opt-in"),
        )
        .unwrap();
        assert!(disabled.attempts.is_empty());
        let mut calls = 0;
        let report = recover_using(
            input(),
            &OcrRecoveryOptions {
                retry_missing_context: true,
                max_regions: 1,
            },
            &mut words,
            |_, _| {
                calls += 1;
                Ok(vec![])
            },
        )
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(report.attempts.len(), 2);
        assert_eq!(report.skipped_by_region_limit, 1);
    }
}
