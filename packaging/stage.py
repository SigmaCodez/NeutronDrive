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
qt_spec = importlib.util.spec_from_file_location("qt_runtime", ROOT / "packaging/qt_runtime.py")
qt_runtime = importlib.util.module_from_spec(qt_spec)
qt_spec.loader.exec_module(qt_runtime)
native_spec = importlib.util.spec_from_file_location("native_frontend", ROOT / "packaging/native_frontend.py")
native_frontend = importlib.util.module_from_spec(native_spec)
native_spec.loader.exec_module(native_frontend)

def stage(destination, prefix, arch, binary, portal_browser=False, host_services=True, native=True):
    if not native:
        raise ValueError("Python runtime staging has been removed; use the verified Rust frontend")
    destination = Path(destination)
    prefix = prefix.strip("/")
    if ".." in Path(prefix).parts:
        raise ValueError("Prefix must not contain '..'")
    base = destination / prefix
    app = base / "lib/neutrondrive"
    if app.exists():
        raise ValueError("Native staging requires a fresh destination to exclude old Python payloads")
    source = ROOT / "src/neutrondrive"
    if not (source / "locales/en.json").is_file():
        raise ValueError("Native application English fallback catalog is missing")
    entry = json.loads(backend.LOCK.read_text())["architectures"][backend.architecture(arch)]
    binary = Path(binary) if binary else ROOT / "build/backend/proton-drive"
    if not binary.is_file():
        raise ValueError("Build the current pinned SDK with scripts/build-backend or pass --backend")
    # Keep source provenance even after this exact SDK binary becomes a pinned
    # release asset. Arch repackaging verifies this report independently.
    source_build = binary.with_name(binary.name + ".provenance.json").is_file()
    if source_build:
        backend.verify_source_build(binary, arch)
    else:
        backend.verify(binary, entry["sha512"])
    native_frontend.stage(app, base, arch)
    # Retain the installed payload identity used by update-format detection.
    shutil.copytree(source / "locales", app / "neutrondrive/locales")
    qt_runtime.stage(app, base, arch, native=True)
    (app / "bin").mkdir(parents=True, exist_ok=True)
    shutil.copyfile(binary, app / "bin/proton-drive")
    (app / "bin/proton-drive").chmod(0o755)
    if portal_browser:
        shutil.copyfile(ROOT / "packaging/xdg-open-portal-native", app / "bin/xdg-open")
        (app / "bin/xdg-open").chmod(0o755)
    (base / "bin").mkdir(parents=True, exist_ok=True)
    (base / "bin/neutrondrive").write_text((ROOT / "packaging/native-launcher").read_text())
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
    if source_build:
        shutil.copyfile(binary.with_name(binary.name + ".provenance.json"),
                        base / "share/doc/neutrondrive/backend-build.provenance.json")
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
    parser.add_argument("--native", action="store_true", default=True, help="Stage the Rust frontend (always enabled)")
    args = parser.parse_args()
    stage(args.destdir, args.prefix, args.arch, args.backend, args.portal_browser, not args.no_host_services, args.native)
