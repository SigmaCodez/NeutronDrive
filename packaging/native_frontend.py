"""Build-time provenance checks for the Rust/Qt executable; no Python is staged."""
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
BINARY = 'neutrondrive-qt'
REPORT = 'native-build.json'
FILE_MANAGER_ARTIFACTS = ('libneutrondrive_file_manager.so', 'libneutrondrive_nautilus3.so', 'neutrondrive-file-manager')


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def arch(value=None):
    return {'amd64':'x86_64', 'arm64':'aarch64', 'x86_64':'x86_64', 'aarch64':'aarch64'}[value or platform.machine()]


def sources():
    files = [ROOT / name for name in (
        'file-manager/Cargo.toml', 'file-manager/Cargo.lock', 'desktop/Cargo.toml', 'desktop/Cargo.lock', 'desktop/build.rs', 'desktop/ui.qrc',
        'core/Cargo.toml', 'core/Cargo.lock', 'packaging/qt-lock.json',
        'data/io.github.SigmaCodez.NeutronDrive.svg',
        'data/neutrondrive-available.svg',
    )]
    for relative, pattern in [('desktop/src', '*.rs'), ('desktop/include', '*.h'),
                              ('desktop/qml', '*.qml'), ('desktop/data', '*'),
                              ('file-manager/src', '*.rs'), ('file-manager/kde', '*'), ('core/src', '*.rs'), ('src/neutrondrive/qml', '*.qml'),
                              ('src/neutrondrive/locales', '*.json')]:
        files.extend(p for p in (ROOT / relative).rglob(pattern) if p.is_file())
    return {str(p.relative_to(ROOT)): sha(p) for p in sorted(set(files))}


def inspect(binary, architecture):
    with Path(binary).open('rb') as stream:
        header = stream.read(20)
    if len(header) != 20 or header[:6] != b'\x7fELF\x02\x01' or struct.unpack_from('<H', header, 18)[0] != {'x86_64':62, 'aarch64':183}[arch(architecture)]:
        raise ValueError('Native frontend ELF architecture mismatch')
    symbols = subprocess.check_output(['readelf', '--version-info', str(binary)], text=True)
    if 'GLIBC_ABI_' in symbols:
        raise ValueError('Native frontend uses a newer named GLIBC ABI than the supported baseline')
    maximum = lambda prefix: max((tuple(map(int, v.split('.'))) for v in re.findall(prefix + r'([0-9]+(?:\.[0-9]+)+)', symbols)), default=(0, 0))
    glibc, glibcxx = maximum('GLIBC_'), maximum('GLIBCXX_')
    # The pinned ARM64 Qt runtime is manylinux_2_39; its documented baseline is
    # Ubuntu 24.04. Keep x86_64 at Ubuntu 22.04 independently.
    baseline = ((2, 35), (3, 4, 30)) if arch(architecture) == 'x86_64' else ((2, 39), (3, 4, 32))
    if glibc > baseline[0] or glibcxx > baseline[1]:
        raise ValueError(f'Native frontend exceeds {arch(architecture)} baseline: GLIBC {glibc}, GLIBCXX {glibcxx}')
    dynamic = subprocess.check_output(['readelf', '-d', str(binary)], text=True)
    needed = re.findall(r'\(NEEDED\).*?\[(.*?)\]', dynamic)
    if any(name.startswith(('libpython', 'libpyside', 'libshiboken', 'libneutrondrive_core')) for name in needed):
        raise ValueError('Native frontend unexpectedly requires Python bindings or a separate core library')
    runpaths = re.findall(r'\((?:RUNPATH|RPATH)\).*?\[(.*?)\]', dynamic)
    if runpaths != ['$ORIGIN/../qt/lib']:
        raise ValueError('Native frontend requires the relocatable Qt library path for direct autostart')
    return dict(maximum_glibc=list(glibc), maximum_glibcxx=list(glibcxx), needed=needed, runpath=runpaths[0])


