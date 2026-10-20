"""Backend contract and process tests; never contact Proton or read credentials.

The child executable implements only a deterministic test protocol. The JSON
fixtures mirror the public Proton Drive CLI 0.9.0 contracts, including partial
operation failures that do not produce a nonzero process exit code.
"""
from __future__ import annotations

import fcntl
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from neutrondrive import backend as module
from neutrondrive.backend import (
    Backend, BackendError, Cancelled, Node, checked_json, friendly_error,
    join_remote, local_path, validate_name, validate_remote,
)


NODE_UID = "A" * 22 + "~" + "B" * 22
SAFE_URL = "https://account.proton.me/drive?external=opaque-test-value"


def node_json(name="report.txt", **overrides):
    value = {
        "uid": NODE_UID, "name": {"ok": True, "value": name},
        "type": "file", "modificationTime": "2026-10-20T12:00:00.000Z",
        "activeRevision": {
            "uid": "revision-1", "claimedSize": 123,
            "claimedDigests": {"sha1": "e" * 40, "sha1Verified": True},
        },
    }
    value.update(overrides)
    return value


FAKE_PROGRAM = r'''
import json, os, signal, sys, time
from pathlib import Path

def record(event):
    with open(os.environ["FAKE_CALLS"], "a", encoding="utf-8") as target:
        target.write(json.dumps({"event": event, "pid": os.getpid(), "argv": sys.argv[1:],
            "api_host": os.environ.get("PROTON_DRIVE_BASE_URL"),
            "unsafe_cache": os.environ.get("PROTON_DRIVE_UNSAFE_CACHE")}) + "\n")

record("start")
mode = os.environ.get("FAKE_MODE", "response")
if mode == "login":
    print(json.dumps({"signInUrl": os.environ["FAKE_URL"]}), flush=True)
    deadline = time.monotonic() + 3
    while not Path(os.environ["FAKE_ACK"]).exists():
        if time.monotonic() > deadline:
            print("callback was not delivered before process completion", file=sys.stderr)
            sys.exit(9)
        time.sleep(0.005)
elif mode == "sleep":
    Path(os.environ["FAKE_PID"]).write_text(str(os.getpid()))
    time.sleep(10)
elif mode == "delay":
    time.sleep(0.05)
    print(os.environ.get("FAKE_RESPONSE", "null"), flush=True)
elif mode == "flood":
    sys.stderr.write("private-account-data" * 20000)
    sys.stderr.flush()
    print("[]", flush=True)
elif mode == "exit":
    print(os.environ.get("FAKE_RESPONSE", ""), flush=True)
    print(os.environ.get("FAKE_ERROR", "private-account-data"), file=sys.stderr, flush=True)
    sys.exit(1)
else:
    response = os.environ.get("FAKE_RESPONSE", "null")
    if response:
        print(response, flush=True)
record("end")
'''


class BackendTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="neutrondrive-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.script = self.root / "fake-proton-drive"
        self.script.write_text(f"#!{sys.executable}\n" + FAKE_PROGRAM)
        self.script.chmod(0o700)
        self.calls = self.root / "calls.jsonl"
        self.pid_file = self.root / "child.pid"
        self.ack = self.root / "login.ack"
        self.env = mock.patch.dict(os.environ, {
            "XDG_STATE_HOME": str(self.root / "state"),
            "PROTON_DRIVE_CREDENTIALS_STORE": "keychain",
            "PROTON_DRIVE_BASE_URL": "drive-api.proton.me",
            "PROTON_DRIVE_UNSAFE_CACHE": "false",
            "FAKE_MODE": "response", "FAKE_RESPONSE": "[]",
            "FAKE_CALLS": str(self.calls), "FAKE_PID": str(self.pid_file),
            "FAKE_URL": SAFE_URL, "FAKE_ACK": str(self.ack),
        })
        self.env.start()
        self.addCleanup(self.env.stop)
        self.backend = Backend(str(self.script))

    def call_records(self):
        if not self.calls.exists():
            return []
        return [json.loads(line) for line in self.calls.read_text().splitlines()]

    def response(self, value):
        os.environ["FAKE_RESPONSE"] = json.dumps(value)

    def wait_for(self, predicate, timeout=2):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(0.005)
        self.fail("Timed out waiting for the deterministic fake child")

    def assert_child_reaped(self):
        pid = int(self.pid_file.read_text())
        with self.assertRaises(ProcessLookupError):
            os.kill(pid, 0)

    def test_login_callback_arrives_before_process_exit(self):
        os.environ["FAKE_MODE"] = "login"
        seen = []

        def on_url(url):
            seen.append(url)
            self.assertEqual([r["event"] for r in self.call_records()], ["start"])
            self.ack.touch()

        result = self.backend.login(on_url)
        self.assertEqual(seen, [SAFE_URL])
        self.assertEqual(result, {"signInUrl": SAFE_URL})

    def test_login_rejects_foreign_and_credential_bearing_urls(self):
        os.environ["FAKE_MODE"] = "login"
        for url in (
            "http://account.proton.me/login", "https://account.proton.me.evil.test/login",
            "https://attacker.test/login", "https://user:password@account.proton.me/login",
            "https://:password@account.proton.me/login",
            "https://account.proton.me:444/login", "https://account.proton.me:invalid/login",
            "file:///tmp/login", "not-a-url",
        ):
            with self.subTest(url=url):
                os.environ["FAKE_URL"] = url
                called = []
                with self.assertRaises(BackendError):
                    self.backend.login(called.append)
                self.assertEqual(called, [])

    def test_login_with_no_url_does_not_report_success(self):
        os.environ["FAKE_RESPONSE"] = ""
        with self.assertRaises(BackendError):
            self.backend.login(lambda url: self.fail("Unexpected login URL"))

    def test_partial_operation_failure_with_exit_zero_is_rejected(self):
        self.response([{"uid": "a", "ok": True}, {"uid": "b", "ok": False, "error": {}}])
        with self.assertRaisesRegex(BackendError, "fehlgeschlagene"):
            self.backend.trash(["/my-files/file"])

    def test_partial_transfer_summary_with_exit_zero_is_rejected(self):
        self.response({"transferredItems": 1, "failedItems": 1, "failures": [{"error": "private-data"}]})
        with self.assertRaisesRegex(BackendError, "Teilübertragungen") as raised:
            self.backend._run(["filesystem", "upload", "--json"])
        self.assertNotIn("private-data", str(raised.exception))

    def test_nonzero_exit_overrides_successful_json(self):
        os.environ["FAKE_MODE"] = "exit"
        self.response({"failedItems": 0})
        with self.assertRaises(BackendError) as raised:
            self.backend._run(["filesystem", "upload", "--json"])
        self.assertNotIn("private-account-data", str(raised.exception))

    def test_incomplete_streamed_array_is_not_success(self):
        os.environ["FAKE_RESPONSE"] = '[\n{"uid":"x","ok":true},\n'
        with self.assertRaisesRegex(BackendError, "vollständige JSON"):
            self.backend.list_nodes("/my-files")

    def test_blank_logout_output_is_success(self):
        os.environ["FAKE_RESPONSE"] = ""
        self.assertIsNone(self.backend.logout())

    def test_unshared_status_accepts_upstream_undefined_result(self):
        # Upstream printObject(undefined, true) prints literal undefined.
        os.environ["FAKE_RESPONSE"] = "undefined"
        self.assertIsNone(self.backend.sharing("/my-files/file"))

    def test_remove_link_accepts_upstream_undefined_result(self):
        os.environ["FAKE_RESPONSE"] = "undefined"
        self.assertIsNone(self.backend.remove_link("/my-files/file"))

    def test_required_results_still_reject_upstream_undefined(self):
        os.environ["FAKE_RESPONSE"] = "undefined"
        operations = {
            "create_link": lambda: self.backend.create_link("/my-files/file"),
            "mkdir": lambda: self.backend.mkdir("/my-files", "new-folder"),
            "list_nodes": lambda: self.backend.list_nodes("/my-files"),
            "info": lambda: self.backend.info("/my-files/file"),
            "rename": lambda: self.backend.rename("/my-files/file", "new-name"),
            "trash": lambda: self.backend.trash(["/my-files/file"]),
        }
        for operation, invoke in operations.items():
            with self.subTest(operation=operation), self.assertRaises(BackendError):
                invoke()

    def test_nonzero_exit_rejects_optional_undefined_result(self):
        os.environ["FAKE_MODE"] = "exit"
        os.environ["FAKE_RESPONSE"] = "undefined"
        for invoke in (self.backend.sharing, self.backend.remove_link):
            with self.subTest(operation=invoke.__name__), self.assertRaises(BackendError):
                invoke("/my-files/file")

    def test_large_stderr_does_not_deadlock_stdout_reader(self):
        os.environ["FAKE_MODE"] = "flood"
        self.assertEqual(self.backend._run(["filesystem", "list"], timeout=3), [])

    def test_output_limit_stops_child(self):
        os.environ["FAKE_MODE"] = "flood"
        with mock.patch.object(module, "MAX_OUTPUT", 1024):
            with self.assertRaisesRegex(BackendError, "Größenbegrenzung"):
                self.backend._run(["filesystem", "list"], timeout=3)

    def test_timeout_reaps_child(self):
        os.environ["FAKE_MODE"] = "sleep"
        with self.assertRaisesRegex(BackendError, "Zeitlimit"):
            self.backend._run(["filesystem", "list"], timeout=0.2)
        self.assert_child_reaped()

    def test_active_cancellation_reaps_child(self):
        os.environ["FAKE_MODE"] = "sleep"
        failures = []

        def task():
            try:
                self.backend._run(["filesystem", "list"], timeout=3)
            except Exception as exc:
                failures.append(exc)

        thread = threading.Thread(target=task)
        thread.start()
        self.addCleanup(thread.join, 4)
        self.wait_for(self.pid_file.exists)
        self.backend.cancel()
        thread.join(3)
        self.assertFalse(thread.is_alive())
        self.assertEqual(len(failures), 1)
        self.assertIsInstance(failures[0], Cancelled)
        self.assert_child_reaped()

    def test_cancellation_before_start_never_launches_process(self):
        self.backend.cancel()
        with self.assertRaises(Cancelled):
            self.backend.logout()
        self.assertEqual(self.call_records(), [])
        self.backend.reset_cancel()
        self.backend.logout()
        self.assertEqual(len(self.call_records()), 2)

    def test_cancellation_while_waiting_for_mutex_never_launches_process(self):
        waiting = threading.Event()
        lock = threading.Lock()
        lock.acquire()

        class ObservedLock:
            def __enter__(self):
                waiting.set()
                lock.acquire()

            def __exit__(self, *exc):
                lock.release()

        self.backend._mutex = ObservedLock()
        failures = []

        def task():
            try:
                self.backend.logout()
            except Exception as exc:
                failures.append(exc)

        with mock.patch.object(module.subprocess, "Popen", wraps=module.subprocess.Popen) as spawn:
            thread = threading.Thread(target=task)
            thread.start()
            self.assertTrue(waiting.wait(2))
            self.backend.cancel()
            lock.release()
            thread.join(3)
            self.assertFalse(thread.is_alive())
            self.assertEqual(spawn.call_count, 0, "Cancelled queued operation must never spawn a child")
        self.assertEqual(len(failures), 1)
        self.assertIsInstance(failures[0], Cancelled)
        self.assertEqual(self.call_records(), [])

    def test_same_backend_serializes_concurrent_commands(self):
        os.environ["FAKE_MODE"] = "delay"
        failures = []

        def task():
            try:
                self.backend.logout()
            except Exception as exc:
                failures.append(exc)

        threads = [threading.Thread(target=task) for _ in range(2)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(3)
            self.assertFalse(thread.is_alive())
        self.assertEqual(failures, [])
        self.assertEqual([r["event"] for r in self.call_records()], ["start", "end", "start", "end"])

    def test_other_instance_lock_prevents_process_start(self):
        lock_path = module.data_dir() / "backend.lock"
        with lock_path.open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaisesRegex(BackendError, "beschäftigt"):
                self.backend._run(["auth", "logout", "--json"], timeout=0.1)
        self.assertEqual(self.call_records(), [])

    def test_waiting_process_continues_after_other_instance_releases_lock(self):
        lock_path = module.data_dir() / "backend.lock"
        failures = []
        with lock_path.open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            def run():
                try:
                    self.backend._run(["version"], timeout=2)
                except Exception as exc:
                    failures.append(exc)
            thread = threading.Thread(target=run)
            thread.start()
            self.assertEqual(self.call_records(), [])
            fcntl.flock(lock, fcntl.LOCK_UN)
            thread.join(3)
        self.assertFalse(thread.is_alive())
        self.assertEqual(failures, [])
        self.assertEqual(len(self.call_records()), 2)

    def test_plaintext_credentials_store_is_rejected_before_spawn(self):
        os.environ["PROTON_DRIVE_CREDENTIALS_STORE"] = "unsafe_file"
        with self.assertRaises(BackendError):
            self.backend.logout()
        self.assertEqual(self.call_records(), [])

    def test_environment_cannot_redirect_cli_to_another_api_host(self):
        os.environ["PROTON_DRIVE_BASE_URL"] = "untrusted.example"
        try:
            self.backend.logout()
        except BackendError:
            self.assertEqual(self.call_records(), [])
        else:
            self.assertIn(self.call_records()[0]["api_host"], (None, "drive-api.proton.me"))

    def test_environment_cannot_enable_plaintext_crypto_cache(self):
        os.environ["PROTON_DRIVE_UNSAFE_CACHE"] = "true"
        try:
            self.backend.logout()
        except BackendError:
            self.assertEqual(self.call_records(), [])
        else:
            self.assertIn(self.call_records()[0]["unsafe_cache"], (None, "false", "0"))

    def test_rename_preserves_option_like_and_shell_metacharacter_name(self):
        name = "--help $(touch nope); `touch nope`"
        self.response(node_json(name))
        self.backend.rename("/my-files/source", name)
        argv = self.call_records()[0]["argv"]
        self.assertEqual(argv, ["filesystem", "rename", "--json", "--", "/my-files/source", name])
        self.assertFalse((self.root / "nope").exists())

    def test_root_info_parses_successfully(self):
        self.response(node_json("My files", type="folder", activeRevision=None))
        node = self.backend.info("/my-files")
        self.assertEqual(node.type, "folder")
        self.assertTrue(node.safe)

    def test_device_can_be_navigated_as_folder(self):
        self.response([{
            "uid": "device-id", "type": "Windows", "name": {"ok": True, "value": "My PC"},
            "rootFolderUid": NODE_UID, "creationTime": "2026-01-01T00:00:00Z",
        }])
        node = self.backend.list_nodes("/devices")[0]
        self.assertEqual(node.type, "folder")
        self.assertEqual(node.uid, NODE_UID)
        self.assertEqual(node.path, "/devices/My PC")
        self.assertTrue(node.safe)

    def test_ambiguous_trash_entries_cannot_be_restored(self):
        self.response([node_json("same.txt", uid="one"), node_json("same.txt", uid="two")])
        nodes = self.backend.list_nodes("/trash")
        self.assertTrue(all(not node.safe for node in nodes))
        with self.assertRaisesRegex(BackendError, "Mehrdeutiger"):
            self.backend.restore(["/trash/same.txt"])
        self.assertTrue(all(r["argv"][1] == "list" for r in self.call_records()))

    def test_upload_rejects_nested_symlinks_before_spawn(self):
        folder = self.root / "upload"
        folder.mkdir()
        (folder / "link").symlink_to(self.script)
        with self.assertRaises(BackendError):
            self.backend.upload([folder], "/my-files")
        self.assertEqual(self.call_records(), [])

    def test_download_rejects_overwrite_strategy(self):
        with self.assertRaises(BackendError):
            self.backend.download(["/my-files/file"], self.root, file_strategy="remove")
        self.assertEqual(self.call_records(), [])

    def test_upload_argv_has_explicit_conflict_policy_and_delimiter(self):
        source = self.root / "document with spaces.txt"
        source.write_text("test")
        self.response({"transferredItems": 1, "failedItems": 0})
        self.backend.upload([source], "/my-files")
        self.assertEqual(self.call_records()[0]["argv"], [
            "filesystem", "upload", "--json", "--file-conflict-strategy", "rename",
            "--folder-conflict-strategy", "merge", "--", str(source), "/my-files",
        ])


class ValueTests(unittest.TestCase):
    def test_mutation_failure_and_invalid_json_contracts(self):
        for output in ('[{"ok":false}]', '{"failedItems":1}', 'null trailing', '[{', 'undefined'):
            with self.subTest(output=output), self.assertRaises(BackendError):
                checked_json(output)
        self.assertEqual(checked_json('[{"ok":true}]'), [{"ok": True}])
        self.assertIsNone(checked_json("  "))

    def test_official_logged_out_message_is_actionable(self):
        self.assertIn("Browser", friendly_error("You need to login first\n"))

    def test_valid_uid_is_used_for_unambiguous_my_files_path(self):
        node = Node.parse(node_json("name/with/slashes"), "/my-files")
        self.assertEqual(node.path, "/my-files/" + NODE_UID)
        self.assertTrue(node.safe)
        self.assertEqual(node.size, 123)

    def test_decryption_failure_disables_operations(self):
        item = node_json()
        item["name"] = {"ok": False, "error": {}}
        node = Node.parse(item, "/my-files")
        self.assertFalse(node.safe)

    def test_negative_or_bool_size_is_not_rendered_as_valid_size(self):
        for size in (-1, True, "123"):
            with self.subTest(size=size):
                node = Node.parse(node_json(activeRevision={"claimedSize": size}), "/my-files")
                self.assertIsNone(node.size)

    def test_remote_validation_and_name_boundaries(self):
        for path in ("relative", "/photos", "/my-files-evil/a", "/my-files/x\x00y"):
            with self.subTest(path=path), self.assertRaises(BackendError):
                validate_remote(path)
        for name in ("", "   ", ".", "..", "a/b", "a\\b", "a\nb"):
            with self.subTest(name=name), self.assertRaises(BackendError):
                validate_name(name)
        self.assertEqual(join_remote("/my-files/", "report.txt"), "/my-files/report.txt")

    def test_local_path_rejects_globs_and_symlinks(self):
        with tempfile.TemporaryDirectory(prefix="neutrondrive-path-") as temp:
            root = Path(temp)
            for name in ("star*", "question?", "bracket[", "brace{"):
                path = root / name
                path.touch()
                with self.subTest(name=name), self.assertRaises(BackendError):
                    local_path(path)
            regular = root / "regular"
            regular.touch()
            link = root / "link"
            link.symlink_to(regular)
            with self.assertRaises(BackendError):
                local_path(link)
            self.assertEqual(local_path(regular), regular)


if __name__ == "__main__":
    unittest.main()
