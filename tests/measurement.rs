// Copyright (C) 2026 Toit contributors.
use unredact::{
    calibration::{self, WidthMetric, Word, audit_held_out},
    native::{Font, MeasurementMode},
    raster::Rect,
};

#[test]
fn hinted_measurements_are_grid_dependent_and_do_not_mutate_rendering() {
    let path = calibration::default_fonts().remove(0);
    let font = Font::open(&path).unwrap();
    let text = "AV Measurement office";
    let before = font
        .render(text, 24.0, true, 0.0, 10.0, 30.0, 400, 50)
        .unwrap();
    let design = font.measure(text, true).unwrap();
    let mut differences = 0;
    for ppem in [9, 12, 16, 24, 48, 96] {
        let mode = MeasurementMode::FreeTypeHinted { ppem };
        let m = font.measure_mode(text, true, mode).unwrap();
        assert!(m.advance > 0.0 && m.ink_width() > 0.0 && m.missing == 0);
        assert_eq!(m.glyphs, design.glyphs);
        differences += usize::from((m.advance - design.advance).abs() > 1e-4);
        let same = font.measure_mode("HHH", false, mode).unwrap();
        let pixels = same.advance * f64::from(ppem);
        assert!((pixels - pixels.round()).abs() < 1e-8);
    }
    assert!(differences >= 3);
    assert!(
        font.measure_mode(text, true, MeasurementMode::FreeTypeHinted { ppem: 0 })
            .is_err()
    );
    let after = font
        .render(text, 24.0, true, 0.0, 10.0, 30.0, 400, 50)
        .unwrap();
    assert_eq!(before.pixels, after.pixels);
    assert_eq!(font.measure(text, true).unwrap().advance, design.advance);
}

#[test]
fn selects_and_roundtrips_hinted_fit_without_selecting_on_audit() {
    let path = calibration::default_fonts().remove(0);
    let font = Font::open(&path).unwrap();
    let truth = MeasurementMode::FreeTypeHinted { ppem: 12 };
    let mut words = vec![];
    for i in 0..150 {
        let text = format!(
            "{}{}",
            ["aligned", "Minimum", "WWW", "office", "narrow", "HMNQ"][i % 6],
            i
        );
        let m = font.measure_mode(&text, true, truth).unwrap();
        words.push(Word {
            text,
            confidence: 99.0,
            block: "body".into(),
            line: i.to_string(),
            bounds: Rect {
                x: 1.0,
                y: i as f64 * 40.0,
                w: m.ink_width() * 36.0,
                h: 30.0,
            },
        });
    }
    let modes = [
        MeasurementMode::Design,
        truth,
        MeasurementMode::FreeTypeHinted { ppem: 24 },
    ];
    let fit = |words: &[Word]| {
        calibration::calibrate_with_modes(
            "f".into(),
            "p".into(),
            None,
            words,
            std::slice::from_ref(&path),
            WidthMetric::Ink,
            &modes,
        )
        .unwrap()
    };
    let profile = fit(&words);
    assert_eq!(profile.fits[0].measurement_mode, truth);
    assert!(profile.fits[0].audit.as_ref().unwrap().rmse < 0.01);
    let encoded = serde_json::to_vec(&profile).unwrap();
    let decoded: calibration::Profile = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded.fits[0].measurement_mode, truth);
    let text = "Unseen wide phrase";
    let measured = font.measure_mode(text, true, truth).unwrap().ink_width() * 36.0;
    assert!((decoded.fits[0].predict(&font, text).unwrap() - measured).abs() < 0.01);
    for w in words.iter_mut().filter(|w| audit_held_out(&w.text)) {
        w.bounds.w += 200.0;
    }
    let changed = fit(&words);
    for (a, b) in profile.fits.iter().zip(&changed.fits) {
        assert_eq!(a.measurement_mode, b.measurement_mode);
        assert_eq!(a.validation_rmse, b.validation_rmse);
        assert_eq!(a.size, b.size);
    }
    assert!(changed.fits[0].audit.as_ref().unwrap().rmse > 190.0);
}

#[test]
fn loaded_fonts_are_immutable_snapshots_of_the_hashed_bytes() {
    let fonts = calibration::default_fonts();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("font.ttf");
    std::fs::copy(&fonts[0], &path).unwrap();
    let font = Font::open(&path).unwrap();
    let hash = font.sha256().to_string();
    let original = font.measure("Stable font snapshot", true).unwrap().advance;
    std::fs::copy(&fonts[1], &path).unwrap();
    assert_ne!(Font::open(&path).unwrap().sha256(), hash);
    assert_eq!(
        font.measure("Stable font snapshot", true).unwrap().advance,
        original
    );
    assert_eq!(font.sha256(), hash);
    // Cached rasterization must also retain its memory after the source is removed.
    std::fs::remove_file(&path).unwrap();
    assert!(
        font.render("Snapshot", 24.0, true, 0.0, 10.0, 30.0, 200, 50)
            .is_ok()
    );
}
