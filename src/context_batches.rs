// Copyright (C) 2026 Toit contributors.
//! Bounded context-provider calls, preserving each candidate/occurrence exactly once.
use crate::{
    contextual::{Config, Context, Score},
    project::hash,
};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

pub(crate) fn score(
    contexts: &BTreeMap<String, Context>,
    items: &[Value],
    project_sha256: &str,
    config: &Config,
) -> Result<(Vec<Score>, Value)> {
    ensure!(
        (1..=2048).contains(&config.batch_items),
        "Context batch size must be 1–2048 items"
    );
    ensure!(
        !items.is_empty() && items.len() <= 100_000,
        "Context ranking supports 1–100,000 candidate/occurrence items"
    );
    let total: BTreeSet<_> = items
        .iter()
        .map(|i| (i["id"].as_str(), i["target"].as_str()))
        .collect();
    ensure!(
        total.len() == items.len()
            && total
                .iter()
                .all(|(id, target)| id.is_some() && target.is_some()),
        "Duplicate or malformed context request items"
    );
    let timeout = Duration::from_secs(config.timeout_seconds);
    let started = Instant::now();
    let mut worker = config
        .persistent_provider
        .then(|| crate::provider::Worker::start(&config.executable, &config.args))
        .transpose()?;
    let mut all_scores = vec![];
    let mut traces = vec![];
    for batch in items.chunks(config.batch_items) {
        let remaining = timeout.saturating_sub(started.elapsed());
        ensure!(
            !remaining.is_zero(),
            "Context provider total timeout exceeded"
        );
        let request = json!({"schema_version":1,"task":"score_context_candidates","contexts":contexts,"items":batch});
        let response = if let Some(worker) = &mut worker {
            worker.call(&request, remaining)?
        } else {
            crate::provider::call(&config.executable, &config.args, &request, remaining)?
        };
        ensure!(
            response["schema_version"] == 1,
            "Invalid context-score response version"
        );
        let scores: Vec<Score> = serde_json::from_value(response["scores"].clone())?;
        let expected: BTreeSet<_> = batch
            .iter()
            .map(|i| (i["id"].as_str().unwrap(), i["target"].as_str().unwrap()))
            .collect();
        let mut seen = BTreeSet::new();
        for s in &scores {
            ensure!(
                expected.contains(&(s.id.as_str(), s.target.as_str()))
                    && seen.insert((s.id.as_str(), s.target.as_str())),
                "Unknown or duplicate context-score candidate/target in batch"
            );
            ensure!(
                s.token_log_score_sum.is_finite()
                    && s.token_log_score_sum <= 0.0
                    && (1..=128).contains(&s.token_count),
                "Invalid context token log score"
            );
        }
        ensure!(
            seen == expected,
            "Context provider omitted candidate/target scores in batch"
        );
        all_scores.extend(scores);
        traces.push(json!({"executable":config.executable,"args":config.args,
            "project_sha256":project_sha256,
            "request_sha256":hash(&serde_json::to_vec(&request)?),"response_sha256":hash(&serde_json::to_vec(&response)?),
            "request":request,"response":response}));
    }
    let mut trace = if traces.len() == 1 {
        traces.pop().unwrap()
    } else {
        json!({"executable":config.executable,"args":config.args,"project_sha256":project_sha256,
               "batch_item_limit":config.batch_items,"total_items":items.len(),"total_timeout_seconds":config.timeout_seconds,
               "batches":traces})
    };
    if config.persistent_provider {
        trace["transport"] = json!({"mode":"persistent_json_lines", "worker_protocol_version":1,
            "appended_args":["--persistent"], "requests":items.len().div_ceil(config.batch_items)});
    }
    Ok((all_scores, trace))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Config, BTreeMap<String, Context>) {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("scores.py");
        std::fs::write(&script,r#"import json,sys,time,os
mode=sys.argv[1] if len(sys.argv)>1 else ''
def score(r):
 if mode=='slow': time.sleep(.4)
 scores=[dict(id=i['id'],target=i['target'],token_count=2,token_log_score_sum=-float(len(i['text']))) for i in r['items']]
 if mode=='future': scores[0]['id']='late'
 if mode=='missing': scores.pop()
 if mode=='duplicate': scores.append(scores[0])
 return dict(schema_version=1,scores=scores,provider_pid=os.getpid())
if '--persistent' in sys.argv:
 for raw in sys.stdin:
  frame=json.loads(raw)
  print(json.dumps(dict(worker_protocol_version=1,request_id=frame['request_id'],response=score(frame['request']))),flush=True)
else:
 json.dump(score(json.load(sys.stdin)),sys.stdout)
"#).unwrap();
        let config = Config {
            executable: "python3".into(),
            args: vec![script.display().to_string()],
            timeout_seconds: 5,
            paragraph_context: false,
            word_normalization: false,
            sequence_score: true,
            max_compatible: 10_000,
            batch_items: 2048,
            persistent_provider: false,
            physical_weight: 0.0,
        };
        let context = Context {
            left: "The".into(),
            right: "arrived".into(),
            left_source: "supplied".into(),
            right_source: "supplied".into(),
            left_word_indices: vec![],
            right_word_indices: vec![],
            layout_notes: vec![],
        };
        (
            directory,
            config,
            BTreeMap::from([("a".into(), context.clone()), ("b".into(), context)]),
        )
    }
    #[test]
    fn batches_cover_large_linked_requests_and_preserve_single_call_scores() {
        let (_dir, mut config, contexts) = fixture();
        let items: Vec<_> = (0..2049)
            .flat_map(|i| {
                ["a","b"].map(move |target|
            json!({"id":format!("c{i}"),"target":target,"text":format!("visible {i}")}))
            })
            .collect();
        let (scores, trace) = score(&contexts, &items, "project", &config).unwrap();
        assert_eq!(scores.len(), 4098);
        assert_eq!(trace["batches"].as_array().unwrap().len(), 3);
        assert_eq!(trace["total_items"], 4098);
        assert_eq!(
            trace["batches"][2]["request"]["items"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let small = &items[..8];
        let (single, one) = score(&contexts, small, "project", &config).unwrap();
        config.batch_items = 3;
        let (split, many) = score(&contexts, small, "project", &config).unwrap();
        assert_eq!(
            serde_json::to_value(single).unwrap(),
            serde_json::to_value(split).unwrap()
        );
        assert!(one.get("request").is_some() && one.get("batches").is_none());
        assert_eq!(many["batches"].as_array().unwrap().len(), 3);
    }
    #[test]
    fn a_batch_cannot_supply_another_batchs_scores_or_omit_or_duplicate_items() {
        let (_dir, mut config, contexts) = fixture();
        config.batch_items = 1;
        let items = vec![
            json!({"id":"early","target":"a","text":"earlier"}),
            json!({"id":"late","target":"a","text":"later"}),
        ];
        for mode in ["future", "missing", "duplicate"] {
            config.args.push(mode.into());
            assert!(
                score(&contexts, &items, "project", &config).is_err(),
                "{mode}"
            );
            config.args.pop();
        }
    }
    #[test]
    fn timeout_covers_all_batches_instead_of_restarting_per_call() {
        let (_dir, mut config, contexts) = fixture();
        config.batch_items = 1;
        config.timeout_seconds = 1;
        config.args.push("slow".into());
        let items: Vec<_> = (0..4)
            .map(|i| json!({"id":format!("c{i}"),"target":"a","text":"word"}))
            .collect();
        let start = Instant::now();
        let error = score(&contexts, &items, "project", &config).unwrap_err();
        assert!(
            error.to_string().contains("timeout") || error.to_string().contains("timed out"),
            "{error}"
        );
        assert!(start.elapsed() < Duration::from_millis(2500));
    }

    #[test]
    fn persistent_batches_reuse_one_process_without_changing_scores() {
        let (_dir, mut config, contexts) = fixture();
        let items: Vec<_> = (0..5)
            .map(|i| json!({"id":format!("c{i}"),"target":"a","text":"word"}))
            .collect();
        config.batch_items = 2;
        let (one_shot, _) = score(&contexts, &items, "project", &config).unwrap();
        config.persistent_provider = true;
        let (persistent, trace) = score(&contexts, &items, "project", &config).unwrap();
        assert_eq!(
            serde_json::to_value(one_shot).unwrap(),
            serde_json::to_value(persistent).unwrap()
        );
        let batches = trace["batches"].as_array().unwrap();
        assert_eq!(batches.len(), 3);
        let pid = &batches[0]["response"]["provider_pid"];
        assert!(
            batches
                .iter()
                .all(|batch| &batch["response"]["provider_pid"] == pid)
        );
        assert_eq!(trace["transport"]["mode"], "persistent_json_lines");
        assert_eq!(trace["transport"]["requests"], 3);
        config.batch_items = 1;
        config.timeout_seconds = 1;
        config.args.push("slow".into());
        let start = Instant::now();
        let error = score(&contexts, &items, "project", &config).unwrap_err();
        assert!(error.to_string().contains("timed out"), "{error}");
        assert!(start.elapsed() < Duration::from_millis(2500));
    }
}
