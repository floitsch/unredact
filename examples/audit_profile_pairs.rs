// Copyright (C) 2026 Toit contributors.
//! Compare fitted profiles only on identical, withheld OCR word observations.
use anyhow::{Context, Result, ensure};
use std::path::PathBuf;
use unredact::{native::Font, project::Project};

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("Supply JSON array of [before, after] project paths")?;
    let pairs: Vec<(PathBuf, PathBuf)> = serde_json::from_slice(&std::fs::read(path)?)?;
    let mut results = vec![];
    for (before_dir, after_dir) in pairs {
        let before = Project::load(&before_dir)?;
        let after = Project::load(&after_dir)?;
        let a = before.profiles.first().context("Missing before profile")?;
        let b = after.profiles.first().context("Missing after profile")?;
        let pa = before.page(&a.page)?;
        let pb = after.page(&b.page)?;
        ensure!(
            pa.image_sha256 == pb.image_sha256,
            "Different source pixels"
        );
        let fa = a.fits.first().context("Missing before fit")?;
        let fb = b.fits.first().context("Missing after fit")?;
        let font_a = Font::open(&fa.font)?;
        let font_b = Font::open(&fb.font)?;
        let mut seen = std::collections::BTreeSet::new();
        let mut observations = vec![];
        let (mut sum_a, mut sum_b) = (0.0, 0.0);
        for w in &pb.words {
            if !a.audit_texts.contains(&w.text) || !b.audit_texts.contains(&w.text) {
                continue;
            }
            let r = w.bounds;
            let identical = pa.words.iter().any(|old| {
                old.text == w.text
                    && [old.bounds.x, old.bounds.y, old.bounds.w, old.bounds.h]
                        == [r.x, r.y, r.w, r.h]
            });
            if !identical || !seen.insert(w.text.clone()) {
                continue;
            }
            let pred_a = fa.predict(&font_a, &w.text)?;
            let pred_b = fb.predict(&font_b, &w.text)?;
            sum_a += (pred_a - r.w).powi(2);
            sum_b += (pred_b - r.w).powi(2);
            observations.push(serde_json::json!({"text":w.text,"bounds":r,
                "before":pred_a,"after":pred_b,"observed":r.w}));
        }
        let n = observations.len();
        results.push(
            serde_json::json!({"before_project":before_dir,"after_project":after_dir,
            "pixel_sha256":pa.image_sha256,"matched_words":n,
            "before_rmse":(n>0).then(|| (sum_a/n as f64).sqrt()),
            "after_rmse":(n>0).then(|| (sum_b/n as f64).sqrt()),"observations":observations}),
        );
    }
    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}
