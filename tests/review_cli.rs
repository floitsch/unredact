// Copyright (C) 2026 Toit contributors.
use serde_json::json;
use std::process::Command;

fn context_args() -> Vec<&'static str> {
    vec![
        "context-rank",
        "--project",
        "/nonexistent-review-project",
        "--group",
        "g",
        "--candidates",
        "candidates.txt",
        "--provider",
        "provider",
    ]
}

#[test]
fn ranking_preset_requires_explicit_opt_in_and_rejects_overrides() {
    let executable = env!("CARGO_BIN_EXE_unredact");
    let help = Command::new(executable)
        .args(["context-rank", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("paragraph-sequence-physical-v1"));
    for conflicting in [
        vec!["--paragraph-context"],
        vec!["--sequence-score"],
        vec!["--word-normalization"],
        vec!["--physical-weight", "0"],
    ] {
        let mut args = context_args();
        args.extend(["--ranking-preset", "paragraph-sequence-physical-v1"]);
        args.extend(conflicting);
        let output = Command::new(executable).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("cannot be used with")
        );
    }
    // Both explicit preset and legacy default invocation pass CLI validation.
    // Their deliberately absent project fails only after parsing.
    for preset in [false, true] {
        let mut args = context_args();
        if preset {
            args.extend(["--ranking-preset", "paragraph-sequence-physical-v1"]);
        }
        let output = Command::new(executable).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("No such file")
        );
    }
}

#[test]
fn review_help_explains_local_review_and_requires_project() {
    let executable = env!("CARGO_BIN_EXE_unredact");
    let output = Command::new(executable)
        .args(["review", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--project"));
    assert!(help.contains("--page"));
    assert!(help.contains("Loopback port"));
    assert_eq!(
        Command::new(executable)
            .arg("review")
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn named_preset_matches_explicit_settings_and_retains_legacy_defaults() {
    let executable = env!("CARGO_BIN_EXE_unredact");
    let dir = tempfile::tempdir().unwrap();
    let font_path = unredact::calibration::default_fonts().remove(0);
    let font = unredact::native::Font::open(&font_path).unwrap();
    let width = font.measure("answer", true).unwrap().ink_width() * 24.;
    let image = unredact::raster::GrayImage::white(400, 100).unwrap();
    image.save(&dir.path().join("page.pgm")).unwrap();
    let project = json!({
        "schema_version": 1,
        "pages": [{"id":"p", "source":"synthetic", "source_sha256":"source", "page_number":1,
            "dpi":72., "width":400, "height":100, "image":"page.pgm",
            "image_sha256":unredact::project::hash(&image.pixels), "likely_scan":true,
            "image_coverage":1., "native_chars":[], "words":[], "proposed_redactions":[]}],
        "profiles":[{"id":"f", "page":"p", "block":null, "training_texts":[], "validation_texts":[],
            "fits":[{"font":font_path, "kern":true, "size":24., "tracking":0., "ink_bias":0.,
                "corrections":{}, "validation_rmse":0.1, "validation_max_error":0.2,
                "training_words":30, "validation_words":8}]}],
        "targets":[{"id":"t", "page":"p", "bounds":{"x":50.,"y":30.,"w":100.,"h":20.},
            "profile":"f", "group":"g", "surface_kind":"canonical", "dependence_key":null,
            "left_context":"Hello", "right_context":"world", "evidence":{"kind":"width", "width":width,"measurement_sigma":0.1}}]
    });
    std::fs::write(
        dir.path().join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let candidates = dir.path().join("candidates.txt");
    std::fs::write(&candidates, "answer\n").unwrap();
    let provider = dir.path().join("provider.py");
    std::fs::write(&provider, "import sys,json\nr=json.load(sys.stdin)\njson.dump(dict(schema_version=1,scores=[dict(id=i['id'],target=i['target'],token_log_score_sum=-4,token_count=2) for i in r['items']]),sys.stdout)\n").unwrap();
    let run = |settings: &[&str]| {
        let output = Command::new(executable)
            .args(["context-rank", "--project"])
            .arg(dir.path())
            .args(["--group", "g", "--candidates"])
            .arg(&candidates)
            .args(["--provider", "python3", "--provider-arg"])
            .arg(&provider)
            .args(settings)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let preset = run(&["--ranking-preset", "paragraph-sequence-physical-v1"]);
    let explicit = run(&[
        "--paragraph-context",
        "--sequence-score",
        "--physical-weight",
        "1",
    ]);
    assert_eq!(preset, explicit);
    assert_eq!(
        preset["normalization"],
        "sequence_sum_mean_over_occurrences"
    );
    assert_eq!(preset["physical_weight"], 1.);
    let legacy = run(&[]);
    assert!(legacy.get("normalization").is_none());
    assert!(legacy.get("physical_weight").is_none());
    assert_eq!(legacy["ranked"][0]["mean_token_log_score"], -2.);
}
