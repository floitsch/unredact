// Copyright (C) 2026 Toit contributors.
//! Compare measurement hypotheses on identical held-out visible scan words.
use anyhow::{Context, Result};
use serde_json::json;
use std::{collections::BTreeMap, path::Path};
use unredact::{
    calibration::{self, WidthMetric, audit_held_out},
    native::{Font, MeasurementMode},
    project::Project,
    typography,
};

fn main() -> Result<()> {
    let dir = std::env::args()
        .nth(1)
        .context("usage: audit_measurement PROJECT [PPEM,...]")?;
    let ppem = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "12,16,24,48,96,192".into());
    let mut modes = vec![MeasurementMode::Design];
    for value in ppem.split(',') {
        modes.push(MeasurementMode::FreeTypeHinted {
            ppem: value.parse()?,
        });
    }
    let project = Project::load(Path::new(&dir))?;
    let fonts = calibration::default_fonts();
    let mut pages = vec![];
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
        let mut groups = vec![];
        for group in grouping.groups {
            let selected = group.select(&words);
            let design =
                calibration::calibrate(group.id.clone(), page.id.clone(), None, &selected, &fonts);
            let expanded = calibration::calibrate_with_modes(
                group.id.clone(),
                page.id.clone(),
                None,
                &selected,
                &fonts,
                WidthMetric::Ink,
                &modes,
            );
            match (design, expanded) {
                (Ok(design), Ok(expanded)) => {
                    let d = &design.fits[0];
                    let e = &expanded.fits[0];
                    let df = Font::open(&d.font)?;
                    let ef = Font::open(&e.font)?;
                    let mut unique = BTreeMap::new();
                    for w in selected.iter().filter(|w| w.confidence >= 85.0 && w.bounds.valid()
                        && w.text.chars().count() >= 3 && w.text.len() <= 128 && audit_held_out(&w.text)) {
                        unique.entry(w.text.clone()).or_insert(w);
                    }
                    let observations: Result<Vec<_>> = unique.values().map(|w| Ok(json!({
                        "text":w.text,"observed":w.bounds.w,"design_prediction":d.predict(&df,&w.text)?,
                        "expanded_prediction":e.predict(&ef,&w.text)?}))).collect();
                    groups.push(json!({"group":group.id,"status":"ok","design":design,
                        "expanded":expanded,"audit":observations?}));
                }
                (d, e) => groups.push(json!({"group":group.id,"status":"error",
                    "design_error":d.err().map(|e|e.to_string()),"expanded_error":e.err().map(|e|e.to_string())})),
            }
        }
        pages.push(json!({"page":page.id,"source":page.source,"groups":groups}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"modes":modes,"pages":pages,
        "scope":"Same cached OCR audit word widths. Training-only size groups; validation-only mode selection. No audit-based mode choice. OCR labels uncorrected; not unknown-redaction recovery or producer identification."}))?
    );
    Ok(())
}
