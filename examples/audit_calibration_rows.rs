// Copyright (C) 2026 Toit contributors.
//! Export visible-word regression rows without changing a calibration or project.
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};
use unredact::{
    calibration::{audit_held_out, held_out},
    native::Font,
    project::Project,
};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 3,
        "usage: audit_calibration_rows PROJECT FONT_CATALOG"
    );
    let project = Project::load(Path::new(&args[1]))?;
    let catalog: Vec<Value> = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let mut pages = vec![];
    for page in &project.pages {
        let mut words = BTreeMap::new();
        for word in page.words.iter().filter(|w| {
            w.confidence >= 85.0
                && w.bounds.valid()
                && w.text.chars().count() >= 3
                && w.text.len() <= 128
        }) {
            words.entry(word.text.clone()).or_insert(word);
        }
        let mut measurements = vec![];
        for entry in &catalog {
            let path = entry["path"].as_str().context("font path")?;
            let font = Font::open(Path::new(path))?;
            for kern in [false, true] {
                let rows: Result<Vec<_>> = words
                    .values()
                    .map(|w| {
                        let m = font.measure(&w.text, kern)?;
                        Ok(json!({"text": w.text, "observed": w.bounds.w,
                        "row": [m.ink_width(), (m.glyphs - 1).max(0) as f64, 1.0],
                        "split": if audit_held_out(&w.text) { "audit" }
                            else if held_out(&w.text) { "validation" } else { "train" }}))
                    })
                    .collect();
                measurements.push(
                    json!({"font":path,"font_sha256":font.sha256(),"kern":kern,"rows":rows?}),
                );
            }
        }
        pages.push(json!({"page":page.id,"measurements":measurements}));
    }
    println!("{}", serde_json::to_string(&pages)?);
    Ok(())
}
