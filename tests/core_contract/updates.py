"""Bounded public release checks and verified downloads; no credentials or cloud data."""
from dataclasses import dataclass
from pathlib import Path

REPOSITORY = 'https://github.com/SigmaCodez/NeutronDrive'
API = 'https://api.github.com/repos/SigmaCodez/NeutronDrive/releases?per_page=100'
MAX_PACKAGE = 512 * 1024 * 1024


def version_key(value):
    from .rust_core import call, CoreError
    try:
        return tuple(call('update_version', value=value))
    except CoreError as error:
        raise ValueError('Unsupported version') from error


def _runtime(action, cancelled=None, **values):
    from .rust_core import backend_value
    result = backend_value('update_runtime', operation=dict(action=action, **values), cancel=cancelled)
    if 'error_kind' in result:
        errors = dict(validation=ValueError, io=OSError, timeout=TimeoutError, cancelled=InterruptedError)
        raise errors[result['error_kind']](result['message'])
    return result['value']


def install_format():
    return _runtime('install_format', payload=str(Path(__file__).resolve().parent))


@dataclass(frozen=True)
class Release:
    tag: str
    url: str
    asset_url: str = ''
    digest: str = ''
    size: int = 0


def choose_release(releases, current, preview, package_format, machine):
    from .rust_core import call, CoreError
    try:
        result = call('update_select', releases=releases, current=current,
                      preview=preview, format=package_format, machine=machine)
    except CoreError as error:
        raise ValueError('Invalid release response') from error
    return Release(**result) if result is not None else None


def check(current, preview, package_format):
    result = _runtime('check', current=current, preview=preview, format=package_format)
    return Release(**result) if result is not None else None


def download(release, destination, cancelled=None):
    return Path(_runtime('download', cancelled, release=release, destination=str(destination)))


def verify_download(path, release):
    _runtime('verify', path=str(path), release=release)
