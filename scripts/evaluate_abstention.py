#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Freeze empirical abstention rules on development data, then evaluate them.

Only the standard library is needed. Archived-report import is retrospective:
it does not rerun OCR, geometry, providers, or the current solver.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path

TRACK = "local-profile-cost-weight-1"
INTERPRETATION = (
    "Retrospective empirical thresholds on archived pre-review engine reports. "
    "This is not validation of the current scoring/OCR fixes, a calibrated "
    "probability, or verified recovery. Missing-font and incorrect-link controls "
    "are not present in these archives; no robustness claim is made for them."
)


def digest_bytes(data):
    return hashlib.sha256(data).hexdigest()


def read_json(path):
    return json.loads(Path(path).read_text(), parse_constant=lambda value: (_ for _ in ()).throw(ValueError(f"Nonfinite JSON: {value}")))


def write_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n")


def language_signature(report, base):
    provider = report.get("provider")
    if not provider:
        raise ValueError("Language provider provenance is absent")
    identities = []
    for trace in provider.get("batches", [provider]):
        response = trace["response"]
        method = response["method"]
        if method == "exact_replay_of_common_language_control":
            args = provider["args"]
            if not isinstance(args, list) or "--replay" not in args or args.index("--replay") + 1 >= len(args):
                raise ValueError("Replay cache path is absent")
            cache_path = Path(base) / args[args.index("--replay") + 1]
            data = cache_path.read_bytes()
            if digest_bytes(data) != response["reference_sha256"]:
                raise ValueError("Replay cache hash mismatch")
            cache = read_json(cache_path)
            request = trace["request"]
            if request["contexts"] != cache["request"]["contexts"]:
                raise ValueError("Replay contexts differ from original scores")
            originals = cache["request"]["items"]
            cached_scores = {(r["id"], r["target"]): r for r in cache["scores"]}
            response_scores = {(r["id"], r["target"]): r for r in response["scores"]}
            if len(request["items"]) != len(response["scores"]):
                raise ValueError("Replay item/score counts differ")
            for item in request["items"]:
                if item not in originals:
                    raise ValueError("Replay surface differs from original scores")
                original, score = cached_scores[item["id"], item["target"]], response_scores[item["id"], item["target"]]
                if original["token_count"] != score["token_count"] or not math.isclose(
                        original["token_log_score_sum"], score["token_log_score_sum"], rel_tol=0., abs_tol=1e-9):
                    raise ValueError("Replay score differs from original cache")
            model = cache["model"]
            identity = {"method": method, "provenance": model, "source_hashes": None, "domain_adapter": None, "options": None}
        else:
            model = response["provenance"]
            identity = {"method": method, "provenance": model,
                        **{k: response.get(k) for k in ("source_hashes", "domain_adapter", "options")}}
        if not isinstance(model, dict) or not model:
            raise ValueError("Language model identity is absent")
        identities.append(identity)
    if not identities or any(identity != identities[0] for identity in identities):
        raise ValueError("Missing language batches or mixed model identity")
    identities[0]["context_sources"] = sorted({c.get(side, "unspecified") for c in report["contexts"].values()
                                               for side in ("left_source", "right_source")})
    return identities[0]


def signature(report, base=Path(".")):
    return {
        "normalization": report.get("normalization"),
        "physical_weight": report.get("physical_weight"),
        "physical_interpretation": report["physical"]["interpretation"],
        "language": language_signature(report, base),
    }


def signature_matches(expected, actual):
    if actual is None:
        return False
    expected, actual = dict(expected), dict(actual)
    expected["language"], actual["language"] = dict(expected["language"]), dict(actual["language"])
    allowed = expected["language"].pop("context_sources", [])
    observed = actual["language"].pop("context_sources", [])
    return bool(observed) and set(observed) <= set(allowed) and expected == actual


