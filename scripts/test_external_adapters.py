# Copyright (C) 2026 Toit contributors.
"""Optional adapter fidelity checks against the actual baseline checkouts."""
import contextlib
import importlib.util
import io
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts/adapters"))
from external_common import groups, single_target
from morten_unredactron import scoring_loop
from alex_browser import extract_function
from external_joint import aggregate, geometry


class AdapterTests(unittest.TestCase):
    def test_joint_extension_combines_complementary_ambiguities(self):
        targets = [{"id": name, "page": name, "surface_kind": "canonical", "dependence_key": None,
                    "bounds": {"x": 0., "y": 0., "w": 10., "h": 5.},
                    "evidence": {"kind": "width", "width": 10., "measurement_sigma": 1.}}
                   for name in ("x", "y")]
        data = {"targets": targets, "pages": [{"id": name, "image_sha256": name} for name in ("x", "y")],
                "candidates": [{"id": name, "surfaces": [{"text": name, "kind": "canonical"}]}
                               for name in ("a", "b", "c")]}
        widths = {"x": {"a": 10., "b": 10., "c": 20.}, "y": {"a": 10., "b": 20., "c": 10.}}
        single, _ = aggregate({**data, "targets": targets[:1]}, widths)
        self.assertEqual(single["ranked_groups"][0], ["a", "b"])
        joint, detail = aggregate(data, widths)
        self.assertEqual(joint["ranked_groups"], [["a"], ["b", "c"]])
        self.assertEqual(joint["compatible"], ["a"])
        missing, _ = aggregate({**data, "candidates": data["candidates"][1:]}, widths)
        self.assertEqual(missing["compatible"], [])
        self.assertEqual(missing["ranked_groups"], [["b", "c"]])
        duplicate = {**targets[0], "id": "copy"}
        copied, copied_detail = aggregate({**data, "targets": [*targets, duplicate]}, widths)
        self.assertEqual(copied, joint)
        self.assertEqual(copied_detail["candidates"], detail["candidates"])
        self.assertEqual(copied_detail["ignored_targets"], ["copy"])
        # Explicit dependence also suppresses a differently located copy.
        targets[0]["dependence_key"] = "shared"
        duplicate.update(dependence_key="shared", bounds={"x": 30., "y": 0., "w": 10., "h": 5.})
        self.assertEqual(aggregate({**data, "targets": [*targets, duplicate]}, widths)[0], joint)

    def test_joint_extension_rejects_unsupported_or_invalid_geometry(self):
        for evidence in [{"kind": "mosaic"}, {"kind": "width", "width": 10., "measurement_sigma": 0.},
                         {"kind": "width", "width": float("nan"), "measurement_sigma": 1.}]:
            with self.assertRaises(ValueError):
                geometry({"evidence": evidence})

    def test_ties_and_nonfinite_scores(self):
        self.assertEqual(groups([("a", 2.), ("b", 1.), ("c", 1.)]), [["b", "c"], ["a"]])
        with self.assertRaises(ValueError):
            groups([("a", float("nan"))])

    def test_joint_inputs_are_explicitly_unsupported(self):
        with self.assertRaisesRegex(ValueError, "one occurrence"):
            single_target({"targets": [{}, {}]})

    @unittest.skipUnless((ROOT / "baseline-alex").exists(), "Requires pinned Alex checkout")
    def test_private_browser_functions_are_exported_without_body_edits(self):
        for filename, signature in [("main.js", "async function analyzeRedaction("),
                                     ("solver.js", "function measureWidths(")]:
            path = ROOT / "baseline-alex/unredact/static" / filename
            extracted = extract_function(path, signature)
            self.assertIn(extracted.removeprefix("export "), path.read_text())
            self.assertTrue(extracted.endswith("\n}\n"))

    @unittest.skipUnless((ROOT / "baseline-morten").exists(), "Requires pinned Morten checkout")
    def test_extracted_loop_matches_original_analyze_pdf(self):
        import numpy as np
        from PIL import Image, ImageFont
        repo = ROOT / "baseline-morten"
        sys.path.insert(0, str(repo))
        spec = importlib.util.spec_from_file_location("baseline_unredactron", repo / "unredactron.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        font_path = repo / "fonts/fonts/times.ttf"
        profile = SimpleNamespace(font_size=20, font_path=str(font_path), font_name="times.ttf",
                                  scale_factor=3., tracking_offset=.2)
        font = ImageFont.truetype(str(font_path), 20)
        candidates = [{"name": name, "confidence": confidence, "notes": "fixture"}
                      for name, confidence in [("Avery Frost", 0), ("Casey Hart", 5), ("Amber Vale", 2)]]
        # Supply a contour satisfying upstream's fixed minimum dimensions, while
        # preserving the original PDF-loading, scoring, and acceptance function.
        contour = np.array([[[10, 20]], [[321, 20]], [[321, 139]], [[10, 139]]], dtype=np.int32)
        image = Image.new("RGB", (600, 600), "white")
        with patch.object(module, "convert_from_path", return_value=[image]), \
             patch.object(module.cv2, "findContours", return_value=([contour], None)), \
             contextlib.redirect_stdout(io.StringIO()):
            original = module.analyze_pdf("unused.pdf", str(font_path), candidates, font_profile=profile)
        extracted = scoring_loop(repo / "unredactron.py")(
            font, profile.scale_factor, profile.tracking_offset, candidates,
            [(10, 20, 312, 120)], SimpleNamespace(shape=(600, 600)))
        self.assertEqual(original, extracted)
        self.assertTrue(original[0])


if __name__ == "__main__":
    unittest.main()
