"""Test-only compatibility API for native core regression contracts.

Never installed or imported by the Rust application. UI/application entrypoints
were deleted; these adapters retain existing fixtures and callback assertions.
"""
from pathlib import Path
import re

__version__ = re.search(r'^version = "([^"]+)"', (Path(__file__).resolve().parents[2] / 'core/Cargo.toml').read_text(), re.M)[1].replace('-alpha.', 'a')
APP_ID = "io.github.SigmaCodez.NeutronDrive"
