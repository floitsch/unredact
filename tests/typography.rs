// Copyright (C) 2026 Toit contributors.
use unredact::{
    calibration::{Word, default_fonts, held_out},
    native::Font,
    raster::Rect,
    typography,
};

fn fixture() -> (Vec<Word>, Vec<std::path::PathBuf>) {
    let fonts = vec![default_fonts().remove(0)];
    let font = Font::open(&fonts[0]).unwrap();
    let training: Vec<_> = (0..100)
        .map(|i| format!("reading{i}"))
        .filter(|s| !held_out(s))
        .take(5)
        .collect();
    let withheld: Vec<_> = (0..100)
        .map(|i| format!("letters{i}"))
        .filter(|s| held_out(s))
        .take(3)
        .collect();
    let mut words = vec![];
    for (line, size) in [20.0, 20.5, 30.0, 30.5].into_iter().enumerate() {
        let mut x = 10.0;
        for text in training.iter().chain(&withheld) {
            let m = font.measure(text, true).unwrap();
            words.push(Word {
                text: text.clone(),
                confidence: 99.0,
                block: format!("block-{line}"),
                line: "1".into(),
                bounds: Rect {
                    x,
                    y: line as f64 * 50.0,
                    w: m.ink_width() * size,
                    h: (m.bottom - m.top) * size,
                },
            });
            x += m.advance * size + 10.0;
        }
    }
    (words, fonts)
}

#[test]
fn groups_sizes_across_blocks_without_colliding_line_ids() {
    let (words, fonts) = fixture();
    let report = typography::group(&words, &fonts, &[]).unwrap();
    assert_eq!(report.groups.len(), 2);
    assert_eq!(report.groups[0].lines.len(), 2);
    assert_eq!(report.groups[1].lines.len(), 2);
    assert!((report.groups[0].normalized_height - 20.25).abs() < 0.001);
    let chosen = report.groups[0].select(&words);
    assert_eq!(chosen.len(), words.len() / 2);
    assert!(chosen.iter().any(|w| held_out(&w.text)));
}

#[test]
fn neither_validation_nor_audit_pixels_influence_grouping() {
    let (mut words, fonts) = fixture();
    let before = serde_json::to_value(typography::group(&words, &fonts, &[]).unwrap()).unwrap();
    for word in words.iter_mut().filter(|w| held_out(&w.text)) {
        word.bounds.h += 1000.0;
        word.bounds.w += 5000.0;
    }
    let after = serde_json::to_value(typography::group(&words, &fonts, &[]).unwrap()).unwrap();
    assert_eq!(before, after);
}

#[test]
fn masks_and_unreliable_training_leave_lines_unassigned() {
    let (mut words, fonts) = fixture();
    let mask = words[0].bounds;
    words[1].confidence = 20.0;
    words[2].confidence = 20.0;
    let report = typography::group(&words, &fonts, &[mask]).unwrap();
    assert!(report.unassigned_lines.iter().any(|l| l.block == "block-0"));
    assert!(
        report
            .groups
            .iter()
            .all(|g| g.lines.iter().all(|l| l.block != "block-0"))
    );
}
