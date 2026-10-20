# Copyright (C) 2026 Toit contributors.
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parent/'adapters'))
from alex_catalog import install_catalog


class CatalogTests(unittest.TestCase):
    def test_only_assets_and_manifest_change_and_bytes_are_verified(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);site=root/'site';(site/'fonts').mkdir(parents=True)
            code=site/'font_detect.js';code.write_text('unchanged upstream code')
            font=root/'font.ttf';font.write_bytes(b'exact font bytes')
            digest=hashlib.sha256(font.read_bytes()).hexdigest()
            catalog=root/'catalog.json'
            entries=[dict(id='quoted " name',path='font.ttf',sha256=digest)]*2
            catalog.write_text(json.dumps(entries));install_catalog(site,catalog)
            manifest=json.loads((site/'fonts/manifest.json').read_text())
            self.assertEqual(len(manifest['fonts']),1)
            selected=manifest['fonts'][0]
            self.assertEqual((site/'fonts'/selected['file']).read_bytes(),font.read_bytes())
            self.assertEqual(code.read_text(),'unchanged upstream code')
            self.assertNotIn('"',selected['name'])
            entries[0]['sha256']='bad';catalog.write_text(json.dumps(entries))
            with self.assertRaisesRegex(ValueError,'hash mismatch'):install_catalog(site,catalog)


if __name__=='__main__':unittest.main()
