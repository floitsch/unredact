// Copyright (C) 2026 Toit contributors.
use unredact::{
    calibration::{Word, default_fonts},
    contextual::visible_context,
    native::Font,
    ocr_recovery::{OcrRecoveryOptions, diagnose_target, diagnose_words},
    project::{Evidence, Page, Project, Target, hash, refresh_visible_ocr_with_options},
    raster::Rect,
};

fn word(text: &str, x: f64, confidence: f64) -> Word {
    Word {
        text: text.into(),
        bounds: Rect {
            x,
            y: 90.0,
            w: 25.0,
            h: 20.0,
        },
        confidence,
        block: "1".into(),
        line: "1:1".into(),
    }
}
fn target(bounds: Rect) -> Target {
    Target {
        id: "t".into(),
        page: "p".into(),
        bounds,
        profile: "not-calibrated".into(),
        group: "g".into(),
        surface_kind: "canonical".into(),
        evidence: Evidence::Width {
            width: bounds.w,
            measurement_sigma: 1.0,
        },
        left_context: String::new(),
        right_context: String::new(),
        dependence_key: None,
    }
}

#[test]
fn diagnostics_explain_confidence_gaps_masks_and_empty_rows() {
    let bounds = Rect {
        x: 140.0,
        y: 89.0,
        w: 50.0,
        h: 26.0,
    };
    let words = vec![word("before", 100.0, 99.0), word("after", 210.0, 30.0)];
    let d = diagnose_words(&words, 400, 200, bounds, &[bounds]);
    assert_eq!(d.left.word_indices, [0]);
    assert_eq!(d.right.stop_reason, "low_confidence");
    assert_eq!(d.right.stop_word_index, Some(1));
    assert_eq!(d.right.gap_pixels, Some(20.0));
    let d = diagnose_words(&[word("far", 0.0, 99.0)], 400, 200, bounds, &[bounds]);
    assert_eq!(d.left.stop_reason, "gap_exceeds_four_target_heights");
    let barrier = Rect {
        x: 130.0,
        y: 89.0,
        w: 4.0,
        h: 26.0,
    };
    let d = diagnose_words(&words, 400, 200, bounds, &[bounds, barrier]);
    assert_eq!(d.left.stop_reason, "masked_span");
    let d = diagnose_words(&[], 400, 200, bounds, &[bounds]);
    assert_eq!(d.left.stop_reason, "no_same_row_words");
    assert_eq!(d.right.stop_reason, "no_same_row_words");
}

#[test]
fn refresh_keeps_logical_masks_and_ocr_fringes_separate() {
    let dir = tempfile::tempdir().unwrap();
    let font = Font::open(&default_fonts()[0]).unwrap();
    let mut image = font
        .render("Before", 24.0, true, 0.0, 40.0, 110.0, 400, 200)
        .unwrap();
    let right = font
        .render("after", 24.0, true, 0.0, 210.0, 110.0, 400, 200)
        .unwrap();
    for (pixel, other) in image.pixels.iter_mut().zip(&right.pixels) {
        *pixel = (*pixel).min(*other);
    }
    let bounds = Rect {
        x: 140.0,
        y: 89.0,
        w: 50.0,
        h: 26.0,
    };
    image
        .fill(
            Rect {
                x: 138.0,
                y: 87.0,
                w: 54.0,
                h: 30.0,
            },
            150,
        )
        .unwrap();
    image.fill(bounds, 0).unwrap();
    let image_path = dir.path().join("page.pgm");
    image.save(&image_path).unwrap();
    let original_bytes = std::fs::read(&image_path).unwrap();
    let p = Project {
        pages: vec![Page {
            id: "p".into(),
            source: "synthetic".into(),
            source_sha256: "synthetic".into(),
            page_number: 1,
            dpi: 72.0,
            width: 400,
            height: 200,
            image: "page.pgm".into(),
            image_sha256: hash(&image.pixels),
            likely_scan: true,
            image_coverage: 1.0,
            native_chars: vec![],
            words: vec![],
            proposed_redactions: vec![],
        }],
        ..Project::default()
    };
    p.save(dir.path()).unwrap();
    let options = OcrRecoveryOptions {
        retry_missing_context: true,
        max_regions: 2,
    };
    refresh_visible_ocr_with_options(dir.path(), "p", &[bounds], None, &options).unwrap();
    let mut p = Project::load(dir.path()).unwrap();
    assert_eq!(p.pages[0].proposed_redactions[0].x, 140.0);
    let fringe = Rect {
        x: 138.0,
        y: 87.0,
        w: 54.0,
        h: 30.0,
    };
    assert!(
        p.pages[0]
            .words
            .iter()
            .all(|w| !w.bounds.intersects(fringe))
    );
    p.targets.push(target(bounds));
    let context = visible_context(&p, &p.targets[0]).unwrap();
    assert!(!context.left.is_empty(), "{context:?}");
    assert!(!context.right.is_empty(), "{context:?}");
    let d = diagnose_target(&p, &p.targets[0]).unwrap();
    assert_eq!(context.left_word_indices, d.left.word_indices);
    assert_eq!(context.right_word_indices, d.right.word_indices);
    let record: serde_json::Value = std::fs::read_dir(dir.path().join("ocr-inputs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|f| f.extension().is_some_and(|x| x == "json"))
        .map(|f| serde_json::from_slice(&std::fs::read(f).unwrap()).unwrap())
        .next()
        .unwrap();
    assert_eq!(record["masks"][0]["x"], 138.0);
    assert_eq!(record["input_masks"][0]["x"], 140.0);
    assert_eq!(record["recovery"]["enabled"], true);
    assert_eq!(original_bytes, std::fs::read(&image_path).unwrap());
    // A separate mask remains a hard barrier, even within the gap beside this target.
    p.pages[0].proposed_redactions.push(Rect {
        x: 130.0,
        y: 89.0,
        w: 4.0,
        h: 26.0,
    });
    assert!(visible_context(&p, &p.targets[0]).unwrap().left.is_empty());
    p.targets[0].left_context = "Reviewed supplied context".into();
    let d = diagnose_target(&p, &p.targets[0]).unwrap();
    assert_eq!(d.left.source, "supplied");
    assert_eq!(d.left.stop_reason, "supplied_context");
    assert!(d.left.word_indices.is_empty());
    p.save(dir.path()).unwrap();
    let before = std::fs::read(dir.path().join("project.json")).unwrap();
    assert!(refresh_visible_ocr_with_options(dir.path(), "p", &[bounds], None, &options).is_err());
    assert_eq!(
        before,
        std::fs::read(dir.path().join("project.json")).unwrap()
    );
    // Metadata corruption is rejected before any OCR mutation.
    p.targets.clear();
    p.pages[0].width = 401;
    p.save(dir.path()).unwrap();
    let error =
        refresh_visible_ocr_with_options(dir.path(), "p", &[bounds], None, &options).unwrap_err();
    assert!(error.to_string().contains("dimensions"));
}
