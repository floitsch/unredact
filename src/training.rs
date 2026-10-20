// Copyright (C) 2026 Toit contributors.
//! Visible-text span examples with pixel-page splits and explicit OCR provenance.
use crate::{
    contextual::Context,
    harvest,
    project::{Evidence, Project, Target, hash},
    raster::Rect,
};
use anyhow::{Result, ensure};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub max_train: usize,
    pub max_validation: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            max_train: 4000,
            max_validation: 512,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Example {
    pub id: String,
    pub split: String,
    pub text: String,
    pub context: Context,
    pub origin: harvest::Origin,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub examples: Vec<Example>,
    pub inputs: Vec<harvest::Input>,
    pub harvest_config: harvest::Config,
    pub harvest_counts: harvest::Counts,
    pub config: Config,
    pub excluded_empty_context: usize,
    pub duplicate_examples: usize,
    pub omitted_train: usize,
    pub omitted_validation: usize,
    pub interpretation: String,
}

pub fn run(
    directories: &[PathBuf],
    harvest_config: &harvest::Config,
    config: &Config,
) -> Result<Report> {
    ensure!(
        (1..=100_000).contains(&config.max_train) && (1..=100_000).contains(&config.max_validation),
        "Training and validation example limits must be 1–100,000"
    );
    let harvested = harvest::run(directories, harvest_config)?;
    let mut projects = BTreeMap::new();
    let mut masks: BTreeMap<String, Vec<Rect>> = BTreeMap::new();
    for input in &harvested.inputs {
        ensure!(
            hash(&fs::read(input.project.join("project.json"))?) == input.project_sha256,
            "Training source changed after harvesting"
        );
        let project = Project::load(&input.project)?;
        for page in &project.pages {
            let excluded = masks.entry(page.image_sha256.clone()).or_default();
            excluded.extend(&page.proposed_redactions);
            excluded.extend(
                project
                    .targets
                    .iter()
                    .filter(|t| t.page == page.id)
                    .map(|t| t.bounds),
            );
        }
        projects.insert(input.project.clone(), project);
    }
    for project in projects.values_mut() {
        for page in &mut project.pages {
            page.proposed_redactions = masks[&page.image_sha256].clone();
            // Keep unreliable words as barriers, rather than joining across them.
            for word in &mut page.words {
                if !word.confidence.is_finite() || word.confidence < harvest_config.min_confidence {
                    word.confidence = 0.;
                }
            }
        }
    }
    let mut examples = vec![];
    let mut excluded_empty_context = 0;
    for (id, origin) in harvested.origins {
        let project = projects.get_mut(&origin.project).unwrap();
        ensure!(!origin.bounds.is_empty(), "Missing training span bounds");
        let x = origin
            .bounds
            .iter()
            .map(|b| b.x)
            .fold(f64::INFINITY, f64::min);
        let y = origin
            .bounds
            .iter()
            .map(|b| b.y)
            .fold(f64::INFINITY, f64::min);
        let right = origin.bounds.iter().map(|b| b.x + b.w).fold(0., f64::max);
        let bottom = origin.bounds.iter().map(|b| b.y + b.h).fold(0., f64::max);
        let bounds = Rect {
            x,
            y,
            w: right - x,
            h: bottom - y,
        };
        let target = Target {
            id: id.clone(),
            page: origin.page.clone(),
            bounds,
            profile: "unused-training-context".into(),
            group: "training-context".into(),
            surface_kind: "canonical".into(),
            evidence: Evidence::Width {
                width: bounds.w,
                measurement_sigma: 1.,
            },
            left_context: String::new(),
            right_context: String::new(),
            dependence_key: None,
        };
        // The label is visible in this source project. Treat its bounds as an
        // additional mask while walking paragraph continuations, just as a
        // registered real target would be masked during inference.
        let page_index = project
            .pages
            .iter()
            .position(|p| p.id == origin.page)
            .unwrap();
        project.pages[page_index].proposed_redactions.push(bounds);
        let context = crate::context_layout::paragraph_context(project, &target);
        project.pages[page_index].proposed_redactions.pop();
        let context = context?;
        let page = project.page(&origin.page)?;
        for index in context
            .left_word_indices
            .iter()
            .chain(&context.right_word_indices)
        {
            ensure!(
                !origin.word_indices.contains(index)
                    && !page.words[*index].bounds.intersects(bounds)
                    && !masks[&page.image_sha256]
                        .iter()
                        .any(|m| m.intersects(page.words[*index].bounds)),
                "Training context includes a label or masked word"
            );
        }
        if context.left.is_empty() && context.right.is_empty() {
            excluded_empty_context += 1;
            continue;
        }
        // Preserve the punctuation actually covered by the OCR span. Harvested
        // candidate strings trim edge punctuation and are not training labels.
        let text = origin.raw_tokens.join(" ");
        let split_hash = hash(format!("domain-page-split-v1:{}", origin.pixel_sha256).as_bytes());
        let validation = u32::from_str_radix(&split_hash[..8], 16)? % 5 == 0;
        examples.push(Example {
            id: hash(format!("domain-example-v1:{id}").as_bytes()),
            split: if validation { "validation" } else { "train" }.into(),
            text,
            context,
            origin,
        });
    }
    examples.sort_by(|a, b| a.id.cmp(&b.id));
    let mut seen = BTreeSet::new();
    let before = examples.len();
    examples.retain(|e| {
        seen.insert((
            e.context.left.clone(),
            e.context.right.clone(),
            e.text.clone(),
        ))
    });
    let duplicate_examples = before - examples.len();
    let mut counts = [0usize; 2];
    let mut omitted = [0usize; 2];
    examples.retain(|e| {
        let index = usize::from(e.split == "validation");
        let limit = if index == 0 {
            config.max_train
        } else {
            config.max_validation
        };
        counts[index] += 1;
        if counts[index] <= limit {
            true
        } else {
            omitted[index] += 1;
            false
        }
    });
    // Recheck source metadata after context extraction; page pixels were already
    // verified by harvesting, which supplies the visible span origins.
    for input in &harvested.inputs {
        ensure!(
            hash(&fs::read(input.project.join("project.json"))?) == input.project_sha256,
            "Training source changed during context extraction"
        );
    }
    Ok(Report { schema_version: 1, examples, inputs: harvested.inputs,
        harvest_config: harvest_config.clone(), harvest_counts: harvested.counts,
        config: config.clone(), excluded_empty_context, duplicate_examples,
        omitted_train: omitted[0], omitted_validation: omitted[1],
        interpretation: "Self-supervised examples from visible OCR only. Known masks are unioned across duplicate pixel pages; labels and context retain source indices. Train/validation split by pixel-page hash; exact context/label duplicates are removed. Same-document correlations and OCR errors remain. Exclude evaluation pages before export; no hidden text or truth labels are consumed.".into() })
}
