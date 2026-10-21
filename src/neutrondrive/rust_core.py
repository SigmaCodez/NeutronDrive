"""Versioned Rust policy and native Linux runtime. No silent Python fallback or runtime builds.

Policy calls carry observations and decisions. The separate backend ABI owns
processes, private state and inotify; callbacks have synchronous, bounded lifetimes.
"""
import ctypes
from dataclasses import asdict, is_dataclass
import json
import os
from pathlib import Path
import threading

from .backend import BackendError, Cancelled


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
            library.neutrondrive_backend_call.argtypes = [ctypes.c_void_p, ctypes.c_size_t, _CALLBACK, ctypes.c_void_p]
            library.neutrondrive_backend_call.restype = ctypes.c_void_p
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


_CALLBACK = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p)


def run_backend(executable, args, *, timeout, parse, on_login, cancel, max_output):
    """Temporary Python binding: Rust owns child, pipes, timeout and lock."""
    payload = json.dumps(dict(version=1, executable=executable, args=args,
                             timeout=timeout, parse=parse, login=on_login is not None,
                             max_output=max_output), ensure_ascii=True, allow_nan=False).encode()
    if len(payload) > MAX_MESSAGE:
        raise CoreError("Der Backend-Auftrag überschreitet die sichere Nachrichtengröße.")
    return _backend_request(payload, on_login=on_login, cancel=cancel)


def backend_value(operation_name, *, cancel=None, on_event=None, **arguments):
    payload = json.dumps(dict(version=1, op=operation_name, **arguments), ensure_ascii=True,
                         allow_nan=False, default=_encode).encode()
    if len(payload) > MAX_MESSAGE:
        raise CoreError("Der Backend-Auftrag überschreitet die sichere Nachrichtengröße.")
    return _backend_request(payload, cancel=cancel, on_event=on_event)


def _backend_request(payload, *, on_login=None, cancel=None, on_event=None):
    library = _load()
    callback_errors = []

    @_CALLBACK
    def callback(pointer, length, context):
        try:
            if cancel is not None and cancel.is_set(): return 1
            event = json.loads(ctypes.string_at(pointer, length))
            if event["kind"] == "login": on_login(event["url"])
            elif event["kind"] != "poll":
                if on_event is None: raise CoreError("Unerwartetes natives Ereignis")
                on_event(event)
            return int(cancel is not None and cancel.is_set())
        except BaseException as error:
            callback_errors.append(error)
            return 1

    buffer = ctypes.create_string_buffer(payload)
    pointer = library.neutrondrive_backend_call(buffer, len(payload), callback, None)
    if not pointer: raise CoreError("Der Rust-Backendprozess lieferte keine Antwort.")
    try:
        result = json.loads(ctypes.string_at(pointer))
    finally:
        library.neutrondrive_core_free(pointer)
    if callback_errors: raise callback_errors[0]
    if result.get("ok") is not True:
        error_type = Cancelled if result.get("cancelled") else BackendError
        raise error_type(result.get("error", "Ungültige Backend-Antwort"))
    return result["result"]
