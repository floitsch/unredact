// Copyright (C) 2026 Toit contributors.
use unredact::{
    calibration::{Profile, Word, default_fonts},
    native::Font,
    raster::Rect,
    vertical,
};

#[test]
fn held_out_heights_cannot_change_fitted_scale() {
    let path = &default_fonts()[0];
    let font = Font::open(path).unwrap();
    let training = [
        "Amber", "Birch", "Cedar", "Dawn", "Elm", "Fir", "Grove", "Hazel", "Ivy", "Juniper",
        "Kite", "Laurel",
    ];
    let profile: Profile = serde_json::from_value(serde_json::json!({
        "id":"f","page":"p","block":null,"training_texts":training,
        "validation_texts":["Maple"],"audit_texts":["Oak"],
        "fits":[{"font":path,"kern":true,"size":40.,"tracking":0.,"ink_bias":0.,"corrections":{},
            "validation_rmse":1.,"validation_max_error":2.,"training_words":12,"validation_words":1}]
    })).unwrap();
    let mut words = training
        .iter()
        .chain(["Maple", "Oak"].iter())
        .map(|text| {
            let m = font.measure(text, true).unwrap();
            Word {
                text: (*text).into(),
                bounds: Rect {
                    x: 0.,
                    y: 0.,
                    w: 100.,
                    h: (m.bottom - m.top) * 20.,
                },
                confidence: 99.,
                block: "b".into(),
                line: "l".into(),
            }
        })
        .collect::<Vec<_>>();
    let first = vertical::learn(&profile, &words).unwrap();
    assert!((first[0].scale - 0.5).abs() < 1e-12);
    assert!(first[0].audit.scaled_rmse.unwrap() < 1e-12);
    words[12].bounds.h = 1000.;
    words[13].bounds.h = 2000.;
    let changed = vertical::learn(&profile, &words).unwrap();
    assert_eq!(first[0].scale, changed[0].scale);
    assert!(changed[0].validation.scaled_rmse.unwrap() > 900.);
    let mut overlapping = profile.clone();
    overlapping.audit_texts.push(training[0].into());
    assert!(vertical::learn(&overlapping, &words).is_err());
}

#[test]
fn vertical_transform_preserves_unit_render_and_horizontal_size() {
    let font = Font::open(&default_fonts()[0]).unwrap();
    let original = font
        .render("Amber Vale", 40., true, 0., 10.25, 60.5, 400, 100)
        .unwrap();
    let unchanged = font
        .render_vertical("Amber Vale", 40., true, 0., 10.25, 60.5, 1., 400, 100)
        .unwrap();
    assert_eq!(original.pixels, unchanged.pixels);
    let scaled = font
        .render_vertical("Amber Vale", 40., true, 0., 10.25, 60.5, 0.5, 400, 100)
        .unwrap();
    let bounds = |image: &unredact::raster::GrayImage| {
        let points = image
            .pixels
            .iter()
            .enumerate()
            .filter(|(_, p)| **p < 250)
            .map(|(i, _)| (i % image.width, i / image.width))
            .collect::<Vec<_>>();
        (
            points.iter().map(|p| p.0).max().unwrap() - points.iter().map(|p| p.0).min().unwrap(),
            points.iter().map(|p| p.1).max().unwrap() - points.iter().map(|p| p.1).min().unwrap(),
        )
    };
    let (w, h) = bounds(&original);
    let (sw, sh) = bounds(&scaled);
    assert!(w.abs_diff(sw) <= 1);
    assert!((sh as f64 - h as f64 * 0.5).abs() <= 2.);
    assert!(
        font.render_vertical("A", 40., true, 0., 0., 0., f64::NAN, 20, 20)
            .is_err()
    );
}
