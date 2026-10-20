// Copyright (C) 2026 Toit contributors.
//! Apply current pixel-coverage checks to saved comparison projects without
//! changing OCR, profiles, candidates, or scores.
use anyhow::Result;
use std::path::PathBuf;
use unredact::{
    project::{Evidence, Project},
    scan_span,
};

fn main() -> Result<()> {
    let mut rows = vec![];
    for root in std::env::args().skip(1).map(PathBuf::from) {
        let mut cases = std::fs::read_dir(root)?.collect::<std::io::Result<Vec<_>>>()?;
        cases.sort_by_key(|e| e.path());
        for case in cases {
            let dir = case.path().join("work/project");
            if !dir.join("project.json").is_file() {
                continue;
            }
            let project = Project::load(&dir)?;
            for target in &project.targets {
                if !matches!(target.evidence, Evidence::ScanSpan { .. }) {
                    continue;
                }
                let result = scan_span::validate_pixels(project.page(&target.page)?, target, &dir);
                rows.push(serde_json::json!({"project": dir, "target": target.id,
                    "valid": result.is_ok(), "error": result.err().map(|e| e.to_string())}));
            }
        }
    }
    println!("{}", serde_json::to_string_pretty(&rows)?);
    Ok(())
}
