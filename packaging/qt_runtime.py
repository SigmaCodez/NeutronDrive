"""Fetch verified Qt for Python wheels into a private runtime; never install at startup."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import platform
import re
import subprocess
import shutil
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
LOCK = ROOT / 'packaging/qt-lock.json'


def arch(value=None):
    return {'amd64':'x86_64','arm64':'aarch64','x86_64':'x86_64','aarch64':'aarch64'}[value or platform.machine()]


def fetch(architecture=None):
    architecture=arch(architecture)
    lock=json.loads(LOCK.read_text())
    cache=ROOT / 'build/qt-wheels' / architecture
    cache.mkdir(parents=True,exist_ok=True)
    paths=[]
    for entry in lock['architectures'][architecture]:
        path=cache / entry['filename']
        if not path.exists():
            temporary=path.with_suffix('.download')
            try:
                with urllib.request.urlopen(entry['url'],timeout=120) as response, temporary.open('wb') as out:
                    shutil.copyfileobj(response,out)
                if hashlib.sha256(temporary.read_bytes()).hexdigest()!=entry['sha256']:
                    raise ValueError('Qt wheel checksum mismatch')
                temporary.replace(path)
            finally:temporary.unlink(missing_ok=True)
        if hashlib.sha256(path.read_bytes()).hexdigest()!=entry['sha256']:
            raise ValueError('Qt wheel checksum mismatch')
        paths.append(path)
    return paths


MODULES = {"Core", "Gui", "Widgets", "Network", "DBus", "OpenGL", "OpenGLWidgets", "Svg", "SvgWidgets", "Xml", "Test", "WaylandClient", "WlShellIntegration", "XcbQpa"}
PLUGIN_GROUPS = {"iconengines", "imageformats", "networkinformation", "platforminputcontexts", "platformthemes", "tls", "wayland-decoration-client", "wayland-graphics-integration-client", "wayland-shell-integration", "xcbglintegrations"}


def runtime_member(name):
    parts = PurePosixPath(name).parts
    if not parts or parts[0] != "PySide6": return True
    if len(parts) < 2: return False
    if parts[1] == "Qt":
        if len(parts) < 4: return False
        category, filename = parts[2], parts[-1]
        if category == "lib":
            return filename.startswith("libicu") or any(filename == "libQt6" + module + ".so.6" for module in MODULES)
        if category == "translations": return filename.startswith("qtbase_")
        if category == "plugins":
            group = parts[3]
            if group == "imageformats": return filename in {"libqgif.so", "libqico.so", "libqjpeg.so", "libqsvg.so"}
            if group == "platforms": return filename in {"libqxcb.so", "libqwayland.so", "libqoffscreen.so", "libqminimal.so"}
            return group in PLUGIN_GROUPS and filename not in {"libqgtk3.so", "libqtvirtualkeyboardplugin.so"}
        return False
    if parts[1] in {"support", "lib"}: return True
    filename = parts[1]
    return (filename.startswith("libpyside6.abi3") or filename in {"__init__.py", "_config.py", "_git_pyside_version.py", "py.typed"}
            or any(filename in {"Qt" + module + ".abi3.so", "Qt" + module + ".pyi"} for module in MODULES))


def extract(paths,destination):
    destination=Path(destination);destination.mkdir(parents=True,exist_ok=True)
    for path in paths:
        with zipfile.ZipFile(path) as archive:
            for info in archive.infolist():
                name=PurePosixPath(info.filename)
                if name.is_absolute() or '..' in name.parts or (info.external_attr >> 16) & 0o170000 == 0o120000:
                    raise ValueError('Unsafe Qt wheel member')
                if not info.is_dir() and runtime_member(info.filename):
                    target=destination.joinpath(*name.parts);target.parent.mkdir(parents=True,exist_ok=True)
                    with archive.open(info) as source,target.open('wb') as out:shutil.copyfileobj(source,out)
                    target.chmod(0o755 if (info.external_attr >> 16) & 0o111 else 0o644)
    # Fail at build time if a selected plugin requires an omitted Qt module.
    libraries = {p.name for p in destination.rglob("*.so*") if p.is_file()}
    for binary in destination.rglob("*.so*"):
        if not binary.is_file(): continue
        with binary.open("rb") as stream:
            if stream.read(4) != b"\x7fELF": continue
        dynamic = subprocess.check_output(["readelf", "-d", str(binary)], text=True)
        for dependency in re.findall(r"\(NEEDED\).*?\[(.*?)\]", dynamic):
            if dependency.startswith("libQt6") and dependency not in libraries:
                raise ValueError(f"Qt plugin {binary.name} requires omitted module {dependency}")
    return destination


def stage(app,base,architecture):
    extract(fetch(architecture),app / 'qt')
    docs=base / 'share/doc/neutrondrive';docs.mkdir(parents=True,exist_ok=True)
    shutil.copyfile(LOCK,docs / 'qt-lock.json')
    shutil.copyfile(ROOT / 'packaging/qt-sources-lock.json', docs / 'qt-sources-lock.json')
    shutil.copytree(ROOT / 'packaging/licenses/qt', base / 'share/licenses/neutrondrive/qt', dirs_exist_ok=True)
    shutil.copyfile(ROOT / 'packaging/QT-NOTICES.md',docs / 'QT-NOTICES.md')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--arch');parser.add_argument('--dest',type=Path)
    args=parser.parse_args();paths=fetch(args.arch)
    if args.dest:print(extract(paths,args.dest))
    else:
        for path in paths:print(path)
