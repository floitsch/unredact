// Copyright (C) 2026 Toit contributors.
//! Attributed candidate text from visible OCR, never from hidden PDF text layers.
use crate::{
    calibration::Word,
    project::{Project, hash},
    raster::{GrayImage, Rect},
    solver::{Candidate, Surface},
};
use anyhow::{Result, ensure};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub min_confidence: f64,
    pub max_words: usize,
    pub max_candidates: usize,
    pub max_sources: usize,
    pub surface_kind: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            min_confidence: 85.0,
            max_words: 3,
            max_candidates: 10_000,
            max_sources: 8,
            surface_kind: "canonical".into(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Input {
    pub project: PathBuf,
    pub project_sha256: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct Origin {
    pub project: PathBuf,
    pub project_sha256: String,
    pub page: String,
    pub document: PathBuf,
    pub source_sha256: String,
    pub page_number: usize,
    pub pixel_sha256: String,
    pub word_indices: Vec<usize>,
    pub raw_tokens: Vec<String>,
    pub bounds: Vec<Rect>,
}
#[derive(Debug, Serialize)]
pub struct Statistics {
    pub distinct_pages: usize,
    pub occurrences: usize,
    pub omitted_origins: usize,
}
#[derive(Debug, Default, Serialize)]
pub struct Counts {
    pub input_pages: usize,
    pub unique_pixel_pages: usize,
    pub duplicate_pages: usize,
    pub observed_words: usize,
    pub duplicate_words: usize,
    pub accepted_words: usize,
    pub excluded_words: usize,
    pub unique_candidates: usize,
    pub omitted_candidates: usize,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub candidates: Vec<Candidate>,
    pub origins: BTreeMap<String, Origin>,
    pub statistics: BTreeMap<String, Statistics>,
    pub inputs: Vec<Input>,
    pub config: Config,
    pub counts: Counts,
    pub interpretation: String,
}

struct Collected {
    text: String,
    pages: BTreeSet<String>,
    occurrences: usize,
    origins: BTreeMap<String, Origin>,
}

/// Trim punctuation at word edges, preserving case and internal apostrophes,
/// hyphens and abbreviation periods. No spelling or line-hyphen repair is made.
fn token(raw: &str) -> Option<(&str, bool, bool)> {
    if raw.is_empty() || raw.len() > 128 || raw.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    if raw.starts_with(['-', '‐', '‑']) || raw.ends_with(['-', '‐', '‑']) {
        return None;
    }
    let text = raw.trim_matches(|c: char| !c.is_alphanumeric());
    if text.is_empty()
        || !text
            .chars()
            .all(|c| c.is_alphanumeric() || "'-’.‐‑".contains(c))
    {
        return None;
    }
    Some((text, !raw.starts_with(text), !raw.ends_with(text)))
}

fn span(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Rect {
        x,
        y,
        w: (a.x + a.w).max(b.x + b.w) - x,
        h: (a.y + a.h).max(b.y + b.h) - y,
    }
}
fn adjacent(a: &Word, b: &Word, masks: &[Rect]) -> bool {
    let (a, b) = (a.bounds, b.bounds);
    let overlap = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    let gap = b.x - a.x - a.w;
    gap >= 0.0
        && gap <= a.h.max(b.h) * 2.0
        && overlap >= a.h.min(b.h) * 0.5
        && a.h.min(b.h) >= a.h.max(b.h) * 0.5
        && !masks.iter().any(|m| m.intersects(span(a, b)))
}

pub fn run(directories: &[PathBuf], config: &Config) -> Result<Report> {
    ensure!(!directories.is_empty(), "Supply at least one project");
    ensure!(
        config.min_confidence.is_finite()
            && (0.0..=100.0).contains(&config.min_confidence)
            && (1..=6).contains(&config.max_words)
            && (1..=100_000).contains(&config.max_candidates)
            && (1..=32).contains(&config.max_sources)
            && !config.surface_kind.is_empty()
            && config.surface_kind.len() <= 128
            && config.surface_kind != "any"
            && !config.surface_kind.chars().any(char::is_control),
        "Invalid harvesting limits or surface kind"
    );
    let mut paths = BTreeSet::new();
    for path in directories {
        paths.insert(path.canonicalize()?);
    }
    let mut inputs = vec![];
    let mut projects = vec![];
    let mut counts = Counts::default();
    // Identical pixels may have different targets or OCR in different projects.
    // Union all their masks before choosing one deterministic OCR copy.
    let mut groups: BTreeMap<(String, usize, usize), Vec<(usize, usize)>> = BTreeMap::new();
    for path in paths {
        let file = path.join("project.json");
        ensure!(
            fs::metadata(&file)?.len() <= 128_000_000,
            "Project exceeds 128 MB"
        );
        let raw = fs::read(file)?;
        let project: Project = serde_json::from_slice(&raw)?;
        ensure!(project.schema_version == 1, "Unsupported project version");
        let mut page_ids = BTreeSet::new();
        for (index, page) in project.pages.iter().enumerate() {
            ensure!(page_ids.insert(&page.id), "Duplicate page ID in project");
            counts.input_pages += 1;
            counts.observed_words += page.words.len();
            ensure!(
                counts.input_pages <= 10_000 && counts.observed_words <= 1_000_000,
                "Corpus exceeds 10,000 pages or 1,000,000 OCR observations; split the input"
            );
            let image = GrayImage::read(&path.join(&page.image))?;
            ensure!(
                image.width == page.width
                    && image.height == page.height
                    && hash(&image.pixels) == page.image_sha256,
                "Page pixels changed: {}; refresh OCR before harvesting",
                page.id
            );
            groups
                .entry((page.image_sha256.clone(), page.width, page.height))
                .or_default()
                .push((projects.len(), index));
        }
        inputs.push(Input {
            project: path,
            project_sha256: hash(&raw),
        });
        projects.push(project);
    }
    counts.unique_pixel_pages = groups.len();
    counts.duplicate_pages = counts.input_pages - groups.len();
    let mut collected: BTreeMap<String, Collected> = BTreeMap::new();
    for ((pixels, _, _), mut copies) in groups {
        copies.sort_by(|&(pa, ia), &(pb, ib)| {
            inputs[pa]
                .project
                .cmp(&inputs[pb].project)
                .then(projects[pa].pages[ia].id.cmp(&projects[pb].pages[ib].id))
        });
        counts.duplicate_words += copies
            .iter()
            .skip(1)
            .map(|&(pi, wi)| projects[pi].pages[wi].words.len())
            .sum::<usize>();
        let mut masks = vec![];
        for &(pi, wi) in &copies {
            let page = &projects[pi].pages[wi];
            masks.extend(page.proposed_redactions.iter().copied());
            masks.extend(
                projects[pi]
                    .targets
                    .iter()
                    .filter(|t| t.page == page.id)
                    .map(|t| t.bounds),
            );
        }
        ensure!(
            masks.iter().all(|m| m.valid()),
            "Invalid redaction rectangle in corpus"
        );
        let (pi, wi) = copies[0];
        let page = &projects[pi].pages[wi];
        let image = GrayImage::read(&inputs[pi].project.join(&page.image))?;
        ensure!(
            hash(&image.pixels) == pixels,
            "Page changed during harvesting"
        );
        let valid_bounds = |b: Rect| {
            b.valid() && b.x + b.w <= page.width as f64 && b.y + b.h <= page.height as f64
        };
        let mut tokens = vec![];
        let mut seen_occurrences = BTreeSet::new();
        let mut lines: BTreeMap<(&str, &str), Vec<usize>> = BTreeMap::new();
        for (i, word) in page.words.iter().enumerate() {
            let candidate = token(&word.text).filter(|_| {
                word.confidence.is_finite()
                    && word.confidence >= config.min_confidence
                    && word.confidence <= 100.0
                    && valid_bounds(word.bounds)
                    && !masks.iter().any(|m| m.intersects(word.bounds))
            });
            let candidate = if candidate.is_some() {
                let crop = image.crop(word.bounds)?;
                let ink = crop.pixels.iter().filter(|&&p| p < 160).count();
                // Reject empty patches and almost-solid bars even if OCR
                // assigned a confident label and detection missed the bar.
                candidate.filter(|_| ink > 0 && (ink as f64) < crop.pixels.len() as f64 * 0.95)
            } else {
                None
            };
            counts.accepted_words += usize::from(candidate.is_some());
            counts.excluded_words += usize::from(candidate.is_none());
            tokens.push(candidate);
            lines.entry((&word.block, &word.line)).or_default().push(i);
        }
        for ((block, line), mut indices) in lines {
            let allow_phrases = !block.is_empty()
                && !line.is_empty()
                && indices.iter().all(|&i| valid_bounds(page.words[i].bounds));
            indices.sort_by(|&a, &b| {
                page.words[a]
                    .bounds
                    .x
                    .total_cmp(&page.words[b].bounds.x)
                    .then(a.cmp(&b))
            });
            for start in 0..indices.len() {
                let mut text = String::new();
                let mut chosen = vec![];
                for &i in indices[start..].iter().take(config.max_words) {
                    let Some((word, boundary_before, boundary_after)) = tokens[i] else {
                        break;
                    };
                    if let Some(&previous) = chosen.last() {
                        if !allow_phrases
                            || boundary_before
                            || !adjacent(&page.words[previous], &page.words[i], &masks)
                        {
                            break;
                        }
                        text.push(' ');
                    }
                    text.push_str(word);
                    if text.len() > 512 {
                        break;
                    }
                    chosen.push(i);
                    let origin = Origin {
                        project: inputs[pi].project.clone(),
                        project_sha256: inputs[pi].project_sha256.clone(),
                        page: page.id.clone(),
                        document: page.source.clone(),
                        source_sha256: page.source_sha256.clone(),
                        page_number: page.page_number,
                        pixel_sha256: pixels.clone(),
                        word_indices: chosen.clone(),
                        raw_tokens: chosen.iter().map(|&n| page.words[n].text.clone()).collect(),
                        bounds: chosen.iter().map(|&n| page.words[n].bounds).collect(),
                    };
                    let source_id = format!(
                        "visible-origin-{}",
                        hash(&serde_json::to_vec(&(
                            &pixels,
                            &origin.bounds,
                            &origin.raw_tokens,
                            &text
                        ))?)
                    );
                    if !seen_occurrences.insert(source_id.clone()) {
                        if boundary_after {
                            break;
                        }
                        continue;
                    }
                    let entry = collected.entry(text.clone()).or_insert_with(|| Collected {
                        text: text.clone(),
                        pages: BTreeSet::new(),
                        occurrences: 0,
                        origins: BTreeMap::new(),
                    });
                    entry.pages.insert(pixels.clone());
                    entry.occurrences += 1;
                    entry.origins.insert(source_id, origin);
                    if entry.origins.len() > config.max_sources {
                        entry.origins.pop_last();
                    }
                    ensure!(
                        collected.len() <= 100_000,
                        "More than 100,000 distinct candidates; reduce --max-words or split the corpus"
                    );
                    if boundary_after {
                        break;
                    }
                }
            }
        }
    }
    counts.unique_candidates = collected.len();
    let mut ranked: Vec<_> = collected.into_values().collect();
    ranked.sort_by(|a, b| {
        b.pages
            .len()
            .cmp(&a.pages.len())
            .then(b.occurrences.cmp(&a.occurrences))
            .then(a.text.cmp(&b.text))
    });
    counts.omitted_candidates = ranked.len().saturating_sub(config.max_candidates);
    ranked.truncate(config.max_candidates);
    let mut candidates = vec![];
    let mut origins = BTreeMap::new();
    let mut statistics = BTreeMap::new();
    for entry in ranked {
        let id = format!(
            "visible-{}",
            hash(&serde_json::to_vec(&(&config.surface_kind, &entry.text))?)
        );
        statistics.insert(
            id.clone(),
            Statistics {
                distinct_pages: entry.pages.len(),
                occurrences: entry.occurrences,
                omitted_origins: entry.occurrences.saturating_sub(entry.origins.len()),
            },
        );
        candidates.push(Candidate {
            id,
            canonical_name: entry.text.clone(),
            surfaces: vec![Surface {
                text: entry.text,
                kind: config.surface_kind.clone(),
            }],
            source_ids: entry.origins.keys().cloned().collect(),
            context_score: None,
        });
        origins.extend(entry.origins);
    }
    Ok(Report { schema_version: 1, candidates, origins, statistics, inputs, config: config.clone(), counts,
        interpretation: "Candidates copied from pixel-verified visible OCR with edge punctuation trimmed, case preserved and no spelling repair. Registered/detected masks from all identical page copies are excluded; native PDF text layers are not read. Phrases stay within one reliable OCR line and do not cross masks, sentence punctuation or large gaps. Frequency counts unique pixel pages, not independent documents or confidence. Candidate/source caps and duplicate-page handling are explicit. Visible repetition suggests text to test; it does not prove what was behind a redaction.".into() })
}

pub fn write(report: &Report, output: Option<&Path>) -> Result<()> {
    let json = serde_json::to_string_pretty(report)?;
    if let Some(path) = output {
        fs::write(path, json)?;
    } else {
        println!("{json}");
    }
    Ok(())
}
