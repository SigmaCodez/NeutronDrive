import copy
import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location('checker', pathlib.Path(__file__).parents[1] / 'scripts/check-content-release.py')
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

class ContentReleaseTests(unittest.TestCase):
    def test_chronological_beta_after_higher_numbered_alpha(self):
        releases = [{'tag_name': 'v0.6-alpha', 'published_at': '2026-10-20T12:00:00Z'},
                    {'tag_name': 'v0.1-beta', 'published_at': '2026-10-25T12:00:00Z', 'prerelease': True}]
        self.assertEqual(checker.latest(releases)['tag_name'], 'v0.1-beta')

    def test_notes_and_assets_trigger_review(self):
        release = {'body': 'First', 'assets': [{'name': 'package.deb', 'size': 10}]}
        marker = {'release_fingerprint': checker.fingerprint(release)}
        self.assertFalse(checker.needs_update(release, marker))
        for changed in ({**release, 'body': 'Corrected'}, {'body': 'First', 'assets': [{'name': 'package.deb', 'size': 20}]}):
            self.assertTrue(checker.needs_update(changed, marker))
        marker['screenshots_pending'] = True
        self.assertTrue(checker.needs_update(release, marker))
        self.assertTrue(checker.needs_update(release, {}))
