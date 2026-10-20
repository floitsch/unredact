// Copyright (C) 2026 Toit contributors.
use unredact::{
    calibration::{Fit, Profile, WidthMetric, Word, audit_held_out, default_fonts},
    native::Font,
    raster::Rect,
    spacing::{self, Split},
};

fn fixture(extra_space: f64) -> (Profile, Vec<Word>) {
    let path = default_fonts().remove(0);
    let font = Font::open(&path).unwrap();
    let fit = Fit {
        font_sha256: None,
        font: path,
        kern: true,
        size: 30.0,
        tracking: 0.0,
        ink_bias: 0.0,
        corrections: Default::default(),
        validation_rmse: 0.2,
        validation_max_error: 0.4,
        training_words: 80,
        validation_words: 10,
        width_metric: WidthMetric::Ink,
        measurement_mode: Default::default(),
        audit: None,
        spacing: None,
    };
    let mut words = vec![];
    for i in 0..120 {
        let a = format!("reading{i}");
        let b = format!("letters{i}");
        let first = font.measure(&a, true).unwrap();
        let second = font.measure(&b, true).unwrap();
        let advance = font.measure(&format!("{a} "), true).unwrap().advance;
        for (text, left, width) in [
            (a, 10.0 + first.left * 30.0, first.ink_width() * 30.0),
            (
                b,
                10.0 + (advance + second.left) * 30.0 + extra_space,
                second.ink_width() * 30.0,
            ),
        ] {
            words.push(Word {
                text,
                bounds: Rect {
                    x: left,
                    y: i as f64 * 40.0,
                    w: width,
                    h: 25.0,
                },
                confidence: 99.0,
                block: "body".into(),
                line: i.to_string(),
            });
        }
    }
    (
        Profile {
            id: "f".into(),
            page: "p".into(),
            block: None,
            fits: vec![fit],
            training_texts: vec![],
            validation_texts: vec![],
            audit_texts: vec![],
            font_search: None,
            spacing: None,
            typography_group: None,
            neural_search: None,
        },
        words,
    )
}

#[test]
fn learns_extra_spacing_without_changing_single_word_predictions() {
    let (mut profile, words) = fixture(5.0);
    let font = Font::open(&profile.fits[0].font).unwrap();
    let before = profile.fits[0].predict(&font, "ordinary").unwrap();
    let pair_before = profile.fits[0].predict(&font, "ordinary phrase").unwrap();
    spacing::calibrate(&mut profile, &words, &[]).unwrap();
    let fit = &profile.fits[0];
    let model = fit.spacing.as_ref().unwrap();
    assert!(model.valid());
    assert!((model.adjustment - 5.0).abs() < 0.001);
    assert!(model.audit_after.unwrap() < 0.001);
    assert_eq!(before, fit.predict(&font, "ordinary").unwrap());
    assert!((fit.predict(&font, "ordinary phrase").unwrap() - pair_before - 5.0).abs() < 0.001);
}

#[test]
fn audit_pixels_do_not_select_spacing_but_increase_uncertainty() {
    let (mut original, mut words) = fixture(3.0);
    spacing::calibrate(&mut original, &words, &[]).unwrap();
    for pair in words.chunks_mut(2) {
        if pair.iter().any(|w| audit_held_out(&w.text)) {
            pair[1].bounds.x += 6.0;
        }
    }
    let mut changed = original.clone();
    spacing::calibrate(&mut changed, &words, &[]).unwrap();
    let a = original.fits[0].spacing.as_ref().unwrap();
    let b = changed.fits[0].spacing.as_ref().unwrap();
    assert_eq!(a.adjustment, b.adjustment);
    assert_eq!(a.validation_after, b.validation_after);
    assert!(b.audit_after.unwrap() > 5.9);
    assert!(changed.fits[0].residual_scale_for_text("some longer phrase") > 11.8);
    assert_eq!(changed.fits[0].residual_scale_for_text("word"), 0.2);
}

#[test]
fn rejects_mask_crossings_duplicates_and_unreliable_intervening_words() {
    let (_, mut words) = fixture(3.0);
    let clean = spacing::samples(&words, None, &[]);
    let excluded = clean[0].bounds;
    assert_eq!(
        spacing::samples(&words, None, &[excluded]).len(),
        clean.len() - 1
    );
    let mut duplicate = words[..2].to_vec();
    for w in &mut duplicate {
        w.line = "duplicate".into();
    }
    words.extend(duplicate);
    assert_eq!(spacing::samples(&words, None, &[]).len(), clean.len());
    let mut intervening = words[0].clone();
    intervening.text = "bad".into();
    intervening.confidence = 10.0;
    intervening.bounds.x = words[0].bounds.x + words[0].bounds.w + 0.5;
    intervening.bounds.w = 1.0;
    words.truncate(240);
    words.push(intervening);
    assert_eq!(spacing::samples(&words, None, &[]).len(), clean.len() - 1);
    for sample in clean {
        if sample.text.split(' ').any(audit_held_out) {
            assert_eq!(sample.split, Split::Audit);
        }
    }
}

#[test]
fn insufficient_pairs_clear_a_previous_model_and_explain_why() {
    let (mut profile, words) = fixture(3.0);
    spacing::calibrate(&mut profile, &words, &[]).unwrap();
    spacing::calibrate(&mut profile, &words[..4], &[]).unwrap();
    assert!(profile.fits[0].spacing.is_none());
    assert!(profile.spacing.as_ref().unwrap().reason.is_some());
}

#[test]
fn validation_can_reject_a_training_only_gain() {
    let (mut profile, mut words) = fixture(3.0);
    for pair in words.chunks_mut(2) {
        if !pair.iter().any(|w| audit_held_out(&w.text))
            && pair
                .iter()
                .any(|w| unredact::calibration::held_out(&w.text))
        {
            pair[1].bounds.x -= 6.0;
        }
    }
    spacing::calibrate(&mut profile, &words, &[]).unwrap();
    let model = profile.fits[0].spacing.as_ref().unwrap();
    assert!((model.proposed_adjustment - 3.0).abs() < 0.001);
    assert_eq!(model.adjustment, 0.0);
    assert_eq!(model.validation_before, model.validation_after);
}
