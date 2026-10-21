"""Per-user Qt window activation, including a hidden Flatpak background window."""
import os
from pathlib import Path
import stat

from PySide6.QtCore import QLockFile
from PySide6.QtNetwork import QLocalServer, QLocalSocket


class WindowInstance:
    def __init__(self, directory, activate):
        self.directory = Path(directory)
        if any(p.is_symlink() for p in (self.directory, *self.directory.parents)):
            raise RuntimeError('Unsicherer Pfad für die Fensteraktivierung.')
        self.directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        info = self.directory.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
            raise RuntimeError('Fensteraktivierung benötigt ein privates Verzeichnis.')
        self.path = str(self.directory / 'window.sock')
        self.lock = QLockFile(str(self.directory / 'window.lock'))
        self.lock.setStaleLockTime(0)
        self.server = QLocalServer()
        self.server.setSocketOptions(QLocalServer.SocketOption.UserAccessOption)
        self.activate = activate
        self.server.newConnection.connect(self._connected)

    def claim(self):
        if self.lock.tryLock(0):
            QLocalServer.removeServer(self.path)
            if not self.server.listen(self.path):
                self.lock.unlock()
                raise RuntimeError('Lokale Fensteraktivierung konnte nicht gestartet werden.')
            return True
        client = QLocalSocket()
        client.connectToServer(self.path)
        if not client.waitForConnected(1500):
            raise RuntimeError('NeutronDrive startet bereits oder reagiert nicht. Bitte erneut versuchen.')
        client.disconnectFromServer()
        return False

    def _connected(self):
        while self.server.hasPendingConnections():
            socket = self.server.nextPendingConnection()
            socket.abort();socket.deleteLater()
            self.activate()

    def close(self):
        self.server.close()
        if self.lock.isLocked():self.lock.unlock()
