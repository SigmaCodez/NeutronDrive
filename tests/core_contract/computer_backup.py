"""Temporary UI binding; computer setup and identity pinning execute in Rust."""
from pathlib import Path
from . import rust_core
from .background import Job


def add_folder(backend, state_dir, local_root):
    with backend._mutex:
        value = rust_core.backend_value("computer_setup", executable=backend.executable,
            state=str(Path(state_dir).absolute()), root=str(Path(local_root).expanduser().absolute()),
            cancel=backend.cancel_event)
    return Job(**value)
