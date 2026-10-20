// Copyright (C) 2026 Toit contributors.
//! Optional visible-training-text font proposals, verified by ordinary calibration.
use crate::{
    calibration::{Word, held_out},
    project::{Page, hash},
    raster::{GrayImage, Rect},
    typography::Line,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub python: PathBuf,
    pub script: PathBuf,
    pub model: PathBuf,
    pub catalog: PathBuf,
    pub top_k: usize,
    pub timeout_seconds: u64,
}
impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let mut config: Self = serde_json::from_slice(&fs::read(path)?)?;
        ensure!(
            (1..=32).contains(&config.top_k) && (1..=3600).contains(&config.timeout_seconds),
            "Invalid neural budget"
        );
        let config_path = path.canonicalize()?;
        let base = config_path.parent().context("Config has no parent")?;
        // Resolving a venv's python symlink would bypass its package environment.
        config.python = base.join(&config.python);
        ensure!(
            config.python.is_file(),
            "Neural Python executable is missing"
        );
        for value in [&mut config.script, &mut config.model, &mut config.catalog] {
            *value = base.join(&*value).canonicalize()?;
        }
        Ok(config)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    pub text: String,
    pub bounds: Rect,
    pub line: Line,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Montage {
    pub path: PathBuf,
    pub sha256: String,
    pub samples: Vec<Sample>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    pub id: String,
    pub path: PathBuf,
    pub sha256: String,
    pub score: f64,
    pub per_crop_scores: Vec<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub model_sha256: String,
    pub script_sha256: String,
    pub catalog_sha256: String,
    pub page_image_sha256: String,
    pub request_sha256: String,
    pub response_sha256: String,
    pub inputs: Vec<Montage>,
    pub suggestions: Vec<Suggestion>,
    pub interpretation: String,
}
#[derive(Deserialize)]
struct Reply {
    schema_version: u32,
    status: String,
    model_sha256: String,
    suggestions: Vec<Suggestion>,
    evidence: Vec<Evidence>,
}
#[derive(Deserialize)]
struct Evidence {
    path: PathBuf,
    sha256: String,
}
#[derive(Deserialize)]
struct CatalogFont {
    id: String,
    path: PathBuf,
    sha256: String,
}

/// Only interior pixels of eligible training boxes are copied; no between-word
/// page pixels enter a montage. Reject overlaps with masks or other OCR boxes.
pub fn training_montages(
    image: &GrayImage,
    words: &[Word],
    all_words: &[Word],
    excluded: &[Rect],
) -> Result<Vec<(GrayImage, Vec<Sample>)>> {
    let mut lines: BTreeMap<Line, Vec<(&Word, Rect)>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for w in words {
        if held_out(&w.text)
            || !w.confidence.is_finite()
            || w.confidence < 85.0
            || !w.bounds.valid()
            || w.text.chars().count() < 3
            || w.text.len() > 128
        {
            continue;
        }
        let r = Rect {
            x: w.bounds.x.ceil(),
            y: w.bounds.y.ceil(),
            w: (w.bounds.x + w.bounds.w).floor() - w.bounds.x.ceil(),
            h: (w.bounds.y + w.bounds.h).floor() - w.bounds.y.ceil(),
        };
        if !r.valid()
            || r.x + r.w > image.width as f64
            || r.y + r.h > image.height as f64
            || excluded.iter().any(|e| e.intersects(r))
            || all_words.iter().any(|other| {
                let same = other.text == w.text
                    && other.block == w.block
                    && other.line == w.line
                    && other.bounds.x == w.bounds.x
                    && other.bounds.y == w.bounds.y;
                !same && other.bounds.intersects(r)
            })
            || !seen.insert(w.text.clone())
        {
            continue;
        }
        lines.entry(Line::of(w)).or_default().push((w, r));
    }
    let mut result = vec![];
    for (line, mut entries) in lines {
        entries.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
        for chunk in entries.chunks(3) {
            if chunk.len() < 2 {
                continue;
            }
            let top = chunk.iter().map(|(_, r)| r.y as usize).min().unwrap();
            let bottom = chunk
                .iter()
                .map(|(_, r)| (r.y + r.h) as usize)
                .max()
                .unwrap();
            let width =
                chunk.iter().map(|(_, r)| r.w as usize).sum::<usize>() + 8 * (chunk.len() + 1);
            let mut montage = GrayImage::white(width, bottom - top + 8)?;
            let mut x = 8;
            let mut samples = vec![];
            for (word, r) in chunk {
                let crop = image.crop(*r)?;
                montage.paste(&crop, x, r.y as usize - top + 4)?;
                x += crop.width + 8;
                samples.push(Sample {
                    text: word.text.clone(),
                    bounds: *r,
                    line: line.clone(),
                });
            }
            if montage.pixels.iter().filter(|&&p| p < 180).count() < 5 {
                continue;
            }
            result.push((montage, samples));
            if result.len() == 24 {
                return Ok(result);
            }
        }
    }
    Ok(result)
}

