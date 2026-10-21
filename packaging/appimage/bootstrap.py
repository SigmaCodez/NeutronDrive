"""Keep the portable runtime at a stable private path for background/autostart.

A mounted AppImage disappears when its foreground process exits. Materializing a
content-addressed runtime also keeps persisted backend and autostart paths valid.
No network access, package installation or account operation occurs here.
"""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import sys
import tempfile


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def private_directory(path):
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise RuntimeError('AppImage runtime cache must be a private directory owned by you: ' + str(path))


def valid(root, manifest):
    for name, expected in manifest.items():
        relative = Path(name)
        if relative.is_absolute() or '..' in relative.parts:
            raise ValueError('Unsafe runtime manifest path')
        path = root / relative
        if expected.startswith('symlink:'):
            if not path.is_symlink() or os.readlink(path) != expected[8:]:
                return False
        elif path.is_symlink() or not path.is_file() or digest(path) != expected:
            return False
    return True


def prepare(source, cache):
    raw = (source / 'runtime-manifest.json').read_bytes()
    manifest = json.loads(raw)
    key = hashlib.sha256(raw).hexdigest()
    private_directory(cache)
    target = cache / key
    lock = os.open(cache / '.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        info = os.fstat(lock)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_nlink != 1:
            raise RuntimeError('Unsafe runtime cache lock')
        fcntl.flock(lock, fcntl.LOCK_EX)
        if target.is_symlink():
            raise RuntimeError('Unsafe runtime cache link')
        if target.exists():
            private_directory(target)
            if not valid(target, manifest):
                raise RuntimeError('AppImage runtime cache is damaged. Stop NeutronDrive, remove ' + str(target) + ' and launch the AppImage again.')
            return target
        temporary = Path(tempfile.mkdtemp(prefix='.stage-', dir=cache))
        try:
            shutil.copytree(source, temporary, symlinks=True, dirs_exist_ok=True)
            temporary.chmod(0o700)
            if not valid(temporary, manifest):
                raise RuntimeError('AppImage runtime verification failed')
            temporary.rename(target)
        finally:
            if temporary.exists():
                shutil.rmtree(temporary)
        return target
    finally:
        os.close(lock)


def main():
    source = Path(__file__).resolve().parents[3]
    cache = Path(os.environ.get('XDG_CACHE_HOME', str(Path.home() / '.cache'))) / 'neutrondrive-appimage'
    target = prepare(source, cache)
    if target != source:
        os.execv(str(target / 'AppRun'), [str(target / 'AppRun'), *sys.argv[1:]])
    os.execv(sys.executable, [sys.executable, '-m', 'neutrondrive', *sys.argv[1:]])


if __name__ == '__main__':
    main()
