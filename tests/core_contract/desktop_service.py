"""Thin launch/client binding for the Rust-owned desktop service."""
import os
from pathlib import Path
import signal
import threading
from . import APP_ID
from .backend import Backend, data_dir

BUS_NAME = APP_ID + ('.SnapService' if os.environ.get('SNAP') else '.Service')
OBJECT_PATH = '/io/github/SigmaCodez/NeutronDrive/Service'
INTERFACE = BUS_NAME


class DesktopClient:
    def __init__(self, executable=None):
        self.executable = executable

    def request(self, action, **options):
        from .rust_core import backend_value
        command = [str(Path(__file__).resolve().parents[2] / 'bin/neutrondrive'), '--desktop-service']
        if self.executable:
            command += ['--cli', self.executable]
        return backend_value('desktop_bus', operation=dict(action='request',
            request=dict(action=action, **options),
            launcher=dict(argv=command, source=str(Path(__file__).resolve().parents[1]))))



def run_service(backend=None):
    from .rust_core import backend_value
    backend = backend or Backend()
    cancelled = threading.Event()
    previous = {sig: signal.signal(sig, lambda *_: cancelled.set())
                for sig in (signal.SIGTERM, signal.SIGINT)}
    try:
        backend_value('desktop_runtime', config=dict(
            state_dir=str(data_dir() / 'ondemand-service'), executable=backend.executable),
            cancel=cancelled)
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)
    return 0
