import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('backend_packaging',ROOT/'packaging/fetch-backend.py')
backend=importlib.util.module_from_spec(spec);spec.loader.exec_module(backend)

class BackendProvenanceTests(unittest.TestCase):
    def test_explicit_source_build_checks_target_sources_and_payload(self):
        bridge=ROOT/'packaging/bridge'
        with tempfile.TemporaryDirectory() as folder:
            binary=Path(folder)/'proton-drive';binary.write_bytes(b'test binary')
            report={**json.loads((bridge/'source-lock.json').read_text()),
                    'target':'bun-linux-x64-baseline',
                    'bridge_files':{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in bridge.glob('*.ts')},
                    'builder_sha256':hashlib.sha256((ROOT/'scripts/build-backend').read_bytes()).hexdigest(),
                    'sha512':hashlib.sha512(binary.read_bytes()).hexdigest()}
            provenance=binary.with_name(binary.name+'.provenance.json')
            provenance.write_text(json.dumps(report))
            backend.verify_source_build(binary,'x86_64')
            with self.assertRaises(ValueError):backend.verify_source_build(binary,'aarch64')
            for key,value in [('bridge_files',{}),('builder_sha256','wrong'),('commit','wrong')]:
                provenance.write_text(json.dumps({**report,key:value}))
                with self.assertRaises(ValueError):backend.verify_source_build(binary,'x86_64')
            provenance.write_text(json.dumps(report));binary.write_bytes(b'corrupt binary')
            with self.assertRaises(ValueError):backend.verify_source_build(binary,'x86_64')