pub fn suggest(
    config: &Config,
    dir: &Path,
    page: &Page,
    words: &[Word],
    excluded: &[Rect],
) -> Result<Report> {
    ensure!(
        (1..=32).contains(&config.top_k) && (1..=3600).contains(&config.timeout_seconds),
        "Invalid neural budget"
    );
    let page_path = dir.join(&page.image);
    let page_bytes = fs::read(&page_path)?;
    let image = GrayImage::from_pgm(&page_bytes)?;
    let page_hash = hash(&image.pixels);
    ensure!(
        page_hash == page.image_sha256,
        "Page image hash changed before neural inference"
    );
    ensure!(
        image.width == page.width && image.height == page.height,
        "Neural page dimensions differ from project"
    );
    let montages = training_montages(&image, words, &page.words, excluded)?;
    ensure!(
        !montages.is_empty(),
        "Neural inference needs at least two distinct eligible training words on one line"
    );
    let output = dir.join("neural-inputs");
    fs::create_dir_all(&output)?;
    let mut inputs = vec![];
    for (image, samples) in montages {
        let mut bytes = format!("P5\n{} {}\n255\n", image.width, image.height).into_bytes();
        bytes.extend(&image.pixels);
        let sha256 = hash(&bytes);
        let path = output.join(format!("{sha256}.pgm"));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => file.write_all(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => ensure!(
                hash(&fs::read(&path)?) == sha256,
                "Stored neural input hash mismatch"
            ),
            Err(e) => return Err(e.into()),
        }
        inputs.push(Montage {
            path: path.canonicalize()?,
            sha256,
            samples,
        });
    }
    let model_sha256 = hash(&fs::read(&config.model)?);
    let script_sha256 = hash(&fs::read(&config.script)?);
    let catalog_bytes = fs::read(&config.catalog)?;
    let catalog_sha256 = hash(&catalog_bytes);
    let catalog: Vec<CatalogFont> = serde_json::from_slice(&catalog_bytes)?;
    ensure!(
        !catalog.is_empty() && catalog.len() <= 1000,
        "Invalid neural font catalog"
    );
    let mut ids = BTreeSet::new();
    let mut canonical_catalog = vec![];
    for font in &catalog {
        ensure!(ids.insert(&font.id), "Duplicate neural catalog ID");
        let path = config
            .catalog
            .parent()
            .unwrap()
            .join(&font.path)
            .canonicalize()?;
        ensure!(
            hash(&fs::read(&path)?) == font.sha256,
            "Neural catalog font hash mismatch"
        );
        canonical_catalog.push(json!({"id":font.id,"path":path,"sha256":font.sha256}));
    }
    let request = json!({"schema_version":1,"model":config.model,"catalog":canonical_catalog,
        "images":inputs.iter().map(|i|&i.path).collect::<Vec<_>>(),"top_k":config.top_k});
    let response = crate::provider::call(
        &config.python,
        &[config.script.to_string_lossy().into()],
        &request,
        Duration::from_secs(config.timeout_seconds),
    )?;
    let reply: Reply =
        serde_json::from_value(response.clone()).context("Invalid neural response")?;
    ensure!(
        reply.schema_version == 1 && reply.status == "ok" && reply.model_sha256 == model_sha256,
        "Unexpected neural model/response"
    );
    ensure!(
        hash(&fs::read(&config.model)?) == model_sha256
            && hash(&fs::read(&config.script)?) == script_sha256
            && hash(&fs::read(&config.catalog)?) == catalog_sha256,
        "Neural artifacts changed during inference"
    );
    ensure!(
        reply.evidence.len() == inputs.len()
            && reply
                .evidence
                .iter()
                .zip(&inputs)
                .all(|(a, b)| a.path == b.path && a.sha256 == b.sha256),
        "Neural evidence does not match training crops"
    );
    ensure!(
        !reply.suggestions.is_empty() && reply.suggestions.len() <= config.top_k,
        "Invalid neural suggestion count"
    );
    let mut seen = BTreeSet::new();
    for s in &reply.suggestions {
        ensure!(
            seen.insert(&s.id)
                && s.score.is_finite()
                && s.score.abs() <= 1.001
                && s.per_crop_scores.len() == inputs.len()
                && s.per_crop_scores
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 1.001),
            "Invalid neural scores"
        );
        ensure!(
            canonical_catalog.iter().any(|f| f["id"] == s.id
                && f["path"] == s.path.to_string_lossy().as_ref()
                && f["sha256"] == s.sha256),
            "Neural suggestion is outside supplied catalog"
        );
        ensure!(
            hash(&fs::read(&s.path)?) == s.sha256,
            "Suggested font changed during inference"
        );
    }
    Ok(Report { model_sha256,script_sha256,catalog_sha256,page_image_sha256:page_hash,
        request_sha256:hash(&serde_json::to_vec(&request)?),response_sha256:hash(&serde_json::to_vec(&response)?),
        inputs,suggestions:reply.suggestions,
        interpretation:"Font proposals from visible training-only word montages; cosine similarities are not probabilities. Suggestions supplement the existing font search. Validation selects physical fits; audit text is withheld. Mixed styles can dilute pooled suggestions.".into() })
}
