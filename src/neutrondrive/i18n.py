"""Offline UI catalogs; locale selection never changes paths or protocol values."""
from __future__ import annotations
from functools import lru_cache
import json
import os
from pathlib import Path
import re
from string import Formatter

SUPPORTED = ('en', 'de', 'fr', 'nl', 'es', 'es-419', 'it', 'pl', 'pt-BR', 'ru',
             'tr', 'ca-ES', 'cs-CZ', 'da-DK', 'hu', 'id', 'nb-NO', 'pt', 'ro',
             'sk', 'sv-SE', 'el-GR', 'be-BY', 'zh-CN', 'zh-TW')
_LATIN_AMERICA = frozenset('AR BO CL CO CR CU DO EC GT HN MX NI PA PY PE PR SV UY VE'.split())
_BASE = {'ca': 'ca-ES', 'cs': 'cs-CZ', 'da': 'da-DK', 'nb': 'nb-NO', 'no': 'nb-NO',
         'sv': 'sv-SE', 'el': 'el-GR', 'be': 'be-BY'}
_ROOT = Path(__file__).with_name('locales')
_current = None


def match_locale(value):
    """Resolve a POSIX/BCP47 locale, preserving script and region distinctions."""
    value = str(value).split('.', 1)[0].replace('_', '-')
    value = value.split('@', 1)[0]
    parts = value.lower().split('-')
    base = parts[0]
    if base in {'c', 'posix', ''}:
        return 'en'
    if base == 'zh':
        if 'hant' in parts or (not 'hans' in parts and any(p in parts for p in ('tw', 'hk', 'mo'))):
            return 'zh-TW'
        return 'zh-CN'
    if base == 'es' and any(p.upper() in _LATIN_AMERICA or p == '419' for p in parts[1:]):
        return 'es-419'
    if base == 'pt' and 'br' in parts:
        return 'pt-BR'
    if base in _BASE:
        return _BASE[base]
    return base if base in SUPPORTED else None


def select_language(environment=None, ui_languages=()):
    env = os.environ if environment is None else environment
    effective = env.get('LC_ALL') or env.get('LC_MESSAGES') or env.get('LANG')
    if effective and effective.split('.', 1)[0].upper() in {'C', 'POSIX'}:
        return 'en'
    # LANGUAGE is the user's ordered Linux UI preference (GNU gettext semantics).
    preferred = [x for x in env.get('LANGUAGE', '').split(':') if x]
    if not preferred:
        preferred = [effective] if effective else list(ui_languages)
    for value in preferred:
        result = match_locale(value)
        if result:
            return result
    return 'en'


def placeholders(text):
    try:
        return sorted(name for _, name, _, _ in Formatter().parse(text) if name is not None)
    except ValueError:
        if re.search(r'\{p\d+', text):
            raise
        return []  # Literal brace in a plain diagnostic, not a format template.


@lru_cache(maxsize=26)
def catalog(code):
    if code not in SUPPORTED:
        code = 'en'
    try:
        value = json.loads((_ROOT / (code + '.json')).read_text(encoding='utf-8'))
    except (OSError, ValueError):
        if code == 'en':
            raise RuntimeError('The English UI catalog is missing or invalid.')
        return {}
    if not isinstance(value, dict):
        if code == 'en':
            raise RuntimeError('The English UI catalog is invalid.')
        return {}
    # A damaged optional translation cannot introduce format fields or break UI.
    valid = {key: text for key, text in value.items()
            if isinstance(key, str) and isinstance(text, str) and text
            and _valid_placeholders(key, text)}
    if code == 'en' and len(valid) != len(value):
        raise RuntimeError('The English UI catalog contains invalid entries.')
    return valid


def _valid_placeholders(source, target):
    def fields(text):
        try:
            return sorted((name, spec, conversion or '') for _, name, spec, conversion
                          in Formatter().parse(text) if name is not None)
        except ValueError:
            if re.search(r'\{p\d+', text):
                raise
            return []
    try:
        return fields(source) == fields(target)
    except ValueError:
        return False


def configure(ui_languages=()):
    global _current
    _current = select_language(ui_languages=ui_languages)
    return _current


def language():
    return _current if _current is not None else select_language()


def tr(source, *, code=None):
    code = code or language()
    english = catalog('en')
    return catalog(code).get(source) or english.get(source, source)


@lru_cache(maxsize=1)
def _diagnostic_patterns():
    result = []
    for source in catalog('en'):
        if '{' not in source:
            continue
        try:
            chunks = list(Formatter().parse(source))
        except ValueError:
            continue
        if not any(len(literal.strip()) > 8 for literal, _, _, _ in chunks):
            continue
        pattern = ''.join(re.escape(literal) + ('(.*?)' if name is not None else '')
                          for literal, name, _, _ in chunks)
        names = [name for _, name, _, _ in chunks if name is not None]
        result.append((re.compile(pattern, re.DOTALL), source, names))
    return result


def diagnostic(message):
    """Translate only known application diagnostics; captured values stay literal.

    Call only at an error/status display boundary, never on names or protocol data.
    Unknown third-party diagnostic text is retained rather than mistranslated.
    """
    text = str(message)
    if text in catalog('en'):
        return tr(text)
    for pattern, source, names in _diagnostic_patterns():
        match = pattern.fullmatch(text)
        if match:
            return tr(source).format(**dict(zip(names, match.groups())))
    return text
