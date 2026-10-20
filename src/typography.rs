// Copyright (C) 2026 Toit contributors.
//! Size grouping from training text only. Normalizing by each word's expected
//! glyph height avoids confusing ascenders/descenders with different point sizes.
//! Groups are size hypotheses, not font/style classifications.
use crate::{
    calibration::{Word, held_out},
    native::Font,
    raster::Rect,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Line {
    pub block: String,
    pub line: String,
}
impl Line {
    pub fn of(word: &Word) -> Self {
        Self {
            block: word.block.clone(),
            line: word.line.clone(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SizeGroup {
    pub id: String,
    pub normalized_height: f64,
    pub min_height: f64,
    pub max_height: f64,
    pub training_observations: usize,
    pub lines: Vec<Line>,
}
impl SizeGroup {
    pub fn select(&self, words: &[Word]) -> Vec<Word> {
        let lines: BTreeSet<_> = self.lines.iter().cloned().collect();
        words
            .iter()
            .filter(|w| lines.contains(&Line::of(w)))
            .cloned()
            .collect()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub groups: Vec<SizeGroup>,
    pub unassigned_lines: Vec<Line>,
    pub reference_fonts: Vec<PathBuf>,
    pub max_height_ratio: f64,
    pub interpretation: String,
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    (values[(values.len() - 1) / 2] + values[values.len() / 2]) * 0.5
}

pub fn group(words: &[Word], font_paths: &[PathBuf], excluded: &[Rect]) -> Result<Report> {
    ensure!(
        !font_paths.is_empty(),
        "Size grouping needs reference fonts"
    );
    let fonts: Vec<_> = font_paths
        .iter()
        .map(|p| Font::open(p))
        .collect::<Result<_>>()?;
    let mut lines: BTreeMap<Line, BTreeMap<&str, f64>> = BTreeMap::new();
    for word in words {
        let samples = lines.entry(Line::of(word)).or_default();
        if held_out(&word.text)
            || word.confidence < 85.0
            || !word.confidence.is_finite()
            || !word.bounds.valid()
            || word.text.chars().count() < 3
            || word.text.len() > 128
            || excluded.iter().any(|r| r.intersects(word.bounds))
        {
            continue;
        }
        let mut heights: Vec<_> = fonts
            .iter()
            .filter_map(|font| font.measure(&word.text, true).ok())
            .map(|m| m.bottom - m.top)
            .filter(|h| h.is_finite() && *h > 0.05)
            .collect();
        if heights.is_empty() {
            continue;
        }
        let height = word.bounds.h / median(&mut heights);
        if height.is_finite() && height > 0.0 {
            samples.entry(&word.text).or_insert(height);
        }
    }
    let mut qualified = vec![];
    let mut unassigned_lines = vec![];
    for (line, samples) in lines {
        if samples.len() < 3 {
            unassigned_lines.push(line);
            continue;
        }
        let mut values: Vec<_> = samples.into_values().collect();
        qualified.push((median(&mut values), line, values.len()));
    }
    qualified.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let ratio = 1.10;
    let mut clusters: Vec<Vec<(f64, Line, usize)>> = vec![];
    for observation in qualified {
        if clusters
            .last()
            .is_none_or(|c| observation.0 > c[0].0 * ratio)
        {
            clusters.push(vec![]);
        }
        clusters.last_mut().unwrap().push(observation);
    }
    let groups = clusters
        .into_iter()
        .enumerate()
        .map(|(i, cluster)| {
            let mut heights: Vec<_> = cluster.iter().map(|o| o.0).collect();
            SizeGroup {
                id: format!("size-{i}"),
                normalized_height: median(&mut heights),
                min_height: heights[0],
                max_height: *heights.last().unwrap(),
                training_observations: cluster.iter().map(|o| o.2).sum(),
                lines: cluster.into_iter().map(|o| o.1).collect(),
            }
        })
        .collect();
    Ok(Report { groups, unassigned_lines, reference_fonts: font_paths.to_vec(), max_height_ratio: ratio,
        interpretation: "Training-only line size hypotheses; same-size font families/styles may remain mixed. Lines with fewer than three distinct training words are unassigned. Heights are normalized image measurements, not exact point sizes.".into() })
}
