"""Recoverable, same-filesystem trash with a private operation journal.

Desktop trash follows https://specifications.freedesktop.org/trash/latest/ .
No content is ever unlinked or copied then deleted. Linux renameat2(NOREPLACE)
provides atomic destination exclusion. Linux has no inode-conditional rename:
pre/post identity+content proofs detect a concurrent source replacement; rollback
is attempted without overwrite, otherwise the preserved object is recovery data.
Only this application's receipts are listed. Desktop trash may independently be
restored/emptied by the user; a missing object is reported, never reconstructed.
"""
from __future__ import annotations

from dataclasses import asdict, dataclass
import os
from pathlib import Path
import uuid

from .backend import BackendError, Cancelled, data_dir

@dataclass(frozen=True)
class TrashReceipt:
    id: str
    original_path: str
    trash_path: str
    metadata_path: str
    deleted_at: str
    kind: str
    status: str
    backend: str
    error: str = ""
    restore_path: str = ""

    def to_dict(self):
        return asdict(self)


class LocalTrashError(BackendError):
    def __init__(self, message, receipt=None):
        super().__init__(message)
        self.receipt = receipt


class LocalTrash:
    def __init__(self, state_dir=None, *, data_home=None, prefer_desktop=True):
        self.state_dir = Path(state_dir if state_dir is not None else data_dir() / 'local-trash')
        self.data_home = Path(data_home if data_home is not None else os.environ.get('XDG_DATA_HOME', str(Path.home() / '.local/share')))
        self.prefer_desktop = prefer_desktop
        self.entries_dir = self.state_dir / 'entries'
        from . import rust_core
        try:
            rust_core.backend_value('trash_init', state=str(self.state_dir))
        except BackendError as error:
            raise LocalTrashError(str(error)) from error

    @staticmethod
    def _receipt(record):
        return TrashReceipt(**{key: record.get(key, '') for key in TrashReceipt.__dataclass_fields__})

    def trash(self, root_fd, relative, expected_entry, *, root_path, expected_tree=None, validate=None, operation_id=None, check_cancel=None):
        from . import rust_core
        receipt = None
        def event(value):
            nonlocal receipt
            if value['kind'] == 'trash_receipt':
                receipt = self._receipt(value['record'])
            elif value['kind'] == 'trash_validate' and validate:
                validate()
        try:
            result = rust_core.backend_value('trash_move', request=dict(
                state=str(self.state_dir), data_home=str(self.data_home), desktop=self.prefer_desktop,
                root_fd=root_fd, root=str(root_path), relative=relative,
                expected=expected_entry, expected_tree=expected_tree,
                identifier=str(uuid.UUID(operation_id)) if operation_id else None),
                on_poll=check_cancel, on_event=event)
            receipt = self._receipt(result['record'])
            if not result['ok']:
                error = Cancelled(result['error']) if result.get('cancelled') else LocalTrashError(result['error'])
                error.receipt = receipt
                raise error
            return receipt
        except Cancelled as error:
            error.receipt = receipt
            raise
        except Exception as error:
            raise LocalTrashError(str(error), receipt) from error

    def list_entries(self):
        from . import rust_core
        try:
            return [self._receipt(record) for record in
                    rust_core.backend_value('trash_list', state=str(self.state_dir))]
        except BackendError as error:
            raise LocalTrashError(str(error)) from error

    def get_receipt(self, identifier):
        identifier = str(uuid.UUID(identifier))
        return next((entry for entry in self.list_entries() if entry.id == identifier), None)

    def restore(self, identifier, destination=None, *, check_cancel=None):
        from . import rust_core
        identifier = str(uuid.UUID(identifier))
        receipt = None
        def event(value):
            nonlocal receipt
            if value['kind'] == 'trash_receipt':
                receipt = self._receipt(value['record'])
        try:
            result = rust_core.backend_value('trash_restore', state=str(self.state_dir),
                identifier=identifier, destination=str(destination) if destination is not None else None,
                on_poll=check_cancel, on_event=event)
            receipt = self._receipt(result['record'])
            if not result['ok']:
                error = Cancelled(result['error']) if result.get('cancelled') else LocalTrashError(result['error'])
                error.receipt = receipt
                raise error
            return receipt
        except Cancelled as error:
            error.receipt = receipt
            raise
        except Exception as error:
            raise LocalTrashError(str(error), receipt) from error
