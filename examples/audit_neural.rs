// Copyright (C) 2026 Toit contributors.
//! Matched audit comparing default, full-catalog, and neural-proposed fonts.
use anyhow::{Context, Result};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Instant,
};
use unredact::{
    calibration::{self, audit_held_out},
    native::Font,
    neural::{self, Config},
    project::Project,
    typography,
};
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().context("usage: audit_neural PROJECT CONFIG")?);
    let config = Config::load(Path::new(&args.next().context("missing CONFIG")?))?;
    let project = Project::load(&dir)?;
    let default = calibration::default_fonts();
    let catalog: serde_json::Value = serde_json::from_slice(&std::fs::read(&config.catalog)?)?;
    let mut all = default.clone();
    for entry in catalog.as_array().context("catalog must be array")? {
        all.push(
            config
                .catalog
                .parent()
                .unwrap()
                .join(entry["path"].as_str().context("font path")?)
                .canonicalize()?,
        );
    }
    all.sort();
    all.dedup();
    let mut pages = vec![];
    for page in &project.pages {
        let mut excluded = page.proposed_redactions.clone();
        excluded.extend(
            project
                .targets
                .iter()
                .filter(|t| t.page == page.id)
                .map(|t| t.bounds),
        );
        let words: Vec<_> = page
            .words
            .iter()
            .filter(|w| !excluded.iter().any(|r| r.intersects(w.bounds)))
            .cloned()
            .collect();
        let grouping = typography::group(&words, &default, &excluded)?;
        let mut groups = vec![];
        for group in grouping.groups {
            let selected = group.select(&words);
            let baseline = calibration::calibrate(
                group.id.clone(),
                page.id.clone(),
                None,
                &selected,
                &default,
            );
            let Ok(baseline) = baseline else {
                groups.push(json!({"group":group.id,"status":"baseline_error","error":baseline.err().unwrap().to_string()}));
                continue;
            };
            let start = Instant::now();
            let report = neural::suggest(&config, &dir, page, &selected, &excluded);
            let Ok(report) = report else {
                groups.push(json!({"group":group.id,"status":"neural_error","error":report.err().unwrap().to_string()}));
                continue;
            };
            let neural_seconds = start.elapsed().as_secs_f64();
            let mut proposed = default.clone();
            proposed.extend(report.suggestions.iter().map(|s| s.path.clone()));
            proposed.sort();
            proposed.dedup();
            let mut neural = calibration::calibrate(
                group.id.clone(),
                page.id.clone(),
                None,
                &selected,
                &proposed,
            )?;
            neural.neural_search = Some(report);
            let full =
                calibration::calibrate(group.id.clone(), page.id.clone(), None, &selected, &all)?;
            let bf = Font::open(&baseline.fits[0].font)?;
            let nf = Font::open(&neural.fits[0].font)?;
            let ff = Font::open(&full.fits[0].font)?;
            let mut unique = BTreeMap::new();
            for w in selected.iter().filter(|w| {
                w.confidence >= 85.0
                    && w.bounds.valid()
                    && w.text.chars().count() >= 3
                    && w.text.len() <= 128
                    && audit_held_out(&w.text)
            }) {
                unique.entry(w.text.clone()).or_insert(w);
            }
            let audit: Result<Vec<_>> = unique
                .values()
                .map(|w| {
                    Ok(json!({"text":w.text,"observed":w.bounds.w,
                "default_prediction":baseline.fits[0].predict(&bf,&w.text)?,
                "neural_prediction":neural.fits[0].predict(&nf,&w.text)?,
                "catalog_prediction":full.fits[0].predict(&ff,&w.text)?}))
                })
                .collect();
            groups.push(json!({"group":group.id,"status":"ok","neural_seconds":neural_seconds,
                "default":baseline,"neural":neural,"catalog":full,"audit":audit?,
                "fonts_searched":{"default":default.len(),"neural":proposed.len(),"catalog":all.len()}}));
        }
        pages.push(json!({"page":page.id,"source":page.source,"groups":groups}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"pages":pages,
        "scope":"Training-only image proposals; validation-only physical model selection; audit labels excluded. Full-catalog control isolates candidate expansion from neural shortlisting. Cached uncorrected OCR, mixed styles can remain. No original-redaction recovery claim."}))?
    );
    Ok(())
}
