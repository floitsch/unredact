// Copyright (C) 2026 Toit contributors.
use std::{collections::BTreeSet, path::Path};
use unredact::{
    calibration::Word,
    harvest,
    native::NativeChar,
    project::{Page, Project, hash},
    raster::{GrayImage, Rect},
    solver,
};

fn word(text: &str, x: f64, y: f64, line: &str) -> Word {
    Word {
        text: text.into(),
        bounds: Rect {
            x,
            y,
            w: 30.0,
            h: 10.0,
        },
        confidence: 98.0,
        block: "body".into(),
        line: line.into(),
    }
}
fn save(dir: &Path, words: Vec<Word>, masks: Vec<Rect>) -> Project {
    let mut image = GrayImage::white(600, 300).unwrap();
    for word in &words {
        if word.bounds.x < 0.0 {
            continue;
        }
        for y in word.bounds.y as usize..(word.bounds.y + word.bounds.h) as usize {
            for x in word.bounds.x as usize..(word.bounds.x + word.bounds.w) as usize {
                if x % 3 == 0 || word.text == "bar" {
                    image.pixels[y * image.width + x] = 0;
                }
            }
        }
    }
    std::fs::create_dir_all(dir).unwrap();
    image.save(&dir.join("page.pgm")).unwrap();
    let project = Project {
        schema_version: 1,
        profiles: vec![],
        targets: vec![],
        pages: vec![Page {
            id: "p".into(),
            source: dir.join("document.pdf"),
            source_sha256: "source".into(),
            page_number: 7,
            dpi: 300.0,
            width: image.width,
            height: image.height,
            image: "page.pgm".into(),
            image_sha256: hash(&image.pixels),
            likely_scan: true,
            image_coverage: 1.0,
            native_chars: vec![NativeChar {
                text: 'X',
                x1: 0.,
                y1: 0.,
                x2: 10.,
                y2: 10.,
                size: 10.,
                font: "hidden-layer".into(),
            }],
            words,
            proposed_redactions: masks,
        }],
    };
    project.save(dir).unwrap();
    project
}
fn texts(report: &harvest::Report) -> BTreeSet<&str> {
    report
        .candidates
        .iter()
        .map(|c| c.surfaces[0].text.as_str())
        .collect()
}

#[test]
fn training_examples_exclude_masks_preserve_punctuation_and_split_whole_pages() {
    let root = tempfile::tempdir().unwrap();
    let mut directories = vec![];
    for index in 0..12 {
        let directory = root.path().join(format!("page-{index}"));
        let y = 10. + index as f64 * 3.;
        let mut words = vec![
            word("Before", 10., y, "line"),
            word("O'Neill.", 50., y, "line"),
            word("after", 90., y, "line"),
            word("SECRET", 130., y, "line"),
            word("separate", 170., y, "line"),
        ];
        words[0].text = format!("Before{index}");
        save(
            &directory,
            words,
            vec![Rect {
                x: 129.,
                y: y - 1.,
                w: 32.,
                h: 12.,
            }],
        );
        directories.push(directory);
    }
    // A copied page contributes additional masks but never another example or
    // a conflicting train/validation split.
    let duplicate = root.path().join("duplicate");
    std::fs::create_dir(&duplicate).unwrap();
    std::fs::copy(directories[0].join("page.pgm"), duplicate.join("page.pgm")).unwrap();
    let mut copy = Project::load(&directories[0]).unwrap();
    copy.pages[0].proposed_redactions.push(Rect {
        x: 49.,
        y: 9.,
        w: 32.,
        h: 12.,
    });
    copy.save(&duplicate).unwrap();
    directories.push(duplicate);
    let report = unredact::training::run(
        &directories,
        &harvest::Config::default(),
        &unredact::training::Config::default(),
    )
    .unwrap();
    let mut splits = std::collections::BTreeMap::new();
    let mut examples = BTreeSet::new();
    let mut punctuation = false;
    for example in &report.examples {
        let key = example.origin.pixel_sha256.clone();
        if let Some(previous) = splits.insert(key, example.split.clone()) {
            assert_eq!(previous, example.split);
        }
        assert!(
            !example.text.contains("SECRET")
                && !example.context.left.contains("SECRET")
                && !example.context.right.contains("SECRET")
        );
        assert!(
            !example.origin.word_indices.iter().any(|i| example
                .context
                .left_word_indices
                .contains(i)
                || example.context.right_word_indices.contains(i))
        );
        assert_eq!(example.text, example.origin.raw_tokens.join(" "));
        punctuation |= example.text.contains("O'Neill.");
        assert!(examples.insert((&example.context.left, &example.context.right, &example.text)));
    }
    assert!(punctuation);
    assert_eq!(splits.values().collect::<BTreeSet<_>>().len(), 2);
    directories.reverse();
    let reordered = unredact::training::run(
        &directories,
        &harvest::Config::default(),
        &unredact::training::Config::default(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        serde_json::to_value(&reordered).unwrap()
    );
    let capped = unredact::training::run(
        &directories,
        &harvest::Config::default(),
        &unredact::training::Config {
            max_train: 1,
            max_validation: 1,
        },
    )
    .unwrap();
    assert_eq!(capped.examples.len(), 2);
    assert!(capped.omitted_train > 0 && capped.omitted_validation > 0);
}

#[test]
fn harvest_keeps_provenance_and_respects_visibility_and_phrase_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let mut words = vec![
        word("Élodie", 10., 10., "name"),
        word("O'Neill.", 50., 10., "name"),
        word("Arrived", 90., 10., "name"),
        word("left", 10., 30., "mask-gap"),
        word("right", 50., 30., "mask-gap"),
        word("before", 10., 50., "bad-ocr"),
        word("noisy", 50., 50., "bad-ocr"),
        word("after", 90., 50., "bad-ocr"),
        word("column", 10., 70., "columns"),
        word("other", 200., 70., "columns"),
        word("bar", 10., 90., "bar"),
        word("covered", 50., 90., "covered"),
        word("separate", 10., 110., ""),
        word("lines", 50., 110., ""),
        word("line-", 10., 130., "hyphen"),
        word("ending", 50., 130., "hyphen"),
    ];
    words[6].confidence = 20.;
    save(
        dir.path(),
        words,
        vec![
            Rect {
                x: 42.,
                y: 30.,
                w: 5.,
                h: 10.,
            },
            Rect {
                x: 49.,
                y: 89.,
                w: 32.,
                h: 12.,
            },
        ],
    );
    let report = harvest::run(&[dir.path().into()], &harvest::Config::default()).unwrap();
    let found = texts(&report);
    assert!(
        found.contains("Élodie O'Neill") && found.contains("O'Neill") && found.contains("Arrived")
    );
    for forbidden in [
        "X",
        "Élodie O'Neill Arrived",
        "O'Neill Arrived",
        "left right",
        "before after",
        "noisy",
        "column other",
        "bar",
        "covered",
        "separate lines",
        "line",
        "line-",
        "line ending",
    ] {
        assert!(!found.contains(forbidden), "{forbidden}");
    }
    let candidate = report
        .candidates
        .iter()
        .find(|c| c.canonical_name == "Élodie O'Neill")
        .unwrap();
    let origin = &report.origins[&candidate.source_ids[0]];
    assert_eq!(origin.word_indices, vec![0, 1]);
    assert_eq!(origin.raw_tokens, vec!["Élodie", "O'Neill."]);
    assert_eq!(origin.page_number, 7);
    assert_eq!(
        origin.project_sha256,
        hash(&std::fs::read(dir.path().join("project.json")).unwrap())
    );
    let file = dir.path().join("candidates.json");
    harvest::write(&report, Some(&file)).unwrap();
    assert_eq!(
        solver::read_candidates(&file).unwrap().len(),
        report.candidates.len()
    );
    assert_eq!(
        report.counts.observed_words,
        report.counts.accepted_words + report.counts.excluded_words
    );
}