def finite(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def same_score(actual, expected):
    return finite(actual) and finite(expected) and abs(actual - expected) <= 1e-8


def provider_scores(report):
    provider = report.get("provider")
    if not provider:
        raise ValueError("Provider trace is absent")
    result = {}
    physical = {c["id"]: c for c in report["physical"]["candidates"] if c["compatible"]}
    for trace in provider.get("batches", [provider]):
        request, response = trace["request"], trace["response"]["scores"]
        if request["contexts"] != report["contexts"]:
            raise ValueError("Report contexts differ from provider request")
        if len(request["items"]) != len(response):
            raise ValueError("Provider request/response counts differ")
        keys = set()
        for item in request["items"]:
            key = item["id"], item["target"]
            if key in keys:
                raise ValueError("Duplicate provider request item")
            keys.add(key)
            candidate = physical[item["id"]]
            observations = candidate.get("compatibility_witness") or candidate["observations"]
            surface = next((o["surface"] for o in observations if o["target"] == item["target"]), None)
            if surface != item["text"]:
                raise ValueError("Provider text differs from compatible physical surface")
            if sum(s["id"] == item["id"] and s["target"] == item["target"] for s in response) != 1:
                raise ValueError("Provider response item is missing or duplicated")
        for score in response:
            key = score["id"], score["target"]
            if key not in keys or key in result:
                raise ValueError("Unsolicited or duplicate provider score")
            result[key] = score
    return result


def features(report, metric):
    """Match abstention::features gates; never use truth labels in extraction."""
    physical = report["physical"]
    ranks = report["ranked"]
    count = physical["independent_observations"]
    if report["schema_version"] != 1 or count < 1:
        raise ValueError("Unsupported report or no independent observations")
    if len(physical["candidates"]) != physical["tested_candidates"]:
        raise ValueError("Physical report is incomplete or truncated")
    if len(ranks) != physical["compatible_candidates"]:
        raise ValueError("Context report omits compatible candidates")
    if len(ranks) < 2:
        raise ValueError("At least two eligible candidates are required for a score gap")
    contexts = set(report["contexts"])
    if len(contexts) != count or any(not (c["left"].strip() or c["right"].strip()) for c in report["contexts"].values()):
        raise ValueError("Missing occurrence context")
    ids, compatible_ids = set(), set()
    for candidate in physical["candidates"]:
        if not candidate["id"] or candidate["id"] in ids or not finite(candidate["evidence_score"]):
            raise ValueError("Duplicate/empty physical ID or nonfinite physical score")
        ids.add(candidate["id"])
        if candidate["compatible"]:
            compatible_ids.add(candidate["id"])
            observations = candidate.get("compatibility_witness") or candidate["observations"]
            if len(observations) != count or {o["target"] for o in observations} != contexts:
                raise ValueError("Incomplete physical observations")
            if any(not finite(o["score"]) or not finite(o["standardized_error"]) or o["standardized_error"] > 3 for o in observations):
                raise ValueError("Invalid physical compatibility witness")
    if len(compatible_ids) != physical["compatible_candidates"]:
        raise ValueError("Inconsistent physical compatible count")
    traced = provider_scores(report)
    if len(traced) != len(ranks) * count:
        raise ValueError("Provider trace is incomplete")
    ranked_ids = [r["id"] for r in ranks]
    if len(set(ranked_ids)) != len(ranks) or set(ranked_ids) != compatible_ids:
        raise ValueError("Context/physical candidate membership differs")
    for rank in ranks:
        observations = rank["observations"]
        if len(observations) != count or {o["target"] for o in observations} != contexts:
            raise ValueError("Incomplete contextual observations")
        if any(o["id"] != rank["id"] or o["token_count"] < 1 or not finite(o["token_log_score_sum"]) or o["token_log_score_sum"] > 0 for o in observations):
            raise ValueError("Invalid contextual observations")
        candidate = next(c for c in physical["candidates"] if c["id"] == rank["id"] and c["compatible"])
        for observation in observations:
            original = traced[rank["id"], observation["target"]]
            if original["token_count"] != observation["token_count"] or not same_score(original["token_log_score_sum"], observation["token_log_score_sum"]):
                raise ValueError("Ranked observation differs from provider response")
        sequence = sum(o["token_log_score_sum"] / count for o in observations)
        token = sum(o["token_log_score_sum"] / o["token_count"] / count for o in observations)
        if not same_score(rank["mean_token_log_score"], token):
            raise ValueError("Token score aggregate is inconsistent")
        if rank.get("mean_sequence_log_score") is not None and not same_score(rank["mean_sequence_log_score"], sequence):
            raise ValueError("Sequence score aggregate is inconsistent")
        if rank.get("mean_word_log_score") is not None:
            surfaces = candidate.get("compatibility_witness") or candidate["observations"]
            words = {o["target"]: len(o["surface"].split()) for o in surfaces}
            if not all(words.values()) or not same_score(rank["mean_word_log_score"], sum(o["token_log_score_sum"] / words[o["target"]] / count for o in observations)):
                raise ValueError("Word score aggregate is inconsistent")
        physical_cost = candidate["evidence_score"] / count
        if rank.get("mean_physical_cost") is not None and not same_score(rank["mean_physical_cost"], physical_cost):
            raise ValueError("Physical score aggregate is inconsistent")
        if rank.get("combined_score") is not None:
            weight = report.get("physical_weight")
            if not finite(weight) or weight <= 0 or rank.get("mean_sequence_log_score") is None or rank.get("mean_physical_cost") is None or not same_score(rank["combined_score"], sequence - weight * physical_cost):
                raise ValueError("Combined score aggregate is inconsistent")
        normalization = report.get("normalization")
        if normalization == "sequence_sum_mean_over_occurrences" and rank.get("mean_sequence_log_score") is None:
            raise ValueError("Missing sequence score")
        if normalization == "whitespace_word_mean" and rank.get("mean_word_log_score") is None:
            raise ValueError("Missing word score")
        if normalization not in (None, "sequence_sum_mean_over_occurrences", "whitespace_word_mean"):
            raise ValueError("Unknown context score normalization")
    groups = report["ranked_groups"]
    if not groups or len(groups[0]) != 1 or [i for group in groups for i in group] != ranked_ids:
        raise ValueError("Ranking groups are tied, incomplete, or inconsistent")
    scores = [r.get(metric) for r in ranks]
    if any(not finite(score) for score in scores) or any(a < b for a, b in zip(scores, scores[1:])):
        raise ValueError("Invalid selected scores or different ranking order")
    gap = scores[0] - scores[1]
    if not finite(gap) or gap <= 1e-9:
        raise ValueError("Top rank tied or invalid score gap")
    return {"top_candidate": ranks[0]["id"], "top_score": scores[0], "score_gap": gap, "eligible_candidates": len(ranks)}


def check_reasons(checks, policy):
    reasons = []
    for name, required in [("font_coverage", policy["require_font_check"]), ("link_validity", policy["require_link_check"])]:
        check = checks.get(name, {"status": "unknown", "source": "", "description": ""})
        if check["status"] == "fail":
            reasons.append(name + "_failed")
        elif check["status"] == "unknown" and required:
            reasons.append(name + "_unassessed")
        elif check["status"] == "pass" and not check.get("source", "").strip():
            reasons.append(name + "_check_has_no_source")
        elif check["status"] not in ("pass", "fail", "unknown"):
            raise ValueError("Invalid evidence check status")
    return reasons


def decide(case, policy):
    reasons = list(case.get("feature_errors", []))
    if not policy["answer_enabled"]:
        reasons.append("development_constraints_unsatisfied")
    signature_ok = case["_signature_ok"] if "_signature_ok" in case else signature_matches(policy["signature"], case.get("signature"))
    if not signature_ok and case.get("features"):
        reasons.append("report_signature_mismatch")
    reasons.extend(check_reasons(case.get("checks", {}), policy))
    f = case.get("features")
    if f is not None:
        if f["top_score"] < policy["min_top_score"]:
            reasons.append("top_score_below_policy_threshold")
        if f["score_gap"] < policy["min_score_gap"]:
            reasons.append("score_gap_below_policy_threshold")
    else:
        reasons.append("unusable_report")
    return not reasons, reasons


def load_manifest(path, metric):
    path = Path(path).resolve()
    manifest = read_json(path)
    if manifest.get("schema_version") != 1 or not manifest.get("cases"):
        raise ValueError("Manifest needs schema_version 1 and nonempty cases")
    seen = set()
    cases = []
    for original in manifest["cases"]:
        case = dict(original)
        if not case.get("case_id") or case["case_id"] in seen or not case.get("document") or not case.get("source_group"):
            raise ValueError("Manifest needs unique case IDs and document/source groups")
        seen.add(case["case_id"])
        if case["availability"] not in ("present", "absent") or not isinstance(case["top_correct"], bool):
            raise ValueError("Invalid case ground truth")
        if case["availability"] == "absent" and case["top_correct"]:
            raise ValueError("Absent truth cannot be a correct top answer")
        case["features"] = None
        case["feature_errors"] = []
        if case.get("status") != "ok":
            case["feature_errors"].append(case.get("error", "upstream_pipeline_failure"))
        else:
            try:
                report_path = path.parent / case["report"]
                data = report_path.read_bytes()
                if digest_bytes(data) != case["report_sha256"]:
                    raise ValueError("Report changed after manifest freeze")
                report = read_json(report_path)
                case["signature"] = signature(report, report_path.parent)
                case["features"] = features(report, metric)
            except (OSError, ValueError, KeyError, TypeError, IndexError, StopIteration, ZeroDivisionError) as error:
                case["feature_errors"].append(str(error))
        cases.append(case)
    return manifest, cases


def metrics(cases, policy, include_rows=False):
    answered = correct = absent = absent_accepted = failures = 0
    rows = []
    for case in cases:
        answer, reasons = decide(case, policy)
        answered += answer
        correct += answer and case["top_correct"]
        absent += case["availability"] == "absent"
        absent_accepted += answer and case["availability"] == "absent"
        failures += case["features"] is None
        if include_rows:
            rows.append({k: case[k] for k in ("case_id", "document", "source_group", "availability", "top_correct", "features")} | {
                "decision": "answer" if answer else "insufficient_evidence", "reasons": reasons,
                "report_sha256": case.get("report_sha256"),
            })
    result = {
        "total_cases": len(cases), "usable_reports": len(cases) - failures,
        "failed_or_unusable_reports": failures, "answered": answered, "correct_answers": correct,
        "wrong_answers": answered - correct, "coverage": answered / len(cases),
        "selective_accuracy": correct / answered if answered else None,
        "correct_answer_yield": correct / len(cases),
        "absent_cases": absent, "absent_wrong_acceptances": absent_accepted,
        "absent_wrong_acceptance_rate": absent_accepted / absent if absent else None,
    }
    if include_rows:
        result["cases"] = rows
    return result


def reject_overlap(policy, manifest):
    documents = set(policy["development_documents"]) & {c["document"] for c in manifest["cases"]}
    groups = set(policy["development_source_groups"]) & {c["source_group"] for c in manifest["cases"]}
    if documents or groups:
        raise ValueError(f"Development/evaluation document or source-group overlap: {sorted(documents | groups)}")


def fit(args):
    manifest, cases = load_manifest(args.manifest, args.metric)
    if not 0 <= args.min_accuracy <= 1 or not 0 <= args.max_absent_acceptance <= 1 or args.min_answers < 1:
        raise ValueError("Invalid development objective")
    if not any(c["availability"] == "absent" for c in cases):
        raise ValueError("Development data needs absent-answer controls")
    usable = [c for c in cases if c["features"] is not None]
    if not usable:
        raise ValueError("No usable development reports")
    sig = json.loads(json.dumps(usable[0]["signature"]))
    sig["language"]["context_sources"] = sorted({source for c in usable for source in c["signature"]["language"]["context_sources"]})
    if any(not signature_matches(sig, c["signature"]) for c in usable):
        raise ValueError("Mixed scoring/report-generation signatures in development")
    base = {
        "schema_version": 1, "id": "empirical-development-policy", "answer_enabled": True,
        "metric": args.metric, "min_score_gap": 0., "min_top_score": 0., "signature": sig,
        "require_font_check": args.require_font_check, "require_link_check": args.require_link_check,
        "development_documents": sorted({c["document"] for c in cases}),
        "development_source_groups": sorted({c["source_group"] for c in cases}),
        "provenance": {
            "interpretation": manifest.get("interpretation", "Empirical development selection, not calibrated probabilities"),
            "development_manifest_sha256": digest_bytes(Path(args.manifest).read_bytes()),
            "development_dataset_id": manifest["dataset_id"],
            "report_generation": manifest.get("report_generation", "unspecified"),
            "selection_rule": "Maximize answered cases subject to development selective accuracy, absent wrong acceptance, and minimum answer count; then maximize correct answers and prefer stricter thresholds. No evaluation labels used.",
            "objective": {"min_accuracy": args.min_accuracy, "max_absent_acceptance": args.max_absent_acceptance, "min_answers": args.min_answers},
            "model_provenance": manifest.get("model_provenance", []),
            "development_report_hashes": {c["case_id"]: c.get("report_sha256") for c in cases},
        },
    }
    for case in cases:
        case["_signature_ok"] = signature_matches(base["signature"], case.get("signature"))
    levels = sorted({c["features"]["top_score"] for c in usable})
    gaps = sorted({0.} | {c["features"]["score_gap"] for c in usable})
    levels.append(math.nextafter(levels[-1], math.inf))
    gaps.append(math.nextafter(gaps[-1], math.inf))
    choices, frontier = [], {}
    for level in levels:
        for gap in gaps:
            policy = base | {"min_top_score": level, "min_score_gap": gap}
            result = metrics(cases, policy)
            key = result["answered"]
            row = {"min_top_score": level, "min_score_gap": gap, "metrics": result}
            quality = (result["correct_answers"], -result["absent_wrong_acceptances"], gap, level)
            if key not in frontier or quality > frontier[key][0]:
                frontier[key] = (quality, row)
            if key >= args.min_answers and result["selective_accuracy"] >= args.min_accuracy and result["absent_wrong_acceptance_rate"] <= args.max_absent_acceptance:
                choices.append((key, quality, row))
    if choices:
        chosen = max(choices, key=lambda item: (item[0], item[1]))[2]
        policy = base | {k: chosen[k] for k in ("min_top_score", "min_score_gap")}
    else:
        policy = base | {"answer_enabled": False, "min_top_score": levels[-1], "min_score_gap": gaps[-1]}
    policy["id"] = "dev-" + digest_bytes(json.dumps(policy, sort_keys=True).encode())[:16]
    write_json(args.output_policy, policy)
    frozen = [value[1] for _, value in sorted(frontier.items())]
    write_json(args.output_development, {
        "schema_version": 1, "role": "development_only", "policy_sha256": digest_bytes(Path(args.output_policy).read_bytes()),
        "interpretation": base["provenance"]["interpretation"], "threshold_pairs_considered": len(levels) * len(gaps),
        "selected": metrics(cases, policy, include_rows=True),
        "frozen_curve_rules": frozen,
        "curve_note": "For each development answer count, freeze the thresholds with most correct development answers. All these rules are fixed before held-out evaluation; the curve is exploratory, and the selected rule is not changed using evaluation outcomes.",
    })
    print(json.dumps({"policy": str(args.output_policy), "selected": metrics(cases, policy)}, sort_keys=True))


def evaluate(args):
    policy = read_json(args.policy)
    manifest, cases = load_manifest(args.manifest, policy["metric"])
    reject_overlap(policy, manifest)
    result = {
        "schema_version": 1, "role": "held_out_evaluation_of_frozen_policy",
        "interpretation": manifest.get("interpretation", policy["provenance"]["interpretation"]),
        "policy_sha256": digest_bytes(Path(args.policy).read_bytes()),
        "evaluation_manifest_sha256": digest_bytes(Path(args.manifest).read_bytes()),
        "evaluation_documents": sorted({c["document"] for c in cases}),
        "selected": metrics(cases, policy, include_rows=True),
        "unconditional_top_baseline": {
            "note": "Every usable unique top answer is accepted, with all upstream failures retained in the denominator.",
            **metrics(cases, policy | {"answer_enabled": True, "min_score_gap": 0., "min_top_score": -1e300,
                                      "require_font_check": False, "require_link_check": False}),
        },
        "unmeasured_controls": ["missing_fonts", "incorrect_links"],
    }
    if args.development:
        development = read_json(args.development)
        if development["policy_sha256"] != result["policy_sha256"]:
            raise ValueError("Frozen curve belongs to a different policy")
        result["frozen_curve_accuracy_vs_coverage"] = [
            {"thresholds": {k: row[k] for k in ("min_top_score", "min_score_gap")},
             "development": row["metrics"],
             "held_out": metrics(cases, policy | {"answer_enabled": True, "min_top_score": row["min_top_score"], "min_score_gap": row["min_score_gap"]})}
            for row in development["frozen_curve_rules"]
        ]
        result["development_file_sha256"] = digest_bytes(Path(args.development).read_bytes())
    write_json(args.output, result)
    print(json.dumps({"evaluation": str(args.output), "selected": {k:v for k,v in result["selected"].items() if k != "cases"}}, sort_keys=True))


def import_archive(summary_path, output_path, dataset_id):
    summary_path, output_path = Path(summary_path).resolve(), Path(output_path).resolve()
    summary = read_json(summary_path)
    cases, models = [], {}
    for row in summary["rows"]:
        track = row["tracks"][TRACK]
        folder = summary_path.parent / row["case"]
        report_path = folder / (TRACK + ".json")
        evaluation_path, scores_path = folder / "evaluation.json", folder / "scores.json"
        case = {
            "case_id": row["case"], "document": row["document"], "source_group": row["source_group"],
            "availability": row["availability"], "top_correct": bool(track.get("unique_correct_first", False)),
            "status": track["status"], "kind": row["kind"],
            "label_source": os.path.relpath(evaluation_path, output_path.parent),
            "label_sha256": digest_bytes(evaluation_path.read_bytes()) if evaluation_path.exists() else None,
            "checks": {},
        }
        if track["status"] == "ok" and report_path.exists():
            case["report"] = os.path.relpath(report_path, output_path.parent)
            case["report_sha256"] = digest_bytes(report_path.read_bytes())
            if scores_path.exists():
                scores = read_json(scores_path)
                case["scores_sha256"] = digest_bytes(scores_path.read_bytes())
                model = scores.get("model", {})
                models[digest_bytes(json.dumps(model, sort_keys=True).encode())] = model
        else:
            case["status"] = "error"
            case["error"] = track.get("reason", "Archived report missing")
        cases.append(case)
    write_json(output_path, {
        "schema_version": 1, "dataset_id": dataset_id, "interpretation": INTERPRETATION,
        "report_generation": "archived_pre_review_fixes", "summary_sha256": digest_bytes(summary_path.read_bytes()),
        "track": TRACK, "model_provenance": list(models.values()), "cases": cases,
    })


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    prepare = sub.add_parser("prepare-archives")
    prepare.add_argument("--development-summary", type=Path, required=True)
    prepare.add_argument("--evaluation-summary", type=Path, required=True)
    prepare.add_argument("--output-dir", type=Path, required=True)
    train = sub.add_parser("fit")
    train.add_argument("--manifest", type=Path, required=True)
    train.add_argument("--metric", choices=["combined_score", "mean_sequence_log_score", "mean_word_log_score", "mean_token_log_score"], default="combined_score")
    train.add_argument("--min-accuracy", type=float, required=True)
    train.add_argument("--max-absent-acceptance", type=float, required=True)
    train.add_argument("--min-answers", type=int, required=True)
    train.add_argument("--require-font-check", action="store_true")
    train.add_argument("--require-link-check", action="store_true")
    train.add_argument("--output-policy", type=Path, required=True)
    train.add_argument("--output-development", type=Path, required=True)
    test = sub.add_parser("evaluate")
    test.add_argument("--manifest", type=Path, required=True)
    test.add_argument("--policy", type=Path, required=True)
    test.add_argument("--development", type=Path)
    test.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "prepare-archives":
        import_archive(args.development_summary, args.output_dir / "development-manifest.json", "archived-four-document-development")
        import_archive(args.evaluation_summary, args.output_dir / "evaluation-manifest.json", "archived-eight-document-evaluation")
    elif args.command == "fit":
        fit(args)
    else:
        evaluate(args)


if __name__ == "__main__":
    main()
