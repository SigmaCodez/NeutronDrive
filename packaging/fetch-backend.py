#!/usr/bin/env python3
"""Fetch exactly the pinned SDK-based bridge; never execute unverified downloads."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import tempfile
import urllib.request

LOCK = Path(__file__).with_name("backend-lock.json")


def architecture(value=None):
    value = value or platform.machine()
    aliases = {"amd64": "x86_64", "x64": "x86_64", "arm64": "aarch64"}
    value = aliases.get(value.lower(), value.lower())
    if value not in ("x86_64", "aarch64"):
        raise ValueError(f"Unsupported architecture: {value}; supported: x86_64, aarch64")
    return value


def verify(path, expected):
    digest = hashlib.sha512()
    with Path(path).open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    if digest.hexdigest() != expected:
        raise ValueError(f"SHA-512 mismatch for {path}; refusing to package the backend")


def verify_source_build(path, arch):
    """Accept explicit developer builds only when provenance matches this checkout."""
    path = Path(path)
    bridge = LOCK.parent / "bridge"
    lock = json.loads((bridge / "source-lock.json").read_text())
    report = json.loads(path.with_name(path.name + ".provenance.json").read_text())
    for key, value in lock.items():
        if report.get(key) != value:
            raise ValueError("Backend source provenance does not match the pinned SDK")
    expected = {"x86_64": "bun-linux-x64-baseline", "aarch64": "bun-linux-arm64"}[architecture(arch)]
    hashes = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in bridge.glob("*.ts")}
    if report.get("target") != expected or report.get("bridge_files") != hashes:
        raise ValueError("Backend build target or bridge sources do not match")
    builder = LOCK.parent.parent / "scripts/build-backend"
    if report.get("builder_sha256") != hashlib.sha256(builder.read_bytes()).hexdigest():
        raise ValueError("Backend build script provenance does not match")
    for filename, key in [("account-status.patch", "account_patch_sha256"), ("version-branding.patch", "version_patch_sha256")]:
        if report.get(key) != hashlib.sha256((bridge / filename).read_bytes()).hexdigest():
            raise ValueError("Backend SDK patch provenance does not match")
    verify(path, report["sha512"])


def fetch(destination, arch=None):
    entry = json.loads(LOCK.read_text())["architectures"][architecture(arch)]
    destination = Path(destination)
    if destination.exists():
        verify(destination, entry["sha512"])
        destination.chmod(0o755)
        return destination
    destination.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix="proton-drive-", dir=destination.parent)
    try:
        request = urllib.request.Request(entry["url"], headers={"User-Agent": "NeutronDrive-build/0.2"})
        with os.fdopen(fd, "wb") as target, urllib.request.urlopen(request, timeout=120) as response:
            for chunk in iter(lambda: response.read(1024 * 1024), b""):
                target.write(chunk)
        verify(temporary, entry["sha512"])
        os.chmod(temporary, 0o755)
        os.replace(temporary, destination)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)
    return destination


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--arch", default=None)
    args = parser.parse_args()
    print(fetch(args.destination, args.arch))
