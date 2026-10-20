// Copyright (C) 2026 Toit contributors.
//! Matched-observation diagnostic, not original-redaction recovery.
use anyhow::{Context, Result};
use serde_json::json;
use std::{collections::BTreeMap, path::Path};
use unredact::{
    calibration::{self, Word, audit_held_out},
    native::Font,
    project::Project,
    typography,
};

fn unique_audit(words: &[Word]) -> Vec<&Word> {
    let mut unique = BTreeMap::new();
    for w in words.iter().filter(|w| {
        w.confidence >= 85.0
            && w.bounds.valid()
            && w.text.chars().count() >= 3
            && w.text.len() <= 128
            && audit_held_out(&w.text)
    }) {
        unique.entry(&w.text).or_insert(w);
    }
    unique.into_values().collect()
}

fn main() -> Result<()> {
    let dir = std::env::args()
        .nth(1)
        .context("usage: audit_typography PROJECT_DIRECTORY")?;
    let project = Project::load(Path::new(&dir))?;
    let fonts = calibration::default_fonts();
    let mut results = vec![];
    for page in &project.pages {
        let mut exclusions = page.proposed_redactions.clone();
        exclusions.extend(
            project
                .targets
                .iter()
                .filter(|t| t.page == page.id)
                .map(|t| t.bounds),
        );
        let words: Vec<_> = page
            .words
            .iter()
            .filter(|w| !exclusions.iter().any(|r| r.intersects(w.bounds)))
            .cloned()
            .collect();
        let grouping = typography::group(&words, &fonts, &exclusions)?;
        let whole = calibration::calibrate("whole".into(), page.id.clone(), None, &words, &fonts)?;
        let whole_font = Font::open(&whole.fits[0].font)?;
        let mut block_fits = BTreeMap::new();
        for word in &words {
            if !block_fits.contains_key(&word.block) {
                block_fits.insert(
                    word.block.clone(),
                    calibration::calibrate(
                        word.block.clone(),
                        page.id.clone(),
                        Some(word.block.clone()),
                        &words,
                        &fonts,
                    )
                    .ok(),
                );
            }
        }
        let mut groups = vec![];
        for group in &grouping.groups {
            let selected = group.select(&words);
            match calibration::calibrate(group.id.clone(), page.id.clone(), None, &selected, &fonts)
            {
                Err(error) => {
                    groups.push(json!({"group":group,"status":"error","reason":error.to_string()}))
                }
                Ok(mut profile) => {
                    profile.typography_group = Some(group.clone());
                    let font = Font::open(&profile.fits[0].font)?;
                    let mut observations = vec![];
                    for w in unique_audit(&selected) {
                        let prediction = profile.fits[0].predict(&font, &w.text)?;
                        let baseline = whole.fits[0].predict(&whole_font, &w.text)?;
                        let block = if let Some(Some(p)) = block_fits.get(&w.block) {
                            let f = Font::open(&p.fits[0].font)?;
                            Some(p.fits[0].predict(&f, &w.text)?)
                        } else {
                            None
                        };
                        observations.push(json!({"text":w.text,"block":w.block,"line":w.line,
                            "observed":w.bounds.w,"group_prediction":prediction,
                            "whole_prediction":baseline,"block_prediction":block}));
                    }
                    groups.push(
                        json!({"group":group,"status":"ok","profile":profile,"audit":observations}),
                    );
                }
            }
        }
        results.push(json!({"page":page.id,"source":page.source,"grouping":grouping,
            "whole_profile":whole,"groups":groups,
            "scope":"Same observed audit widths for whole-page, group, and available OCR-block fits. Grouping and fitting exclude validation/audit heights and audit widths. Font selection uses validation words. OCR labels uncorrected. Within-page splits; no claim of unseen-page generalization."}));
    }
    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}
