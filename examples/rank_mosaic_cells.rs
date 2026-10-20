// Copyright (C) 2026 Toit contributors.
use anyhow::{Result, ensure};
use std::{env, fs};
use unredact::mosaic_rank::{self, Request};
fn main() -> Result<()> {
    let args = env::args().collect::<Vec<_>>();
    ensure!(args.len() == 2, "Usage: rank_mosaic_cells REQUEST.json");
    let request: Request = serde_json::from_slice(&fs::read(&args[1])?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&mosaic_rank::rank(request)?)?
    );
    Ok(())
}
