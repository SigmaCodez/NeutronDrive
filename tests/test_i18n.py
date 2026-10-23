"""Locale resolution, catalog safety and public presentation coverage."""
import json
import os
from pathlib import Path
import re
import unittest
from unittest import mock
from core_contract import i18n

ROOT = Path(__file__).resolve().parents[1]

class LocaleTests(unittest.TestCase):
    def test_windows_language_set_and_region_fallbacks(self):
        self.assertEqual(len(i18n.SUPPORTED),25)
        cases = {'de_CH.UTF-8':'de','fr_CA':'fr','en_GB':'en','es_MX':'es-419',
                 'es_AR':'es-419','es_ES':'es','pt_BR':'pt-BR','pt_PT':'pt','pt_AO':'pt',
                 'zh_HK':'zh-TW','zh_Hant_CN':'zh-TW','zh_Hans_TW':'zh-CN',
                 'zh_SG':'zh-CN','zh_CN':'zh-CN','ca_AD':'ca-ES','cs':'cs-CZ',
                 'da':'da-DK','nb':'nb-NO','no_NO':'nb-NO','sv_FI':'sv-SE',
                 'el':'el-GR','be':'be-BY','sk_SK':'sk','C.UTF-8':'en','POSIX':'en'}
        for source, expected in cases.items():
            with self.subTest(source=source):self.assertEqual(i18n.match_locale(source),expected)
        for code in i18n.SUPPORTED:self.assertEqual(i18n.match_locale(code),code)

    def test_linux_precedence_and_english_fallback(self):
        for env,expected in [({},'en'),({'LANG':'ja_JP.UTF-8'},'en'),
            ({'LANG':'de_CH.UTF-8'},'de'),({'LANG':'de_CH','LC_MESSAGES':'fr_CH'},'fr'),
            ({'LANG':'de_CH','LC_MESSAGES':'fr_CH','LC_ALL':'it_IT'},'it'),
            ({'LANG':'de_CH','LANGUAGE':'pt_BR:en'},'pt-BR'),
            ({'LANG':'de_CH','LANGUAGE':'xx:fr'},'fr'),
            ({'LC_ALL':'C','LANGUAGE':'de'},'en'),({'LANG':'C.UTF-8','LANGUAGE':'de'},'en')]:
            with self.subTest(env=env):self.assertEqual(i18n.select_language(env),expected)
        self.assertEqual(i18n.select_language({},['zh-Hant-TW']),'zh-TW')
        self.assertEqual(i18n.select_language({},['ja-JP']),'en')

    def test_explicit_language_overrides_system_and_can_be_reset(self):
        with mock.patch.dict(os.environ, {"LC_ALL": "de_CH.UTF-8", "LANGUAGE": "de"}), mock.patch.object(i18n, "_current", None):
            self.assertEqual(i18n.configure(override="fr"), "fr")
            self.assertEqual(i18n.configure(override="system"), "de")
            self.assertEqual(i18n.configure(override="invalid"), "de")

    def test_exact_coverage_and_placeholder_integrity(self):
        english=i18n.catalog('en')
        self.assertGreaterEqual(len(english),180)
        for code in i18n.SUPPORTED:
            with self.subTest(code=code):
                raw=json.loads((i18n._ROOT/(code+'.json')).read_text())
                self.assertEqual(set(raw),set(english))
                self.assertEqual(len(i18n.catalog(code)),len(english))
                for key,text in raw.items():
                    self.assertTrue(text.strip(),key)
                    self.assertEqual(i18n.placeholders(key),i18n.placeholders(text),key)
                    self.assertNotRegex(text,r'9876[5-9][0-9]')
                    self.assertNotRegex(text,r'[\ufffd\u2047]|<unk>')

    def test_missing_or_invalid_optional_translation_uses_english(self):
        real=i18n.catalog
        with mock.patch.object(i18n,'catalog',side_effect=lambda code: {} if code=='fr' else real(code)):
            self.assertEqual(i18n.tr('Meine Dateien',code='fr'),'My files')
        self.assertEqual(i18n.tr('Meine Dateien',code='xx'),'My files')
        self.assertFalse(i18n._valid_placeholders('{p0}', '{p0.__class__}'))
        self.assertFalse(i18n._valid_placeholders('{p0}', '{p1}'))
        self.assertFalse(i18n._valid_placeholders('{p0}', '{p0:1000000000}'))
        self.assertFalse(i18n._valid_placeholders('{p0}', '{p0!r}'))
        self.assertFalse(i18n._valid_placeholders('{p0}', '{'))

    def test_diagnostic_values_are_literal_and_not_retranslated(self):
        with mock.patch.object(i18n,'_current','en'):
            text=i18n.diagnostic('Interner Fehler (Meine Dateien). Es wurde kein Erfolg angenommen.')
            self.assertEqual(text,'Internal error (Meine Dateien). The operation was not marked as successful.')
            self.assertEqual(i18n.diagnostic('/home/user/Meine Dateien.txt'),'/home/user/Meine Dateien.txt')
            self.assertEqual(i18n.diagnostic('Unknown SDK detail'), 'Unknown SDK detail')
            self.assertEqual(i18n.diagnostic('Die Proton-CLI behandelt *, ?, [ und { in lokalen Pfaden als Suchmuster. Bitte einen anderen Pfad verwenden.'),
                'The Proton CLI interprets wildcard characters in local paths as patterns. Please use a different path.')

    def test_all_explicit_ui_messages_are_catalogued(self):
        english=i18n.catalog('en')
        for path in (ROOT/'desktop/src').glob('*.rs'):
            for literal in re.findall(r'\btr\(\s*("(?:[^"\\]|\\.)*")', path.read_text()):
                self.assertIn(json.loads(literal), english, str(path))
        qml=(ROOT/'src/neutrondrive/qml/Main.qml').read_text()
        for literal in re.findall(r'window\.tr\(("(?:[^"\\]|\\.)*")\)',qml):
            self.assertIn(json.loads(literal),english)
        self.assertNotIn('modelData.kind===',qml)

    def test_native_diagnostics_preserve_literal_values_for_every_catalog(self):
        from string import Formatter
        values = {'p0': 'Meine Dateien {p1} 中文.txt', 'p1': 'ü😀\nsecond line', 'p2': 'path/with spaces'}
        english = i18n.catalog('en')
        for source in english:
            try:
                fields = [name for _, name, _, _ in Formatter().parse(source) if name is not None]
            except ValueError:
                continue
            if not fields:
                continue
            message = source.format(**values)
            for code in i18n.SUPPORTED:
                with self.subTest(source=source, code=code), mock.patch.object(i18n, '_current', code):
                    self.assertEqual(i18n.diagnostic(message), i18n.catalog(code)[source].format(**values))

    def test_native_catalog_failure_and_placeholder_validation(self):
        import tempfile
        self.assertEqual(tuple(i18n._native('supported')), i18n.SUPPORTED)
        with tempfile.TemporaryDirectory() as directory, mock.patch.object(i18n, '_ROOT', Path(directory)):
            try:
                i18n.catalog.cache_clear()
                with self.assertRaises(RuntimeError): i18n.catalog('en')
                (Path(directory) / 'en.json').write_text(json.dumps({'Test {p0}': 'Test {p0}'}))
                (Path(directory) / 'fr.json').write_text(json.dumps({'Test {p0}': 'Test {p0.__class__}'}))
                self.assertEqual(i18n.tr('Test {p0}', code='fr'), 'Test {p0}')
                os.mkfifo(Path(directory) / 'de.json')
                self.assertEqual(i18n.catalog('de'), {})
            finally:
                i18n.catalog.cache_clear()
