// Copyright (C) 2026 Toit contributors.
use unredact::{
    abstention::{self, CheckStatus, Decision, EvidenceCheck, EvidenceChecks, Policy},
    contextual::ContextReport,
};

fn fixture() -> (ContextReport, Policy) {
    let candidates: Vec<_> = [("a", -2.), ("b", -5.)].into_iter().map(|(id, _)| serde_json::json!({
        "id":id,"name":id,"evidence_score":1.,"context_score":null,"compatible":true,
        "observations":[{"target":"t","surface":id,"predicted_width":10.,"allowed_width":[9.,11.],
            "standardized_error":0.,"image_rmse":null,"score":1.,"fit_index":0}],
        "source_ids":[],"evidence_rank_min":1,"evidence_rank_max":2
    })).collect();
    let ranks: Vec<_> = [("a", -2.), ("b", -5.)]
        .into_iter()
        .map(|(id, score)| {
            serde_json::json!({
                "id":id,"mean_token_log_score":score,"mean_sequence_log_score":score,
                "observations":[{"id":id,"target":"t","token_log_score_sum":score,"token_count":1}]
            })
        })
        .collect();
    let mut report: ContextReport = serde_json::from_value(serde_json::json!({
        "schema_version":1,"interpretation":"test rank","physical":{
            "group":"g","status":"ambiguous","interpretation":"test engine",
            "tested_candidates":2,"independent_observations":1,"ignored_dependent_targets":[],
            "compatible_candidates":2,"best_evidence_ties":2,"candidates":candidates,"warnings":[]
        },"contexts":{"t":{"left":"visible","right":"context","left_source":"supplied",
            "right_source":"supplied","left_word_indices":[],"right_word_indices":[]}},
        "ranked_groups":[["a"],["b"]],"ranked":ranks,"provider":{"response":{
            "method":"synthetic-test-scorer","provenance":{"model_sha256":"synthetic-model-a"}}},
        "normalization":"sequence_sum_mean_over_occurrences"
    }))
    .unwrap();
    report.provider = Some(serde_json::json!({"request":{"contexts":report.contexts,
        "items":[{"id":"a","target":"t","text":"a"},{"id":"b","target":"t","text":"b"}]},
        "response":{"method":"synthetic-test-scorer","provenance":{"model_sha256":"synthetic-model-a"},
        "scores":[{"id":"a","target":"t","token_count":1,"token_log_score_sum":-2.},
            {"id":"b","target":"t","token_count":1,"token_log_score_sum":-5.}]}}));
    let policy = serde_json::from_value(serde_json::json!({
        "schema_version":1,"id":"development-rule","answer_enabled":true,
        "metric":"mean_sequence_log_score","min_score_gap":2.,"min_top_score":-3.,
        "signature":{"normalization":"sequence_sum_mean_over_occurrences","physical_weight":null,
            "physical_interpretation":"test engine","language":{
                "method":"synthetic-test-scorer","provenance":{"model_sha256":"synthetic-model-a"},
                "source_hashes":null,"domain_adapter":null,"options":null,"context_sources":["supplied"]}},
        "require_font_check":false,"require_link_check":false,
        "development_documents":["dev-document"],"development_source_groups":["dev-source"],
        "provenance":{"role":"synthetic unit test, no measured performance claim"}
    }))
    .unwrap();
    (report, policy)
}

#[test]
fn assessment_keeps_ranking_separate_and_records_exact_input_hashes() {
    let (report, mut policy) = fixture();
    let before = serde_json::to_value(&report).unwrap();
    let result = abstention::assess(&report, &policy, EvidenceChecks::default()).unwrap();
    assert_eq!(result.decision, Decision::Answer);
    assert_eq!(result.answer_candidate.as_deref(), Some("a"));
    assert_eq!(result.checks.font_coverage.status, CheckStatus::Unknown);
    assert_eq!(serde_json::to_value(&report).unwrap(), before);
    policy.min_score_gap = 4.;
    let result = abstention::assess(&report, &policy, EvidenceChecks::default()).unwrap();
    assert_eq!(result.decision, Decision::InsufficientEvidence);
    assert!(result.answer_candidate.is_none());
    assert_eq!(result.features.unwrap().top_candidate, "a");
    let dir = tempfile::tempdir().unwrap();
    let report_path = dir.path().join("report.json");
    let policy_path = dir.path().join("policy.json");
    let report_bytes = serde_json::to_vec_pretty(&report).unwrap();
    let policy_bytes = serde_json::to_vec_pretty(&policy).unwrap();
    std::fs::write(&report_path, &report_bytes).unwrap();
    std::fs::write(&policy_path, &policy_bytes).unwrap();
    let result = abstention::assess_files(&report_path, &policy_path, None).unwrap();
    assert_eq!(result.report_sha256, unredact::project::hash(&report_bytes));
    assert_eq!(result.policy_sha256, unredact::project::hash(&policy_bytes));
}

