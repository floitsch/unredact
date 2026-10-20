// Copyright (C) 2026 Toit contributors.
use serde_json::{Value, json};
use std::{fs, process::Command};

#[test]
fn diagnoses_registered_context_without_pixels_or_calibration_and_rejects_unknown_ids() {
    let dir = tempfile::tempdir().unwrap();
    let project = json!({
        "schema_version": 1, "profiles": [],
        "pages": [{
            "id": "page", "source": "source.pdf", "source_sha256": "source",
            "page_number": 1, "dpi": 72, "width": 200, "height": 100,
            "image": "unavailable.pgm", "image_sha256": "pixels",
            "likely_scan": true, "image_coverage": 1, "native_chars": [],
            "proposed_redactions": [],
            "words": [
                {"text": "Before", "bounds": {"x": 10, "y": 10, "w": 30, "h": 10},
                 "confidence": 99, "block": "b", "line": "l"},
                {"text": "uncertain", "bounds": {"x": 75, "y": 10, "w": 40, "h": 10},
                 "confidence": 49, "block": "b", "line": "l"}
            ]
        }],
        "targets": [{
            "id": "target", "group": "group", "page": "page", "profile": "unavailable",
            "bounds": {"x": 45, "y": 10, "w": 25, "h": 10},
            "surface_kind": "canonical", "left_context": "", "right_context": "",
            "dependence_key": null,
            "evidence": {"kind": "opaque", "padding_min": 0, "padding_max": 0,
                         "measurement_sigma": 1}
        }]
    });
    let bytes = serde_json::to_vec(&project).unwrap();
    let path = dir.path().join("project.json");
    fs::write(&path, &bytes).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_unredact"))
        .args(["diagnose-context", "--project"])
        .arg(dir.path())
        .args(["--target", "target"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let diagnostics = &report["targets"][0]["diagnostics"];
    assert_eq!(diagnostics["left"]["word_indices"], json!([0]));
    assert_eq!(diagnostics["right"]["stop_reason"], "low_confidence");
    assert_eq!(diagnostics["right"]["stop_word_index"], 1);
    assert_eq!(fs::read(&path).unwrap(), bytes);

    let output = Command::new(env!("CARGO_BIN_EXE_unredact"))
        .args(["diagnose-context", "--project"])
        .arg(dir.path())
        .args(["--group", "unknown"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("No targets match"));
}