def inspect_file_manager(binary, architecture):
    with Path(binary).open('rb') as stream:
        header = stream.read(20)
    if len(header) != 20 or header[:6] != b'\x7fELF\x02\x01' or struct.unpack_from('<H', header, 18)[0] != {'x86_64':62, 'aarch64':183}[arch(architecture)]:
        raise ValueError('File-manager ELF architecture mismatch')
    versions = subprocess.check_output(['readelf', '--version-info', str(binary)], text=True)
    maximum = max((tuple(map(int, v.split('.'))) for v in re.findall(r'GLIBC_([0-9]+(?:\.[0-9]+)+)', versions)), default=(0,0))
    if 'GLIBC_ABI_' in versions or maximum > ((2,35) if arch(architecture) == 'x86_64' else (2,39)):
        raise ValueError('File-manager library exceeds supported glibc baseline')
    dynamic = subprocess.check_output(['readelf', '-d', str(binary)], text=True)
    if re.search(r'\[(?:libQt|libpython|libpyside|libshiboken)', dynamic):
        raise ValueError('File-manager extension must not load Qt or Python into the host')


def verify(directory, architecture):
    directory = Path(directory)
    binary = directory / BINARY
    report = json.loads((directory / REPORT).read_text())
    actual = inspect(binary, architecture)
    if (report.get('format') != 1 or report.get('architecture') != arch(architecture)
            or report.get('source_sha256') != sources() or report.get('sha256') != sha(binary)
            or any(report.get(k) != v for k, v in actual.items())):
        raise ValueError('Native frontend provenance does not match the source tree and binary')
    for name in FILE_MANAGER_ARTIFACTS:
        plugin = directory / name
        if report.get('file_manager_sha256', {}).get(name) != sha(plugin):
            raise ValueError('File-manager artifact provenance mismatch')
        inspect_file_manager(plugin, architecture)
    return binary


def record(binary, directory, architecture=None):
    """Publish only a verified baseline build; never overwrite an executable inode in place."""
    architecture = arch(architecture)
    inspected = inspect(binary, architecture)
    for name in FILE_MANAGER_ARTIFACTS:
        inspect_file_manager(Path(binary).parent/name, architecture)
    file_manager_sha256 = {name:sha(Path(binary).parent/name) for name in FILE_MANAGER_ARTIFACTS}
    report = dict(file_manager_sha256=file_manager_sha256, format=1, architecture=architecture, source_sha256=sources(), sha256=sha(binary), **inspected)
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=directory, delete=False) as stream:
        temporary = Path(stream.name)
    try:
        shutil.copyfile(binary, temporary)
        temporary.chmod(0o755)
        os.replace(temporary, directory / BINARY)
    finally:
        temporary.unlink(missing_ok=True)
    for name in FILE_MANAGER_ARTIFACTS:
        shutil.copyfile(Path(binary).parent/name, directory/name)
        (directory/name).chmod(0o755)
    (directory / REPORT).write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')
    verify(directory, architecture)
    return directory


def stage(app, base, architecture):
    configured = os.environ.get('NEUTRONDRIVE_NATIVE_BUILD')
    if not configured:
        raise RuntimeError('Set NEUTRONDRIVE_NATIVE_BUILD to a verified portable native frontend build')
    directory = Path(configured)
    binary = verify(directory, architecture)
    destination = app / 'bin' / BINARY
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(binary, destination)
    destination.chmod(0o755)
    (destination.parent / 'qt.conf').write_text('[Paths]\nPrefix=../qt\nLibraries=lib\nPlugins=plugins\nQmlImports=qml\nTranslations=translations\n')
    docs = base / 'share/doc/neutrondrive'
    docs.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(directory / REPORT, docs / REPORT)
    for name in FILE_MANAGER_ARTIFACTS:
        shutil.copyfile(directory/name, destination.parent/name)
        (destination.parent/name).chmod(0o755)
    for folder in ('core', 'desktop', 'file-manager'):
        shutil.copyfile(ROOT / folder / 'Cargo.lock', docs / (folder + '-Cargo.lock'))
    licenses = base / 'share/licenses/neutrondrive/rust'
    shutil.copytree(ROOT / 'packaging/licenses/rust', licenses, dirs_exist_ok=True)
    shutil.copyfile(ROOT / 'desktop/data/UNICODE-LICENSE.txt', licenses / 'UNICODE-LICENSE.txt')
    for crate in (ROOT / 'core/vendor').iterdir():
        if crate.is_dir():
            for notice in crate.glob('LICENSE*'):
                if notice.is_file():
                    target = licenses / crate.name / notice.name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(notice, target)
