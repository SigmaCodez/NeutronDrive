"""Build-only, checksum-pinned Qt generators for the Linux baseline."""
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
REPORT = 'qt-tools-build.json'


def source_lock():
    lock = json.loads((ROOT/'packaging/qt-sources-lock.json').read_text())
    return lock['version'], next(item for item in lock['sources'] if item['name'] == 'qtbase')


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def source_archive(path=None):
    _, entry = source_lock()
    path = Path(path) if path else ROOT/'build/qt-sources'/entry['filename']
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as stream:
            temporary = Path(stream.name)
            try:
                with urllib.request.urlopen(entry['url'], timeout=120) as response:
                    shutil.copyfileobj(response, stream)
                stream.close()
                if sha(temporary) != entry['sha256']:
                    raise ValueError('Qt source archive checksum mismatch')
                os.replace(temporary, path)
            finally:
                temporary.unlink(missing_ok=True)
    if sha(path) != entry['sha256']:
        raise ValueError('Qt source archive checksum mismatch')
    return path.resolve()


def inspect(binary):
    output = subprocess.check_output(['readelf', '--version-info', str(binary)], text=True)
    maximum = max((tuple(map(int, v.split('.'))) for v in re.findall(r'GLIBC_([0-9.]+)', output)), default=(0,))
    baseline = (2, 35) if platform.machine() == 'x86_64' else (2, 39)
    if maximum > baseline:
        raise ValueError('Qt generator exceeds the supported Linux baseline')
    dynamic = subprocess.check_output(['readelf', '-d', str(binary)], text=True)
    if re.search(r'\(NEEDED\).*\[libQt', dynamic):
        raise ValueError('Qt generators must link Qt statically')
    return {'sha256': sha(binary), 'maximum_glibc': list(maximum)}


def record(qmake, moc, output):
    version, entry = source_lock()
    facts = {name: inspect(path) for name, path in [('qmake', qmake), ('moc', moc)]}
    actual = subprocess.check_output([str(qmake), '-query', 'QT_VERSION'], text=True).strip()
    if actual != version:
        raise ValueError('Qt generator version mismatch')
    output = Path(output)
    output.mkdir(parents=True, exist_ok=False)
    for name, path in [('qmake', qmake), ('moc', moc)]:
        shutil.copyfile(path, output/name)
        (output/name).chmod(0o755)
    report = dict(qt_version=version, architecture=platform.machine(), source_archive_sha256=entry['sha256'], tools=facts)
    (output/REPORT).write_text(json.dumps(report, indent=2, sort_keys=True)+'\n')
    verify(output)
    return output


def verify(directory):
    directory = Path(directory)
    report = json.loads((directory/REPORT).read_text())
    version, entry = source_lock()
    if (report['qt_version'], report['architecture'], report['source_archive_sha256']) != (version, platform.machine(), entry['sha256']):
        raise ValueError('Qt generator provenance mismatch')
    for name in ('qmake', 'moc'):
        if inspect(directory/name) != report['tools'][name]:
            raise ValueError('Qt generator binary mismatch')
    return directory


def build(work, output, archive=None, jobs=4):
    archive = source_archive(archive)
    work = Path(work).resolve()
    work.mkdir(parents=True, exist_ok=False)
    source, target = work/'source', work/'build'
    source.mkdir()
    subprocess.run(['tar', '-xJf', str(archive), '-C', str(source), '--strip-components=1'], check=True)
    subprocess.run(['cmake', '-S', str(source), '-B', str(target), '-G', 'Ninja',
                    '-DCMAKE_BUILD_TYPE=Release', '-DBUILD_SHARED_LIBS=OFF',
                    '-DQT_BUILD_TESTS=OFF', '-DQT_BUILD_EXAMPLES=OFF', '-DQT_BUILD_TOOLS_BY_DEFAULT=OFF',
                    '-DQT_FEATURE_gui=OFF', '-DQT_FEATURE_network=OFF', '-DQT_FEATURE_dbus=OFF',
                    '-DQT_FEATURE_sql=OFF', '-DQT_FEATURE_testlib=OFF', '-DQT_FEATURE_icu=OFF'], check=True)
    subprocess.run(['cmake', '--build', str(target), '--target', 'moc', 'qmake', '--parallel', str(jobs)], check=True)
    return record(target/'bin/qmake', target/'libexec/moc', output)
