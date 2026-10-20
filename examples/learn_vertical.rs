// Copyright (C) 2026 Toit contributors.
use anyhow::{Result, ensure};
use std::{env, path::Path};
use unredact::{project::Project, vertical};
fn main() -> Result<()> {
    let args = env::args().collect::<Vec<_>>();
    ensure!(args.len() == 2, "Supply cached project directory");
    let p = Project::load(Path::new(&args[1]))?;
    let mut rows = vec![];
    for profile in &p.profiles {
        let page = p.page(&profile.page)?;
        rows.push(
            serde_json::json!({"profile":profile.id,"fits":vertical::learn(profile,&page.words)?}),
        );
    }
    println!("{}", serde_json::to_string_pretty(&rows)?);
    Ok(())
}
