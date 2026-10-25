"""Native artifact provenance, Qt-only selection and relocatable launcher behavior."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import zipfile
ROOT=Path(__file__).resolve().parents[1]
def module(name):
    spec=importlib.util.spec_from_file_location(name,ROOT/'packaging'/f'{name}.py')
    value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value
qt=module('qt_runtime');native=module('native_frontend')
class NativePackagingTests(unittest.TestCase):
    def test_application_sources_have_no_python_runtime_modules(self):
        self.assertEqual(list((ROOT / 'src').rglob('*.py')), [])
        self.assertFalse((ROOT / 'pyproject.toml').exists())
        self.assertFalse((ROOT / 'packaging/xdg-open-portal').exists())
        for module_name in ('app', 'appearance', 'update_controller', 'qt_instance', 'demo'):
            self.assertFalse((ROOT / 'tests/core_contract' / (module_name + '.py')).exists())
    def test_source_entrypoint_executes_native_binary_and_preserves_arguments(self):
        with tempfile.TemporaryDirectory(prefix='nd source launcher ') as temporary:
            root = Path(temporary)
            binary = root / 'native program'
            binary.write_text('#!/bin/sh\nprintf "%s\\n" "$1" "$2" "$QT_PLUGIN_PATH"\n')
            binary.chmod(0o755)
            env = dict(os.environ, NEUTRONDRIVE_NATIVE_EXECUTABLE=str(binary), NEUTRONDRIVE_QT_RUNTIME=str(root / 'qt'), NEUTRONDRIVE_NATIVE_PROFILE='debug')
            result = subprocess.check_output([str(ROOT / 'bin/neutrondrive'), 'a b', '$(never-run)'], env=env, text=True)
            self.assertEqual(result.splitlines(), ['a b', '$(never-run)', str(root / 'qt/plugins')])
            env['NEUTRONDRIVE_NATIVE_PROFILE'] = '../unexpected'
            self.assertEqual(subprocess.run([str(ROOT / 'bin/neutrondrive')], env=env, stderr=subprocess.DEVNULL).returncode, 2)
    def test_default_staging_does_not_require_or_copy_a_python_entrypoint(self):
        stage = module('stage')
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'source'; root.mkdir()
            for relative in ('LICENSE', 'packaging/native-launcher', 'packaging/licenses/proton-drive-MIT.txt',
                             'packaging/backend-lock.json', 'packaging/bridge/source-lock.json'):
                target = root / relative; target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / relative, target)
            shutil.copytree(ROOT / 'data', root / 'data')
            shutil.copytree(ROOT / 'file-manager/kde', root / 'file-manager/kde')
            catalogs = root / 'src/neutrondrive/locales'; catalogs.mkdir(parents=True)
            (catalogs / 'en.json').write_text('{}')
            backend = root / 'sdk'; backend.write_bytes(b'fixture')
            destination = Path(temporary) / 'package'
            with patch.object(stage, 'ROOT', root), patch.object(stage.backend, 'verify'), \
                 patch.object(stage.native_frontend, 'stage') as native_stage, patch.object(stage.qt_runtime, 'stage'):
                stage.stage(destination, '/usr', 'x86_64', backend)
                native_stage.assert_called_once()
                with self.assertRaisesRegex(ValueError, 'removed'):
                    stage.stage(Path(temporary) / 'legacy', '/usr', 'x86_64', backend, native=False)
            self.assertTrue((destination / 'usr/bin/neutrondrive').read_text().startswith('#!/bin/sh'))
            self.assertTrue((destination / 'usr/lib/neutrondrive/neutrondrive/locales/en.json').is_file())
            self.assertEqual(list(destination.rglob('*.py')), [])
            self.assertTrue((destination / 'usr/share/neutrondrive/integrations/dolphin/CMakeLists.txt').is_file())
            self.assertTrue((destination / 'usr/share/icons/hicolor/scalable/emblems/neutrondrive-available.svg').is_file())
            for libdir in ('lib', 'lib64', 'lib/x86_64-linux-gnu'):
                for manager in ('nautilus/extensions-3.0','nautilus/extensions-4','nemo/extensions-3.0','caja/extensions-2.0','thunarx-3'):
                    link=destination/'usr'/libdir/manager/'libneutrondrive.so'
                    self.assertTrue(link.is_symlink())
                    expected='libneutrondrive_nautilus3.so' if manager.endswith('extensions-3.0') and manager.startswith('nautilus') else 'libneutrondrive_file_manager.so'
                    self.assertEqual(link.resolve(),destination/'usr/lib/neutrondrive/bin'/expected)
            for mode in ('pin','evict','share'):
                text=(destination/'usr/share/file-manager/actions'/('neutrondrive-'+mode+'.desktop')).read_text()
                self.assertIn('Exec=/usr/bin/neutrondrive --file-action '+mode+' -- %f',text)
                self.assertIn('ShowIfTrue=/usr/lib/neutrondrive/bin/neutrondrive-file-manager %f',text)
            # A release-pinned SDK must still retain its adjacent source report.
            provenance = backend.with_name('sdk.provenance.json')
            provenance.write_text('{"source": "fixture"}')
            pinned_destination = Path(temporary) / 'pinned-package'
            with patch.object(stage, 'ROOT', root), patch.object(stage.backend, 'verify') as verify_pin, \
                 patch.object(stage.backend, 'verify_source_build') as verify_source, \
                 patch.object(stage.native_frontend, 'stage'), patch.object(stage.qt_runtime, 'stage'):
                stage.stage(pinned_destination, '/usr', 'x86_64', backend)
                verify_source.assert_called_once_with(backend, 'x86_64')
                verify_pin.assert_not_called()
            self.assertEqual((pinned_destination / 'usr/share/doc/neutrondrive/backend-build.provenance.json').read_bytes(), provenance.read_bytes())
    def test_native_portal_helper_passes_one_literal_url_and_rejects_wrong_arity(self):
        with tempfile.TemporaryDirectory(prefix='nd browser helper ') as temporary:
            app=Path(temporary);(app/'bin').mkdir()
            helper=app/'bin/xdg-open';shutil.copyfile(ROOT/'packaging/xdg-open-portal-native',helper);helper.chmod(0o755)
            executable=app/'bin/neutrondrive-qt';executable.write_text('#!/bin/sh\nprintf "%s\\n" "$1" "$2"\n');executable.chmod(0o755)
            url='https://example.test/auth?value=$(never-run)#fragment'
            result=subprocess.check_output([str(helper),url],text=True,env={'PATH':'/usr/bin:/bin'})
            self.assertEqual(result.splitlines(),['--open-uri',url])
            self.assertEqual(subprocess.run([str(helper)],stderr=subprocess.DEVNULL).returncode,2)
    def test_qt_only_payload_keeps_qml_and_translations_without_python_bindings(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);archive=root/'fixture.whl'
            with zipfile.ZipFile(archive,'w') as wheel:
                for name in ['PySide6/Qt/lib/libQt6Core.so.6','PySide6/Qt/qml/QtQuick/fixture.qml','PySide6/Qt/translations/qtbase_de.qm','PySide6/__init__.py','PySide6/libpyside6.abi3.so','shiboken6/anything.so','PySide6/Qt/qml/QtQuick/hidden.py']:
                    wheel.writestr(name,'synthetic')
            qt.extract([archive],root/'native',native=True)
            self.assertEqual({str(p.relative_to(root/'native')) for p in (root/'native').rglob('*') if p.is_file()}, {'lib/libQt6Core.so.6','qml/QtQuick/fixture.qml','translations/qtbase_de.qm'})
    def test_build_generators_are_explicit_and_never_enter_native_runtime(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary); archive=root/'tools.whl'
            with zipfile.ZipFile(archive,'w') as wheel:
                for name in qt.BUILD_TOOLS | {'unexpected-tool'}:
                    wheel.writestr('PySide6/Qt/libexec/'+name,'synthetic')
                for name in qt.BUILD_LIBRARIES:
                    wheel.writestr('PySide6/Qt/lib/'+name,'synthetic')
            qt.extract([archive],root/'sdk',build_tools=True)
            self.assertEqual({p.name for p in (root/'sdk').rglob('*') if p.is_file()},qt.BUILD_TOOLS | qt.BUILD_LIBRARIES)
            qt.extract([archive],root/'runtime',native=True)
            self.assertEqual([p for p in (root/'runtime').rglob('*') if p.is_file()],[])
            with self.assertRaisesRegex(ValueError,'Build tools'):
                qt.extract([archive],root/'invalid',native=True,build_tools=True)
    def test_native_license_staging_preserves_notices_without_upstream_python_examples(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / 'source'
            licenses = source / 'packaging/licenses/qt'
            licenses.mkdir(parents=True)
            for name in ('LICENSE', 'qt_attribution.json', 'licensewizard.py', 'licensewizard.pyproject', 'license_check.py'):
                (licenses / name).write_text('fixture')
            for name in ('qt-lock.json', 'qt-sources-lock.json', 'QT-NATIVE-NOTICES.md'):
                (source / 'packaging' / name).write_text('fixture')
            with patch.object(qt, 'ROOT', source), patch.object(qt, 'LOCK', source / 'packaging/qt-lock.json'), \
                 patch.object(qt, 'fetch', return_value=[]), patch.object(qt, 'extract'):
                qt.stage(root / 'app', root / 'usr', 'x86_64', native=True)
            staged = root / 'usr/share/licenses/neutrondrive/qt'
            self.assertEqual({p.name for p in staged.iterdir()}, {'LICENSE', 'qt_attribution.json'})
            self.assertEqual((staged / 'LICENSE').read_text(), 'fixture')
    def test_unsafe_archive_paths_are_rejected_even_when_not_selected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);archive=root/'fixture.whl'
            with zipfile.ZipFile(archive,'w') as wheel:wheel.writestr('../escape.py','bad')
            with self.assertRaises(ValueError):qt.extract([archive],root/'native',native=True)
            self.assertFalse((root/'escape.py').exists())
    def test_elf_baseline_and_python_dependency_checks_read_actual_binary_metadata(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary=Path(temporary)/'fixture';header=bytearray(20);header[:6]=b'\x7fELF\x02\x01';struct.pack_into('<H',header,18,62);binary.write_bytes(header)
            with patch.object(native.subprocess,'check_output',return_value='GLIBC_2.38'):
                with self.assertRaisesRegex(ValueError,'baseline'):native.inspect(binary,'amd64')
            with patch.object(native.subprocess,'check_output',side_effect=['GLIBC_2.34 GLIBCXX_3.4.30','(NEEDED) [libpython3.10.so]']):
                with self.assertRaisesRegex(ValueError,'Python'):native.inspect(binary,'amd64')
            with self.assertRaisesRegex(ValueError,'architecture'):native.inspect(binary,'arm64')
            with patch.object(native.subprocess,'check_output',side_effect=['GLIBC_2.34 GLIBCXX_3.4.30','(NEEDED) [libQt6Core.so.6]']):
                with self.assertRaisesRegex(ValueError,'autostart'):native.inspect(binary,'amd64')
            with patch.object(native.subprocess,'check_output',side_effect=['GLIBC_2.34 GLIBCXX_3.4.30','(NEEDED) [libQt6Core.so.6]\n(RUNPATH) [$ORIGIN/../qt/lib]']):
                self.assertEqual(native.inspect(binary,'amd64')['runpath'],'$ORIGIN/../qt/lib')
    def test_source_or_binary_tampering_invalidates_report(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);source=root/'input';source.write_bytes(b'fixture')
            header=bytearray(20);header[:6]=b'\x7fELF\x02\x01';struct.pack_into('<H',header,18,62)
            for name in native.FILE_MANAGER_ARTIFACTS:(root/name).write_bytes(header)
            facts=dict(maximum_glibc=[2,34],maximum_glibcxx=[3,4,30],needed=['libQt6Core.so.6'])
            with patch.object(native,'inspect',return_value=facts),patch.object(native,'inspect_file_manager'),patch.object(native,'sources',return_value={'source':'hash'}):
                native.record(source,root/'output','x86_64');native.verify(root/'output','x86_64')
                (root/'output'/native.BINARY).write_bytes(b'tampered')
                with self.assertRaisesRegex(ValueError,'provenance'):native.verify(root/'output','x86_64')
                (root/'output'/native.BINARY).write_bytes(b'fixture')
                with patch.object(native,'sources',return_value={'source':'changed'}):
                    with self.assertRaisesRegex(ValueError,'provenance'):native.verify(root/'output','x86_64')
    def test_file_manager_artifacts_reject_wrong_architecture_new_glibc_and_qt(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary=Path(temporary)/'plugin';header=bytearray(20);header[:6]=b'\x7fELF\x02\x01';struct.pack_into('<H',header,18,62);binary.write_bytes(header)
            with self.assertRaisesRegex(ValueError,'architecture'):native.inspect_file_manager(binary,'arm64')
            with patch.object(native.subprocess,'check_output',return_value='GLIBC_2.38'):
                with self.assertRaisesRegex(ValueError,'baseline'):native.inspect_file_manager(binary,'x86_64')
            with patch.object(native.subprocess,'check_output',side_effect=['GLIBC_2.34','(NEEDED) [libQt6Core.so.6]']):
                with self.assertRaisesRegex(ValueError,'Qt or Python'):native.inspect_file_manager(binary,'x86_64')
    def test_arm64_baseline_matches_the_pinned_qt_runtime_without_relaxing_x86(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary=Path(temporary)/'arm';header=bytearray(20);header[:6]=b'\x7fELF\x02\x01';struct.pack_into('<H',header,18,183);binary.write_bytes(header)
            dynamic='(NEEDED) [libQt6Core.so.6]\n(RUNPATH) [$ORIGIN/../qt/lib]'
            with patch.object(native.subprocess,'check_output',side_effect=['GLIBC_2.39 GLIBCXX_3.4.32',dynamic]):
                self.assertEqual(native.inspect(binary,'arm64')['maximum_glibc'],[2,39])
            with patch.object(native.subprocess,'check_output',return_value='GLIBC_2.40'):
                with self.assertRaisesRegex(ValueError,'baseline'):native.inspect(binary,'arm64')
    def test_launcher_handles_spaces_and_passes_literal_arguments_without_python(self):
        with tempfile.TemporaryDirectory(prefix='nd native launcher ') as temporary:
            root=Path(temporary);(root/'bin').mkdir();app=root/'lib/neutrondrive';(app/'bin').mkdir(parents=True)
            shutil.copyfile(ROOT/'packaging/native-launcher',root/'bin/neutrondrive');(root/'bin/neutrondrive').chmod(0o755)
            executable=app/'bin/neutrondrive-qt';executable.write_text('#!/bin/sh\nprintf "%s\\n" "$1" "$2" "$QT_PLUGIN_PATH" "$QML_IMPORT_PATH"\n');executable.chmod(0o755)
            result=subprocess.check_output([str(root/'bin/neutrondrive'),'a b','$(never-run)'],text=True,env={'PATH':'/usr/bin:/bin'})
            self.assertEqual(result.splitlines(),['a b','$(never-run)',str(app/'qt/plugins'),str(app/'qt/qml')])
