// Copyright (C) 2026 Toit contributors.
use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use std::{fs, path::PathBuf, time::Duration};
use unredact::{
    calibration, comparison, harvest, linking, pixelation,
    project::{self, Project, Target},
    raster::{GrayImage, Rect},
    search::{self, SearchConfig},
    solver,
};

#[derive(Parser)]
#[command(
    version,
    about = "Calibrate typography and compare redaction candidates across documents"
)]
struct Args {
    #[command(subcommand)]
    command: Cmd,
}
#[derive(serde::Deserialize)]
struct PixelationOccurrenceFile {
    id: String,
    report: PathBuf,
    #[serde(default)]
    dependence_key: Option<String>,
}
#[derive(serde::Deserialize)]
struct PixelationLinksFile {
    group: String,
    occurrences: Vec<PixelationOccurrenceFile>,
}
#[derive(Subcommand)]
enum Cmd {
    /// Jointly rank explicitly linked pixelation reports across pages/PDFs; copies count once.
    CombinePixelation {
        #[arg(long)]
        occurrences: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Infer a common page grid from disjoint pixelated regions.
    AnalyzePixelRegions {
        #[arg(long)]
        image: PathBuf,
        #[arg(long)]
        regions: PathBuf,
        #[arg(long, default_value_t = 4)]
        min_block: usize,
        #[arg(long, default_value_t = 20)]
        max_block: usize,
        #[arg(long, default_value_t = 3)]
        keep: usize,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Rank candidates using shared grids and partial cells; outside-mask pixels must be unchanged by pixelation.
    RankPixelation {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        profile: String,
        #[arg(long)]
        regions: PathBuf,
        #[arg(long)]
        target_index: usize,
        #[arg(long, default_value_t = 4)]
        min_block: usize,
        #[arg(long, default_value_t = 20)]
        max_block: usize,
        #[arg(long)]
        candidates: PathBuf,
        /// Learn rendering height from visible training words outside all supplied masks.
        #[arg(long)]
        learn_vertical: bool,
        #[arg(long, default_value = "canonical")]
        surface_kind: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Export visible OCR span examples for local model adaptation, split by page pixels.
    ExportTraining {
        #[arg(long, required = true)]
        project: Vec<PathBuf>,
        #[arg(long, default_value_t = 4000)]
        max_train: usize,
        #[arg(long, default_value_t = 512)]
        max_validation: usize,
        #[arg(long)]
        output: PathBuf,
    },
    /// Build attributed candidate words/phrases from visible OCR across projects.
    Harvest {
        #[arg(long, required = true)]
        project: Vec<PathBuf>,
        #[arg(long, default_value_t = 85.0)]
        min_confidence: f64,
        #[arg(long, default_value_t = 3)]
        max_words: usize,
        #[arg(long, default_value_t = 10_000)]
        max_candidates: usize,
        #[arg(long, default_value_t = 8)]
        max_sources: usize,
        #[arg(long, default_value = "canonical")]
        surface_kind: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Propose line size groups from training text, pooling across OCR blocks.
    GroupTypography {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        page: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Run pinned adapter comparisons with identical inputs and withheld labels.
    Compare {
        #[arg(long)]
        suite: PathBuf,
        #[arg(long)]
        adapters: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 1)]
        repetitions: usize,
        #[arg(long, default_value_t = 120)]
        timeout_seconds: u64,
    },
    /// Propose cross-page links from visible context; never merges target groups.
    LinkTargets {
        #[arg(long)]
        project: PathBuf,
        #[arg(long, default_value_t = 0.8)]
        min_score: f64,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Compare anchor-only and separate uncertain link/dependence scenarios.
    SolveLinks {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        anchor: String,
        #[arg(long, required = true)]
        candidates: Vec<PathBuf>,
        #[arg(long, default_value_t = 0.8)]
        min_score: f64,
        #[arg(long, default_value_t = 20)]
        max_links: usize,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Infer block size/alignment in an image crop, without supplying candidate text.
    AnalyzePixels {
        #[arg(long)]
        image: PathBuf,
        /// Crop rectangle x,y,width,height, in original image pixels.
        #[arg(long, value_delimiter = ',', num_args = 4)]
        rect: Option<Vec<f64>>,
        #[arg(long, default_value_t = 2)]
        min_block: usize,
        #[arg(long, default_value_t = 32)]
        max_block: usize,
        #[arg(long, default_value_t = 3)]
        keep: usize,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Render PDF/image pages, OCR visible text, and retain native PDF coordinates.
    Import {
        #[arg(long)]
        project: PathBuf,
        #[arg(long, default_value_t = 200.0)]
        dpi: f64,
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Show page IDs, typography blocks, calibration errors, and proposed rectangles.
    Inspect {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        page: Option<String>,
    },
    /// Fit typography using visible words; hold out complete word identities.
    Calibrate {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        page: String,
        #[arg(long)]
        id: String,
        #[arg(long)]
        block: Option<String>,
        #[arg(long)]
        font: Vec<PathBuf>,
        /// Use original PDF layout measurements instead of OCR ink bounds.
        #[arg(long)]
        native: bool,
        /// Screen all installed font files using training words before full calibration.
        #[arg(long)]
        discover_fonts: bool,
        #[arg(long, default_value_t = 24)]
        font_shortlist: usize,
        /// Learn an audited average space adjustment from adjacent visible words.
        #[arg(long, conflicts_with = "native")]
        learn_spaces: bool,
        /// Select a size group reported by group-typography.
        #[arg(long, conflicts_with_all = ["block", "native"])]
        size_group: Option<String>,
        /// Also test FreeType normal hinted metrics at these source pixels-per-em values.
        #[arg(long, value_delimiter = ',')]
        hinted_ppem: Vec<u16>,
        /// Optional local font-network configuration; uses visible training pixels only.
        #[arg(long, conflicts_with = "native")]
        neural_config: Option<PathBuf>,
    },
    /// Re-OCR visible text with known bars blanked; run before calibration.
    RefreshOcr {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        page: String,
        /// JSON array of additional redaction rectangles in page pixels.
        #[arg(long)]
        masks: PathBuf,
        /// Directory containing optional English LSTM model eng.traineddata.
        #[arg(long)]
        tessdata: Option<PathBuf>,
    },
    /// Add an explicit target/group hypothesis from a JSON specification.
    Target {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        spec: PathBuf,
        /// Replace mask-width evidence with nearest visible OCR word anchors.
        #[arg(long)]
        scan_anchors: bool,
        /// Estimate target-row space adjustment from visible training pairs.
        #[arg(long, requires = "scan_anchors")]
        line_spaces: bool,
    },
    /// Rank candidates; optional executable supplies additional names or AI suggestions.
    Solve {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        group: String,
        #[arg(long)]
        candidates: Vec<PathBuf>,
        #[arg(long)]
        provider: Option<PathBuf>,
        #[arg(long)]
        provider_arg: Vec<String>,
        #[arg(long, default_value_t = 30)]
        timeout_seconds: u64,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Rank physically compatible candidates by a separate contextual model.
    ContextRank {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        group: String,
        #[arg(long, required = true)]
        candidates: Vec<PathBuf>,
        #[arg(long)]
        provider: PathBuf,
        #[arg(long)]
        provider_arg: Vec<String>,
        #[arg(long, default_value_t = 300)]
        timeout_seconds: u64,
        /// Extend OCR context across geometrically consistent paragraph lines.
        #[arg(long)]
        paragraph_context: bool,
        /// Normalize context log scores by visible candidate words, not model tokens.
        #[arg(long)]
        word_normalization: bool,
        /// Use whole-replacement log sums, without token or word normalization.
        #[arg(long, conflicts_with = "word_normalization")]
        sequence_score: bool,
        /// Experimental fit-cost penalty per independent occurrence; requires --sequence-score when positive.
        #[arg(long, default_value_t = 0.0)]
        physical_weight: f64,
        /// Maximum compatible candidates to score; exceeding it is an error, not truncation.
        #[arg(long, default_value_t = 10_000)]
        max_compatible: usize,
        /// Candidate/occurrence items per provider call; all calls share the timeout.
        #[arg(long, default_value_t = 2048)]
        batch_items: usize,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Propose additional text from visible context and check every linked occurrence.
    Expand {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        group: String,
        #[arg(long)]
        candidates: Vec<PathBuf>,
        #[arg(long)]
        provider: PathBuf,
        #[arg(long)]
        provider_arg: Vec<String>,
        #[arg(long, default_value_t = 300)]
        timeout_seconds: u64,
        #[arg(long, default_value_t = 512)]
        max_proposals: usize,
        #[arg(long, default_value = "canonical")]
        surface_kind: String,
        #[arg(long)]
        paragraph_context: bool,
        /// Give the proposal model calibrated widths of fixed reference strings.
        #[arg(long)]
        width_guidance: bool,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Ask a provider for successive words/tokens and feed measured constraints back.
    Search {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        group: String,
        #[arg(long)]
        provider: PathBuf,
        #[arg(long)]
        provider_arg: Vec<String>,
        #[arg(long, default_value = "")]
        prefix: String,
        #[arg(long, default_value = "canonical")]
        surface_kind: String,
        #[arg(long, default_value_t = 8)]
        rounds: usize,
        #[arg(long, default_value_t = 32)]
        beam_width: usize,
        #[arg(long, default_value_t = 256)]
        max_extensions: usize,
        #[arg(long, default_value_t = 4096)]
        max_completed: usize,
        #[arg(long, default_value_t = 512)]
        max_text_bytes: usize,
        #[arg(long, default_value_t = 30)]
        timeout_seconds: u64,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Write requests, replies, hashes, and decisions to a new JSONL file.
        #[arg(long)]
        trace: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Run controlled, ground-truth benchmarks and create a reproducible demo project.
    Benchmark {
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        font: Vec<PathBuf>,
    },
    /// List fonts used by the default calibration search.
    Fonts,
}
fn main() -> Result<()> {
    match Args::parse().command {
        Cmd::ExportTraining {
            project,
            max_train,
            max_validation,
            output,
        } => {
            let report = unredact::training::run(
                &project,
                &harvest::Config {
                    max_candidates: 100_000,
                    ..harvest::Config::default()
                },
                &unredact::training::Config {
                    max_train,
                    max_validation,
                },
            )?;
            fs::write(output, serde_json::to_vec_pretty(&report)?)?;
        }
        Cmd::Harvest {
            project,
            min_confidence,
            max_words,
            max_candidates,
            max_sources,
            surface_kind,
            output,
        } => {
            let report = harvest::run(
                &project,
                &harvest::Config {
                    min_confidence,
                    max_words,
                    max_candidates,
                    max_sources,
                    surface_kind,
                },
            )?;
            harvest::write(&report, output.as_deref())?;
        }
        Cmd::GroupTypography {
            project,
            page,
            output,
        } => {
            let p = Project::load(&project)?;
            let data = p.page(&page)?;
            let mut excluded = data.proposed_redactions.clone();
            excluded.extend(
                p.targets
                    .iter()
                    .filter(|t| t.page == page)
                    .map(|t| t.bounds),
            );
            let report =
                unredact::typography::group(&data.words, &calibration::default_fonts(), &excluded)?;
            let text = serde_json::to_string_pretty(&report)?;
            if let Some(path) = output {
                fs::write(path, text)?;
            } else {
                println!("{text}");
            }
        }
        Cmd::Compare {
            suite,
            adapters,
            output,
            repetitions,
            timeout_seconds,
        } => {
            let report = comparison::run(
                &suite,
                &adapters,
                &output,
                repetitions,
                Duration::from_secs(timeout_seconds),
            )?;
            println!("{}", serde_json::to_string_pretty(&report.summary)?);
        }
        Cmd::LinkTargets {
            project,
            min_score,
            output,
        } => {
            let p = Project::load(&project)?;
            let json = serde_json::to_string_pretty(&linking::discover(&p, min_score)?)?;
            if let Some(path) = output {
                fs::write(path, json)?;
            } else {
                println!("{json}");
            }
        }
        Cmd::SolveLinks {
            project,
            anchor,
            candidates,
            min_score,
            max_links,
            limit,
            output,
        } => {
            let p = Project::load(&project)?;
            let mut all = vec![];
            for path in candidates {
                all.extend(solver::read_candidates(&path)?);
            }
            let report =
                linking::solve_links(&p, &project, &anchor, &all, min_score, max_links, limit)?;
            let json = serde_json::to_string_pretty(&report)?;
            if let Some(path) = output {
                fs::write(path, json)?;
            } else {
                println!("{json}");
            }
        }
        Cmd::AnalyzePixels {
            image,
            rect,
            min_block,
            max_block,
            keep,
            output,
        } => {
            let mut image = GrayImage::read(&image)?;
            if let Some(r) = rect {
                image = image.crop(Rect {
                    x: r[0],
                    y: r[1],
                    w: r[2],
                    h: r[3],
                })?;
            }
            let report = pixelation::analyze(&image, min_block, max_block, keep)?;
            let json = serde_json::to_string_pretty(&report)?;
            if let Some(path) = output {
                fs::write(path, json)?;
            } else {
                println!("{json}");
            }
        }
        Cmd::CombinePixelation {
            occurrences,
            output,
        } => {
            let manifest: PixelationLinksFile = serde_json::from_slice(&fs::read(&occurrences)?)?;
            let base = occurrences
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));
            let mut entries = vec![];
            for entry in manifest.occurrences {
                let path = base.join(entry.report);
                let report=serde_json::from_slice(&fs::read(&path)?).with_context(||
                    format!("Report {} needs joint hypothesis metadata; regenerate it with rank-pixelation",path.display()))?;
                entries.push(unredact::mosaic_joint::Occurrence {
                    id: entry.id,
                    dependence_key: entry.dependence_key,
                    report,
                });
            }
            let report = unredact::mosaic_joint::rank(&unredact::mosaic_joint::Request {
                group: manifest.group,
                occurrences: entries,
            })?;
            let json = serde_json::to_string_pretty(&report)?;
            if let Some(path) = output {
                fs::write(path, json)?;
            } else {
                println!("{json}");
            }
        }
        Cmd::AnalyzePixelRegions {
            image,
            regions,
            min_block,
            max_block,
            keep,
            output,
        } => {
            let image = GrayImage::read(&image)?;
            let regions: Vec<Rect> = serde_json::from_slice(&fs::read(regions)?)?;
            let report =
                unredact::shared_grid::analyze(&image, &regions, min_block, max_block, keep)?;
            let json = serde_json::to_string_pretty(&report)?;
            if let Some(path) = output {
                fs::write(path, json)?;
            } else {
                println!("{json}");
            }
        }
        Cmd::RankPixelation {
            project,
            profile,
            regions,
            target_index,
            min_block,
            max_block,
            candidates,
            learn_vertical,
            surface_kind,
            output,
        } => {
            let data = Project::load(&project)?;
            let profile = data.profile(&profile)?.clone();
            let page = data.page(&profile.page)?;
            let regions: Vec<Rect> = serde_json::from_slice(&fs::read(regions)?)?;
            let visible = page
                .words
                .iter()
                .filter(|w| !regions.iter().any(|r| r.intersects(w.bounds)))
                .cloned()
                .collect::<Vec<_>>();
            let height = if learn_vertical {
                Some(unredact::vertical::learn(&profile, &visible)?)
            } else {
                None
            };
            let scales = height
                .as_ref()
                .map(|fits| fits.iter().map(|f| f.scale).collect())
                .unwrap_or_else(|| vec![1.; profile.fits.len()]);
            let mut report = unredact::mosaic_rank::rank(unredact::mosaic_rank::Request {
                image: project.join(&page.image),
                regions,
                target_index,
                profile,
                candidates: solver::read_candidates(&candidates)?,
                vertical_scales: scales,
                surface_kind,
                min_block,
                max_block,
            })?;
            if let Some(height) = height {
                report["visible_height_calibration"] = serde_json::to_value(height)?;
            }
            let json = serde_json::to_string_pretty(&report)?;
            if let Some(path) = output {
                fs::write(path, json)?;
            } else {
                println!("{json}");
            }
        }
        Cmd::Import {
            project,
            dpi,
            files,
        } => {
            let p = project::import(&project, &files, dpi)?;
            println!(
                "Imported project with {} pages into {}",
                p.pages.len(),
                project.display()
            );
        }
        Cmd::Inspect { project, page } => {
            let p = Project::load(&project)?;
            if let Some(id) = page {
                println!("{}", serde_json::to_string_pretty(p.page(&id)?)?);
            } else {
                let pages:Vec<_>=p.pages.iter().map(|p|{let mut blocks=std::collections::BTreeMap::new();for w in &p.words{*blocks.entry(&w.block).or_insert(0)+=1;}
                    serde_json::json!({"id":p.id,"source":p.source,"page_number":p.page_number,"likely_scan":p.likely_scan,"native_characters":p.native_chars.len(),"ocr_words":p.words.len(),"blocks":blocks,"proposed_redactions":p.proposed_redactions})}).collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({"pages":pages,"profiles":p.profiles,"targets":p.targets})
                    )?
                );
            }
        }
        Cmd::Calibrate {
            project,
            page,
            id,
            block,
            font,
            native,
            discover_fonts,
            font_shortlist,
            learn_spaces,
            size_group,
            hinted_ppem,
            neural_config,
        } => {
            let mut p = Project::load(&project)?;
            let page_data = p.page(&page)?;
            ensure!(
                !native || !page_data.likely_scan,
                "Cannot use native calibration on a likely scan/OCR overlay"
            );
            let raw_words = if native {
                project::native_words(page_data)
            } else {
                page_data.words.clone()
            };
            let mut exclusions = page_data.proposed_redactions.clone();
            exclusions.extend(
                p.targets
                    .iter()
                    .filter(|t| t.page == page)
                    .map(|t| t.bounds),
            );
            let mut words: Vec<_> = raw_words
                .iter()
                .filter(|w| {
                    !p.targets
                        .iter()
                        .any(|t| t.page == page && t.bounds.intersects(w.bounds))
                })
                .cloned()
                .collect();
            let selected_group = if let Some(id) = size_group {
                let report = unredact::typography::group(
                    &words,
                    &calibration::default_fonts(),
                    &exclusions,
                )?;
                let group = report
                    .groups
                    .into_iter()
                    .find(|g| g.id == id)
                    .with_context(|| {
                        format!("Unknown size group {id}; run group-typography on this page")
                    })?;
                words = group.select(&words);
                Some(group)
            } else {
                None
            };
            let mut fonts = if discover_fonts {
                let mut installed = calibration::installed_fonts()?;
                installed.extend(font);
                installed.sort();
                installed.dedup();
                installed
            } else if font.is_empty() {
                calibration::default_fonts()
            } else {
                font
            };
            let metric = if native {
                calibration::WidthMetric::Advance
            } else {
                calibration::WidthMetric::Ink
            };
            let search = if discover_fonts {
                eprintln!(
                    "Screening {} installed fonts using training words",
                    fonts.len()
                );
                let report = calibration::shortlist_fonts(
                    &words,
                    block.as_deref(),
                    &fonts,
                    metric,
                    font_shortlist,
                )?;
                fonts = report.shortlist.iter().map(|s| s.font.clone()).collect();
                Some(report)
            } else {
                None
            };
            let neural = if let Some(config) = neural_config {
                let config = unredact::neural::Config::load(&config)?;
                let selected: Vec<_> = words
                    .iter()
                    .filter(|w| block.as_ref().is_none_or(|b| &w.block == b))
                    .cloned()
                    .collect();
                let report = unredact::neural::suggest(
                    &config,
                    &project,
                    page_data,
                    &selected,
                    &exclusions,
                )?;
                fonts.extend(report.suggestions.iter().map(|s| s.path.clone()));
                fonts.sort();
                fonts.dedup();
                Some(report)
            } else {
                None
            };
            let mut modes = vec![unredact::native::MeasurementMode::Design];
            for ppem in hinted_ppem {
                let mode = unredact::native::MeasurementMode::FreeTypeHinted { ppem };
                ensure!(mode.valid(), "Hinted ppem must be 6–256");
                if !modes.contains(&mode) {
                    modes.push(mode);
                }
            }
            let mut profile = calibration::calibrate_with_modes(
                id.clone(),
                page,
                block,
                &words,
                &fonts,
                metric,
                &modes,
            )?;
            if learn_spaces {
                unredact::spacing::calibrate(&mut profile, &words, &exclusions)?;
            }
            profile.font_search = search;
            profile.typography_group = selected_group;
            profile.neural_search = neural;
            println!("{}", serde_json::to_string_pretty(&profile)?);
            p.profiles.retain(|x| x.id != id);
            p.profiles.push(profile);
            p.save(&project)?;
        }
        Cmd::RefreshOcr {
            project,
            page,
            masks,
            tessdata,
        } => {
            let regions: Vec<unredact::raster::Rect> = serde_json::from_slice(&fs::read(masks)?)?;
            project::refresh_visible_ocr_with_tessdata(
                &project,
                &page,
                &regions,
                tessdata.as_deref(),
            )?;
        }
        Cmd::Target {
            project,
            spec,
            scan_anchors,
            line_spaces,
        } => {
            let mut p = Project::load(&project)?;
            let mut t: Target = serde_json::from_slice(&fs::read(spec)?)?;
            if scan_anchors {
                unredact::scan_span::attach(p.page(&t.page)?, &mut t)?;
            }
            if let unredact::project::Evidence::ScanSpan { local_spacing, .. } = &mut t.evidence
                && line_spaces
            {
                *local_spacing = true;
            }
            p.validate_target(&t)?;
            unredact::scan_span::validate_pixels(p.page(&t.page)?, &t, &project)?;
            // A user can mask visible words for evaluation, but must recalibrate without them.
            let profile = p.profile(&t.profile)?;
            let page = p.page(&t.page)?;
            let overlap: Vec<_> = page
                .words
                .iter()
                .filter(|w| {
                    w.bounds.intersects(t.bounds)
                        && (profile.training_texts.contains(&w.text)
                            || profile.validation_texts.contains(&w.text)
                            || profile.audit_texts.contains(&w.text))
                })
                .collect();
            ensure!(
                overlap.is_empty()
                    && profile.spacing.as_ref().is_none_or(|s| s
                        .samples
                        .iter()
                        .all(|sample| !sample.bounds.intersects(t.bounds))),
                "Target overlaps calibration/validation text. Use a held-out region or remove those words and recalibrate."
            );
            p.targets.retain(|x| x.id != t.id);
            p.targets.push(t);
            p.save(&project)?;
        }
        Cmd::Solve {
            project,
            group,
            candidates,
            provider,
            provider_arg,
            timeout_seconds,
            limit,
            output,
        } => {
            let p = Project::load(&project)?;
            let mut all = vec![];
            for path in candidates {
                all.extend(solver::read_candidates(&path)?);
            }
            if let Some(provider) = provider {
                let extra = solver::provider(
                    &provider,
                    &provider_arg,
                    &p,
                    &group,
                    &all,
                    Duration::from_secs(timeout_seconds),
                )?;
                for c in extra {
                    if let Some(existing) = all.iter_mut().find(|x| x.id == c.id) {
                        existing.context_score = c.context_score;
                        for s in c.surfaces {
                            if !existing
                                .surfaces
                                .iter()
                                .any(|x| x.text == s.text && x.kind == s.kind)
                            {
                                existing.surfaces.push(s);
                            }
                        }
                    } else {
                        all.push(c);
                    }
                }
            }
            ensure!(!all.is_empty(), "Supply --candidates or --provider");
            let report = solver::solve(&p, &project, &group, &all, limit)?;
            let json = serde_json::to_string_pretty(&report)?;
            if let Some(path) = output {
                fs::write(path, json).context("Cannot write report")?;
            } else {
                println!("{json}");
            }
        }
        Cmd::ContextRank {
            project,
            group,
            candidates,
            provider,
            provider_arg,
            timeout_seconds,
            paragraph_context,
            word_normalization,
            sequence_score,
            physical_weight,
            max_compatible,
            batch_items,
            output,
        } => {
            let p = Project::load(&project)?;
            let mut all = vec![];
            for path in candidates {
                all.extend(solver::read_candidates(&path)?);
            }
            let report = unredact::contextual::run(
                &p,
                &project,
                &group,
                &all,
                &unredact::contextual::Config {
                    executable: provider,
                    args: provider_arg,
                    timeout_seconds,
                    paragraph_context,
                    word_normalization,
                    sequence_score,
                    physical_weight,
                    max_compatible,
                    batch_items,
                },
            )?;
            let json = serde_json::to_string_pretty(&report)?;
            if let Some(path) = output {
                fs::write(path, json)?;
            } else {
                println!("{json}");
            }
        }
        Cmd::Expand {
            project,
            group,
            candidates,
            provider,
            provider_arg,
            timeout_seconds,
            max_proposals,
            surface_kind,
            paragraph_context,
            width_guidance,
            output,
        } => {
            let p = Project::load(&project)?;
            let mut all = vec![];
            for path in candidates {
                all.extend(solver::read_candidates(&path)?);
            }
            let report = unredact::expansion::run(
                &p,
                &project,
                &group,
                &all,
                &unredact::expansion::Config {
                    executable: provider,
                    args: provider_arg,
                    timeout_seconds,
                    max_proposals,
                    surface_kind,
                    paragraph_context,
                    width_guidance,
                },
            )?;
            let json = serde_json::to_string_pretty(&report)?;
            if let Some(path) = output {
                fs::write(path, json)?;
            } else {
                println!("{json}");
            }
        }
        Cmd::Benchmark { output, font } => {
            let fonts = if font.is_empty() {
                calibration::default_fonts()
            } else {
                font
            };
            let report = unredact::benchmark::run(&output, &fonts)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Cmd::Search {
            project,
            group,
            provider,
            provider_arg,
            prefix,
            surface_kind,
            rounds,
            beam_width,
            max_extensions,
            max_completed,
            max_text_bytes,
            timeout_seconds,
            limit,
            trace,
            output,
        } => {
            let p = Project::load(&project)?;
            let config = SearchConfig {
                rounds,
                beam_width,
                max_extensions_per_round: max_extensions,
                max_completed,
                max_text_bytes,
                timeout_seconds,
                initial_prefix: prefix,
                surface_kind,
                result_limit: limit,
            };
            let result = search::run(
                &p,
                &project,
                &group,
                &provider,
                &provider_arg,
                config,
                trace.as_deref(),
            )?;
            let json = serde_json::to_string_pretty(&result)?;
            if let Some(path) = output {
                fs::write(path, json).context("Cannot write search report")?;
            } else {
                println!("{json}");
            }
        }
        Cmd::Fonts => {
            for p in calibration::default_fonts() {
                println!("{}", p.display());
            }
        }
    }
    Ok(())
}
