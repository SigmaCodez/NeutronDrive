#!/usr/bin/env python3
"""Fetch exactly the pinned upstream CLI; never execute unverified downloads."""

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
        request = urllib.request.Request(entry["url"], headers={"User-Agent": "NeutronDrive-build/0.1"})
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