#[test]
fn missing_fonts_and_incorrect_links_abstain_despite_large_score_margin() {
    let (report, mut policy) = fixture();
    for failed_font in [true, false] {
        let mut checks = EvidenceChecks::default();
        let failed = EvidenceCheck {
            status: CheckStatus::Fail,
            source: "independent control".into(),
            description: if failed_font {
                "Known generating font excluded"
            } else {
                "Linked targets contain different hidden entities"
            }
            .into(),
        };
        if failed_font {
            checks.font_coverage = failed;
        } else {
            checks.link_validity = failed;
        }
        let result = abstention::assess(&report, &policy, checks).unwrap();
        assert_eq!(result.decision, Decision::InsufficientEvidence);
        assert!(result.reasons.iter().any(|r| r.ends_with("_failed")));
    }
    policy.require_font_check = true;
    policy.require_link_check = true;
    let unknown = abstention::assess(&report, &policy, EvidenceChecks::default()).unwrap();
    assert!(unknown.reasons.contains(&"font_coverage_unassessed".into()));
    assert!(unknown.reasons.contains(&"link_validity_unassessed".into()));
    let checked = EvidenceCheck {
        status: CheckStatus::Pass,
        source: "user assertion: visible typography audit".into(),
        description: "Asserted checks, not independently verified by this tool".into(),
    };
    let result = abstention::assess(
        &report,
        &policy,
        EvidenceChecks {
            font_coverage: checked.clone(),
            link_validity: checked,
        },
    )
    .unwrap();
    assert_eq!(result.decision, Decision::Answer);
    assert!(
        result
            .interpretation
            .contains("externally supplied assertions")
    );
}

#[test]
fn archived_replay_binds_hash_verified_original_model_and_scores() {
    let (mut report, mut policy) = fixture();
    let dir = tempfile::tempdir().unwrap();
    let cache_path = dir.path().join("scores.json");
    let items =
        serde_json::json!([{"id":"a","target":"t","text":"a"},{"id":"b","target":"t","text":"b"}]);
    let scores = serde_json::json!([
        {"id":"a","target":"t","token_count":1,"token_log_score_sum":-2.},
        {"id":"b","target":"t","token_count":1,"token_log_score_sum":-5.}]);
    let request = serde_json::json!({"contexts":report.contexts,"items":items});
    let mut cache = serde_json::json!({"request":request,"scores":scores,"model":{"model_sha256":"original-model"}});
    let bytes = serde_json::to_vec_pretty(&cache).unwrap();
    std::fs::write(&cache_path, &bytes).unwrap();
    report.provider = Some(serde_json::json!({
        "args":["--replay",cache_path],"request":request,
        "response":{"method":"exact_replay_of_common_language_control","reference_sha256":unredact::project::hash(&bytes),"scores":scores}
    }));
    policy.signature.language = abstention::language_signature(&report, dir.path()).unwrap();
    assert_eq!(
        abstention::assess(&report, &policy, EvidenceChecks::default())
            .unwrap()
            .decision,
        Decision::Answer
    );
    cache["model"]["model_sha256"] = "other-model".into();
    let changed = serde_json::to_vec_pretty(&cache).unwrap();
    std::fs::write(&cache_path, &changed).unwrap();
    let invalid_hash = abstention::assess(&report, &policy, EvidenceChecks::default()).unwrap();
    assert_eq!(invalid_hash.decision, Decision::InsufficientEvidence);
    assert!(
        invalid_hash
            .reasons
            .iter()
            .any(|r| r.contains("hash mismatch"))
    );
    report.provider.as_mut().unwrap()["response"]["reference_sha256"] =
        unredact::project::hash(&changed).into();
    let changed_model = abstention::assess(&report, &policy, EvidenceChecks::default()).unwrap();
    assert!(
        changed_model
            .reasons
            .contains(&"report_signature_mismatch".into())
    );
    assert_eq!(changed_model.decision, Decision::InsufficientEvidence);
}

