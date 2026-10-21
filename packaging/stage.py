#!/usr/bin/env python3
"""Stage identical runtime contents for native and sandboxed Linux packages."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parent.parent
APP_ID = "io.github.SigmaCodez.NeutronDrive"
spec = importlib.util.spec_from_file_location("fetch_backend", ROOT / "packaging/fetch-backend.py")
backend = importlib.util.module_from_spec(spec)
spec.loader.exec_module(backend)
rust_spec = importlib.util.spec_from_file_location("rust_build", ROOT / "packaging/rust_core.py")
rust_build = importlib.util.module_from_spec(rust_spec)
rust_spec.loader.exec_module(rust_build)
qt_spec = importlib.util.spec_from_file_location("qt_runtime", ROOT / "packaging/qt_runtime.py")
qt_runtime = importlib.util.module_from_spec(qt_spec)
qt_spec.loader.exec_module(qt_runtime)

LAUNCHER = '''#!/usr/bin/env python3
import os
from pathlib import Path
import runpy
import sys

prefix = Path(__file__).resolve().parent.parent
app = prefix / "lib" / "neutrondrive"
sys.path.insert(0, str(app))
os.environ["PATH"] = str(app / "bin") + os.pathsep + os.environ.get("PATH", "")
runpy.run_module("neutrondrive", run_name="__main__")
'''


def stage(destination, prefix, arch, binary, portal_browser=False, host_services=True):
    destination = Path(destination)
    prefix = prefix.strip("/")
    if ".." in Path(prefix).parts:
        raise ValueError("Prefix must not contain '..'")
    base = destination / prefix
    app = base / "lib/neutrondrive"
    source = ROOT / "src/neutrondrive"
    if not (source / "__main__.py").is_file():
        raise ValueError(f"Application entry point missing: {source / '__main__.py'}")
    entry = json.loads(backend.LOCK.read_text())["architectures"][backend.architecture(arch)]
    binary = Path(binary) if binary else backend.fetch(ROOT / "build/backend" / backend.architecture(arch) / "proton-drive", arch)
    backend.verify(binary, entry["sha512"])
    shutil.copytree(source, app / "neutrondrive", dirs_exist_ok=True, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
    rust_build.stage(app, base, arch)
    qt_runtime.stage(app, base, arch)
    (app / "bin").mkdir(parents=True, exist_ok=True)
    shutil.copyfile(binary, app / "bin/proton-drive")
    (app / "bin/proton-drive").chmod(0o755)
    if portal_browser:
        shutil.copyfile(ROOT / "packaging/xdg-open-portal", app / "bin/xdg-open")
        (app / "bin/xdg-open").chmod(0o755)
    (base / "bin").mkdir(parents=True, exist_ok=True)
    (base / "bin/neutrondrive").write_text(LAUNCHER)
    (base / "bin/neutrondrive").chmod(0o755)
    for src, relative in [
        (ROOT / "data" / f"{APP_ID}.desktop", f"share/applications/{APP_ID}.desktop"),
        (ROOT / "data" / f"{APP_ID}.metainfo.xml", f"share/metainfo/{APP_ID}.metainfo.xml"),
        (ROOT / "data" / f"{APP_ID}.svg", f"share/icons/hicolor/scalable/apps/{APP_ID}.svg"),
        (ROOT / "LICENSE", "share/licenses/neutrondrive/LICENSE"),
        (ROOT / "packaging/licenses/proton-drive-MIT.txt", "share/licenses/neutrondrive/proton-drive-MIT.txt"),
        (ROOT / "packaging/backend-lock.json", "share/doc/neutrondrive/backend-lock.json"),
        (ROOT / "packaging/bridge/source-lock.json", "share/doc/neutrondrive/bridge-source-lock.json"),
    ]:
        target = base / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src, target)
        target.chmod(0o644)
    if prefix == "usr" and host_services:
        for name, relative in [
            ("io.github.SigmaCodez.NeutronDrive.Service.service", "share/dbus-1/services/io.github.SigmaCodez.NeutronDrive.Service.service"),
            ("neutrondrive-background.service", "lib/systemd/user/neutrondrive-background.service"),
        ]:
            target = base / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / "data" / name, target)
            target.chmod(0o644)
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "1792454400"))
    for path in sorted(base.rglob("*"), reverse=True):
        os.utime(path, (epoch, epoch), follow_symlinks=False)
    os.utime(base, (epoch, epoch))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destdir", type=Path, required=True)
    parser.add_argument("--prefix", default="/usr")
    parser.add_argument("--arch", default=None)
    parser.add_argument("--backend", type=Path, help="Existing binary; SHA-512 is always checked")
    parser.add_argument("--portal-browser", action="store_true", help="Use the desktop portal to open the sign-in URL")
    parser.add_argument("--no-host-services", action="store_true", help="Omit native host D-Bus/systemd activation files (for Snap)")
    args = parser.parse_args()
    stage(args.destdir, args.prefix, args.arch, args.backend, args.portal_browser, not args.no_host_services)
