// Copyright (C) 2026 Toit contributors.
//! Inspect candidate-independent context extraction on saved comparison inputs.
use anyhow::Result;
use std::path::PathBuf;
use unredact::{context_layout, contextual, project::Project};
fn main() -> Result<()> {
    let root = PathBuf::from(std::env::args().nth(1).expect("Comparison directory"));
    let mut rows = vec![];
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if !path.join("work/project/project.json").exists() {
            continue;
        }
        let execution: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.join("execution.json"))?)?;
        let project = Project::load(&path.join("work/project"))?;
        for target in &project.targets {
            rows.push(serde_json::json!({"case":execution["case"],"adapter":execution["adapter"],"target":target.id,
                "same_row":contextual::visible_context(&project,target)?,
                "paragraph":context_layout::paragraph_context(&project,target)?}));
        }
    }
    println!("{}", serde_json::to_string_pretty(&rows)?);
    Ok(())
}
