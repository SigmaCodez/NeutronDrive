"""Qt-facing bindings for the native offline localization runtime."""
from functools import lru_cache
import os
from pathlib import Path
from . import rust_core

SUPPORTED = ('en', 'de', 'fr', 'nl', 'es', 'es-419', 'it', 'pl', 'pt-BR', 'ru',
             'tr', 'ca-ES', 'cs-CZ', 'da-DK', 'hu', 'id', 'nb-NO', 'pt', 'ro',
             'sk', 'sv-SE', 'el-GR', 'be-BY', 'zh-CN', 'zh-TW')
_ROOT = Path(__file__).resolve().parents[2] / 'src/neutrondrive/locales'
_current = None


def _native(action, **values):
    return rust_core.backend_value('localization', operation=dict(action=action, **values))


def match_locale(value):
    return _native('match', value=str(value))


def select_language(environment=None, ui_languages=()):
    env = os.environ if environment is None else environment
    return _native('select', environment={key: env[key] for key in
                   ('LC_ALL', 'LC_MESSAGES', 'LANG', 'LANGUAGE') if key in env},
                   ui_languages=list(ui_languages))


def placeholders(text):
    from .backend import BackendError
    try:
        return _native('placeholders', text=text)
    except BackendError as error:
        raise ValueError(str(error)) from error


def _valid_placeholders(source, target):
    return _native('valid', source=source, target=target)


@lru_cache(maxsize=26)
def catalog(code):
    return _native('catalog', directory=str(_ROOT), code=code)


def configure(ui_languages=(), *, override="system"):
    global _current
    _current = override if override in SUPPORTED else select_language(ui_languages=ui_languages)
    return _current


def language():
    return _current if _current is not None else select_language()


def tr(source, *, code=None):
    return _native('translate', source=source,
                   english={source: catalog('en').get(source)},
                   translated={source: catalog(code or language()).get(source)})


def diagnostic(message):
    return _native('diagnostic', text=str(message), english=catalog('en'),
                   translated=catalog(language()))
