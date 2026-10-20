"""Versioned Rust decision core. No silent Python fallback or runtime builds.

The immutable JSON boundary carries observations and decisions, never credentials.
All filesystem/SDK effects stay in the guarded host adapters during migration.
"""
import ctypes
from dataclasses import asdict, is_dataclass
import json
import os
from pathlib import Path
import threading

from .backend import BackendError


class CoreError(BackendError):
    pass


_library = None
_lock = threading.Lock()
MAX_MESSAGE = 64 * 1024 * 1024


def _encode(value):
    if is_dataclass(value):
        return asdict(value)
    if isinstance(value, (set, frozenset)):
        return sorted(value, key=lambda item: (item is not None, str(item)))
    raise TypeError(f"Unsupported Rust boundary value: {type(value).__name__}")


def _load():
    global _library
    with _lock:
        if _library is not None:
            return _library
        configured = os.environ.get("NEUTRONDRIVE_CORE_LIBRARY")
        if configured:
            path = Path(configured)
            if not path.is_absolute():
                raise CoreError("NEUTRONDRIVE_CORE_LIBRARY muss ein absoluter Pfad sein.")
        else:
            path = Path(__file__).resolve().parent / "libneutrondrive_core.so"
            source = Path(__file__).resolve().parents[2]
            if not path.exists() and (source / "core/Cargo.toml").is_file():
                path = source / "build/rust/libneutrondrive_core.so"
        try:
            library = ctypes.CDLL(str(path))
            library.neutrondrive_core_abi.argtypes = []
            library.neutrondrive_core_abi.restype = ctypes.c_uint32
            library.neutrondrive_core_call.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
            library.neutrondrive_core_call.restype = ctypes.c_void_p
            library.neutrondrive_core_free.argtypes = [ctypes.c_void_p]
            library.neutrondrive_core_free.restype = None
            if library.neutrondrive_core_abi() != 1:
                raise CoreError("Inkompatible Rust-Kernversion; NeutronDrive-Paket aktualisieren.")
        except (OSError, AttributeError) as error:
            raise CoreError("Der Rust-Kern fehlt oder ist inkompatibel. Paket neu installieren oder scripts/build-rust-core ausführen.") from error
        _library = library
        return library


def call(operation, **arguments):
    payload = json.dumps(dict(version=1, op=operation, **arguments), default=_encode,
                         ensure_ascii=True, allow_nan=False, separators=(",", ":")).encode()
    if len(payload) > MAX_MESSAGE:
        raise CoreError("Der Rust-Kernauftrag überschreitet die sichere Nachrichtengröße.")
    library = _load()
    buffer = ctypes.create_string_buffer(payload)
    pointer = library.neutrondrive_core_call(buffer, len(payload))
    if not pointer:
        raise CoreError("Der Rust-Kern lieferte keine Antwort; Vorgang angehalten.")
    try:
        result = json.loads(ctypes.string_at(pointer))
    finally:
        library.neutrondrive_core_free(pointer)
    if not isinstance(result, dict) or result.get("ok") is not True:
        raise CoreError(result.get("error", "Ungültige Rust-Kernantwort") if isinstance(result, dict) else "Ungültige Rust-Kernantwort")
    return result["result"]
