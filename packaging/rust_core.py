"""Offline native Rust builds and verification of prebuilt core payloads."""
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess

ROOT = Path(__file__).resolve().parent.parent
LIBRARY = "libneutrondrive_core.so"


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def sources():
    files = [ROOT / "core/Cargo.toml", ROOT / "core/Cargo.lock", *sorted((ROOT / "core/src").rglob("*.rs"))]
    return {str(path.relative_to(ROOT)): sha(path) for path in files}


def arch(value=None):
    value = value or platform.machine()
    return {"amd64": "x86_64", "arm64": "aarch64", "x86_64": "x86_64", "aarch64": "aarch64"}[value]


def verify(directory, architecture):
    directory = Path(directory)
    report = json.loads((directory / "core-build.json").read_text())
    binary = directory / LIBRARY
    header = binary.read_bytes()[:20]
    if (header[:6] != b"\x7fELF\x02\x01" or struct.unpack_from("<H", header, 18)[0] != {"x86_64":62,"aarch64":183}[arch(architecture)]):
        raise ValueError("Rust core ELF architecture mismatch")
    if (report["architecture"] != arch(architecture) or report["source_sha256"] != sources()
            or report["sha256"] != sha(binary) or report["abi"] != 1):
        raise ValueError("Rust core provenance/hash does not match this source tree")
    if tuple(report["maximum_glibc"]) > (2, 35):
        raise ValueError("Rust core requires a newer libc than the supported Ubuntu 22.04 baseline")
    return binary


def build(output=None):
    output = Path(output or ROOT / "build/rust").resolve()
    cargo = shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo")
    if not Path(cargo).is_file():
        raise RuntimeError("Rust/Cargo is required to build NeutronDrive; install the pinned toolchain first")
    environment = dict(os.environ)
    environment.setdefault("CARGO_HOME", str(ROOT / "build/cargo-home"))
    environment["CARGO_TARGET_DIR"] = str(ROOT / "build/rust-target")
    subprocess.run([cargo,"build","--release","--locked","--offline"],cwd=ROOT / "core",env=environment,check=True)
    binary = ROOT / "build/rust-target/release" / LIBRARY
    # These symbols are checked rather than assuming the build host's libc
    # version equals the resulting runtime requirement.
    symbols = subprocess.check_output(["readelf","--version-info",str(binary)],text=True)
    maximum = max((tuple(map(int,v)) for v in re.findall(r"GLIBC_(\d+)\.(\d+)",symbols)),default=(0,0))
    if maximum > (2,35):
        raise RuntimeError(f"Rust core requires GLIBC {maximum}; rebuild on the supported baseline")
    compiler = shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc")
    version = subprocess.check_output([compiler,"--version","--verbose"],cwd=ROOT / "core",env=environment,text=True).strip()
    output.mkdir(parents=True,exist_ok=True)
    shutil.copyfile(binary,output / LIBRARY)
    report=dict(abi=1,architecture=arch(),sha256=sha(binary),source_sha256=sources(),compiler=version,
                maximum_glibc=maximum,dependency_policy="Cargo.lock + vendored checksums; locked offline build")
    (output / "core-build.json").write_text(json.dumps(report,indent=2,sort_keys=True)+"\n")
    verify(output,arch())
    return output


def stage(app, base, architecture):
    configured = os.environ.get("NEUTRONDRIVE_CORE_BUILD")
    directory = Path(configured).resolve() if configured else build()
    binary = verify(directory,architecture)
    shutil.copyfile(binary,app / "neutrondrive" / LIBRARY)
    (app / "neutrondrive" / LIBRARY).chmod(0o644)
    docs = base / "share/doc/neutrondrive"
    docs.mkdir(parents=True,exist_ok=True)
    shutil.copyfile(directory / "core-build.json",docs / "rust-core-build.json")
    shutil.copyfile(ROOT / "core/Cargo.lock",docs / "rust-Cargo.lock")
    licenses = base / "share/licenses/neutrondrive/rust"
    shutil.copytree(ROOT / "packaging/licenses/rust",licenses,dirs_exist_ok=True)
    for crate in (ROOT / "core/vendor").iterdir():
        if crate.is_dir():
            for notice in crate.glob("LICENSE*"):
                if notice.is_file():
                    target=licenses / crate.name / notice.name
                    target.parent.mkdir(parents=True,exist_ok=True)
                    shutil.copyfile(notice,target)
