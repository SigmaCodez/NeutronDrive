import copy
import importlib.util
import json
from pathlib import Path
import unittest

SITE = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('faq_sync', SITE/'scripts/sync-server-content.py')
sync = importlib.util.module_from_spec(spec); spec.loader.exec_module(sync)

class ReleaseFaqTests(unittest.TestCase):
    def fields(self, tag='9.2.1'):
        return {'faq': [[f'General {i}?', 'Answer'] for i in range(4)] +
                       [['Old release?', 'STALE'], ['Old upgrade?', 'STALE'], ['Old limits?', 'STALE'], ['Contribute?', 'Help']],
                'releaseNotes': f'What is new in {tag}?\nNew changes.\n\nHow to update to {tag}?\nNew upgrade.\n\nWhich limits remain?\nNew limits.'}

    def test_all_languages_derive_current_faq_and_keep_general_questions(self):
        for language in sync.LANGUAGES:
            fields=self.fields()
            doc={'schema':1,'release':'v9.2.1','locales':{'en':fields,language:fields}}
            actual=sync.locales(doc,'v9.2.1')[language]
            self.assertEqual(actual['faq'][4:7], [['What is new in 9.2.1?', 'New changes.'], ['How to update to 9.2.1?', 'New upgrade.'], ['Which limits remain?', 'New limits.']])
            self.assertEqual(actual['faq'][:4],fields['faq'][:4])
            self.assertEqual(actual['faq'][-1],fields['faq'][-1])
            self.assertNotIn('STALE',str(actual))

    def test_next_release_updates_answers_not_only_version_numbers(self):
        fields=self.fields('9.2.2');fields['releaseNotes']=fields['releaseNotes'].replace('New changes.', 'Different fix.')
        self.assertEqual(sync.release_faq(fields,'v9.2.2',strict=True)[4],['What is new in 9.2.2?','Different fix.'])

    def test_literal_newlines_and_five_row_source(self):
        fields=self.fields();expected=sync.release_faq(fields,'v9.2.1')
        fields['faq']=fields['faq'][:4]+fields['faq'][-1:]
        fields['releaseNotes']=fields['releaseNotes'].replace('\n','\\n')
        self.assertEqual(sync.release_faq(fields,'v9.2.1',strict=True),expected)

    def test_stale_missing_or_malformed_notes_never_restore_old_faq(self):
        for notes in [self.fields('9.2')['releaseNotes'], '', 'Unstructured text', 'New in 9.2.1?\n']:
            fields=self.fields();fields['releaseNotes']=notes
            self.assertEqual(len(sync.release_faq(fields,'v9.2.1')),5)
            with self.assertRaises(ValueError):sync.release_faq(fields,'v9.2.1',strict=True)

    def test_reviewed_source_in_all_five_languages(self):
        path=SITE.parent/'docs/website-content.json'
        if not path.exists():self.skipTest('Reviewed release source outside website Docker context')
        doc=json.loads(path.read_text())
        for language in sync.LANGUAGES:
            self.assertEqual(len(sync.release_faq(doc['locales'][language],doc['release'],strict=True)),8)
