// Copyright (C) 2026 Toit contributors.
use std::{fs, path::Path, process::Command, time::Duration};
use unredact::{
    comparison::*,
    project::{Evidence, hash},
    raster::{GrayImage, Rect},
    solver::{Candidate, Surface},
};

fn input() -> AdapterInput {
    AdapterInput {
        schema_version: 1,
        pages: vec![],
        targets: vec![],
        candidates: ["a", "b", "c"]
            .iter()
            .map(|id| Candidate {
                id: (*id).into(),
                canonical_name: (*id).into(),
                surfaces: vec![Surface {
                    text: (*id).into(),
                    kind: "canonical".into(),
                }],
                source_ids: vec![],
                context_score: None,
            })
            .collect(),
    }
}
fn case() -> Case {
    Case {
        id: "case".into(),
        input: input(),
        truth_id: "b".into(),
    }
}
fn response(groups: Vec<Vec<&str>>) -> AdapterResult {
    AdapterResult {
        schema_version: 1,
        status: AdapterStatus::Ok,
        reason: None,
        ranked_groups: groups
            .into_iter()
            .map(|g| g.into_iter().map(str::to_string).collect())
            .collect(),
        ranking_complete: true,
        compatible: None,
    }
}

#[test]
fn evidence_ties_are_not_arbitrary_top_one_successes() {
    let r = response(vec![vec!["a", "b"], vec!["c"]]);
    let m = assess(&case(), &r).unwrap().unwrap();
    assert!(m.truth_in_best_group);
    assert!(!m.unique_correct_first);
    assert!(!m.uniquely_wrong_first);
    assert_eq!(m.truth_rank, Some([1, 2]));
    assert_eq!(m.truth_compatible, None);
}
#[test]
fn missing_truth_and_missing_decision_are_different() {
    let mut c = case();
    c.input.candidates.retain(|x| x.id != "b");
    let mut r = response(vec![vec!["a"], vec!["c"]]);
    let m = assess(&c, &r).unwrap().unwrap();
    assert!(!m.truth_in_vocabulary);
    assert!(m.uniquely_wrong_first);
    assert_eq!(m.no_compatible, None);
    r.compatible = Some(vec![]);
    assert_eq!(assess(&c, &r).unwrap().unwrap().no_compatible, Some(true));
    r.compatible = Some(vec!["a".into()]);
    assert_eq!(
        assess(&c, &r).unwrap().unwrap().single_wrong_compatible,
        Some(true)
    );
}
#[test]
fn unknown_ids_duplicates_and_claimed_complete_partial_lists_are_errors() {
    for groups in [
        vec![vec!["a", "unknown"]],
        vec![vec!["a", "a"], vec!["b", "c"]],
        vec![vec!["b"]],
    ] {
        assert!(assess(&case(), &response(groups)).is_err());
    }
    let mut r = response(vec![vec!["b"]]);
    r.ranking_complete = false;
    assert_eq!(assess(&case(), &r).unwrap().unwrap().unranked_candidates, 2);
    r.compatible = Some(vec!["b".into(), "b".into()]);
    assert!(assess(&case(), &r).is_err());
}
#[test]
fn unsupported_is_not_a_successful_abstention() {
    let r = AdapterResult {
        schema_version: 1,
        status: AdapterStatus::Unsupported,
        reason: Some("No mosaic support".into()),
        ranked_groups: vec![],
        ranking_complete: false,
        compatible: None,
    };
    assert!(assess(&case(), &r).unwrap().is_none());
    let mut r = r;
    r.compatible = Some(vec![]);
    assert!(assess(&case(), &r).is_err());
}
fn fixture(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let image = dir.join("page.pgm");
    GrayImage::white(100, 100).unwrap().save(&image).unwrap();
    let mut c = case();
    let bounds = Rect {
        x: 10.0,
        y: 10.0,
        w: 30.0,
        h: 10.0,
    };
    c.input.pages.push(ImagePage {
        id: "p".into(),
        image: image.clone(),
        image_sha256: hash(&fs::read(image).unwrap()),
        dpi: 200.0,
        redaction_regions: vec![bounds],
    });
    c.input.targets.push(InputTarget {
        id: "t".into(),
        page: "p".into(),
        bounds,
        surface_kind: "canonical".into(),
        evidence: Evidence::Opaque {
            padding_min: 0.0,
            padding_max: 4.0,
            measurement_sigma: 1.0,
        },
        left_context: "".into(),
        right_context: "".into(),
        dependence_key: None,
    });
    let suite = Suite {
        schema_version: 1,
        track: "candidate_ranking_from_images".into(),
        description: "Test fixture, not a baseline".into(),
        cases: vec![c],
    };
    let suite_path = dir.join("suite.json");
    fs::write(&suite_path, serde_json::to_vec(&suite).unwrap()).unwrap();
    let script = dir.join("adapter.py");
    fs::write(&script,r#"import json, sys
r=json.load(sys.stdin)
assert set(r)=={'schema_version','input','work_dir'}
assert 'truth_id' not in r['input']
assert 'profiles' not in r['input']
assert all('words' not in p for p in r['input']['pages'])
print(json.dumps({'schema_version':1,'status':'ok','reason':None,'ranked_groups':[['a','b','c']],'ranking_complete':True,'compatible':None}))
"#).unwrap();
    let adapter = Adapter {
        id: "test-adapter".into(),
        kind: ToolKind::Test,
        executable: "/usr/bin/python3".into(),
        args: vec![script.to_string_lossy().into_owned()],
        artifacts: vec![script],
        repository: None,
    };
    let manifest = Manifest {
        schema_version: 1,
        required_external: vec!["external-required".into()],
        adapters: vec![adapter],
    };
    let manifest_path = dir.join("adapters.json");
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    (suite_path, manifest_path)
}
#[test]
fn executable_symlinks_preserve_python_virtual_environment() {
    let dir = tempfile::tempdir().unwrap();
    let (suite, manifest_path) = fixture(dir.path());
    let environment = dir.path().join("venv");
    assert!(
        Command::new("/usr/bin/python3")
            .args(["-m", "venv", "--without-pip"])
            .arg(&environment)
            .status()
            .unwrap()
            .success()
    );
    let mut manifest: Manifest =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest.adapters[0].executable = environment.join("bin/python");
    let script = &manifest.adapters[0].artifacts[0];
    let original = fs::read_to_string(script).unwrap();
    fs::write(
        script,
        format!("import sys\nassert sys.prefix != sys.base_prefix, 'venv lost'\n{original}"),
    )
    .unwrap();
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let report = run(
        &suite,
        &manifest_path,
        &dir.path().join("run"),
        1,
        Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(report.summary["test-adapter"].successful, 1);
    assert!(report.executions[0].error.is_none());
}

#[test]
fn harness_withholds_labels_records_repetitions_and_reports_missing_baselines() {
    let dir = tempfile::tempdir().unwrap();
    let (suite, manifest) = fixture(dir.path());
    let report = run(
        &suite,
        &manifest,
        &dir.path().join("run"),
        2,
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(report.missing_verified_external, vec!["external-required"]);
    assert_eq!(report.executions.len(), 2);
    assert_eq!(report.summary["test-adapter"].successful, 2);
    assert_eq!(report.summary["test-adapter"].unique_correct_first, 0);
    assert!(!report.provenance[0].files.is_empty());
    assert!(
        run(
            &suite,
            &manifest,
            &dir.path().join("run"),
            1,
            Duration::from_secs(5)
        )
        .is_err()
    );
}
#[test]
fn source_pin_and_clean_tree_are_checked_before_external_execution() {
    let dir = tempfile::tempdir().unwrap();
    let (suite, manifest_path) = fixture(dir.path());
    let repo = dir.path().join("repo");
    fs::create_dir(&repo).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    git(&["init", "--quiet"]);
    fs::write(repo.join("code.txt"), "fixture").unwrap();
    git(&["add", "code.txt"]);
    git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.invalid",
        "commit",
        "--quiet",
        "--no-gpg-sign",
        "-m",
        "fixture",
    ]);
    let commit = git(&["rev-parse", "HEAD"]);
    let mut manifest: Manifest =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest.adapters[0].kind = ToolKind::External;
    manifest.adapters[0].repository = Some(RepositoryPin {
        directory: repo.clone(),
        commit: commit.clone(),
    });
    manifest.required_external = vec!["test-adapter".into()];
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let ok = run(
        &suite,
        &manifest_path,
        &dir.path().join("clean"),
        1,
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(ok.missing_verified_external.is_empty());
    assert_eq!(ok.provenance[0].repository_commit, Some(commit));
    // This is still only a harness test; the pin does not establish adapter fidelity.
    fs::write(repo.join("code.txt"), "modified").unwrap();
    let dirty = run(
        &suite,
        &manifest_path,
        &dir.path().join("dirty"),
        1,
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(dirty.summary["test-adapter"].failed, 1);
    assert!(dirty.provenance[0].error.is_some());
    assert_eq!(dirty.missing_verified_external, vec!["test-adapter"]);
}
#[test]
fn tampered_image_and_malformed_adapter_response_are_not_results() {
    let dir = tempfile::tempdir().unwrap();
    let (suite, manifest) = fixture(dir.path());
    fs::write(dir.path().join("adapter.py"), "print('{}')").unwrap();
    let malformed = run(
        &suite,
        &manifest,
        &dir.path().join("malformed"),
        1,
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(malformed.summary["test-adapter"].failed, 1);
    fs::write(dir.path().join("page.pgm"), "tampered").unwrap();
    assert!(
        run(
            &suite,
            &manifest,
            &dir.path().join("tampered"),
            1,
            Duration::from_secs(5)
        )
        .is_err()
    );
}

#[test]
fn adapter_changes_during_execution_invalidate_the_result() {
    let dir = tempfile::tempdir().unwrap();
    let (suite, manifest) = fixture(dir.path());
    fs::write(dir.path().join("adapter.py"),r#"import json,sys
from pathlib import Path
json.load(sys.stdin)
Path(__file__).write_text('modified')
print(json.dumps({'schema_version':1,'status':'ok','reason':None,'ranked_groups':[['a','b','c']],'ranking_complete':True,'compatible':None}))
"#).unwrap();
    let report = run(
        &suite,
        &manifest,
        &dir.path().join("run"),
        1,
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(report.summary["test-adapter"].successful, 0);
    assert!(
        report.executions[0]
            .error
            .as_ref()
            .unwrap()
            .contains("changed during comparison")
    );
}
