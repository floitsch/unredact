// Copyright (C) 2026 Toit contributors.
use std::path::Path;
use unredact::{
    calibration::{Fit, Profile, default_fonts, held_out},
    native::{Font, Pdf, PdfCanvas},
    project::{Evidence, Page, Project, Target, parse_tsv},
    raster::{GrayImage, Rect},
    search::{self, SearchConfig},
    solver::{self, Candidate, Surface},
};

fn fixture() -> (tempfile::TempDir, Project, Vec<Candidate>) {
    let dir = tempfile::tempdir().unwrap();
    let font_path = default_fonts().remove(0);
    let font = Font::open(&font_path).unwrap();
    let fit = Fit {
        font_sha256: None,
        font: font_path,
        kern: true,
        size: 24.0,
        tracking: 0.0,
        ink_bias: 0.0,
        corrections: Default::default(),
        validation_rmse: 0.1,
        validation_max_error: 0.2,
        training_words: 30,
        validation_words: 8,
        width_metric: unredact::calibration::WidthMetric::Ink,
        measurement_mode: Default::default(),
        audit: None,
        spacing: None,
    };
    let width = fit.predict(&font, "Amber Vale").unwrap();
    let page = Page {
        id: "p".into(),
        source: "synthetic".into(),
        source_sha256: "source".into(),
        page_number: 1,
        dpi: 72.0,
        width: 800,
        height: 400,
        image: "page.pgm".into(),
        image_sha256: "same-image".into(),
        likely_scan: true,
        image_coverage: 1.0,
        native_chars: vec![],
        words: vec![],
        proposed_redactions: vec![],
    };
    let profile = Profile {
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
    };
    let target = Target {
        id: "t".into(),
        page: "p".into(),
        bounds: Rect {
            x: 10.0,
            y: 10.0,
            w: width + 4.0,
            h: 30.0,
        },
        profile: "f".into(),
        group: "g".into(),
        surface_kind: "canonical".into(),
        evidence: Evidence::Opaque {
            padding_min: 4.0,
            padding_max: 4.0,
            measurement_sigma: 0.1,
        },
        left_context: "Hello".into(),
        right_context: "arrived".into(),
        dependence_key: None,
    };
    let project = Project {
        schema_version: 1,
        pages: vec![page],
        profiles: vec![profile],
        targets: vec![target],
    };
    let candidates = [("right", "Amber Vale"), ("wrong", "WWWWWWWWWWWWW")]
        .iter()
        .map(|(id, text)| Candidate {
            id: (*id).into(),
            canonical_name: (*text).into(),
            surfaces: vec![Surface {
                text: (*text).into(),
                kind: "canonical".into(),
            }],
            source_ids: vec![],
            context_score: None,
        })
        .collect();
    (dir, project, candidates)
}

#[test]
fn contextual_ranking_keeps_physical_report_and_rejects_provider_injection() {
    let (dir, mut p, mut candidates) = fixture();
    if let Evidence::Opaque {
        measurement_sigma, ..
    } = &mut p.targets[0].evidence
    {
        *measurement_sigma = 5.0;
    }
    let mut alternative = candidates[0].clone();
    alternative.id = "alternative".into();
    alternative.canonical_name = "Amber Vail".into();
    alternative.surfaces[0].text = alternative.canonical_name.clone();
    candidates.push(alternative);
    let physical = solver::solve(&p, dir.path(), "g", &candidates, 3).unwrap();
    assert_eq!(physical.compatible_candidates, 2);
    let script = dir.path().join("context.py");
    std::fs::write(
        &script,
        r#"import json,sys
r=json.load(sys.stdin)
assert r['task']=='score_context_candidates'
assert r['contexts']['t']['left']=='Hello'
assert set(i['id'] for i in r['items'])=={'right','alternative'}
scores=[dict(id=i['id'],target=i['target'],token_count=2,
    token_log_score_sum=-1 if i['id']=='alternative' else -5) for i in r['items']]
if len(sys.argv)>1:
    if sys.argv[1]=='unknown': scores[0]['id']='wrong'
    elif sys.argv[1]=='duplicate': scores.append(scores[0])
    elif sys.argv[1]=='missing': scores.pop()
    elif sys.argv[1]=='invalid': scores[0]['token_count']=0
json.dump(dict(schema_version=1,scores=scores),sys.stdout)
"#,
    )
    .unwrap();
    let mut config = unredact::contextual::Config {
        max_compatible: 10_000,
        batch_items: 2048,
        persistent_provider: false,
        physical_weight: 0.0,
        sequence_score: false,
        word_normalization: false,
        paragraph_context: false,
        executable: "python3".into(),
        args: vec![script.display().to_string()],
        timeout_seconds: 5,
    };
    let report = unredact::contextual::run(&p, dir.path(), "g", &candidates, &config).unwrap();
    assert_eq!(report.ranked_groups[0], vec!["alternative"]);
    assert_eq!(
        serde_json::to_value(&report.physical).unwrap(),
        serde_json::to_value(&physical).unwrap()
    );
    assert_eq!(report.physical.status, "ambiguous");
    assert!(report.provider.is_some());
    config.max_compatible = 1;
    assert!(
        unredact::contextual::run(&p, dir.path(), "g", &candidates, &config)
            .unwrap_err()
            .to_string()
            .contains("--max-compatible")
    );
    config.max_compatible = 10_000;
    for bad in ["unknown", "duplicate", "missing", "invalid"] {
        config.args.push(bad.into());
        assert!(unredact::contextual::run(&p, dir.path(), "g", &candidates, &config).is_err());
        config.args.pop();
    }
    // No compatible candidates: no attempt to contact a language model.
    config.executable = "/missing-provider".into();
    let report =
        unredact::contextual::run(&p, dir.path(), "g", &candidates[1..2], &config).unwrap();
    assert!(report.ranked.is_empty());
    assert!(report.provider.is_none());
}

#[test]
fn contextual_ocr_stops_at_other_masks_and_unreliable_words() {
    let (_, mut p, _) = anchored_fixture();
    p.targets[0].left_context.clear();
    p.targets[0].right_context.clear();
    let mut more = p.pages[0].words[0].clone();
    more.text = "Earlier,".into();
    more.bounds.x = 10.0;
    more.bounds.w = 65.0;
    p.pages[0].words.push(more);
    let c = unredact::contextual::visible_context(&p, &p.targets[0]).unwrap();
    assert_eq!(c.left, "Earlier, Before");
    assert_eq!(c.right, "after");
    assert_eq!(c.left_word_indices, vec![2, 0]);
    p.pages[0].proposed_redactions.push(Rect {
        x: 80.0,
        y: 30.0,
        w: 5.0,
        h: 20.0,
    });
    let c = unredact::contextual::visible_context(&p, &p.targets[0]).unwrap();
    assert_eq!(c.left, "Before");
    p.pages[0].words[0].confidence = 10.0;
    assert!(
        unredact::contextual::visible_context(&p, &p.targets[0])
            .unwrap()
            .left
            .is_empty()
    );
    p.targets[0].left_context = "User-supplied, punctuation!".into();
    let c = unredact::contextual::visible_context(&p, &p.targets[0]).unwrap();
    assert_eq!(c.left, p.targets[0].left_context);
    assert_eq!(c.left_source, "supplied");
    assert!(c.left_word_indices.is_empty());
}

