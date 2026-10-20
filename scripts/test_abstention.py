#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
import argparse
import contextlib
import copy
import io
import json
from pathlib import Path
import tempfile
import unittest

import evaluate_abstention as abstention


def report(top=-2., gap=3.):
    candidates, ranks = [], []
    for candidate_id, score in [("a", top), ("b", top - gap)]:
        candidates.append({"id": candidate_id, "compatible": True, "evidence_score": 1.,
                           "observations": [{"target": "t", "surface":candidate_id, "standardized_error": 0., "score": 1.}]})
        ranks.append({"id": candidate_id, "combined_score": score,
                      "mean_token_log_score":score+1, "mean_sequence_log_score":score+1,"mean_physical_cost":1.,
                      "observations": [{"id": candidate_id, "target": "t", "token_count": 1, "token_log_score_sum": score+1}]})
    contexts = {"t": {"left": "visible", "right": ""}}
    return {"schema_version": 1, "normalization": "sequence_sum_mean_over_occurrences", "physical_weight": 1.,
            "physical": {"interpretation": "synthetic test generation", "independent_observations": 1,
                         "tested_candidates": 2, "compatible_candidates": 2, "candidates": candidates},
            "ranked": ranks, "ranked_groups": [["a"], ["b"]], "contexts": contexts,
            "provider": {"request":{"contexts":contexts,"items":[{"id":i,"target":"t","text":i} for i in ("a","b")]},
                "response": {"scores":[r["observations"][0] for r in ranks], "method": "synthetic-test-scorer", "provenance": {"model_sha256": "test-model-a"}}}}


class AbstentionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def manifest(self, name, rows):
        cases = []
        for index, row in enumerate(rows):
            case = {"case_id": f"{name}-{index}", "document": name, "source_group": name,
                    "availability": row.get("availability", "present"), "top_correct": row.get("top_correct", False),
                    "status": row.get("status", "ok"), "checks": row.get("checks", {})}
            if case["status"] == "ok":
                report_path = self.root / (case["case_id"] + ".json")
                abstention.write_json(report_path, report(row.get("top", -2.), row.get("gap", 3.)))
                case.update(report=report_path.name, report_sha256=abstention.digest_bytes(report_path.read_bytes()))
            else:
                case["error"] = "No local context available"
            cases.append(case)
        path = self.root / (name + "-manifest.json")
        abstention.write_json(path, {"schema_version": 1, "dataset_id": name, "cases": cases})
        return path

    def fit(self, path):
        args = argparse.Namespace(manifest=path, metric="combined_score", min_accuracy=1.,
                                  max_absent_acceptance=0., min_answers=1, require_font_check=False,
                                  require_link_check=False, output_policy=self.root / "policy.json",
                                  output_development=self.root / "development.json")
        with contextlib.redirect_stdout(io.StringIO()):
            abstention.fit(args)
        return args.output_policy, args.output_development

    def test_fit_freezes_before_heldout_evaluation_and_keeps_failures(self):
        development = self.manifest("development", [
            {"top_correct": True, "gap": 4.}, {"top_correct": True, "gap": 3.},
            {"top_correct": False, "gap": .2}, {"availability": "absent", "gap": .1},
        ])
        policy_path, dev_path = self.fit(development)
        policy_bytes = policy_path.read_bytes()
        heldout = self.manifest("heldout", [
            {"top_correct": False, "gap": 6.}, {"availability": "absent", "gap": 6.},
            {"status": "error"},
        ])
        args = argparse.Namespace(manifest=heldout, policy=policy_path, development=dev_path,
                                  output=self.root / "evaluation.json")
        with contextlib.redirect_stdout(io.StringIO()):
            abstention.evaluate(args)
        result = abstention.read_json(args.output)
        self.assertEqual(policy_path.read_bytes(), policy_bytes)
        self.assertEqual(result["selected"]["total_cases"], 3)
        self.assertEqual(result["selected"]["failed_or_unusable_reports"], 1)
        self.assertEqual(result["selected"]["coverage"], 2 / 3)
        self.assertEqual(result["selected"]["selective_accuracy"], 0.)
        self.assertEqual(result["selected"]["absent_wrong_acceptances"], 1)
        self.assertTrue(result["frozen_curve_accuracy_vs_coverage"])

    def test_document_and_source_group_overlap_are_rejected(self):
        policy = {"development_documents": ["dev-document"], "development_source_groups": ["same-source"]}
        for document, group in [("dev-document", "other"), ("alias", "same-source")]:
            with self.assertRaisesRegex(ValueError, "overlap"):
                abstention.reject_overlap(policy, {"cases": [{"document": document, "source_group": group}]})

    def test_failed_font_and_link_controls_cannot_pass_a_score_rule(self):
        policy = {"answer_enabled": True, "signature": abstention.signature(report()),
                  "min_top_score": -10., "min_score_gap": .1,
                  "require_font_check": False, "require_link_check": False}
        for name in ("font_coverage", "link_validity"):
            case = {"features": abstention.features(report(), "combined_score"),
                    "signature": abstention.signature(report()),
                    "checks": {name: {"status": "fail", "source": "known synthetic control"}}}
            answer, reasons = abstention.decide(case, policy)
            self.assertFalse(answer)
            self.assertIn(name + "_failed", reasons)

    def test_disabled_policy_when_development_constraints_have_no_solution(self):
        path = self.manifest("development", [{"top_correct": False}, {"availability": "absent"}])
        policy_path, _ = self.fit(path)
        self.assertFalse(abstention.read_json(policy_path)["answer_enabled"])

    def test_ties_truncation_and_tampered_reports_are_unusable(self):
        tied = report(gap=0.)
        with self.assertRaises(ValueError):
            abstention.features(tied, "combined_score")
        truncated = copy.deepcopy(report())
        truncated["physical"]["tested_candidates"] = 3
        with self.assertRaisesRegex(ValueError, "truncated"):
            abstention.features(truncated, "combined_score")
        path = self.manifest("tampered", [{"top_correct": True}])
        case = abstention.read_json(path)["cases"][0]
        (self.root / case["report"]).write_text("{}")
        _, cases = abstention.load_manifest(path, "combined_score")
        self.assertIsNone(cases[0]["features"])
        self.assertIn("changed", cases[0]["feature_errors"][0])

    def test_language_model_and_adapter_identity_change_the_signature(self):
        original = report()
        changed = copy.deepcopy(original)
        changed["provider"]["response"]["provenance"]["model_sha256"] = "other-model"
        self.assertNotEqual(abstention.signature(original), abstention.signature(changed))
        changed = copy.deepcopy(original)
        changed["provider"]["response"]["domain_adapter"] = {"weights_sha256": "adapter-b"}
        self.assertNotEqual(abstention.signature(original), abstention.signature(changed))
        del changed["provider"]["response"]["provenance"]
        with self.assertRaises(KeyError):
            abstention.signature(changed)

    def test_inconsistent_aggregates_context_and_trace_are_rejected(self):
        for kind in ("aggregate", "observation", "context", "surface"):
            changed = json.loads(json.dumps(report()))
            if kind == "aggregate":
                changed["ranked"][0]["combined_score"] = 1_000_000
            elif kind == "observation":
                changed["ranked"][0]["observations"][0]["token_log_score_sum"] = -.1
            elif kind == "context":
                changed["contexts"]["t"]["left"] = "different visible text"
            else:
                changed["provider"]["request"]["items"][0]["text"] = "other surface"
            with self.assertRaises(ValueError, msg=kind):
                abstention.features(changed, "combined_score")

    def test_malformed_replay_is_counted_as_unusable_in_manifest(self):
        path = self.manifest("malformed", [{"top_correct": True}])
        manifest = abstention.read_json(path)
        item = manifest["cases"][0]
        report_path = self.root / item["report"]
        malformed = report()
        malformed["provider"]["args"] = ["--replay"]
        malformed["provider"]["response"]["method"] = "exact_replay_of_common_language_control"
        abstention.write_json(report_path, malformed)
        item["report_sha256"] = abstention.digest_bytes(report_path.read_bytes())
        abstention.write_json(path, manifest)
        _, cases = abstention.load_manifest(path, "combined_score")
        self.assertEqual(len(cases), 1)
        self.assertIsNone(cases[0]["features"])
        self.assertIn("path is absent", cases[0]["feature_errors"][0])


if __name__ == "__main__":
    unittest.main()
