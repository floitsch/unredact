# Copyright (C) 2026 Toit contributors.
import copy
import tempfile
import unittest
from pathlib import Path

import numpy as np
from PIL import Image
import torch

from infer import infer
from train import TRAIN_WORDS, VALID_WORDS, TEST_WORDS, catalog
from typography import FontEncoder, normalize, render


class NeuralTests(unittest.TestCase):
    def test_split_and_catalog_integrity(self):
        self.assertFalse(set(TRAIN_WORDS) & set(VALID_WORDS))
        self.assertFalse(set(TRAIN_WORDS) & set(TEST_WORDS))
        self.assertFalse(set(VALID_WORDS) & set(TEST_WORDS))
        fonts = catalog()
        trained = {f["family"] for f in fonts if f["split"] == "train"}
        unseen = {f["family"] for f in fonts if f["split"] != "train"}
        self.assertFalse(trained & unseen)
        self.assertEqual(len({f["sha256"] for f in fonts}), len(fonts))

    def test_render_is_reproducible_and_not_blank(self):
        font = catalog()[0]["path"]
        a = render(font, "visible calibration words", np.random.default_rng(24))
        b = render(font, "visible calibration words", np.random.default_rng(24))
        self.assertTrue(np.array_equal(a, b))
        self.assertEqual(a.shape, (48, 256))
        self.assertGreater(a.std(), 10)
        with self.assertRaises(ValueError):
            normalize(Image.new("L", (256, 48), 255))

    def test_json_inference_and_hash_validation(self):
        torch.set_num_threads(2)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            model = FontEncoder()
            torch.save(dict(state_dict=model.state_dict(), classes=24, architecture="font-cnn-v1"), root/"model.pt")
            fonts = catalog()[:2]
            crop = render(fonts[0]["path"], "several visible words")
            Image.fromarray(255-crop).save(root/"crop.png")
            request = dict(schema_version=1, model=str(root/"model.pt"), catalog=fonts,
                           images=[str(root/"crop.png")], top_k=2)
            response = infer(request)
            self.assertEqual(response["status"], "ok")
            self.assertEqual(len(response["suggestions"]), 2)
            self.assertTrue(all(np.isfinite(x["score"]) for x in response["suggestions"]))
            bad = copy.deepcopy(request)
            bad["catalog"][0]["sha256"] = "0"*64
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                infer(bad)
            bad = copy.deepcopy(request)
            bad["images"] = [{"path": str(root/"crop.png"), "box": [-1, 0, 10, 10]}]
            with self.assertRaisesRegex(ValueError, "box must"):
                infer(bad)


if __name__ == "__main__":
    unittest.main()