#[test]
fn combined_context_ranking_preserves_evidence_and_counts_duplicates_once() {
    let (dir, mut project, mut candidates) = fixture();
    if let Evidence::Opaque {
        measurement_sigma, ..
    } = &mut project.targets[0].evidence
    {
        *measurement_sigma = 5.;
    }
    let mut other = candidates[0].clone();
    other.id = "alternative".into();
    other.surfaces[0].text = "Amber Vail".into();
    candidates.push(other);
    let physical = solver::solve(&project, dir.path(), "g", &candidates, candidates.len()).unwrap();
    let compatible: Vec<_> = physical
        .candidates
        .iter()
        .filter(|c| c.compatible)
        .collect();
    assert_eq!(compatible.len(), 2);
    let better = compatible[0];
    let worse = compatible[1];
    let gap = worse.evidence_score - better.evidence_score;
    assert!(gap > 1e-6);
    let script = dir.path().join("fusion.py");
    std::fs::write(&script, r#"import json,sys
r=json.load(sys.stdin)
json.dump(dict(schema_version=1,scores=[dict(id=i['id'],target=i['target'],token_count=2,
 token_log_score_sum=-1-float(sys.argv[2]) if i['id']==sys.argv[1] else -1) for i in r['items']]),sys.stdout)
"#).unwrap();
    let mut config = unredact::contextual::Config {
        executable: "python3".into(),
        args: vec![
            script.display().to_string(),
            better.id.clone(),
            (gap / 2.).to_string(),
        ],
        timeout_seconds: 5,
        paragraph_context: false,
        word_normalization: false,
        sequence_score: true,
        max_compatible: 10_000,
        batch_items: 1,
        persistent_provider: false,
        physical_weight: 0.,
    };
    let language =
        unredact::contextual::run(&project, dir.path(), "g", &candidates, &config).unwrap();
    assert_eq!(language.ranked_groups[0], vec![worse.id.clone()]);
    let serialized = serde_json::to_value(&language).unwrap();
    assert!(serialized.get("physical_weight").is_none());
    assert!(serialized["ranked"][0].get("combined_score").is_none());
    config.physical_weight = 1.;
    let combined =
        unredact::contextual::run(&project, dir.path(), "g", &candidates, &config).unwrap();
    assert_eq!(combined.ranked_groups[0], vec![better.id.clone()]);
    assert_eq!(
        serde_json::to_value(&combined.physical).unwrap(),
        serde_json::to_value(&language.physical).unwrap()
    );
    for c in &combined.ranked {
        let original = language.ranked.iter().find(|l| l.id == c.id).unwrap();
        assert_eq!(
            serde_json::to_value(&original.observations).unwrap(),
            serde_json::to_value(&c.observations).unwrap()
        );
    }
    let mut duplicate = project.targets[0].clone();
    duplicate.id = "z-copy".into();
    project.targets.push(duplicate);
    let deduped =
        unredact::contextual::run(&project, dir.path(), "g", &candidates, &config).unwrap();
    assert_eq!(deduped.physical.ignored_dependent_targets, vec!["z-copy"]);
    assert_eq!(
        serde_json::to_value(&deduped.ranked).unwrap(),
        serde_json::to_value(&combined.ranked).unwrap()
    );
    let mut independent = project.targets[0].clone();
    independent.id = "independent".into();
    independent.bounds.y += 100.;
    project.targets.push(independent);
    let joint = unredact::contextual::run(&project, dir.path(), "g", &candidates, &config).unwrap();
    for c in &joint.ranked {
        assert_eq!(c.observations.len(), 2);
        let physical = joint
            .physical
            .candidates
            .iter()
            .find(|p| p.id == c.id)
            .unwrap();
        assert_eq!(c.mean_physical_cost, Some(physical.evidence_score / 2.));
        assert_eq!(
            c.combined_score,
            Some(c.mean_sequence_log_score.unwrap() - physical.evidence_score / 2.)
        );
    }
    config.executable = "/nonexistent-provider".into();
    for weight in [-1., f64::NAN, f64::INFINITY] {
        config.physical_weight = weight;
        assert!(
            unredact::contextual::run(&project, dir.path(), "g", &candidates, &config)
                .unwrap_err()
                .to_string()
                .contains("Physical weight")
        );
    }
    config.physical_weight = 1.;
    config.sequence_score = false;
    assert!(
        unredact::contextual::run(&project, dir.path(), "g", &candidates, &config)
            .unwrap_err()
            .to_string()
            .contains("requires sequence")
    );
}

#[test]
fn word_normalization_uses_selected_surface_words_without_changing_physics() {
    let (dir, mut p, mut candidates) = fixture();
    if let Evidence::Opaque {
        measurement_sigma, ..
    } = &mut p.targets[0].evidence
    {
        *measurement_sigma = 5.;
    }
    let mut alternative = candidates[0].clone();
    alternative.id = "alternative".into();
    alternative.canonical_name = "Different canonical display name".into();
    alternative.surfaces[0].text = "Amber Vail".into();
    candidates.push(alternative);
    let script = dir.path().join("normalization.py");
    std::fs::write(
        &script,
        r#"import json,sys
r=json.load(sys.stdin)
json.dump(dict(schema_version=1,scores=[dict(id=i['id'],target=i['target'],
    token_count=3 if i['id']=='alternative' else 1,
    token_log_score_sum=-9 if i['id']=='alternative' else -4) for i in r['items']]),sys.stdout)
"#,
    )
    .unwrap();
    let mut config = unredact::contextual::Config {
        max_compatible: 10_000,
        batch_items: 2048,
        persistent_provider: false,
        physical_weight: 0.0,
        sequence_score: false,
        executable: "python3".into(),
        args: vec![script.display().to_string()],
        timeout_seconds: 5,
        paragraph_context: false,
        word_normalization: false,
    };
    let token = unredact::contextual::run(&p, dir.path(), "g", &candidates, &config).unwrap();
    assert_eq!(token.ranked_groups[0], vec!["alternative"]);
    assert!(token.normalization.is_none());
    config.word_normalization = true;
    let word = unredact::contextual::run(&p, dir.path(), "g", &candidates, &config).unwrap();
    assert_eq!(word.ranked_groups[0], vec!["right"]);
    assert_eq!(word.ranked[0].mean_word_log_score, Some(-2.));
    assert_eq!(word.ranked[1].mean_word_log_score, Some(-4.5));
    config.word_normalization = false;
    config.sequence_score = true;
    let sequence = unredact::contextual::run(&p, dir.path(), "g", &candidates, &config).unwrap();
    assert_eq!(sequence.ranked_groups[0], vec!["right"]);
    assert_eq!(sequence.ranked[0].mean_sequence_log_score, Some(-4.));
    assert_eq!(sequence.ranked[1].mean_sequence_log_score, Some(-9.));
    assert_eq!(sequence.ranked[0].mean_word_log_score, None);
    assert_eq!(
        serde_json::to_value(&sequence.physical).unwrap(),
        serde_json::to_value(&token.physical).unwrap()
    );
    config.word_normalization = true;
    assert!(unredact::contextual::run(&p, dir.path(), "g", &candidates, &config).is_err());
    config.word_normalization = false;
    let mut other = p.targets[0].clone();
    other.id = "second".into();
    other.bounds.y += 100.;
    p.targets.push(other);
    let joint = unredact::contextual::run(&p, dir.path(), "g", &candidates, &config).unwrap();
    assert_eq!(joint.ranked[0].observations.len(), 2);
    assert_eq!(joint.ranked[0].mean_sequence_log_score, Some(-4.));
    assert_eq!(joint.ranked[1].mean_sequence_log_score, Some(-9.));
    config.batch_items = 1;
    let batched = unredact::contextual::run(&p, dir.path(), "g", &candidates, &config).unwrap();
    assert_eq!(
        serde_json::to_value(&batched.physical).unwrap(),
        serde_json::to_value(&joint.physical).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&batched.ranked).unwrap(),
        serde_json::to_value(&joint.ranked).unwrap()
    );
    assert_eq!(batched.ranked_groups, joint.ranked_groups);
    assert_eq!(
        batched.provider.unwrap()["batches"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        serde_json::to_value(word.physical).unwrap(),
        serde_json::to_value(token.physical).unwrap()
    );
}

#[test]
fn expansion_restores_omitted_text_without_rewriting_existing_candidates() {
    let (dir, mut p, candidates) = fixture();
    let mut duplicate = p.targets[0].clone();
    duplicate.id = "z-duplicate".into();
    p.targets.push(duplicate);
    let provider = dir.path().join("propose.py");
    std::fs::write(&provider, r#"import json,sys
r=json.load(sys.stdin)
assert r['task']=='propose_context_candidates'
assert list(r['contexts'])==['t']
assert 'existing_candidates' not in r
assert r['contexts']['t']['left']=='Hello'
print(json.dumps({'schema_version':1,'proposals':[{'target':'t','text':'Amber Vale'},{'target':'t','text':'WWWWWWWWWWWWW'}]}))
"#).unwrap();
    let config = unredact::expansion::Config {
        executable: "python3".into(),
        args: vec![provider.to_string_lossy().into()],
        timeout_seconds: 5,
        max_proposals: 8,
        surface_kind: "canonical".into(),
        paragraph_context: true,
        width_guidance: false,
    };
    let report = unredact::expansion::run(&p, dir.path(), "g", &candidates[1..], &config).unwrap();
    assert_eq!(report.added_candidate_ids.len(), 1);
    assert_eq!(
        report.compatible_added_candidate_ids,
        report.added_candidate_ids
    );
    assert_eq!(report.initial.as_ref().unwrap().compatible_candidates, 0);
    assert_eq!(
        report.expanded.as_ref().unwrap().independent_observations,
        1
    );
    assert_eq!(
        serde_json::to_value(&report.candidates[0]).unwrap(),
        serde_json::to_value(&candidates[1]).unwrap()
    );
    assert!(report.candidates[1].context_score.is_none());
    let path = dir.path().join("expanded.json");
    std::fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
    assert_eq!(solver::read_candidates(&path).unwrap().len(), 2);
    // The same proposed spelling is already an alias of an existing identity.
    let mut alias = candidates[1].clone();
    alias.surfaces.push(candidates[0].surfaces[0].clone());
    let report = unredact::expansion::run(&p, dir.path(), "g", &[alias.clone()], &config).unwrap();
    assert!(report.added_candidate_ids.is_empty());
    assert_eq!(
        serde_json::to_value(&report.candidates[0]).unwrap(),
        serde_json::to_value(alias).unwrap()
    );
}

#[test]
fn expansion_rejects_provider_injection_and_preserves_empty_search() {
    let (dir, p, candidates) = fixture();
    let config = unredact::expansion::Config {
        executable: "python3".into(),
        args: vec![],
        timeout_seconds: 5,
        max_proposals: 1,
        surface_kind: "canonical".into(),
        paragraph_context: false,
        width_guidance: false,
    };
    let valid = serde_json::json!({"target":"t","text":"Amber Vale"});
    for proposals in [
        serde_json::json!([{"target":"unknown","text":"Amber Vale"}]),
        serde_json::json!([{"target":"t","text":"Amber Vale","context_score":1e30}]),
        serde_json::json!([{"target":"t","text":" Amber Vale"}]),
        serde_json::json!([{"target":"t","text":"Amber\nVale"}]),
        serde_json::json!([valid.clone(), valid.clone()]),
    ] {
        let response = serde_json::json!({"schema_version":1,"proposals":proposals});
        let script = format!("import json; print(json.dumps({}))", response);
        let c = unredact::expansion::Config {
            args: vec!["-c".into(), script],
            ..config.clone()
        };
        assert!(unredact::expansion::run(&p, dir.path(), "g", &candidates, &c).is_err());
    }
    let empty = unredact::expansion::Config {
        args: vec![
            "-c".into(),
            "print('{\"schema_version\":1,\"proposals\":[]}')".into(),
        ],
        ..config
    };
    let report = unredact::expansion::run(&p, dir.path(), "g", &[], &empty).unwrap();
    assert!(report.initial.is_none() && report.expanded.is_none());
    assert!(report.candidates.is_empty());
}

#[test]
fn expansion_checks_joint_geometry_duplicates_and_id_collisions() {
    let (dir, mut p, candidates) = fixture();
    let mut config = unredact::expansion::Config {
        executable: "python3".into(),
        args: vec!["-c".into(), "print('{\"schema_version\":1,\"proposals\":[{\"target\":\"t\",\"text\":\"Amber Vale\"}]}')".into()],
        timeout_seconds: 5, max_proposals: 4, surface_kind: "canonical".into(), paragraph_context: false, width_guidance: false,
    };
    let mut page = p.pages[0].clone();
    page.id = "other-page".into();
    page.source_sha256 = "independent-source".into();
    page.image_sha256 = "different-image".into();
    let mut profile = p.profiles[0].clone();
    profile.id = "other-profile".into();
    profile.page = page.id.clone();
    let mut target = p.targets[0].clone();
    target.id = "other-target".into();
    target.page = page.id.clone();
    target.profile = profile.id.clone();
    target.bounds.w += 100.;
    p.pages.push(page);
    p.profiles.push(profile);
    p.targets.push(target);
    let report = unredact::expansion::run(&p, dir.path(), "g", &[], &config).unwrap();
    assert_eq!(
        report.expanded.as_ref().unwrap().independent_observations,
        2
    );
    assert_eq!(report.added_candidate_ids.len(), 1);
    assert!(report.compatible_added_candidate_ids.is_empty());
    let mut collision = candidates[1].clone();
    collision.id = report.added_candidate_ids[0].clone();
    assert!(
        unredact::expansion::run(&p, dir.path(), "g", &[collision], &config)
            .err()
            .unwrap()
            .to_string()
            .contains("collides")
    );
    config.args[1] = "print('{\"schema_version\":1,\"proposals\":[{\"target\":\"t\",\"text\":\"Amber Vale\"},{\"target\":\"t\",\"text\":\"Amber Vale\"}]}')".into();
    assert!(
        unredact::expansion::run(&p, dir.path(), "g", &[], &config)
            .err()
            .unwrap()
            .to_string()
            .contains("Duplicate")
    );
}

fn paragraph_fixture() -> Project {
    let (_, mut p, _) = fixture();
    p.targets[0].bounds = Rect {
        x: 130.0,
        y: 68.0,
        w: 100.0,
        h: 24.0,
    };
    p.targets[0].left_context.clear();
    p.targets[0].right_context.clear();
    let lines = [
        vec![
            ("The", 40., 60.),
            ("review", 120., 100.),
            ("was", 250., 60.),
            ("nearly", 380., 100.),
            ("complete", 520., 180.),
        ],
        vec![
            ("before", 40., 70.),
            ("the", 250., 60.),
            ("committee", 340., 140.),
            ("adjourned.", 520., 180.),
        ],
        vec![
            ("Further", 40., 100.),
            ("details", 180., 100.),
            ("followed.", 330., 160.),
        ],
    ];
    p.pages[0].words = lines
        .into_iter()
        .enumerate()
        .flat_map(|(line, words)| {
            words
                .into_iter()
                .map(move |(text, x, w)| unredact::calibration::Word {
                    text: text.into(),
                    bounds: Rect {
                        x,
                        y: 40. + line as f64 * 30.,
                        w,
                        h: 20.,
                    },
                    confidence: 95.,
                    block: "body".into(),
                    line: line.to_string(),
                })
        })
        .collect();
    p
}

#[test]
fn proposal_width_guidance_preserves_span_semantics_and_font_uncertainty() {
    for anchored in [false, true] {
        let (dir, mut p, _) = if anchored {
            anchored_fixture()
        } else {
            fixture()
        };
        let original_fit = p.profiles[0].fits[0].clone();
        let mut second = original_fit.clone();
        second.size *= 1.2;
        p.profiles[0].fits.push(second);
        let guidance = unredact::expansion::width_guidance(&p, dir.path(), "g").unwrap();
        let info = &guidance["t"];
        let reference = info["references"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["replacement"] == "example")
            .unwrap();
        let font = Font::open(&original_fit.font).unwrap();
        let text = if anchored {
            "Before example after"
        } else {
            "example"
        };
        let expected = original_fit.predict(&font, text).unwrap();
        assert!((reference["predicted_span_pixels"][0].as_f64().unwrap() - expected).abs() < 1e-9);
        assert!(reference["predicted_span_pixels"][1].as_f64().unwrap() > expected);
        let observed = if anchored {
            p.pages[0].words[1].bounds.x + p.pages[0].words[1].bounds.w
                - p.pages[0].words[0].bounds.x
        } else {
            p.targets[0].bounds.w - 4.
        };
        assert!((info["observed_span_pixels"][0].as_f64().unwrap() - observed).abs() < 1e-9);
        // Unknown padding remains an interval; no midpoint becomes a hard length.
        p.targets[0].evidence = Evidence::Opaque {
            padding_min: 1.,
            padding_max: 8.,
            measurement_sigma: 2.,
        };
        let g = unredact::expansion::width_guidance(&p, dir.path(), "g").unwrap();
        assert_eq!(
            g["t"]["observed_span_pixels"],
            serde_json::json!([p.targets[0].bounds.w - 8., p.targets[0].bounds.w - 1.])
        );
        p.targets[0].evidence = Evidence::MosaicAuto {
            min_block: 2,
            max_block: 8,
            origin_x: 0.,
            baseline_y: 20.,
            noise_sigma: 1.,
        };
        let g = unredact::expansion::width_guidance(&p, dir.path(), "g").unwrap();
        assert!(g["t"]["observed_span_pixels"].is_null());
        assert!(g["t"]["references"].as_array().unwrap().is_empty());
    }
}

#[test]
fn paragraph_context_uses_line_order_and_preserves_supplied_context() {
    let mut p = paragraph_fixture();
    let c = unredact::context_layout::paragraph_context(&p, &p.targets[0]).unwrap();
    assert_eq!(c.left, "The review was nearly complete before");
    assert_eq!(
        c.right,
        "the committee adjourned. Further details followed."
    );
    assert_eq!(c.left_word_indices, vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(c.right_word_indices, vec![6, 7, 8, 9, 10, 11]);
    assert!(c.layout_notes.iter().any(|n| n.contains("continued")));
    p.targets[0].left_context = "Exact supplied context!".into();
    let c = unredact::context_layout::paragraph_context(&p, &p.targets[0]).unwrap();
    assert_eq!(c.left, p.targets[0].left_context);
    assert!(c.left_word_indices.is_empty());
    assert_eq!(c.left_source, "supplied");
}

#[test]
fn paragraph_context_bounds_extended_ocr_bytes() {
    // The long OCR word is on an adjacent line, outside the initial row check.
    // Check both directions and UTF-8 byte length rather than character count.
    for index in [4, 9] {
        let mut p = paragraph_fixture();
        p.pages[0].words[index].text = "é".repeat(2049);
        assert!(unredact::contextual::visible_context(&p, &p.targets[0]).is_ok());
        let error = unredact::context_layout::paragraph_context(&p, &p.targets[0])
            .expect_err("Extended context must respect the byte limit");
        assert!(error.to_string().contains("4096 bytes"));
    }
}

#[test]
fn paragraph_context_stops_at_masks_unreliable_words_and_line_edge_masks() {
    let original = paragraph_fixture();
    for alteration in 0..4 {
        let mut p = original.clone();
        match alteration {
            0 => p.pages[0].words[2].confidence = 10.,
            1 => p.pages[0].proposed_redactions.push(Rect {
                x: 320.,
                y: 40.,
                w: 40.,
                h: 20.,
            }),
            2 => {
                let mut mask = p.targets[0].clone();
                mask.id = "other".into();
                mask.bounds = Rect {
                    x: 320.,
                    y: 40.,
                    w: 40.,
                    h: 20.,
                };
                p.targets.push(mask);
            }
            _ => p.pages[0].proposed_redactions.push(Rect {
                x: 710.,
                y: 70.,
                w: 20.,
                h: 20.,
            }),
        }
        let c = unredact::context_layout::paragraph_context(&p, &p.targets[0]).unwrap();
        if alteration < 3 {
            assert_eq!(c.left, "nearly complete before");
        } else {
            assert_eq!(c.right, "the committee adjourned.");
        }
    }
}

#[test]
fn paragraph_context_rejects_columns_large_line_gaps_and_hyphenation() {
    let original = paragraph_fixture();
    for alteration in 0..5 {
        let mut p = original.clone();
        match alteration {
            0 => {
                for w in &mut p.pages[0].words[9..] {
                    w.bounds.y += 70.;
                }
            }
            1 => {
                for w in &mut p.pages[0].words[9..] {
                    w.block = "other-column".into();
                }
            }
            2 => {
                for w in &mut p.pages[0].words[9..] {
                    w.bounds.x += 200.;
                }
            }
            3 => p.pages[0].words[8].text = "adjourn-".into(),
            _ => {
                let mut w = p.pages[0].words[9].clone();
                w.line = "overlapping-column".into();
                w.bounds.y = 71.;
                w.bounds.x = 730.;
                w.bounds.w = 50.;
                p.pages[0].words.push(w);
            }
        }
        let c = unredact::context_layout::paragraph_context(&p, &p.targets[0]).unwrap();
        assert!(
            !c.right.contains("Further"),
            "alteration {alteration}: {}",
            c.right
        );
    }
}

fn anchored_fixture() -> (tempfile::TempDir, Project, Vec<Candidate>) {
    let (dir, mut p, candidates) = fixture();
    let font = Font::open(&p.profiles[0].fits[0].font).unwrap();
    let size = p.profiles[0].fits[0].size;
    let origin = 100.0;
    let after = origin + size * font.measure("Before Amber Vale ", true).unwrap().advance;
    p.pages[0].words = [("Before", origin), ("after", after)]
        .into_iter()
        .map(|(text, x)| {
            let m = font.measure(text, true).unwrap();
            unredact::calibration::Word {
                text: text.into(),
                confidence: 99.0,
                block: "body".into(),
                line: "1".into(),
                bounds: Rect {
                    x: x + m.left * size,
                    y: 30.0,
                    w: m.ink_width() * size,
                    h: 20.0,
                },
            }
        })
        .collect();
    p.targets[0].bounds = Rect {
        x: origin + size * font.measure("Before ", true).unwrap().advance - 2.0,
        y: 25.0,
        w: size * font.measure("Amber Vale", true).unwrap().advance + 4.0,
        h: 30.0,
    };
    unredact::scan_span::attach(&p.pages[0], &mut p.targets[0]).unwrap();
    let image = GrayImage::white(800, 400).unwrap();
    image.save(&dir.path().join("page.pgm")).unwrap();
    p.pages[0].image_sha256 = unredact::project::hash(&image.pixels);
    (dir, p, candidates)
}

// Place visible words by shaped advances, independently of the estimator's
// pair-width formula. The hidden candidate never enters training_texts.
fn justified_fixture(extra: f64) -> (tempfile::TempDir, Project, Vec<Candidate>) {
    let (dir, mut p, candidates) = anchored_fixture();
    let fit = &p.profiles[0].fits[0];
    let font = Font::open(&fit.font).unwrap();
    p.pages[0].words[1].bounds.x += 3.0 * extra;
    p.targets[0].bounds.x += extra;
    p.targets[0].bounds.w += extra;
    let texts: Vec<_> = (0..100)
        .map(|i| format!("word{i}"))
        .filter(|s| !held_out(s))
        .take(5)
        .collect();
    let mut prefix = String::new();
    for (i, text) in texts.iter().enumerate() {
        let m = font.measure(text, true).unwrap();
        let advance = if prefix.is_empty() {
            0.0
        } else {
            font.measure(&prefix, true).unwrap().advance
        };
        let x = 500.0 + advance * fit.size + i as f64 * extra + m.left * fit.size;
        p.pages[0].words.push(unredact::calibration::Word {
            text: text.clone(),
            confidence: 99.0,
            block: "body".into(),
            line: "1".into(),
            bounds: Rect {
                x,
                y: 30.0,
                w: m.ink_width() * fit.size,
                h: 20.0,
            },
        });
        prefix.push_str(text);
        prefix.push(' ');
    }
    p.profiles[0].training_texts = texts;
    p.pages[0].width = 2048;
    let image = GrayImage::white(2048, 400).unwrap();
    image.save(&dir.path().join("page.pgm")).unwrap();
    p.pages[0].image_sha256 = unredact::project::hash(&image.pixels);
    (dir, p, candidates)
}

#[test]
fn line_spacing_learns_visible_pairs_and_is_opt_in() {
    let (dir, mut p, candidates) = justified_fixture(3.0);
    let before = solver::solve(&p, dir.path(), "g", &candidates, 2).unwrap();
    assert!(before.line_spacing.is_empty());
    assert!(
        serde_json::to_value(&before)
            .unwrap()
            .get("line_spacing")
            .is_none()
    );
    let before_right = before.candidates.iter().find(|c| c.id == "right").unwrap();
    assert!(before_right.observations[0].standardized_error > 5.0);
    if let Evidence::ScanSpan { local_spacing, .. } = &mut p.targets[0].evidence {
        *local_spacing = true;
    }
    let after = solver::solve(&p, dir.path(), "g", &candidates, 2).unwrap();
    let estimate = &after.line_spacing["t"][0];
    assert_eq!(estimate.samples.len(), 4);
    assert!((estimate.adjustment - 3.0).abs() < 1e-8);
    assert!(estimate.reason.is_none());
    assert_eq!(estimate.scatter, 0.5);
    assert_eq!(after.candidates[0].id, "right");
    assert!(after.candidates[0].observations[0].standardized_error < 1e-8);
    let difference = after.candidates[0].observations[0].predicted_width.unwrap()
        - before_right.observations[0].predicted_width.unwrap();
    assert!((difference - 9.0).abs() < 1e-8);
}

#[test]
fn line_spacing_excludes_masked_and_withheld_pairs_without_bridging_words() {
    let (_, original, _) = justified_fixture(3.0);
    let font = Font::open(&original.profiles[0].fits[0].font).unwrap();
    for alteration in 0..4 {
        let mut p = original.clone();
        match alteration {
            0 => p.pages[0].words[4].confidence = 20.0,
            1 => {
                let held = (0..100)
                    .map(|i| format!("held{i}"))
                    .find(|s| held_out(s))
                    .unwrap();
                p.profiles[0].training_texts.push(held.clone());
                p.pages[0].words[4].text = held;
            }
            2 => {
                p.profiles[0].training_texts.remove(2);
            }
            _ => {
                let bounds = p.pages[0].words[4].bounds;
                p.pages[0].proposed_redactions.push(bounds);
            }
        }
        let e = unredact::line_spacing::estimate(
            &p.pages[0],
            &p.profiles[0],
            &p.targets[0],
            &p.profiles[0].fits[0],
            &font,
        )
        .unwrap();
        assert_eq!(e.samples.len(), 2, "alteration {alteration}");
        assert_eq!(e.adjustment, 0.0);
        assert_eq!(e.scatter, 0.0);
        assert!(e.reason.is_some());
        assert!(
            e.samples
                .iter()
                .all(|s| s.left_word != 4 && s.right_word != 4)
        );
    }
}

#[test]
fn line_spacing_excludes_other_registered_targets_and_deduplicates_pairs() {
    let (_, mut p, _) = justified_fixture(3.0);
    if let Evidence::ScanSpan { local_spacing, .. } = &mut p.targets[0].evidence {
        *local_spacing = true;
    }
    let mut other = p.targets[0].clone();
    other.id = "other".into();
    other.group = "different".into();
    other.bounds = p.pages[0].words[4].bounds;
    p.targets.push(other);
    let fit = &p.profiles[0].fits[0];
    let fonts = [(fit.font.clone(), Font::open(&fit.font).unwrap())]
        .into_iter()
        .collect();
    let estimates = unredact::line_spacing::prepare(&p, &[&p.targets[0]], &fonts).unwrap();
    assert_eq!(estimates["t"][0].samples.len(), 2);
    assert_eq!(estimates["t"][0].adjustment, 0.0);
    p.targets.pop();
    let mut repeated = p.pages[0].words[2..].to_vec();
    for w in &mut repeated {
        w.bounds.x += 600.0;
    }
    p.pages[0].words.extend(repeated);
    let estimates = unredact::line_spacing::prepare(&p, &[&p.targets[0]], &fonts).unwrap();
    assert_eq!(estimates["t"][0].samples.len(), 4);
}

#[test]
fn incremental_search_uses_the_same_line_spacing_as_complete_scoring() {
    let (dir, mut p, _) = justified_fixture(3.0);
    if let Evidence::ScanSpan { local_spacing, .. } = &mut p.targets[0].evidence {
        *local_spacing = true;
    }
    let fit = &p.profiles[0].fits[0];
    let font = Font::open(&fit.font).unwrap();
    let expected = fit.predict(&font, "Before Amber after").unwrap() + 6.0;
    let script = dir.path().join("line_spacing_provider.py");
    std::fs::write(
        &script,
        format!(
            r#"import json,sys
r=json.load(sys.stdin)
p=r['prefixes'][0]
if r['round']==1:
    extension={{'parent_id':p['id'],'append':'Amber','complete':False}}
else:
    o=p['observations'][0]
    assert abs(o['predicted_width']-{expected})<1e-8
    assert o['sigma']>1.5
    extension={{'parent_id':p['id'],'append':' Vale','complete':True}}
json.dump({{'extensions':[extension],'exhaustive_for_prefixes':True}},sys.stdout)
"#
        ),
    )
    .unwrap();
    let result = search::run(
        &p,
        dir.path(),
        "g",
        Path::new("python3"),
        &[script.display().to_string()],
        SearchConfig::default(),
        None,
    )
    .unwrap();
    assert_eq!(result.rounds.len(), 2);
    let report = result.result.unwrap();
    assert_eq!(report.candidates[0].name, "Amber Vale");
    assert!(report.candidates[0].observations[0].standardized_error < 1e-8);
}

#[test]
fn scan_anchors_reject_ink_missing_from_ocr_and_changed_source_pixels() {
    let (dir, mut p, candidates) = anchored_fixture();
    let mut image = GrayImage::read(&dir.path().join("page.pgm")).unwrap();
    // An unrecognized glyph in the whitespace between the left anchor and bar.
    let x = (p.pages[0].words[0].bounds.x + p.pages[0].words[0].bounds.w).ceil() as usize;
    for y in 35..40 {
        image.pixels[y * image.width + x] = 0;
    }
    image.save(&dir.path().join("page.pgm")).unwrap();
    let changed = solver::solve(&p, dir.path(), "g", &candidates, 2).unwrap_err();
    assert!(
        changed
            .to_string()
            .contains("differs from imported evidence")
    );
    p.pages[0].image_sha256 = unredact::project::hash(&image.pixels);
    let omitted = solver::solve(&p, dir.path(), "g", &candidates, 2).unwrap_err();
    assert!(omitted.to_string().contains("Unexplained ink"));
    let config = SearchConfig {
        rounds: 1,
        ..Default::default()
    };
    // The search path must fail before contacting the provider, too.
    let failed = search::run(
        &p,
        dir.path(),
        "g",
        Path::new("/missing-provider"),
        &[],
        config,
        None,
    );
    assert!(failed.unwrap_err().to_string().contains("Unexplained ink"));
}

#[test]
fn scan_anchors_measure_visible_span_independently_of_mask_padding() {
    let (dir, mut p, candidates) = anchored_fixture();
    let before = solver::solve(&p, dir.path(), "g", &candidates, 2).unwrap();
    assert_eq!(before.candidates[0].id, "right");
    assert!(before.candidates[0].observations[0].standardized_error < 1e-8);
    // A mask-width score would change; anchored endpoints do not move.
    p.targets[0].bounds.x += 3.0;
    p.targets[0].bounds.w -= 6.0;
    let after = solver::solve(&p, dir.path(), "g", &candidates, 2).unwrap();
    for (a, b) in before.candidates.iter().zip(&after.candidates) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.evidence_score, b.evidence_score);
    }
    let serialized = serde_json::to_vec(&p).unwrap();
    let loaded: Project = serde_json::from_slice(&serialized).unwrap();
    loaded.validate_target(&loaded.targets[0]).unwrap();
    p.profiles[0].fits[0].width_metric = unredact::calibration::WidthMetric::Advance;
    assert!(p.validate_target(&p.targets[0]).is_err());
}

#[test]
fn scan_anchors_reject_missing_unreliable_or_omitted_words() {
    let (_, original, _) = anchored_fixture();
    for alteration in 0..5 {
        let mut p = original.clone();
        match alteration {
            0 => p.pages[0].words[0].confidence = 20.0,
            1 => p.pages[0].words[1].bounds.y += 100.0,
            2 => p.pages[0].words[0].bounds.w += 20.0,
            3 => {
                p.pages[0].words.remove(1);
            }
            _ => {
                let mut omitted = p.pages[0].words[0].clone();
                omitted.bounds.x += omitted.bounds.w + 0.5;
                omitted.bounds.w = 1.0;
                p.pages[0].words.push(omitted);
            }
        }
        assert!(
            p.validate_target(&p.targets[0]).is_err(),
            "alteration {alteration}"
        );
    }
    let mut p = original;
    p.targets[0].evidence = Evidence::Width {
        width: 100.0,
        measurement_sigma: 1.0,
    };
    p.pages[0].words[0].confidence = 0.0;
    let mut distant = p.pages[0].words[0].clone();
    distant.confidence = 99.0;
    distant.bounds.x -= 60.0;
    p.pages[0].words.push(distant);
    assert!(unredact::scan_span::attach(&p.pages[0], &mut p.targets[0]).is_err());
    assert!(matches!(p.targets[0].evidence, Evidence::Width { .. }));
}

#[test]
fn scan_anchors_accept_x_height_words_beside_ascenders_and_descenders() {
    let (_, mut p, _) = anchored_fixture();
    p.pages[0].words[0].bounds.y = 28.0;
    p.pages[0].words[0].bounds.h = 31.0;
    p.pages[0].words[1].text = "as".into();
    p.pages[0].words[1].bounds.y = 36.0;
    p.pages[0].words[1].bounds.h = 16.0;
    p.validate_target(&p.targets[0]).unwrap();
}

#[test]
fn mask_aware_ocr_preserves_evidence_and_cannot_invalidate_existing_profiles() {
    let (dir, mut p, _) = fixture();
    let font = Font::open(&p.profiles[0].fits[0].font).unwrap();
    let mut image = font
        .render(
            "Visible words before and after",
            28.0,
            true,
            0.0,
            20.0,
            70.0,
            800,
            400,
        )
        .unwrap();
    let mask = Rect {
        x: 10.0,
        y: 100.0,
        w: 180.0,
        h: 30.0,
    };
    for y in 100..130 {
        image.pixels[y * 800 + 10..y * 800 + 190].fill(0);
    }
    image.save(&dir.path().join("page.pgm")).unwrap();
    p.pages[0].image_sha256 = unredact::project::hash(&image.pixels);
    p.save(dir.path()).unwrap();
    assert!(unredact::project::refresh_visible_ocr(dir.path(), "p", &[mask]).is_err());
    p.profiles.clear();
    p.targets.clear();
    p.save(dir.path()).unwrap();
    unredact::project::refresh_visible_ocr(dir.path(), "p", &[mask]).unwrap();
    let loaded = Project::load(dir.path()).unwrap();
    assert!(!loaded.pages[0].words.is_empty());
    assert!(
        loaded.pages[0]
            .words
            .iter()
            .all(|w| !w.bounds.intersects(mask))
    );
    assert_eq!(
        GrayImage::read(&dir.path().join("page.pgm"))
            .unwrap()
            .pixels,
        image.pixels
    );
    let input = std::fs::read_dir(dir.path().join("ocr-inputs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|s| s == "pgm"))
        .unwrap();
    let whitened = GrayImage::read(&input).unwrap();
    for y in 0..400 {
        for x in 0..800 {
            let expected = if (100..130).contains(&y) && (10..190).contains(&x) {
                255
            } else {
                image.pixels[y * 800 + x]
            };
            assert_eq!(whitened.pixels[y * 800 + x], expected);
        }
    }
    // A model-only directory has no Tesseract TSV config file. Explicit model
    // recognition must request TSV via a parameter and preserve model identity.
    let model_dir = dir.path().join("model-only");
    std::fs::create_dir(&model_dir).unwrap();
    assert!(
        unredact::project::refresh_visible_ocr_with_tessdata(
            dir.path(),
            "p",
            &[mask],
            Some(&model_dir)
        )
        .is_err()
    );
    let installed = [
        "/usr/share/tessdata/eng.traineddata",
        "/usr/share/tesseract-ocr/5/tessdata/eng.traineddata",
    ]
    .into_iter()
    .map(Path::new)
    .find(|p| p.is_file())
    .expect("English Tesseract model required");
    let data = std::fs::read(installed).unwrap();
    let model_hash = unredact::project::hash(&data);
    std::fs::write(model_dir.join("eng.traineddata"), data).unwrap();
    unredact::project::refresh_visible_ocr_with_tessdata(
        dir.path(),
        "p",
        &[mask],
        Some(&model_dir),
    )
    .unwrap();
    let records: Vec<serde_json::Value> = std::fs::read_dir(dir.path().join("ocr-inputs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .map(|p| serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap())
        .collect();
    assert!(records.iter().any(|r| r["model"].is_null()));
    assert!(
        records
            .iter()
            .any(|r| r["model"]["eng_traineddata_sha256"] == model_hash)
    );
    assert!(!Project::load(dir.path()).unwrap().pages[0].words.is_empty());
    let before = std::fs::read(dir.path().join("project.json")).unwrap();
    assert!(
        unredact::project::refresh_visible_ocr(
            dir.path(),
            "p",
            &[Rect {
                x: 790.0,
                w: 20.0,
                ..mask
            }]
        )
        .is_err()
    );
    assert_eq!(
        before,
        std::fs::read(dir.path().join("project.json")).unwrap()
    );
    image.pixels[0] = 0;
    image.save(&dir.path().join("page.pgm")).unwrap();
    assert!(unredact::project::refresh_visible_ocr(dir.path(), "p", &[mask]).is_err());
}

#[test]
fn ocr_masks_include_dense_scan_fringes_without_crossing_white_gaps() {
    let mut image = GrayImage::white(120, 60).unwrap();
    // Gray scanned boundary around a black core, with separate neighboring ink.
    for y in 8..42 {
        for x in 18..82 {
            image.pixels[y * 120 + x] = 150;
        }
    }
    for y in 10..40 {
        for x in 20..80 {
            image.pixels[y * 120 + x] = 0;
        }
    }
    for y in 8..42 {
        image.pixels[y * 120 + 15] = 0;
        image.pixels[y * 120 + 85] = 0;
    }
    let before = image.pixels.clone();
    let r = image
        .ocr_mask_bounds(Rect {
            x: 20.0,
            y: 10.0,
            w: 60.0,
            h: 30.0,
        })
        .unwrap();
    assert_eq!((r.x, r.y, r.w, r.h), (18.0, 8.0, 64.0, 34.0));
    assert_eq!(image.pixels, before);
    let empty = Rect {
        x: 90.0,
        y: 10.0,
        w: 20.0,
        h: 30.0,
    };
    assert_eq!(image.ocr_mask_bounds(empty).unwrap().x, 90.0);
    assert!(image.ocr_mask_bounds(Rect { x: 115.0, ..empty }).is_err());
    let solid = GrayImage {
        width: 120,
        height: 60,
        pixels: vec![0; 7200],
    };
    let capped = solid
        .ocr_mask_bounds(Rect {
            x: 20.0,
            y: 10.0,
            w: 60.0,
            h: 30.0,
        })
        .unwrap();
    assert_eq!(
        (capped.x, capped.y, capped.w, capped.h),
        (12.0, 2.0, 76.0, 46.0)
    );
}

#[test]
fn redaction_proposals_respect_local_text_scale_and_keep_uncertain_bars() {
    use unredact::calibration::Word;
    let small = Rect {
        x: 200.0,
        y: 48.0,
        w: 25.0,
        h: 6.0,
    };
    let bar = Rect {
        x: 200.0,
        y: 10.0,
        w: 100.0,
        h: 50.0,
    };
    let words: Vec<_> = [10.0, 70.0, 130.0, 350.0]
        .into_iter()
        .map(|x| Word {
            text: "visible".into(),
            confidence: 95.0,
            bounds: Rect {
                x,
                y: 10.0,
                w: 50.0,
                h: 50.0,
            },
            block: "b".into(),
            line: "l".into(),
        })
        .collect();
    let kept = unredact::project::plausible_redactions(&[small, bar], &words);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].h, 50.0);
    // Without enough nearby evidence, a proposal remains uncertain, not erased.
    assert_eq!(
        unredact::project::plausible_redactions(&[small], &words[..2]).len(),
        1
    );
    let tiny: Vec<_> = words
        .iter()
        .cloned()
        .map(|mut w| {
            w.bounds.y = 47.0;
            w.bounds.h = 12.0;
            w
        })
        .collect();
    assert_eq!(
        unredact::project::plausible_redactions(&[small], &tiny).len(),
        1
    );
}

#[test]
fn scan_spacing_reaches_solver_and_prevents_target_leakage() {
    let (dir, mut project, candidates) = fixture();
    project.targets[0].bounds.w += 5.0;
    let original = solver::solve(&project, dir.path(), "g", &candidates, 10).unwrap();
    assert!(
        !original
            .candidates
            .iter()
            .find(|c| c.id == "right")
            .unwrap()
            .compatible
    );
    project.profiles[0].fits[0].spacing = Some(unredact::spacing::SpacingFit {
        adjustment: 5.0,
        proposed_adjustment: 5.0,
        training_pairs: 20,
        validation_pairs: 8,
        validation_before: 5.0,
        validation_after: 0.1,
        audit_pairs: 5,
        audit_before: Some(5.0),
        audit_after: Some(0.1),
    });
    let corrected = solver::solve(&project, dir.path(), "g", &candidates, 10).unwrap();
    assert!(
        corrected
            .candidates
            .iter()
            .find(|c| c.id == "right")
            .unwrap()
            .compatible
    );
    project.profiles[0].spacing = Some(unredact::spacing::Report {
        samples: vec![unredact::spacing::Sample {
            text: "Amber Vale".into(),
            bounds: project.targets[0].bounds,
            split: unredact::spacing::Split::Audit,
        }],
        reason: None,
    });
    let error = project.validate_target(&project.targets[0]).unwrap_err();
    assert!(error.to_string().contains("overlaps spacing"));
}

#[test]
fn incremental_provider_uses_both_contexts_and_geometric_feedback() {
    let (dir, mut p, _) = fixture();
    let mut page = p.pages[0].clone();
    page.id = "p2".into();
    page.image_sha256 = "second-image".into();
    let mut profile = p.profiles[0].clone();
    profile.id = "f2".into();
    profile.page = "p2".into();
    profile.fits[0].size = 35.0;
    let font = Font::open(&profile.fits[0].font).unwrap();
    let mut target = p.targets[0].clone();
    target.id = "t2".into();
    target.page = "p2".into();
    target.profile = "f2".into();
    target.bounds.w = profile.fits[0].predict(&font, "Amber Vale").unwrap() + 4.0;
    p.pages.push(page);
    p.profiles.push(profile);
    p.targets.push(target);
    let script = dir.path().join("incremental.py");
    std::fs::write(&script,r#"import json,sys
r=json.load(sys.stdin)
assert r['schema_version']==2 and r['task']=='extend_prefixes'
assert len(r['targets'])==2
assert r['targets'][0]['left_context']=='Hello'
assert r['targets'][0]['right_context']=='arrived'
p=r['prefixes'][0]
if r['round']==1:
    assert p['text']==''
    extensions=[{'parent_id':p['id'],'append':'Amber ','complete':False},
                {'parent_id':p['id'],'append':'WWWWWWWWWWWWW','complete':True,'context_score':1e100}]
else:
    assert p['text']=='Amber '
    assert len(p['observations'])==2
    assert p['observations'][0]['remaining_pixels'][1]>0
    assert p['observations'][1]['predicted_width']>p['observations'][0]['predicted_width']
    assert r['completed_feedback'][0]['compatible']==False
    extensions=[{'parent_id':p['id'],'append':'Vale','complete':True}]
json.dump({'extensions':extensions,'exhaustive_for_prefixes':True},sys.stdout)
"#).unwrap();
    let trace = dir.path().join("trace.jsonl");
    let result = search::run(
        &p,
        dir.path(),
        "g",
        Path::new("python3"),
        &[script.display().to_string()],
        SearchConfig::default(),
        Some(&trace),
    )
    .unwrap();
    assert!(!result.budget_truncated);
    assert_eq!(result.rounds.len(), 2);
    assert_eq!(result.completed_candidates, 2);
    let report = result.result.unwrap();
    assert_eq!(report.independent_observations, 2);
    assert_eq!(report.candidates[0].name, "Amber Vale");
    assert!(report.candidates[0].compatible);
    assert!(!report.candidates[1].compatible);
    let entries: Vec<serde_json::Value> = std::fs::read_to_string(trace)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(entries.iter().filter(|v| v["type"] == "request").count(), 2);
    assert!(
        report.candidates[0]
            .source_ids
            .iter()
            .any(|s| s.contains(&result.rounds[1].request_sha256))
    );
}

#[test]
fn unfinished_prefixes_are_not_rejected_as_complete_short_words() {
    let (dir, p, _) = fixture();
    let script = dir.path().join("partial.py");
    std::fs::write(
        &script,
        r#"import json,sys
r=json.load(sys.stdin)
if r['round']==1:
    root=r['prefixes'][0]['id']
    ex=[{'parent_id':root,'append':'A','complete':False},
        {'parent_id':root,'append':'WWWWWWWWWWWWW','complete':False}]
else:
    assert len(r['prefixes'])==2
    p=next(p for p in r['prefixes'] if p['text']=='A')
    assert next(p for p in r['prefixes'] if p['text'].startswith('W'))['heuristic_overrun']>0
    ex=[{'parent_id':p['id'],'append':'mber Vale','complete':True}]
json.dump({'extensions':ex},sys.stdout)
"#,
    )
    .unwrap();
    let result = search::run(
        &p,
        dir.path(),
        "g",
        Path::new("python3"),
        &[script.display().to_string()],
        SearchConfig::default(),
        None,
    )
    .unwrap();
    assert_eq!(result.result.unwrap().candidates[0].name, "Amber Vale");
    assert!(!result.budget_truncated);
}

#[test]
fn round_and_beam_limits_are_reported_without_fabricating_a_completion() {
    let (dir, p, _) = fixture();
    let script = dir.path().join("limited.py");
    std::fs::write(&script,r#"import json,sys
r=json.load(sys.stdin);p=r['prefixes'][0]
json.dump({'extensions':[{'parent_id':p['id'],'append':'Amber ','complete':False},
                        {'parent_id':p['id'],'append':'WWWWWWWWWWWW ','complete':False,'context_score':1e99}]},sys.stdout)
"#).unwrap();
    let config = SearchConfig {
        rounds: 1,
        beam_width: 1,
        ..SearchConfig::default()
    };
    let result = search::run(
        &p,
        dir.path(),
        "g",
        Path::new("python3"),
        &[script.display().to_string()],
        config,
        None,
    )
    .unwrap();
    assert!(result.result.is_none());
    assert!(result.budget_truncated);
    assert_eq!(result.unexpanded_prefixes, 1);
    assert_eq!(result.rounds[0].beam_discards, 1);
    assert!(result.truncation_reasons.contains(&"beam_width".into()));
    assert!(result.truncation_reasons.contains(&"round_budget".into()));
}

#[test]
fn completion_limit_records_untested_candidates() {
    let (dir, p, _) = fixture();
    let script = dir.path().join("complete.py");
    std::fs::write(
        &script,
        r#"import json,sys
r=json.load(sys.stdin);p=r['prefixes'][0]
json.dump({'extensions':[{'parent_id':p['id'],'append':'Amber Vale','complete':True},
                        {'parent_id':p['id'],'append':'Blair Stone','complete':True}]},sys.stdout)
"#,
    )
    .unwrap();
    let config = SearchConfig {
        max_completed: 1,
        ..SearchConfig::default()
    };
    let result = search::run(
        &p,
        dir.path(),
        "g",
        Path::new("python3"),
        &[script.display().to_string()],
        config,
        None,
    )
    .unwrap();
    assert!(result.budget_truncated);
    assert_eq!(result.completed_candidates, 1);
    assert_eq!(result.rounds[0].completion_budget_discards, 1);
    assert_eq!(result.result.unwrap().tested_candidates, 1);
}

#[test]
fn provider_cannot_inject_an_unrequested_search_parent() {
    let (dir, p, _) = fixture();
    let script = dir.path().join("bad_parent.py");
    std::fs::write(&script,"import json,sys\njson.load(sys.stdin)\njson.dump({'extensions':[{'parent_id':'made-up','append':'Amber Vale','complete':True}]},sys.stdout)\n").unwrap();
    let error = search::run(
        &p,
        dir.path(),
        "g",
        Path::new("python3"),
        &[script.display().to_string()],
        SearchConfig::default(),
        None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("parent outside"));
}

#[test]
fn incremental_search_deduplicates_duplicate_target_feedback() {
    let (dir, mut p, _) = fixture();
    let mut copy = p.targets[0].clone();
    copy.id = "copy".into();
    p.targets.push(copy);
    let script = dir.path().join("one_target.py");
    std::fs::write(&script,"import json,sys\nr=json.load(sys.stdin)\nassert len(r['targets'])==1\njson.dump({'extensions':[{'parent_id':r['prefixes'][0]['id'],'append':'Amber Vale','complete':True}]},sys.stdout)\n").unwrap();
    let result = search::run(
        &p,
        dir.path(),
        "g",
        Path::new("python3"),
        &[script.display().to_string()],
        SearchConfig::default(),
        None,
    )
    .unwrap();
    assert_eq!(result.result.unwrap().independent_observations, 1);
}

#[test]
fn dictionary_word_provider_matches_full_candidate_evaluation() {
    let (dir, p, c) = fixture();
    let dictionary = dir.path().join("candidates.json");
    std::fs::write(
        &dictionary,
        serde_json::to_vec(&solver::Candidates {
            candidates: c.clone(),
        })
        .unwrap(),
    )
    .unwrap();
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/word_provider.py");
    let result = search::run(
        &p,
        dir.path(),
        "g",
        Path::new("python3"),
        &[
            example.display().to_string(),
            dictionary.display().to_string(),
        ],
        SearchConfig::default(),
        None,
    )
    .unwrap();
    let direct = solver::solve(&p, dir.path(), "g", &c, 10).unwrap();
    let incremental = result.result.unwrap();
    assert_eq!(incremental.candidates[0].name, direct.candidates[0].name);
    assert_eq!(
        incremental.candidates[0].evidence_score,
        direct.candidates[0].evidence_score
    );
    assert_eq!(
        incremental.compatible_candidates,
        direct.compatible_candidates
    );
    assert!(
        result
            .rounds
            .iter()
            .all(|r| r.provider_exhaustive_for_prefixes)
    );
}

#[test]
fn provider_timeout_includes_descendants_holding_pipes_after_parent_exit() {
    let (dir, p, _) = fixture();
    let script = dir.path().join("fork.py");
    std::fs::write(&script,"import json,sys,os,time\njson.load(sys.stdin)\nif os.fork()==0:\n time.sleep(10)\n os._exit(0)\njson.dump({'candidates':[]},sys.stdout)\nsys.stdout.flush()\n").unwrap();
    let start = std::time::Instant::now();
    let error = solver::provider(
        Path::new("python3"),
        &[script.display().to_string()],
        &p,
        "g",
        &[],
        std::time::Duration::from_millis(150),
    )
    .unwrap_err();
    assert!(error.to_string().contains("timed out"));
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn oversized_provider_output_is_stopped_before_process_exit() {
    let (dir, p, _) = fixture();
    let script = dir.path().join("too_large.py");
    std::fs::write(&script,"import json,sys,time\njson.load(sys.stdin)\nsys.stdout.write('x'*9000000)\nsys.stdout.flush()\ntime.sleep(10)\n").unwrap();
    let start = std::time::Instant::now();
    let error = solver::provider(
        Path::new("python3"),
        &[script.display().to_string()],
        &p,
        "g",
        &[],
        std::time::Duration::from_secs(5),
    )
    .unwrap_err();
    assert!(error.to_string().contains("too large"));
    assert!(start.elapsed() < std::time::Duration::from_secs(3));
}
#[test]
fn pgm_keeps_whitespace_valued_first_pixel() {
    let bytes = b"P5\n# comment\n3 1\n255\n\n\r ";
    let image = GrayImage::from_pgm(bytes).unwrap();
    assert_eq!(image.pixels, [10, 13, 32]);
    assert!(GrayImage::from_pgm(b"P5\n999999999 999999999\n255\n").is_err());
}
#[test]
fn crop_and_mosaic_use_original_pixels() {
    let mut image = GrayImage::white(4, 2).unwrap();
    image.pixels = vec![0, 100, 200, 255, 0, 100, 200, 255];
    let mixed = image.mosaic(2, 0, 0).unwrap();
    assert_eq!(mixed.pixels, [50, 50, 227, 227, 50, 50, 227, 227]);
    let crop = mixed
        .crop(Rect {
            x: 2.0,
            y: 0.0,
            w: 2.0,
            h: 2.0,
        })
        .unwrap();
    assert_eq!(crop.pixels, [227; 4]);
}
#[test]
fn actual_font_kerning_changes_advance() {
    let fonts = default_fonts();
    assert!(fonts.iter().any(|p| {
        let font = Font::open(p).unwrap();
        let a = font.measure("AVATAR", true).unwrap().advance;
        let b = font.measure("AVATAR", false).unwrap().advance;
        (a - b).abs() > 0.01
    }));
}
#[test]
fn native_pdf_preserves_fractional_coordinates() {
    let dir = tempfile::tempdir().unwrap();
    let font = Font::open(&default_fonts()[0]).unwrap();
    let path = dir.path().join("native.pdf");
    let canvas = PdfCanvas::new(&path, 300.0, 200.0).unwrap();
    canvas
        .text(&font, "Visible typography", 12.0, true, 0.0, 23.25, 40.5)
        .unwrap();
    canvas.finish().unwrap();
    let pdf = Pdf::open(&path).unwrap();
    assert_eq!(pdf.pages(), 1);
    let (chars, coverage) = pdf.chars(0).unwrap();
    let text: String = chars.iter().map(|c| c.text).collect();
    assert!(text.contains("Visible"), "{text}");
    assert!((chars[0].x1 - 23.25).abs() < 0.01);
    assert!(coverage < 0.01);
    assert!(pdf.render(0, 144.0).unwrap().ink_bounds(160).is_some());
}
#[test]
fn native_cache_owns_font_lifetimes() {
    for _ in 0..30 {
        let font = Font::open(&default_fonts()[0]).unwrap();
        let image = font
            .render("Repeated fonts", 20.0, true, 0.0, 10.0, 30.0, 220, 50)
            .unwrap();
        assert!(image.ink_bounds(160).is_some());
    }
}
#[test]
fn duplicate_observation_does_not_boost_evidence() {
    let (dir, mut p, c) = fixture();
    let first = solver::solve(&p, dir.path(), "g", &c, 10).unwrap();
    let mut duplicate = p.targets[0].clone();
    duplicate.id = "z-copy".into();
    p.targets.push(duplicate);
    let second = solver::solve(&p, dir.path(), "g", &c, 10).unwrap();
    assert_eq!(second.independent_observations, 1);
    assert_eq!(second.ignored_dependent_targets, ["z-copy"]);
    assert_eq!(
        first.candidates[1].evidence_score,
        second.candidates[1].evidence_score
    );
    assert_eq!(first.candidates[0].id, "right");
}
#[test]
fn explicit_dependence_deduplicates_different_locations() {
    let (dir, mut p, c) = fixture();
    p.targets[0].dependence_key = Some("same-scan".into());
    let mut duplicate = p.targets[0].clone();
    duplicate.id = "resized-copy".into();
    duplicate.bounds.x += 20.0;
    p.targets.push(duplicate);
    let result = solver::solve(&p, dir.path(), "g", &c, 10).unwrap();
    assert_eq!(result.independent_observations, 1);
}

#[test]
fn retained_shared_fit_can_establish_compatibility_without_winning_cost() {
    let (dir, mut project, candidates) = fixture();
    let mut alternative = project.profiles[0].fits[0].clone();
    alternative.validation_rmse = 0.8;
    alternative.validation_max_error = 0.8;
    alternative.ink_bias = 2.9 * 0.8_f64.hypot(0.1);
    project.profiles[0].fits[0].ink_bias = 1.6;
    let original = solver::solve(&project, dir.path(), "g", &candidates[..1], 10).unwrap();
    assert!(!original.candidates[0].compatible);
    project.profiles[0].fits.push(alternative);
    // Check both a single observation and two linked occurrences. A witness
    // must retain the same model across all occurrences sharing the profile.
    for count in 1..=2 {
        if count == 2 {
            let mut linked = project.targets[0].clone();
            linked.id = "linked".into();
            linked.bounds.y += 100.;
            project.targets.push(linked);
        }
        let report = solver::solve(&project, dir.path(), "g", &candidates[..1], 10).unwrap();
        let ranked = &report.candidates[0];
        assert!(ranked.compatible);
        assert!(ranked.observations.iter().all(|o| o.fit_index == 0));
        assert!(
            ranked
                .observations
                .iter()
                .all(|o| o.standardized_error > 3.)
        );
        assert!(
            (ranked.evidence_score - original.candidates[0].evidence_score * count as f64).abs()
                < 1e-10
        );
        let witness = ranked.compatibility_witness.as_ref().unwrap();
        assert_eq!(witness.len(), count);
        assert!(witness.iter().all(|o| o.fit_index == 1));
        assert!(
            witness
                .iter()
                .all(|o| (o.standardized_error - 2.9).abs() < 1e-10)
        );
        assert_eq!(ranked.compatible_observations()[0].fit_index, 1);
    }
    project.profiles[0].fits[1].ink_bias = 3.1 * 0.8_f64.hypot(0.1);
    let no_witness = solver::solve(&project, dir.path(), "g", &candidates[..1], 10).unwrap();
    assert!(!no_witness.candidates[0].compatible);
    assert!(no_witness.candidates[0].compatibility_witness.is_none());
}

#[test]
fn compatibility_cannot_mix_typography_hypotheses_between_linked_occurrences() {
    let (dir, mut project, candidates) = fixture();
    let mut other = project.targets[0].clone();
    other.id = "other".into();
    other.bounds.y += 100.;
    other.bounds.w += 4.;
    project.targets.push(other);
    let mut alternate = project.profiles[0].fits[0].clone();
    alternate.ink_bias = 4.;
    project.profiles[0].fits.push(alternate);
    let both = solver::solve(&project, dir.path(), "g", &candidates[..1], 10).unwrap();
    assert!(!both.candidates[0].compatible);
    assert!(both.candidates[0].compatibility_witness.is_none());
    let targets = project.targets.clone();
    for target in targets {
        project.targets = vec![target];
        let alone = solver::solve(&project, dir.path(), "g", &candidates[..1], 10).unwrap();
        assert!(alone.candidates[0].compatible);
        assert!(alone.candidates[0].compatibility_witness.is_none());
    }
}

#[test]
fn compatible_surface_witness_supplies_context_text_and_word_normalization() {
    let (dir, mut project, mut candidates) = fixture();
    candidates.truncate(1);
    candidates[0].surfaces[0].text = "AmberVale".into();
    candidates[0].surfaces.push(Surface {
        text: "Amber Vale".into(),
        kind: "canonical".into(),
    });
    let fit = &mut project.profiles[0].fits[0];
    let font = Font::open(&fit.font).unwrap();
    let observed = fit.predict(&font, "AmberVale").unwrap();
    fit.ink_bias = 1.6;
    let adjustment = observed + 2.9 * 2_f64.hypot(0.1) - fit.predict(&font, "Amber Vale").unwrap();
    fit.spacing = Some(unredact::spacing::SpacingFit {
        adjustment,
        proposed_adjustment: adjustment,
        training_pairs: 12,
        validation_pairs: 4,
        validation_before: 2.,
        validation_after: 2.,
        audit_pairs: 0,
        audit_before: None,
        audit_after: None,
    });
    project.targets[0].evidence = Evidence::Width {
        width: observed,
        measurement_sigma: 0.1,
    };
    let physical = solver::solve(&project, dir.path(), "g", &candidates, 10).unwrap();
    let ranked = &physical.candidates[0];
    assert!(ranked.compatible);
    assert_eq!(ranked.observations[0].surface, "AmberVale");
    assert!(ranked.observations[0].standardized_error > 3.);
    let witness = ranked.compatibility_witness.as_ref().unwrap();
    assert_eq!(witness[0].surface, "Amber Vale");
    assert!((witness[0].standardized_error - 2.9).abs() < 1e-10);
    assert_eq!(witness[0].fit_index, ranked.observations[0].fit_index);
    let script = dir.path().join("witness-context.py");
    std::fs::write(&script, r#"import json,sys
r=json.load(sys.stdin)
assert len(r['items']) == 1
i=r['items'][0]
assert i['text'] == 'Amber Vale'
json.dump({'schema_version':1,'scores':[dict(id=i['id'],target=i['target'],token_count=3,token_log_score_sum=-4)]},sys.stdout)
"#).unwrap();
    let config = unredact::contextual::Config {
        executable: "python3".into(),
        args: vec![script.display().to_string()],
        timeout_seconds: 5,
        paragraph_context: false,
        word_normalization: true,
        sequence_score: false,
        max_compatible: 10,
        batch_items: 10,
        persistent_provider: false,
        physical_weight: 0.,
    };
    let context =
        unredact::contextual::run(&project, dir.path(), "g", &candidates, &config).unwrap();
    assert_eq!(context.ranked[0].mean_word_log_score, Some(-2.));
    assert_eq!(
        serde_json::to_value(context.physical).unwrap(),
        serde_json::to_value(physical).unwrap()
    );
}

#[test]
fn dependency_bridges_merge_components_for_every_target_order() {
    let (dir, mut project, candidates) = fixture();
    let mut original = project.targets[0].clone();
    original.id = "a-original".into();
    let mut other = original.clone();
    other.id = "b-rescaled".into();
    other.bounds.x += 20.;
    other.dependence_key = Some("shared-scan".into());
    // The bridge duplicates a-original's pixels and shares b-rescaled's key.
    // It must merge both components even if encountered after both of them.
    let mut bridge = original.clone();
    bridge.id = "c-bridge".into();
    bridge.dependence_key = other.dependence_key.clone();
    // An identical crop on a separately registered page is still a copy.
    let mut copied_page = project.pages[0].clone();
    copied_page.id = "copied-page".into();
    let mut copied_profile = project.profiles[0].clone();
    copied_profile.id = "copied-profile".into();
    copied_profile.page = copied_page.id.clone();
    bridge.page = copied_page.id.clone();
    bridge.profile = copied_profile.id.clone();
    project.pages.push(copied_page);
    project.profiles.push(copied_profile);
    let targets = [original.clone(), other, bridge];
    project.targets = vec![original];
    let expected = solver::solve(&project, dir.path(), "g", &candidates, 10).unwrap();
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        project.targets = order.iter().map(|&i| targets[i].clone()).collect();
        let report = solver::solve(&project, dir.path(), "g", &candidates, 10).unwrap();
        assert_eq!(report.independent_observations, 1, "order {order:?}");
        assert_eq!(
            report.ignored_dependent_targets,
            ["b-rescaled", "c-bridge"],
            "order {order:?}"
        );
        assert_eq!(
            serde_json::to_value(&report.candidates).unwrap(),
            serde_json::to_value(&expected.candidates).unwrap(),
            "order {order:?}"
        );
    }
    // Ignored targets must still pass validation.
    project.targets[1].bounds.x = -1.;
    assert!(solver::solve(&project, dir.path(), "g", &candidates, 10).is_err());
}

#[test]
fn nearby_crops_are_independent_and_output_order_is_stable() {
    let (dir, mut project, candidates) = fixture();
    let mut nearby = project.targets[0].clone();
    nearby.id = "a-nearby".into();
    // These coordinates previously collided after rounding to three decimals.
    nearby.bounds.x += 0.0001;
    project.targets.push(nearby);
    let first = solver::solve(&project, dir.path(), "g", &candidates, 10).unwrap();
    assert_eq!(first.independent_observations, 2);
    assert!(first.ignored_dependent_targets.is_empty());
    project.targets.reverse();
    let reordered = solver::solve(&project, dir.path(), "g", &candidates, 10).unwrap();
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(reordered).unwrap()
    );
}
#[test]
fn huge_ai_score_cannot_override_geometry() {
    let (dir, p, mut c) = fixture();
    c[1].context_score = Some(1e200);
    let result = solver::solve(&p, dir.path(), "g", &c, 10).unwrap();
    assert_eq!(result.candidates[0].id, "right");
    assert!(!result.candidates[1].compatible);
}
#[test]
fn missing_truth_is_reported_without_forcing_a_match() {
    let (dir, p, c) = fixture();
    let result = solver::solve(&p, dir.path(), "g", &c[1..], 10).unwrap();
    assert_eq!(result.status, "no_compatible_candidate");
    assert_eq!(result.compatible_candidates, 0);
}
#[test]
fn unknown_padding_remains_ambiguous() {
    let (dir, mut p, c) = fixture();
    p.targets[0].evidence = Evidence::Opaque {
        padding_min: 0.0,
        padding_max: 20.0,
        measurement_sigma: 0.1,
    };
    let report = solver::solve(&p, dir.path(), "g", &c, 10).unwrap();
    assert!(report.candidates[0].compatible);
    assert!(report.warnings.iter().any(|w| w.contains("padding")));
}

#[test]
fn identical_surface_forms_keep_evidence_ties_despite_ai_ordering() {
    let (dir, p, mut c) = fixture();
    c[1].surfaces = c[0].surfaces.clone();
    c[1].context_score = Some(99.0);
    let report = solver::solve(&p, dir.path(), "g", &c, 10).unwrap();
    assert_eq!(report.status, "ambiguous");
    assert_eq!(report.best_evidence_ties, 2);
    assert_eq!(report.candidates[0].id, "wrong");
    for candidate in &report.candidates {
        assert_eq!(candidate.evidence_rank_min, 1);
        assert_eq!(candidate.evidence_rank_max, 2);
    }
}
#[test]
fn ocr_parser_retains_confidence_and_typography_group() {
    let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t2\t3\t4\t1\t20\t30\t40\t12\t95.5\tVisible\n";
    let words = parse_tsv(tsv).unwrap();
    assert_eq!(words[0].block, "2:3");
    assert_eq!(words[0].confidence, 95.5);
    assert_eq!(words[0].bounds.w, 40.0);
}
#[test]
fn calibration_split_is_stable_by_word_identity() {
    assert_eq!(held_out("typography"), held_out("typography"));
    let words = [
        "typography",
        "measurement",
        "document",
        "the",
        "hello",
        "review",
        "world",
        "validation",
        "original",
        "calibration",
    ];
    assert!(words.iter().any(|w| held_out(w)));
    assert!(words.iter().any(|w| !held_out(w)));
}
#[test]
fn provider_gets_both_contexts_and_returns_candidates() {
    let (dir, p, _) = fixture();
    let script = dir.path().join("provider.py");
    std::fs::write(&script,"import json,sys\nr=json.load(sys.stdin)\nassert r['targets'][0]['left_context']=='Hello'\nassert r['targets'][0]['right_context']=='arrived'\njson.dump({'candidates':[{'id':'phrase','surfaces':[{'text':'the next word'}]}]},sys.stdout)\n").unwrap();
    let output = solver::provider(
        Path::new("python3"),
        &[script.display().to_string()],
        &p,
        "g",
        &[],
        std::time::Duration::from_secs(3),
    )
    .unwrap();
    assert_eq!(output[0].surfaces[0].text, "the next word");
}
#[test]
fn provider_timeout_is_bounded() {
    let (dir, p, _) = fixture();
    let script = dir.path().join("sleep.py");
    std::fs::write(&script, "import time\ntime.sleep(10)\n").unwrap();
    let started = std::time::Instant::now();
    let result = solver::provider(
        Path::new("python3"),
        &[script.display().to_string()],
        &p,
        "g",
        &[],
        std::time::Duration::from_millis(100),
    );
    assert!(result.is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(3));
}

#[test]
fn automatic_pixel_grid_matches_manual_scoring_and_rejects_clipped_completions() {
    let (dir, mut p, mut candidates) = fixture();
    let fit = &p.profiles[0].fits[0];
    let font = Font::open(&fit.font).unwrap();
    let raw = font
        .render("Amber Vale", fit.size, true, 0.0, 8.25, 40.25, 220, 80)
        .unwrap();
    let mosaic = raw.mosaic(5, 2, 3).unwrap();
    let mut page = GrayImage::white(800, 400).unwrap();
    page.paste(&mosaic, 10, 10).unwrap();
    page.save(&dir.path().join("page.pgm")).unwrap();
    p.targets[0].bounds = Rect {
        x: 10.0,
        y: 10.0,
        w: 220.0,
        h: 80.0,
    };
    p.targets[0].evidence = Evidence::MosaicAuto {
        min_block: 2,
        max_block: 12,
        origin_x: 8.0,
        baseline_y: 40.0,
        noise_sigma: 8.0,
    };
    let mut clipped = candidates[0].clone();
    clipped.id = "clipped".into();
    clipped.canonical_name = "Amber Vale followed by much more text".into();
    clipped.surfaces[0].text = clipped.canonical_name.clone();
    candidates.push(clipped);
    let automatic = solver::solve(&p, dir.path(), "g", &candidates, 10).unwrap();
    let predicted = automatic.candidates[0].observations[0]
        .mosaic_fit
        .as_ref()
        .unwrap();
    assert_eq!(automatic.candidates[0].name, "Amber Vale");
    assert!(predicted.inferred_grid);
    assert_eq!(
        predicted.grid,
        unredact::pixelation::Grid {
            block: 5,
            phase_x: 2,
            phase_y: 3
        }
    );
    assert!(!automatic.candidates.iter().any(|c| c.id == "clipped"));
    assert_eq!(
        automatic.pixelation_analysis["t"].hypotheses[0].grid,
        predicted.grid
    );
    p.targets[0].evidence = Evidence::Mosaic {
        block: 5,
        phase_x: 2,
        phase_y: 3,
        origin_x: 8.0,
        baseline_y: 40.0,
        noise_sigma: 8.0,
    };
    let manual = solver::solve(&p, dir.path(), "g", &candidates, 10).unwrap();
    assert_eq!(
        manual.candidates[0].evidence_score,
        automatic.candidates[0].evidence_score
    );
}

#[test]
fn absent_grid_is_not_reported_as_an_incompatible_vocabulary() {
    let (dir, mut p, candidates) = fixture();
    GrayImage::white(800, 400)
        .unwrap()
        .save(&dir.path().join("page.pgm"))
        .unwrap();
    p.targets[0].bounds = Rect {
        x: 0.0,
        y: 0.0,
        w: 200.0,
        h: 80.0,
    };
    p.targets[0].evidence = Evidence::MosaicAuto {
        min_block: 2,
        max_block: 12,
        origin_x: 8.0,
        baseline_y: 40.0,
        noise_sigma: 8.0,
    };
    let error = solver::solve(&p, dir.path(), "g", &candidates, 10).unwrap_err();
    assert!(error.to_string().contains("no supported pixelation grid"));
}

#[test]
fn native_gap_scores_candidates_without_hidden_text_in_pdf() {
    let (dir, mut p, mut candidates) = fixture();
    let path = dir.path().join("gap.pdf");
    let font_path = default_fonts()[0].clone();
    let font = Font::open(&font_path).unwrap();
    let canvas = PdfCanvas::new(&path, 700.0, 900.0).unwrap();
    let mut x = 25.0;
    let mut y = 30.0;
    let words = "document analysis original visible typography measurement observations reference comparisons evidence information different ordinary language uncertainty validation independent reliable scientific research process methods explanation physical parameters geometry shapes position spacing tracking baseline uncertainty separate calibration accurate approximate reporting knowledge practical development history mathematics resolution systematic random distortions compression scanning confidence ambiguous candidates multiple combined letters paragraphs sentences context careful predictions generated estimate boundaries widths character proportional regular printed material compatible exceptional important available relevant additional complete partial robust consistent useful independent meaningful conservative";
    for word in words.split_whitespace() {
        let w = font.measure(word, true).unwrap().advance * 12.0 + 5.0;
        if x + w > 660.0 {
            x = 25.0;
            y += 22.0;
        }
        canvas.text(&font, word, 12.0, true, 0.0, x, y).unwrap();
        x += w;
    }
    let original = "Contact Amber Vale ";
    let span = font.measure(original, true).unwrap().advance * 12.0;
    canvas
        .text(&font, "Contact", 12.0, true, 0.0, 25.0, 800.0)
        .unwrap();
    canvas
        .text(&font, "arrived", 12.0, true, 0.0, 25.0 + span, 800.0)
        .unwrap();
    canvas.rect(
        25.0 + font.measure("Contact ", true).unwrap().advance * 12.0,
        788.0,
        58.0,
        15.0,
    );
    canvas.finish().unwrap();
    let pdf = Pdf::open(&path).unwrap();
    let (chars, _) = pdf.chars(0).unwrap();
    let text: String = chars.iter().map(|c| c.text).collect();
    assert!(!text.contains("Amber"));
    let contact = text.find("Contact").unwrap();
    let arrived = text.find("arrived").unwrap();
    let left = text[..contact].chars().count();
    let right = text[..arrived].chars().count();
    p.pages[0].native_chars = chars;
    p.pages[0].likely_scan = false;
    p.pages[0].width = 700;
    p.pages[0].height = 900;
    let visible = unredact::project::native_words(&p.pages[0]);
    let profile = unredact::calibration::calibrate_for_metric(
        "f".into(),
        "p".into(),
        None,
        &visible,
        &[font_path],
        unredact::calibration::WidthMetric::Advance,
    )
    .unwrap();
    assert!(
        profile.fits[0].validation_rmse < 0.05,
        "{}",
        profile.fits[0].validation_rmse
    );
    p.profiles[0] = profile;
    p.targets[0].bounds = Rect {
        x: 75.0,
        y: 785.0,
        w: 70.0,
        h: 20.0,
    };
    p.targets[0].evidence = Evidence::NativeSpan {
        left_anchor: left,
        right_anchor: right,
        prefix: "Contact ".into(),
        suffix: " ".into(),
        measurement_sigma_points: 0.01,
    };
    candidates[1].canonical_name = "Amber Vole".into();
    candidates[1].surfaces[0].text = "Amber Vole".into();
    // Some letter substitutions have indistinguishable advances in this font.
    // Preserve those collisions, while rejecting a demonstrably narrower string.
    let mut narrower = candidates[1].clone();
    narrower.id = "narrower".into();
    narrower.canonical_name = "Amber Vile".into();
    narrower.surfaces[0].text = "Amber Vile".into();
    candidates.push(narrower);
    let report = solver::solve(&p, dir.path(), "g", &candidates, 10).unwrap();
    assert_eq!(report.candidates[0].id, "right");
    assert!(report.candidates[0].compatible);
    assert!(
        report
            .candidates
            .iter()
            .find(|r| r.id == "wrong")
            .unwrap()
            .compatible
    );
    assert!(
        !report
            .candidates
            .iter()
            .find(|r| r.id == "narrower")
            .unwrap()
            .compatible
    );
}

fn link_fixture() -> (tempfile::TempDir, Project, Vec<Candidate>) {
    let (dir, mut p, candidates) = fixture();
    p.targets[0].left_context = "The detailed meeting record names".into();
    p.targets[0].right_context = "as the designated project coordinator".into();
    let mut page = p.pages[0].clone();
    page.id = "p2".into();
    page.source_sha256 = "independent-source".into();
    page.image_sha256 = "independent-image".into();
    p.pages.push(page);
    let mut profile = p.profiles[0].clone();
    profile.id = "f2".into();
    profile.page = "p2".into();
    p.profiles.push(profile);
    let mut t = p.targets[0].clone();
    t.id = "t2".into();
    t.page = "p2".into();
    t.profile = "f2".into();
    t.group = "unrelated-existing-group".into();
    p.targets.push(t);
    (dir, p, candidates)
}

#[test]
fn context_links_tolerate_ocr_errors_but_require_both_sides() {
    let (_, mut p, _) = link_fixture();
    p.targets[1].left_context = "The detailed meetlng record names".into();
    let r = unredact::linking::discover(&p, 0.8).unwrap();
    assert_eq!(r.links.len(), 1);
    assert!(r.links[0].score < 1.0);
    assert_eq!(r.links[0].dependence, "unknown");
    p.targets[1].right_context = "during unrelated financial proceedings yesterday".into();
    assert!(
        unredact::linking::discover(&p, 0.8)
            .unwrap()
            .links
            .is_empty()
    );
    p.targets[1].right_context = "as the".into();
    let r = unredact::linking::discover(&p, 0.8).unwrap();
    assert!(r.links.is_empty());
    assert!(r.insufficient_context.contains(&"t2".into()));
}

#[test]
fn repeated_templates_retain_competing_links_without_transitive_merge() {
    let (_, mut p, _) = link_fixture();
    let mut other = p.targets[1].clone();
    other.id = "t3".into();
    other.bounds.y += 60.0;
    p.targets.push(other);
    let before = serde_json::to_vec(&p).unwrap();
    let r = unredact::linking::discover(&p, 0.8).unwrap();
    assert_eq!(r.links.len(), 2);
    assert_eq!(r.links[0].competing_a, vec!["t3"]);
    assert_eq!(r.links[1].competing_a, vec!["t2"]);
    assert_eq!(serde_json::to_vec(&p).unwrap(), before);
}

#[test]
fn ocr_context_uses_visible_nearby_words_and_excludes_masked_guesses() {
    let (_, mut p, _) = link_fixture();
    for i in 0..2 {
        p.targets[i].bounds.x = 300.0;
        p.targets[i].left_context.clear();
        p.targets[i].right_context.clear();
        let end = p.targets[i].bounds.x + p.targets[i].bounds.w;
        let mut words = vec![];
        for (j, text) in ["Detailed", "record", "names"].iter().enumerate() {
            words.push(unredact::calibration::Word {
                text: (*text).into(),
                bounds: Rect {
                    x: 160.0 + j as f64 * 45.0,
                    y: 15.0,
                    w: 40.0,
                    h: 20.0,
                },
                confidence: 99.0,
                block: "1".into(),
                line: "1".into(),
            });
        }
        for (j, text) in ["as", "project", "coordinator"].iter().enumerate() {
            words.push(unredact::calibration::Word {
                text: (*text).into(),
                bounds: Rect {
                    x: end + 10.0 + j as f64 * 50.0,
                    y: 15.0,
                    w: 45.0,
                    h: 20.0,
                },
                confidence: 99.0,
                block: "2".into(),
                line: "1".into(),
            });
        }
        words.push(unredact::calibration::Word {
            text: "hallucinated-secret".into(),
            bounds: p.targets[i].bounds,
            confidence: 99.0,
            block: "3".into(),
            line: "1".into(),
        });
        p.pages[i].words = words;
    }
    let r = unredact::linking::discover(&p, 0.8).unwrap();
    assert_eq!(r.links.len(), 1);
    assert_eq!(r.contexts["t"].left, vec!["detailed", "record", "names"]);
    assert_eq!(r.contexts["t"].right, vec!["as", "project", "coordinator"]);
    assert_eq!(r.contexts["t"].left_source, "ocr_same_row");
}

#[test]
fn uncertain_links_show_conflicting_geometry_and_dependence_sensitivity() {
    let (dir, mut p, candidates) = link_fixture();
    // The same template contains a different hidden value: context must not prove identity.
    p.targets[1].bounds.w += 40.0;
    let before = serde_json::to_vec(&p).unwrap();
    let r = unredact::linking::solve_links(&p, dir.path(), "t", &candidates, 0.8, 20, 20).unwrap();
    assert_eq!(r.scenarios.len(), 3);
    let base = r.scenarios[0].result.as_ref().unwrap();
    let independent = r.scenarios[1].result.as_ref().unwrap();
    let dependent = r.scenarios[2].result.as_ref().unwrap();
    assert_eq!(r.scenarios[1].dependence_assumption, "assumed_independent");
    assert_eq!(independent.compatible_candidates, 0);
    assert_eq!(
        serde_json::to_vec(&base.candidates).unwrap(),
        serde_json::to_vec(&dependent.candidates).unwrap()
    );
    assert_eq!(serde_json::to_vec(&p).unwrap(), before);
}

#[test]
fn same_source_page_cannot_gain_independent_evidence_from_linking() {
    let (dir, mut p, candidates) = link_fixture();
    p.pages[1].source_sha256 = p.pages[0].source_sha256.clone();
    // Different image hashes may represent different resolutions of the same source page.
    let r = unredact::linking::solve_links(&p, dir.path(), "t", &candidates, 0.8, 20, 20).unwrap();
    assert_eq!(r.scenarios.len(), 2);
    assert_eq!(r.scenarios[1].dependence_assumption, "same_source_page");
    assert_eq!(
        serde_json::to_vec(&r.scenarios[0].result.as_ref().unwrap().candidates).unwrap(),
        serde_json::to_vec(&r.scenarios[1].result.as_ref().unwrap().candidates).unwrap()
    );
}

#[test]
fn link_scenario_budget_and_global_ids_are_checked() {
    let (dir, mut p, candidates) = link_fixture();
    let mut other = p.targets[1].clone();
    other.id = "t3".into();
    other.bounds.y += 60.0;
    p.targets.push(other);
    let r = unredact::linking::solve_links(&p, dir.path(), "t", &candidates, 0.8, 1, 20).unwrap();
    assert_eq!(r.omitted_link_scenarios, 1);
    assert_eq!(r.scenarios.len(), 3);
    p.targets[2].id = "t2".into();
    assert!(unredact::linking::discover(&p, 0.8).is_err());
}

fn audit_words() -> Vec<unredact::calibration::Word> {
    let font = Font::open(&default_fonts()[0]).unwrap();
    let text = "document analysis original visible typography measurement observations reference comparisons evidence information different ordinary language uncertainty validation independent reliable scientific research process methods explanation physical parameters geometry shapes position spacing tracking baseline uncertainty separate calibration accurate approximate reporting knowledge practical development history mathematics resolution systematic random distortions compression scanning confidence ambiguous candidates multiple combined letters paragraphs sentences context careful predictions generated estimate boundaries widths character proportional regular printed material compatible exceptional important available relevant additional complete partial robust consistent useful independent meaningful conservative surrounding structure assumptions numerical statistics experiments machine learning assessment quantification algorithm identification unknown proposed evaluated recorded correspondence transportation engineering remarkable construction external internal records shapes interactions";
    text.split_whitespace()
        .map(|word| unredact::calibration::Word {
            text: word.into(),
            bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: font.measure(word, true).unwrap().ink_width() * 24.0 + 0.5,
                h: 22.0,
            },
            confidence: 99.0,
            block: "b".into(),
            line: "line".into(),
        })
        .collect()
}

#[test]
fn audit_words_cannot_select_font_or_correction_model() {
    use unredact::calibration::{audit_held_out, calibrate};
    let mut words = audit_words();
    let fonts = default_fonts();
    let first = calibrate("f".into(), "p".into(), None, &words, &fonts).unwrap();
    assert!(first.audit_texts.len() >= 4);
    for text in &first.audit_texts {
        assert!(!first.training_texts.contains(text));
        assert!(!first.validation_texts.contains(text));
    }
    for w in &mut words {
        if audit_held_out(&w.text) {
            w.bounds.w += 20.0;
        }
    }
    let second = calibrate("f".into(), "p".into(), None, &words, &fonts).unwrap();
    assert_eq!(first.fits.len(), second.fits.len());
    for (mut a, mut b) in first.fits.into_iter().zip(second.fits) {
        assert!(b.audit.as_ref().unwrap().rmse > a.audit.as_ref().unwrap().rmse + 10.0);
        a.audit = None;
        b.audit = None;
        assert_eq!(
            serde_json::to_vec(&a).unwrap(),
            serde_json::to_vec(&b).unwrap()
        );
    }
}

#[test]
fn font_screening_excludes_both_selection_and_audit_words() {
    use unredact::calibration::{WidthMetric, shortlist_fonts};
    let mut words = audit_words();
    let fonts = default_fonts();
    let a = shortlist_fonts(&words, None, &fonts, WidthMetric::Ink, 4).unwrap();
    for text in &a.training_texts {
        assert!(!held_out(text));
    }
    for w in &mut words {
        if held_out(&w.text) {
            w.bounds.w += 500.0;
        }
    }
    let b = shortlist_fonts(&words, None, &fonts, WidthMetric::Ink, 4).unwrap();
    assert_eq!(
        serde_json::to_vec(&a).unwrap(),
        serde_json::to_vec(&b).unwrap()
    );
    assert!(a.shortlist.iter().any(|s| s.font == fonts[0]));
}

#[test]
fn audit_residuals_widen_width_uncertainty_and_old_profiles_still_load() {
    let (dir, mut p, candidates) = fixture();
    let mut json = serde_json::to_value(&p).unwrap();
    json["profiles"][0]
        .as_object_mut()
        .unwrap()
        .remove("audit_texts");
    json["profiles"][0]
        .as_object_mut()
        .unwrap()
        .remove("font_search");
    json["profiles"][0]["fits"][0]
        .as_object_mut()
        .unwrap()
        .remove("audit");
    let old: Project = serde_json::from_value(json).unwrap();
    assert!(old.profiles[0].fits[0].audit.is_none());
    p.targets[0].bounds.w += 5.0;
    let before = solver::solve(&p, dir.path(), "g", &candidates, 10).unwrap();
    assert!(
        !before
            .candidates
            .iter()
            .find(|c| c.name == "Amber Vale")
            .unwrap()
            .compatible
    );
    p.profiles[0].fits[0].audit = Some(unredact::calibration::FitAudit {
        words: 20,
        rmse: 3.0,
        max_error: 6.0,
    });
    let after = solver::solve(&p, dir.path(), "g", &candidates, 10).unwrap();
    assert!(
        after
            .candidates
            .iter()
            .find(|c| c.name == "Amber Vale")
            .unwrap()
            .compatible
    );
    assert_eq!(after.calibration_audits["f"][0].as_ref().unwrap().words, 20);
    p.profiles[0].fits[0].audit.as_mut().unwrap().rmse = f64::NAN;
    assert!(p.validate_target(&p.targets[0]).is_err());
}

#[test]
fn large_audit_error_cannot_win_merely_by_inflating_sigma() {
    let (dir, mut p, candidates) = fixture();
    p.targets[0].bounds.w += 0.5;
    let mut broad = p.profiles[0].fits[0].clone();
    broad.audit = Some(unredact::calibration::FitAudit {
        words: 20,
        rmse: 100.0,
        max_error: 100.0,
    });
    p.profiles[0].fits.push(broad);
    let result = solver::solve(&p, dir.path(), "g", &candidates, 10).unwrap();
    let truth = result
        .candidates
        .iter()
        .find(|c| c.name == "Amber Vale")
        .unwrap();
    assert_eq!(truth.observations[0].fit_index, 0);
}

#[test]
fn hinted_mode_scores_widths_and_rejects_incompatible_mosaic_profiles() {
    use unredact::native::MeasurementMode;
    let (dir, mut p, candidates) = fixture();
    let mode = MeasurementMode::FreeTypeHinted { ppem: 12 };
    p.profiles[0].fits[0].measurement_mode = mode;
    let font = Font::open(&p.profiles[0].fits[0].font).unwrap();
    let predicted = p.profiles[0].fits[0].predict(&font, "Amber Vale").unwrap();
    p.targets[0].evidence = Evidence::Width {
        width: predicted,
        measurement_sigma: 0.1,
    };
    let result = solver::solve(&p, dir.path(), "g", &candidates, 10).unwrap();
    assert!(result.candidates[0].compatible);
    assert!(result.candidates[0].evidence_score < 1e-8);
    let mut encoded = serde_json::to_value(&p).unwrap();
    encoded["profiles"][0]["fits"][0]
        .as_object_mut()
        .unwrap()
        .remove("measurement_mode");
    let old: Project = serde_json::from_value(encoded).unwrap();
    assert_eq!(
        old.profiles[0].fits[0].measurement_mode,
        MeasurementMode::Design
    );
    p.targets[0].evidence = Evidence::MosaicAuto {
        min_block: 2,
        max_block: 10,
        noise_sigma: 1.0,
        origin_x: 0.0,
        baseline_y: 20.0,
    };
    assert!(
        p.validate_target(&p.targets[0])
            .unwrap_err()
            .to_string()
            .contains("Hinted measurement")
    );
}

#[test]
fn a_replaced_font_fails_before_candidate_scoring() {
    let (dir, mut p, candidates) = fixture();
    let path = dir.path().join("calibrated.ttf");
    std::fs::copy(&p.profiles[0].fits[0].font, &path).unwrap();
    let font = Font::open(&path).unwrap();
    p.profiles[0].fits[0].font = path.clone();
    p.profiles[0].fits[0].font_sha256 = Some(font.sha256().into());
    assert!(solver::solve(&p, dir.path(), "g", &candidates, 10).is_ok());
    let replacement = default_fonts()
        .into_iter()
        .find(|f| Font::open(f).unwrap().sha256() != font.sha256())
        .unwrap();
    std::fs::copy(replacement, &path).unwrap();
    let error = solver::solve(&p, dir.path(), "g", &candidates, 10).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Font hash differs from calibration")
    );
}

#[test]
fn targets_cannot_overlap_neural_font_selection_pixels() {
    use unredact::{
        neural::{Montage, Report, Sample},
        typography::Line,
    };
    let (_, mut p, _) = fixture();
    p.profiles[0].neural_search = Some(Report {
        model_sha256: "model".into(),
        script_sha256: "script".into(),
        catalog_sha256: "catalog".into(),
        page_image_sha256: p.pages[0].image_sha256.clone(),
        request_sha256: "request".into(),
        response_sha256: "response".into(),
        inputs: vec![Montage {
            path: "training.pgm".into(),
            sha256: "crop".into(),
            samples: vec![Sample {
                text: "training".into(),
                bounds: p.targets[0].bounds,
                line: Line {
                    block: "body".into(),
                    line: "one".into(),
                },
            }],
        }],
        suggestions: vec![],
        interpretation: "test fixture".into(),
    });
    assert!(
        p.validate_target(&p.targets[0])
            .unwrap_err()
            .to_string()
            .contains("neural font-selection")
    );
    p.profiles[0].neural_search.as_mut().unwrap().inputs[0].samples[0]
        .bounds
        .y = 200.0;
    assert!(p.validate_target(&p.targets[0]).is_ok());
}

#[test]
fn uniform_padding_is_explicit_and_changes_ranking_without_claiming_recovery() {
    let (dir, mut project, mut candidates) = fixture();
    let point = solver::solve(&project, dir.path(), "g", &candidates, 10).unwrap();
    project.targets[0].evidence = Evidence::OpaqueUniform {
        padding_min: 4.0,
        padding_max: 4.0,
        measurement_sigma: 0.1,
    };
    let uniform_point = solver::solve(&project, dir.path(), "g", &candidates, 10).unwrap();
    for (a, b) in point.candidates.iter().zip(&uniform_point.candidates) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.compatible, b.compatible);
        assert!((a.evidence_score - b.evidence_score).abs() < 1e-12);
    }
    candidates[1].canonical_name = "Amber Valex".into();
    candidates[1].surfaces[0].text = "Amber Valex".into();
    let fit = &project.profiles[0].fits[0];
    let font = Font::open(&fit.font).unwrap();
    let center = fit.predict(&font, &candidates[0].surfaces[0].text).unwrap();
    let edge = fit.predict(&font, &candidates[1].surfaces[0].text).unwrap();
    let radius = (edge - center).abs();
    assert!(radius > 1. && radius < center);
    project.targets[0].bounds.w = center + radius;
    project.targets[0].evidence = Evidence::Opaque {
        padding_min: 0.,
        padding_max: 2. * radius,
        measurement_sigma: 0.1,
    };
    let interval = solver::solve(&project, dir.path(), "g", &candidates, 10).unwrap();
    assert_eq!(interval.best_evidence_ties, 2);
    project.targets[0].evidence = Evidence::OpaqueUniform {
        padding_min: 0.,
        padding_max: 2. * radius,
        measurement_sigma: 0.1,
    };
    let prior = solver::solve(&project, dir.path(), "g", &candidates, 10).unwrap();
    assert_eq!(prior.best_evidence_ties, 1);
    assert_eq!(prior.candidates[0].id, "right");
    assert_eq!(prior.compatible_candidates, 2);
    assert_eq!(prior.status, "ambiguous");
    for candidate in &prior.candidates {
        let old = interval
            .candidates
            .iter()
            .find(|c| c.id == candidate.id)
            .unwrap();
        assert_eq!(
            candidate.observations[0].predicted_width,
            old.observations[0].predicted_width
        );
        assert_eq!(
            candidate.observations[0].allowed_width,
            old.observations[0].allowed_width
        );
        assert_eq!(
            candidate.observations[0].standardized_error,
            old.observations[0].standardized_error
        );
    }
    assert!(
        prior
            .warnings
            .iter()
            .any(|w| w.contains("explicit uniform"))
    );
    let serialized = serde_json::to_string(&project).unwrap();
    assert!(serialized.contains("opaque_uniform"));
    let restored: Project = serde_json::from_str(&serialized).unwrap();
    assert_eq!(
        serde_json::to_value(solver::solve(&restored, dir.path(), "g", &candidates, 10).unwrap())
            .unwrap(),
        serde_json::to_value(prior).unwrap()
    );
    let mut duplicate = project.targets[0].clone();
    duplicate.id = "duplicate".into();
    project.targets.push(duplicate);
    let dedup = solver::solve(&project, dir.path(), "g", &candidates, 10).unwrap();
    assert_eq!(dedup.independent_observations, 1);
}