#[test]
fn duplicate_page_masks_are_unioned_before_deduplication_and_pixels_are_verified() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    let words = vec![
        word("secret", 10., 10., "one"),
        word("visible", 50., 10., "one"),
    ];
    save(&a, words.clone(), vec![]);
    save(
        &b,
        words,
        vec![Rect {
            x: 9.,
            y: 9.,
            w: 32.,
            h: 12.,
        }],
    );
    let config = harvest::Config::default();
    let first = harvest::run(&[a.clone(), b.clone()], &config).unwrap();
    let second = harvest::run(&[b.clone(), a.clone(), a.clone()], &config).unwrap();
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    assert_eq!(texts(&first), BTreeSet::from(["visible"]));
    assert_eq!(first.counts.duplicate_pages, 1);
    assert_eq!(first.counts.duplicate_words, 2);
    assert_eq!(first.statistics.values().next().unwrap().distinct_pages, 1);
    let mut image = GrayImage::read(&b.join("page.pgm")).unwrap();
    image.pixels[0] = 0;
    image.save(&b.join("page.pgm")).unwrap();
    assert!(
        harvest::run(&[a, b], &config)
            .unwrap_err()
            .to_string()
            .contains("Page pixels changed")
    );
}

#[test]
fn candidate_and_source_caps_are_explicit_and_duplicate_copies_do_not_boost_frequency() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    save(
        &a,
        vec![
            word("common", 10., 10., "a"),
            word("common", 10., 30., "b"),
            word("rare", 10., 50., "c"),
        ],
        vec![],
    );
    save(
        &b,
        vec![
            word("common", 10., 10., "a"),
            word("different", 10., 50., "c"),
        ],
        vec![],
    );
    let config = harvest::Config {
        max_candidates: 1,
        max_sources: 1,
        ..Default::default()
    };
    let r = harvest::run(&[a, b], &config).unwrap();
    assert_eq!(r.candidates.len(), 1);
    assert_eq!(r.candidates[0].canonical_name, "common");
    assert_eq!(r.counts.unique_candidates, 3);
    assert_eq!(r.counts.omitted_candidates, 2);
    let stats = &r.statistics[&r.candidates[0].id];
    assert_eq!(stats.distinct_pages, 2);
    assert_eq!(stats.occurrences, 3);
    assert_eq!(stats.omitted_origins, 2);
    assert_eq!(r.origins.len(), 1);
}
