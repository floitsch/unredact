# Copyright (C) 2026 Toit contributors.
import unittest
from finalize_multidoc_language import normalize


class UnavailableTests(unittest.TestCase):
    def test_context_error_does_not_create_api_coverage(self):
        rows=[dict(tracks=dict(external=dict(status='error'),local=dict(status='error')))]
        normalized=normalize(rows,{'external'})
        self.assertEqual(normalized[0]['tracks']['external']['status'],'not_available')
        self.assertEqual(normalized[0]['tracks']['local'],rows[0]['tracks']['local'])
        self.assertEqual(rows[0]['tracks']['external']['status'],'error')


if __name__=='__main__':unittest.main()
