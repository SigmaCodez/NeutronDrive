#!/usr/bin/env python3
"""Build the pinned pyfuse3 extension privately; never install into system Python."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request

LOCK = Path(__file__).with_name("pyfuse3-lock.json")


def build(payload, source=None):
    lock = json.loads(LOCK.read_text())
    payload = Path(payload).resolve()
    target = payload / "usr/lib/neutrondrive"
    target.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="neutrondrive-pyfuse3-") as temporary:
        temporary = Path(temporary)
        archive = temporary / lock["filename"]
        if source:
            shutil.copyfile(source, archive)
        else:
            with urllib.request.urlopen(lock["url"], timeout=90) as response, archive.open("wb") as output:
                shutil.copyfileobj(response, output)
        if hashlib.sha256(archive.read_bytes()).hexdigest() != lock["sha256"]:
            raise ValueError("pyfuse3 source SHA-256 mismatch; refusing to build")
        # The builder uses only distribution-supplied build tools. No implicit
        # Python dependency resolution or download is permitted during build.
        wheel_dir = temporary / "wheels"
        wheel_dir.mkdir()
        subprocess.run([sys.executable, "-m", "pip", "wheel", "--no-deps", "--no-build-isolation",
                        "--no-index", "--wheel-dir", str(wheel_dir), str(archive)], check=True)
        wheels = list(wheel_dir.glob("pyfuse3-*.whl"))
        if len(wheels) != 1:
            raise RuntimeError("Expected exactly one locally built pyfuse3 wheel")
        subprocess.run([sys.executable, "-m", "pip", "install", "--no-deps", "--no-index",
                        "--no-compile", "--target", str(target), str(wheels[0])], check=True)
        subprocess.run([sys.executable, "-c",
                        "import sys; sys.path.insert(0, sys.argv[1]); import pyfuse3, trio; "
                        "print('Compiled pyfuse3:', pyfuse3.__file__)", str(target)], check=True)
        license_dir = payload / "usr/share/licenses/neutrondrive"
        license_dir.mkdir(parents=True, exist_ok=True)
        with tarfile.open(archive) as package:
            license_data = package.extractfile(f"pyfuse3-{lock['version']}/LICENSE").read()
        (license_dir / "pyfuse3-LGPL-2.1.txt").write_bytes(license_data)
        source_dir = payload / "usr/share/doc/neutrondrive/pyfuse3-source"
        source_dir.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(archive, source_dir / archive.name)
        shutil.copyfile(LOCK, source_dir / LOCK.name)
        shutil.copyfile(__file__, source_dir / "vendor-pyfuse3.py")
        (source_dir / "BUILD.txt").write_text(
            "Unmodified pyfuse3 source, compiled against this RPM's Python ABI.\n"
            "Source and build instructions are provided under LGPL-2.1-or-later.\n"
            "Build dependencies (Fedora): gcc pkgconf-pkg-config fuse3-devel "
            "python3-devel python3-pip python3-setuptools python3-setuptools_scm "
            "python3-cython python3-wheel python3-trio.\n"
            "Rebuild into an empty staging directory (does not modify system Python):\n"
            f"python3 vendor-pyfuse3.py --payload ./replacement --source {archive.name}\n"
            "The replacement/usr/lib/neutrondrive/pyfuse3 extension can replace the bundled "
            "module, provided it uses the same Python ABI and compatible pyfuse3 API.\n"
        )
    return lock["version"]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--payload", type=Path, required=True)
    parser.add_argument("--source", type=Path, help="Offline source archive; SHA-256 is still verified")
    args = parser.parse_args()
    build(args.payload, args.source)
