// Copyright (C) 2026 Toit contributors.
use crate::{
    calibration::{Profile, Word},
    native::{NativeChar, Pdf},
    raster::{GrayImage, Rect},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub id: String,
    pub source: PathBuf,
    pub source_sha256: String,
    pub page_number: usize,
    pub dpi: f64,
    pub width: usize,
    pub height: usize,
    pub image: PathBuf,
    pub image_sha256: String,
    pub likely_scan: bool,
    pub image_coverage: f64,
    pub native_chars: Vec<NativeChar>,
    pub words: Vec<Word>,
    pub proposed_redactions: Vec<Rect>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Evidence {
    /// Explicit uniform prior over total horizontal mask padding.
    OpaqueUniform {
        padding_min: f64,
        padding_max: f64,
        measurement_sigma: f64,
    },
    Opaque {
        padding_min: f64,
        padding_max: f64,
        measurement_sigma: f64,
    },
    Width {
        width: f64,
        measurement_sigma: f64,
    },
    /// Outer ink endpoints of two fully visible OCR words around the mask.
    /// Exactly one ordinary space is assumed on each side of the candidate.
    ScanSpan {
        left_word: usize,
        right_word: usize,
        measurement_sigma: f64,
        #[serde(default)]
        local_spacing: bool,
    },
    NativeSpan {
        left_anchor: usize,
        right_anchor: usize,
        prefix: String,
        suffix: String,
        measurement_sigma_points: f64,
    },
    Mosaic {
        block: usize,
        phase_x: usize,
        phase_y: usize,
        origin_x: f64,
        baseline_y: f64,
        noise_sigma: f64,
    },
    MosaicAuto {
        min_block: usize,
        max_block: usize,
        origin_x: f64,
        baseline_y: f64,
        noise_sigma: f64,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub id: String,
    pub page: String,
    pub bounds: Rect,
    pub profile: String,
    pub group: String,
    pub surface_kind: String,
    pub evidence: Evidence,
    pub left_context: String,
    pub right_context: String,
    // Same key means dependent observations: only one contributes to joint score.
    // Exact page/crop duplicates are additionally detected regardless of this key.
    pub dependence_key: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    pub pages: Vec<Page>,
    pub profiles: Vec<Profile>,
    pub targets: Vec<Target>,
}
impl Default for Project {
    fn default() -> Self {
        Self {
            schema_version: 1,
            pages: vec![],
            profiles: vec![],
            targets: vec![],
        }
    }
}
impl Project {
    pub fn load(dir: &Path) -> Result<Self> {
        let p: Self = serde_json::from_slice(&fs::read(dir.join("project.json"))?)?;
        ensure!(p.schema_version == 1, "Unsupported project version");
        Ok(p)
    }
    pub fn save(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir)?;
        let tmp = dir.join("project.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(tmp, dir.join("project.json"))?;
        Ok(())
    }
    pub fn page(&self, id: &str) -> Result<&Page> {
        self.pages
            .iter()
            .find(|p| p.id == id)
            .with_context(|| format!("Unknown page {id}"))
    }
    pub fn profile(&self, id: &str) -> Result<&Profile> {
        self.profiles
            .iter()
            .find(|p| p.id == id)
            .with_context(|| format!("Unknown profile {id}"))
    }
    pub fn validate_target(&self, t: &Target) -> Result<()> {
        ensure!(
            !t.id.is_empty() && !t.group.is_empty(),
            "Target and group IDs must not be empty"
        );
        let page = self.page(&t.page)?;
        let profile = self.profile(&t.profile)?;
        ensure!(
            profile.page == page.id,
            "Use a profile calibrated for this page (scan scale can differ)"
        );
        ensure!(
            !profile.fits.is_empty(),
            "Profile has no calibrated hypotheses"
        );
        ensure!(
            profile.spacing.as_ref().is_none_or(|s| s
                .samples
                .iter()
                .all(|sample| !sample.bounds.intersects(t.bounds))),
            "Target overlaps spacing calibration/validation/audit text; exclude it and recalibrate"
        );
        ensure!(
            profile.neural_search.as_ref().is_none_or(|n| n
                .inputs
                .iter()
                .all(|input| input.samples.iter().all(|s| !s.bounds.intersects(t.bounds)))),
            "Target overlaps neural font-selection pixels; exclude it and recalibrate"
        );
        for fit in &profile.fits {
            ensure!(
                fit.measurement_mode.valid()
                    && fit.size.is_finite()
                    && fit.size > 0.0
                    && fit.tracking.is_finite()
                    && fit.ink_bias.is_finite()
                    && fit.spacing.as_ref().is_none_or(|s| s.valid())
                    && fit.validation_rmse.is_finite()
                    && fit.validation_rmse >= 0.0
                    && fit.audit.as_ref().is_none_or(|a| a.words >= 4
                        && a.rmse.is_finite()
                        && a.rmse >= 0.0
                        && a.max_error.is_finite()
                        && a.max_error >= a.rmse)
                    && fit.corrections.values().all(|v| v.is_finite()),
                "Invalid profile parameters"
            );
        }
        ensure!(
            !matches!(
                t.evidence,
                Evidence::Mosaic { .. } | Evidence::MosaicAuto { .. }
            ) || profile
                .fits
                .iter()
                .all(|f| f.spacing.as_ref().is_none_or(|s| s.adjustment == 0.0)),
            "Learned space adjustments currently support width evidence; use a profile without --learn-spaces for mosaic rendering"
        );
        ensure!(
            !matches!(
                t.evidence,
                Evidence::Mosaic { .. } | Evidence::MosaicAuto { .. }
            ) || profile
                .fits
                .iter()
                .all(|f| f.measurement_mode == crate::native::MeasurementMode::Design),
            "Hinted measurement profiles support width evidence; mosaic synthesis requires design metrics"
        );
        if matches!(
            t.evidence,
            Evidence::Opaque { .. }
                | Evidence::OpaqueUniform { .. }
                | Evidence::ScanSpan { .. }
                | Evidence::Mosaic { .. }
                | Evidence::MosaicAuto { .. }
        ) {
            ensure!(
                profile
                    .fits
                    .iter()
                    .all(|f| f.width_metric == crate::calibration::WidthMetric::Ink),
                "Image observations require an OCR/ink profile, not a native advance profile"
            );
        }
        ensure!(
            t.bounds.valid()
                && t.bounds.x + t.bounds.w <= page.width as f64
                && t.bounds.y + t.bounds.h <= page.height as f64,
            "Target outside page"
        );
        match t.evidence {
            Evidence::ScanSpan {
                left_word,
                right_word,
                measurement_sigma,
                ..
            } => {
                crate::scan_span::validate(page, t, left_word, right_word)?;
                ensure!(
                    measurement_sigma.is_finite() && measurement_sigma > 0.0,
                    "Invalid scan-span uncertainty"
                );
            }
            Evidence::MosaicAuto {
                min_block,
                max_block,
                origin_x,
                baseline_y,
                noise_sigma,
            } => ensure!(
                (2..=64).contains(&min_block)
                    && (min_block..=64).contains(&max_block)
                    && origin_x.is_finite()
                    && baseline_y.is_finite()
                    && noise_sigma.is_finite()
                    && noise_sigma > 0.0,
                "Invalid automatic mosaic parameters"
            ),
            Evidence::Opaque {
                padding_min,
                padding_max,
                measurement_sigma,
            }
            | Evidence::OpaqueUniform {
                padding_min,
                padding_max,
                measurement_sigma,
            } => ensure!(
                padding_min.is_finite()
                    && padding_max.is_finite()
                    && padding_min >= 0.0
                    && padding_max >= padding_min
                    && padding_max < t.bounds.w
                    && measurement_sigma.is_finite()
                    && measurement_sigma > 0.0,
                "Invalid padding or uncertainty"
            ),
            Evidence::Width {
                width,
                measurement_sigma,
            } => ensure!(
                width.is_finite()
                    && width > 0.0
                    && measurement_sigma.is_finite()
                    && measurement_sigma > 0.0,
                "Invalid width or uncertainty"
            ),
            Evidence::NativeSpan {
                left_anchor,
                right_anchor,
                ref prefix,
                ref suffix,
                measurement_sigma_points,
            } => {
                ensure!(
                    !page.likely_scan,
                    "Native anchors are disabled for scans/OCR overlays"
                );
                ensure!(
                    profile
                        .fits
                        .iter()
                        .all(|f| f.width_metric == crate::calibration::WidthMetric::Advance),
                    "Native spans require a profile fitted with --native"
                );
                let left = page
                    .native_chars
                    .get(left_anchor)
                    .context("Invalid left anchor index")?;
                let right = page
                    .native_chars
                    .get(right_anchor)
                    .context("Invalid right anchor index")?;
                ensure!(
                    right_anchor > left_anchor
                        && right.x1 > left.x1
                        && (right.y1 - left.y1).abs() < left.size.max(1.0) * 0.2,
                    "Anchors must be ordered on the same horizontal line"
                );
                ensure!(
                    left.font == right.font && (left.size - right.size).abs() < 0.01,
                    "Anchors must use the same font and size"
                );
                ensure!(
                    prefix.starts_with(left.text) && suffix.ends_with(char::is_whitespace),
                    "Prefix must start with the left anchor character; suffix must end with whitespace before the right anchor"
                );
                ensure!(
                    measurement_sigma_points.is_finite() && measurement_sigma_points > 0.0,
                    "Invalid native measurement uncertainty"
                );
            }
            Evidence::Mosaic {
                block,
                phase_x,
                phase_y,
                origin_x,
                baseline_y,
                noise_sigma,
            } => ensure!(
                (1..=128).contains(&block)
                    && phase_x < block
                    && phase_y < block
                    && origin_x.is_finite()
                    && baseline_y.is_finite()
                    && noise_sigma.is_finite()
                    && noise_sigma > 0.0,
                "Invalid mosaic parameters"
            ),
        }
        Ok(())
    }
}
/// Build native advance-width observations; all output coordinates use image pixels.
/// Horizontal text only. PDF/OCR classification is checked by the caller.
pub fn native_words(page: &Page) -> Vec<Word> {
    let scale = page.dpi / 72.0;
    let mut words = vec![];
    let mut run: Vec<&NativeChar> = vec![];
    fn flush(run: &mut Vec<&NativeChar>, words: &mut Vec<Word>, scale: f64) {
        if let (Some(first), Some(last)) = (run.first(), run.last()) {
            let bounds = Rect {
                x: first.x1 * scale,
                y: first.y1 * scale,
                w: (last.x2 - first.x1) * scale,
                h: (first.y2 - first.y1) * scale,
            };
            if bounds.valid() {
                words.push(Word {
                    text: run.iter().map(|c| c.text).collect(),
                    bounds,
                    confidence: 100.0,
                    block: format!("{}:{:.3}", first.font, first.size),
                    line: format!("{:.3}", first.y1),
                });
            }
        }
        run.clear();
    }
    for ch in &page.native_chars {
        if ch.text.is_whitespace() {
            flush(&mut run, &mut words, scale);
            continue;
        }
        if run.last().is_some_and(|p| {
            p.font != ch.font
                || (p.size - ch.size).abs() > 0.01
                || (p.y1 - ch.y1).abs() > 0.5
                || ch.x1 < p.x1
        }) {
            flush(&mut run, &mut words, scale);
        }
        run.push(ch);
    }
    flush(&mut run, &mut words, scale);
    words.retain(|w| {
        !page
            .proposed_redactions
            .iter()
            .any(|b| b.intersects(w.bounds))
    });
    words
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn ocr(path: &Path) -> Result<Vec<Word>> {
    ocr_with_tessdata(path, None)
}

/// Reject small solid glyph fragments and rules using nearby text scale.
/// Explicit user masks are not passed through this proposal-only filter.
pub fn plausible_redactions(proposals: &[Rect], words: &[Word]) -> Vec<Rect> {
    proposals
        .iter()
        .copied()
        .filter(|r| {
            let mut neighbors: Vec<_> = words
                .iter()
                .filter(|w| {
                    w.confidence.is_finite()
                        && w.confidence >= 80.0
                        && w.bounds.valid()
                        && w.text.chars().any(char::is_alphanumeric)
                        && !w.bounds.intersects(*r)
                        && (w.bounds.y + w.bounds.h * 0.5 - (r.y + r.h * 0.5)).abs() <= w.bounds.h
                })
                .filter_map(|w| {
                    let gap = (w.bounds.x - (r.x + r.w))
                        .max(r.x - (w.bounds.x + w.bounds.w))
                        .max(0.0);
                    (gap <= w.bounds.h * 12.0).then_some((gap, w.bounds.h))
                })
                .collect();
            neighbors.sort_by(|a, b| a.0.total_cmp(&b.0));
            neighbors.truncate(8);
            if neighbors.len() < 3 {
                return true;
            }
            let mut heights: Vec<_> = neighbors.iter().map(|(_, h)| *h).collect();
            heights.sort_by(f64::total_cmp);
            r.h >= heights[heights.len() / 2] * 0.4
        })
        .collect()
}

fn ocr_with_tessdata(path: &Path, tessdata: Option<&Path>) -> Result<Vec<Word>> {
    let mut command = Command::new("tesseract");
    command.arg(path).args(["stdout", "--psm", "3"]);
    if let Some(directory) = tessdata {
        command
            .args(["--oem", "1", "--tessdata-dir"])
            .arg(directory)
            .args(["-l", "eng", "-c", "tessedit_create_tsv=1"]);
    } else {
        command.arg("tsv");
    }
    let out = command
        .output()
        .context("Tesseract is required for visible-text calibration")?;
    ensure!(
        out.status.success(),
        "OCR failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout)?;
    ensure!(
        text.starts_with("level\tpage_num\t"),
        "Tesseract did not produce TSV: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    parse_tsv(&text)
}

/// Re-OCR visible text with known redaction bars blanked in an auxiliary image.
/// Original page pixels remain the only source of measured ink bounds.
pub fn refresh_visible_ocr(dir: &Path, page_id: &str, masks: &[Rect]) -> Result<()> {
    refresh_visible_ocr_with_tessdata(dir, page_id, masks, None)
}

pub fn refresh_visible_ocr_with_tessdata(
    dir: &Path,
    page_id: &str,
    masks: &[Rect],
    tessdata: Option<&Path>,
) -> Result<()> {
    let model = tessdata
        .map(|directory| -> Result<_> {
            let directory = directory.canonicalize()?;
            let sha256 = hash(&fs::read(directory.join("eng.traineddata"))?);
            Ok((directory, sha256))
        })
        .transpose()?;
    let mut project = Project::load(dir)?;
    ensure!(
        !project.profiles.iter().any(|p| p.page == page_id)
            && !project.targets.iter().any(|t| t.page == page_id),
        "Refresh OCR before calibrating or registering targets on this page"
    );
    let page = project
        .pages
        .iter_mut()
        .find(|p| p.id == page_id)
        .context("Unknown OCR page")?;
    let original = GrayImage::read(&dir.join(&page.image))?;
    ensure!(
        hash(&original.pixels) == page.image_sha256,
        "Page image hash mismatch"
    );
    let mut regions = plausible_redactions(&page.proposed_redactions, &page.words);
    regions.extend_from_slice(masks);
    let input_regions = regions.clone();
    regions = regions
        .into_iter()
        .map(|r| original.ocr_mask_bounds(r))
        .collect::<Result<Vec<_>>>()?;
    let mut masked = original.clone();
    for r in &regions {
        ensure!(
            r.valid() && r.x + r.w <= original.width as f64 && r.y + r.h <= original.height as f64,
            "OCR mask outside page"
        );
        for y in r.y.floor() as usize..(r.y + r.h).ceil() as usize {
            masked.pixels[y * original.width + r.x.floor() as usize
                ..y * original.width + (r.x + r.w).ceil() as usize]
                .fill(255);
        }
    }
    let output_dir = dir.join("ocr-inputs");
    fs::create_dir_all(&output_dir)?;
    let input_id = hash(&masked.pixels);
    let model_suffix = model
        .as_ref()
        .map_or(String::new(), |(_, sha)| format!("-{sha}"));
    let output = output_dir.join(format!("{input_id}{model_suffix}.pgm"));
    masked.save(&output)?;
    let mut words = ocr_with_tessdata(
        &output,
        model.as_ref().map(|(directory, _)| directory.as_path()),
    )?;
    if let Some((directory, expected)) = &model {
        ensure!(
            &hash(&fs::read(directory.join("eng.traineddata"))?) == expected,
            "OCR model changed during recognition"
        );
    }
    words.retain(|w| !regions.iter().any(|r| r.intersects(w.bounds)));
    for word in &mut words {
        if let Ok(crop) = original.crop(word.bounds)
            && let Some(ink) = crop.ink_bounds(160)
        {
            word.bounds = Rect {
                x: word.bounds.x + ink.x,
                y: word.bounds.y + ink.y,
                w: ink.w,
                h: ink.h,
            };
        }
    }
    fs::write(
        output.with_extension("json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "page": page_id, "original_pixel_sha256": page.image_sha256,
            "ocr_pixel_sha256": hash(&masked.pixels), "masks": regions,
            "input_masks": input_regions,
            "model": model.as_ref().map(|(directory, sha256)| serde_json::json!({
                "directory": directory, "eng_traineddata_sha256": sha256, "oem": 1, "language": "eng" })),
            "words": words, "method": "tesseract psm 3; explicit LSTM English model when supplied, otherwise system defaults; dense bar fringes removed in auxiliary input; ink refined on original pixels"
        }))?,
    )?;
    page.words = words;
    page.proposed_redactions = regions;
    project.save(dir)
}
pub fn parse_tsv(tsv: &str) -> Result<Vec<Word>> {
    let mut words = vec![];
    for line in tsv.lines().skip(1) {
        let c: Vec<&str> = line.splitn(12, '\t').collect();
        if c.len() != 12 || c[0] != "5" || c[11].trim().is_empty() {
            continue;
        }
        let bounds = Rect {
            x: c[6].parse()?,
            y: c[7].parse()?,
            w: c[8].parse()?,
            h: c[9].parse()?,
        };
        if !bounds.valid() {
            continue;
        }
        words.push(Word {
            text: c[11].to_owned(),
            bounds,
            confidence: c[10].parse()?,
            block: format!("{}:{}", c[2], c[3]),
            line: format!("{}:{}:{}", c[2], c[3], c[4]),
        });
    }
    Ok(words)
}
pub fn import(dir: &Path, files: &[PathBuf], dpi: f64) -> Result<Project> {
    ensure!((36.0..=1200.0).contains(&dpi), "DPI must be 36–1200");
    fs::create_dir_all(dir.join("pages"))?;
    let mut project = if dir.join("project.json").exists() {
        Project::load(dir)?
    } else {
        Project::default()
    };
    for file in files {
        let source = file.canonicalize()?;
        let source_hash = hash(&fs::read(&source)?);
        let pdf = if file
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("pdf"))
        {
            Some(Pdf::open(&source)?)
        } else {
            None
        };
        let count = pdf.as_ref().map_or(1, Pdf::pages);
        for index in 0..count {
            let id = format!("{}-{}", &source_hash[..12], index + 1);
            if let Some(existing) = project.pages.iter().find(|p| p.id == id) {
                ensure!(
                    (existing.dpi - dpi).abs() < 0.01,
                    "This source was imported at another DPI. Use a new project to change rendering resolution."
                );
                continue;
            }
            eprintln!("Importing {} page {}/{}", file.display(), index + 1, count);
            let image = match &pdf {
                Some(p) => p.render(index, dpi)?,
                None => GrayImage::read(&source)?,
            };
            let native = match &pdf {
                Some(p) => p.chars(index)?,
                None => (vec![], 1.0),
            };
            let rel = PathBuf::from(format!("pages/{id}.pgm"));
            let output = dir.join(&rel);
            image.save(&output)?;
            let raw_proposals = image.rectangles();
            let mut words = ocr(&output)?;
            let proposals = plausible_redactions(&raw_proposals, &words);
            fs::write(
                dir.join(format!("pages/{id}-mask-proposals.json")),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "raw": raw_proposals, "retained": proposals,
                    "method": "Reject proposals below 40% of median ink height of at least three nearby reliable words; explicit masks bypass this filter"
                }))?,
            )?;
            // OCR text inside a black bar is not a training observation.
            words.retain(|w| !proposals.iter().any(|b| b.intersects(w.bounds)));
            // Refine OCR ink extents against original pixels; retain OCR's line/block grouping.
            for word in &mut words {
                if let Ok(crop) = image.crop(word.bounds)
                    && let Some(ink) = crop.ink_bounds(160)
                {
                    word.bounds = Rect {
                        x: word.bounds.x + ink.x,
                        y: word.bounds.y + ink.y,
                        w: ink.w,
                        h: ink.h,
                    };
                }
            }
            project.pages.push(Page {
                id,
                source: source.clone(),
                source_sha256: source_hash.clone(),
                page_number: index + 1,
                dpi,
                width: image.width,
                height: image.height,
                image: rel,
                image_sha256: hash(&image.pixels),
                likely_scan: native.1 > 0.7 || native.0.is_empty(),
                image_coverage: native.1,
                native_chars: native.0,
                words,
                proposed_redactions: proposals,
            });
            project.save(dir)?;
        }
    }
    Ok(project)
}