#[test]
fn fabricated_aggregates_observations_contexts_and_surfaces_cannot_answer() {
    let (mut report, mut policy) = fixture();
    policy.metric = abstention::ScoreMetric::CombinedScore;
    policy.min_top_score = -5.;
    report.physical_weight = Some(1.);
    policy.signature.physical_weight = Some(1.);
    for rank in &mut report.ranked {
        rank.mean_physical_cost = Some(1.);
        rank.combined_score = Some(rank.mean_sequence_log_score.unwrap() - 1.);
    }
    assert_eq!(
        abstention::assess(&report, &policy, EvidenceChecks::default())
            .unwrap()
            .decision,
        Decision::Answer
    );
    let base = serde_json::to_value(&report).unwrap();
    for (kind, mut changed) in [
        ("aggregate", base.clone()),
        ("observation", base.clone()),
        ("context", base.clone()),
        ("surface", base),
    ] {
        match kind {
            "aggregate" => changed["ranked"][0]["combined_score"] = 1_000_000.into(),
            "observation" => {
                changed["ranked"][0]["observations"][0]["token_log_score_sum"] = (-0.1).into()
            }
            "context" => changed["contexts"]["t"]["left"] = "different visible text".into(),
            "surface" => {
                changed["provider"]["request"]["items"][0]["text"] = "other surface".into()
            }
            _ => unreachable!(),
        }
        let changed: ContextReport = serde_json::from_value(changed).unwrap();
        let assessed = abstention::assess(&changed, &policy, EvidenceChecks::default()).unwrap();
        assert_eq!(assessed.decision, Decision::InsufficientEvidence, "{kind}");
        assert!(
            assessed
                .reasons
                .iter()
                .any(|r| r.starts_with("unusable_report:")),
            "{kind}"
        );
    }
}

#[test]
fn incomplete_tied_singleton_nonfinite_and_mismatched_reports_cannot_answer() {
    let (report, policy) = fixture();
    let mut cases = Vec::new();
    let mut truncated: ContextReport =
        serde_json::from_value(serde_json::to_value(&report).unwrap()).unwrap();
    truncated.physical.tested_candidates += 1;
    cases.push(truncated);
    let mut tied: ContextReport =
        serde_json::from_value(serde_json::to_value(&report).unwrap()).unwrap();
    tied.ranked_groups = vec![vec!["a".into(), "b".into()]];
    cases.push(tied);
    let mut singleton: ContextReport =
        serde_json::from_value(serde_json::to_value(&report).unwrap()).unwrap();
    singleton.ranked.pop();
    singleton.ranked_groups.pop();
    singleton.physical.candidates.pop();
    singleton.physical.tested_candidates = 1;
    singleton.physical.compatible_candidates = 1;
    cases.push(singleton);
    let mut nonfinite: ContextReport =
        serde_json::from_value(serde_json::to_value(&report).unwrap()).unwrap();
    nonfinite.ranked[0].mean_sequence_log_score = Some(f64::NAN);
    cases.push(nonfinite);
    let mut mismatch: ContextReport =
        serde_json::from_value(serde_json::to_value(&report).unwrap()).unwrap();
    mismatch.normalization = None;
    cases.push(mismatch);
    let mut changed_model: ContextReport =
        serde_json::from_value(serde_json::to_value(&report).unwrap()).unwrap();
    changed_model.provider.as_mut().unwrap()["response"]["provenance"]["model_sha256"] =
        "synthetic-model-b".into();
    cases.push(changed_model);
    let mut absent_model: ContextReport =
        serde_json::from_value(serde_json::to_value(&report).unwrap()).unwrap();
    absent_model.provider = None;
    cases.push(absent_model);
    for invalid in cases {
        let result = abstention::assess(&invalid, &policy, EvidenceChecks::default()).unwrap();
        assert_eq!(result.decision, Decision::InsufficientEvidence);
        assert!(!result.reasons.is_empty());
    }
    let mut disabled = policy;
    disabled.answer_enabled = false;
    let result = abstention::assess(&report, &disabled, EvidenceChecks::default()).unwrap();
    assert_eq!(result.decision, Decision::InsufficientEvidence);
}
