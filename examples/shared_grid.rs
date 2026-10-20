// Copyright (C) 2026 Toit contributors.
use anyhow::{Result, ensure};
use std::{env, fs};
use unredact::{
    raster::{GrayImage, Rect},
    shared_grid,
};

fn main() -> Result<()> {
    let args = env::args().collect::<Vec<_>>();
    ensure!(args.len() == 3, "Usage: shared_grid PAGE.pgm REGIONS.json");
    let image = GrayImage::read(args[1].as_ref())?;
    let regions: Vec<Rect> = serde_json::from_slice(&fs::read(&args[2])?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&shared_grid::analyze(&image, &regions, 4, 20, 3)?)?
    );
    Ok(())
}
