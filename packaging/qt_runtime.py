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
MODULES |= {"LabsFolderListModel","Qml", "QmlMeta", "QmlCore", "QmlModels", "QmlWorkerScript", "QmlNetwork", "Quick", "QuickControls2", "QuickControls2Impl", "QuickControls2Basic", "QuickControls2BasicStyleImpl", "QuickControls2Fusion", "QuickControls2FusionStyleImpl", "QuickTemplates2", "QuickLayouts", "QuickDialogs2", "QuickDialogs2QuickImpl", "QuickDialogs2Utils", "QuickShapes"}
QML_DIRS = {"Qt/labs/folderlistmodel","QtQml", "QtQml/Models", "QtQml/WorkerScript", "QtQuick", "QtQuick/Window", "QtQuick/Layouts", "QtQuick/Templates", "QtQuick/Controls", "QtQuick/Controls/Basic", "QtQuick/Controls/Basic/impl", "QtQuick/Controls/Fusion", "QtQuick/Controls/Fusion/impl", "QtQuick/Controls/impl", "QtQuick/Dialogs", "QtQuick/Dialogs/quickimpl", "QtQuick/Dialogs/quickimpl/qml", "QtQuick/Shapes"}
PLUGIN_GROUPS = {"iconengines", "imageformats", "networkinformation", "platforminputcontexts", "platformthemes", "tls", "wayland-decoration-client", "wayland-graphics-integration-client", "wayland-shell-integration", "xcbglintegrations"}


def runtime_member(name):
    parts = PurePosixPath(name).parts
    if "__pycache__" in parts or name.endswith(".pyc"): return False
    if not parts or parts[0] != "PySide6": return True
    if len(parts) < 2: return False
    if parts[1] == "Qt":
        if len(parts) < 4: return False
        category, filename = parts[2], parts[-1]
        if category == "lib":
            return filename.startswith("libicu") or any(filename == "libQt6" + module + ".so.6" for module in MODULES)
        if category == "qml": return "/".join(parts[3:-1]) in QML_DIRS
        if category == "translations": return filename.startswith(("qtbase_", "qtdeclarative_"))
        if category == "plugins":
            group = parts[3]
            if group == "imageformats": return filename in {"libqgif.so", "libqico.so", "libqjpeg.so", "libqsvg.so"}
            if group == "platforms": return filename in {"libqxcb.so", "libqwayland.so", "libqoffscreen.so", "libqminimal.so"}
            return group in PLUGIN_GROUPS and filename not in {"libqgtk3.so", "libqtvirtualkeyboardplugin.so"}
        return False
    if parts[1] in {"support", "lib"}: return True
    filename = parts[1]
    return (filename.startswith(("libpyside6.abi3", "libpyside6qml.abi3")) or filename in {"__init__.py", "_config.py", "_git_pyside_version.py", "py.typed"}
            or any(filename in {"Qt" + module + ".abi3.so", "Qt" + module + ".pyi"} for module in MODULES))


BUILD_TOOLS = {"qmlcachegen", "qmltyperegistrar", "qmlimportscanner", "rcc", "uic"}
BUILD_LIBRARIES = {"libQt6QmlCompiler.so.6"}


def extract(paths,destination, *, native=False, build_tools=False):
    if native and build_tools:
        raise ValueError("Build tools must not be included in the installed native runtime")
    destination=Path(destination);destination.mkdir(parents=True,exist_ok=True)
    for path in paths:
        with zipfile.ZipFile(path) as archive:
            for info in archive.infolist():
                name=PurePosixPath(info.filename)
                if name.is_absolute() or '..' in name.parts or (info.external_attr >> 16) & 0o170000 == 0o120000:
                    raise ValueError('Unsafe Qt wheel member')
                if native and (name.parts[:2] != ('PySide6', 'Qt') or name.suffix in {'.py', '.pyi', '.pyc'}):
                    continue
                is_build_tool = name.parts[:3] == ("PySide6", "Qt", "libexec") and len(name.parts) == 4 and name.name in BUILD_TOOLS
                is_build_library = name.parts[:3] == ("PySide6", "Qt", "lib") and len(name.parts) == 4 and name.name in BUILD_LIBRARIES
                if not info.is_dir() and (runtime_member(info.filename) or (build_tools and (is_build_tool or is_build_library))):
                    relative = name.parts[2:] if native else name.parts
                    target=destination.joinpath(*relative);target.parent.mkdir(parents=True,exist_ok=True)
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


def stage(app,base,architecture, *, native=False):
    extract(fetch(architecture),app / 'qt', native=native)
    docs=base / 'share/doc/neutrondrive';docs.mkdir(parents=True,exist_ok=True)
    shutil.copyfile(LOCK,docs / 'qt-lock.json')
    shutil.copyfile(ROOT / 'packaging/qt-sources-lock.json', docs / 'qt-sources-lock.json')
    # Upstream license collection also contains a license-wizard example and
    # license-checking tool. They are not notices or part of the native runtime.
    # Do not turn those unused programs into a package-level Python dependency.
    ignored = shutil.ignore_patterns('*.py', '*.pyc', '*.pyi', '*.pyproject') if native else None
    shutil.copytree(ROOT / 'packaging/licenses/qt', base / 'share/licenses/neutrondrive/qt',
                    dirs_exist_ok=True, ignore=ignored)
    notice = 'QT-NATIVE-NOTICES.md' if native else 'QT-NOTICES.md'
    shutil.copyfile(ROOT / 'packaging' / notice, docs / 'QT-NOTICES.md')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--arch');parser.add_argument('--dest',type=Path)
    parser.add_argument('--build-tools', action='store_true', help='Include pinned Qt generators for the development SDK only')
    args=parser.parse_args();paths=fetch(args.arch)
    if args.dest:print(extract(paths,args.dest,build_tools=args.build_tools))
    else:
        for path in paths:print(path)
