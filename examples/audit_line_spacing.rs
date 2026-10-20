// Copyright (C) 2026 Toit contributors.
//! Diagnose line spacing from saved local comparison projects. No candidate
//! identity is passed to the estimator; this output never alters a saved run.
use anyhow::Result;
use std::path::PathBuf;
use unredact::{
    line_spacing,
    native::Font,
    project::{Evidence, Project},
};

fn main() -> Result<()> {
    let root = PathBuf::from(std::env::args().nth(1).expect("Comparison directory"));
    let mut rows = vec![];
    for entry in std::fs::read_dir(root)? {
        let case = entry?.path();
        if !case.join("execution.json").exists() {
            continue;
        }
        let execution: serde_json::Value =
            serde_json::from_slice(&std::fs::read(case.join("execution.json"))?)?;
        if execution["error"].is_string() {
            continue;
        }
        let project = Project::load(&case.join("work/project"))?;
        for target in &project.targets {
            if !matches!(target.evidence, Evidence::ScanSpan { .. }) {
                continue;
            }
            let profile = project.profile(&target.profile)?;
            let estimates: Vec<_> = profile
                .fits
                .iter()
                .map(|fit| {
                    let font = Font::open(&fit.font)?;
                    line_spacing::estimate(project.page(&target.page)?, profile, target, fit, &font)
                })
                .collect::<Result<_>>()?;
            rows.push(serde_json::json!({"case":execution["case"],"adapter":execution["adapter"],"estimates":estimates}));
        }
    }
    println!("{}", serde_json::to_string_pretty(&rows)?);
    Ok(())
}
