// Copyright (C) 2026 Toit contributors.
use std::{fs, path::PathBuf};
use unredact::{
    calibration::{Word, default_fonts, held_out},
    neural::{self, Config},
    project::{Page, hash},
    raster::{GrayImage, Rect},
};

fn fixture() -> (GrayImage, Vec<Word>) {
    let mut image = GrayImage::white(500, 100).unwrap();
    let mut text: Vec<_> = (0..100)
        .map(|i| format!("sample{i}"))
        .filter(|t| !held_out(t))
        .take(3)
        .collect();
    text.push(
        (0..100)
            .map(|i| format!("audit{i}"))
            .find(|t| held_out(t))
            .unwrap(),
    );
    let words: Vec<_> = text
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let bounds = Rect {
                x: 10.0 + i as f64 * 110.0,
                y: 10.0,
                w: 80.0,
                h: 25.0,
            };
            image.fill(bounds, 30 + i as u8).unwrap();
            Word {
                text,
                bounds,
                confidence: 99.0,
                block: "body".into(),
                line: "one".into(),
            }
        })
        .collect();
    (image, words)
}
#[test]
fn training_pixels_exclude_held_out_words_masks_and_between_word_content() {
    let (mut image, words) = fixture();
    let original = neural::training_montages(&image, &words, &words, &[]).unwrap();
    assert_eq!(original.len(), 1);
    assert_eq!(original[0].1.len(), 3);
    image.fill(words[3].bounds, 0).unwrap();
    image
        .fill(
            Rect {
                x: 92.0,
                y: 10.0,
                w: 25.0,
                h: 25.0,
            },
            0,
        )
        .unwrap();
    let changed = neural::training_montages(&image, &words, &words, &[]).unwrap();
    assert_eq!(original[0].0.pixels, changed[0].0.pixels);
    assert!(changed[0].1.iter().all(|s| !held_out(&s.text)));
    let masked = neural::training_montages(&image, &words, &words, &[words[0].bounds]).unwrap();
    assert_eq!(masked[0].1.len(), 2);
    assert!(masked[0].1.iter().all(|s| s.text != words[0].text));
    let mut overlapping = words.clone();
    overlapping[3].bounds = words[1].bounds;
    let cropped = neural::training_montages(&image, &words, &overlapping, &[]).unwrap();
    assert_eq!(cropped[0].1.len(), 2);
}
#[test]
fn neural_transport_verifies_evidence_catalog_and_model() {
    let (image, words) = fixture();
    let dir = tempfile::tempdir().unwrap();
    let image_path = dir.path().join("page.pgm");
    image.save(&image_path).unwrap();
    let page = Page {
        id: "p".into(),
        source: "synthetic".into(),
        source_sha256: "s".into(),
        page_number: 1,
        dpi: 72.0,
        width: image.width,
        height: image.height,
        image: "page.pgm".into(),
        image_sha256: hash(&image.pixels),
        likely_scan: true,
        image_coverage: 1.0,
        native_chars: vec![],
        words: words.clone(),
        proposed_redactions: vec![],
    };
    let font = default_fonts().remove(0);
    let font_hash = hash(&fs::read(&font).unwrap());
    let catalog = dir.path().join("catalog.json");
    fs::write(
        &catalog,
        serde_json::to_vec(&serde_json::json!([{"id":"font","path":font,"sha256":font_hash}]))
            .unwrap(),
    )
    .unwrap();
    let model = dir.path().join("fake-model");
    fs::write(&model, b"protocol fixture, not model weights").unwrap();
    let script = dir.path().join("provider.py");
    let source = r#"import json,sys,hashlib
from pathlib import Path
r=json.load(sys.stdin)
h=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
f=r['catalog'][0]
print(json.dumps(dict(schema_version=1,status='ok',model_sha256=h(r['model']),
 evidence=[dict(path=p,sha256=h(p)) for p in r['images']],
 suggestions=[dict(f,score=.4,per_crop_scores=[.4]*len(r['images']))])))
"#;
    fs::write(&script, source).unwrap();
    let config = Config {
        python: PathBuf::from("/usr/bin/python3"),
        script: script.clone(),
        model,
        catalog,
        top_k: 8,
        timeout_seconds: 10,
    };
    let report = neural::suggest(&config, dir.path(), &page, &words, &[]).unwrap();
    assert_eq!(report.suggestions[0].sha256, font_hash);
    assert!(
        report
            .inputs
            .iter()
            .all(|i| i.samples.iter().all(|s| !held_out(&s.text)))
    );
    // A syntactically valid model reply may not substitute a different model/catalog.
    fs::write(
        &script,
        source.replace("model_sha256=h(r['model'])", "model_sha256='wrong'"),
    )
    .unwrap();
    assert!(
        neural::suggest(&config, dir.path(), &page, &words, &[])
            .unwrap_err()
            .to_string()
            .contains("model/response")
    );
    fs::write(
        &script,
        source.replace("f=r['catalog'][0]", "f=dict(r['catalog'][0],id='injected')"),
    )
    .unwrap();
    assert!(
        neural::suggest(&config, dir.path(), &page, &words, &[])
            .unwrap_err()
            .to_string()
            .contains("outside supplied catalog")
    );
}
#[test]
fn config_keeps_virtual_environment_executable_path() {
    let dir = tempfile::tempdir().unwrap();
    let venv = dir.path().join("venv/bin");
    fs::create_dir_all(&venv).unwrap();
    std::os::unix::fs::symlink("/usr/bin/python3", venv.join("python")).unwrap();
    for name in ["infer.py", "model.pt", "catalog.json"] {
        fs::write(dir.path().join(name), b"fixture").unwrap();
    }
    let config = dir.path().join("config.json");
    fs::write(&config,r#"{"python":"venv/bin/python","script":"infer.py","model":"model.pt","catalog":"catalog.json","top_k":8,"timeout_seconds":30}"#).unwrap();
    let loaded = Config::load(&config).unwrap();
    assert_eq!(loaded.python, venv.join("python"));
}
