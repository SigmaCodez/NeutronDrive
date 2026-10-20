from __future__ import annotations

import argparse
import json
import sys
from . import __version__


def main(argv=None):
    parser = argparse.ArgumentParser(description="NeutronDrive — unabhängiger Proton Drive Client für Linux")
    parser.add_argument("--version", action="version", version=f"NeutronDrive {__version__}")
    parser.add_argument("--demo", action="store_true", help="Oberfläche mit klar markierten lokalen Beispieldaten öffnen")
    parser.add_argument("--diagnose", action="store_true", help="Backend-Verfügbarkeit ohne Anmeldung prüfen")
    parser.add_argument("--cli", help="Pfad zum kompatiblen NeutronDrive-SDK-Backend")
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--daemon", action="store_true", help="Gespeicherte Sync-Aufträge im Hintergrunddienst ausführen")
    modes.add_argument("--background", action="store_true", help="Sync und gespeicherte Online-Laufwerke bei Sitzungsstart aktivieren")
    modes.add_argument("--desktop-service", action="store_true", help="Nativer D-Bus-Helfer für Online-Laufwerke")
    modes.add_argument("--sync-status", action="store_true", help="Status des Hintergrund-Sync als JSON")
    modes.add_argument("--stop-background", action="store_true", help="Hintergrund-Sync geordnet stoppen")
    modes.add_argument("--mount", metavar="ORDNER", help="Online-Laufwerk über den nativen Helfer einhängen")
    modes.add_argument("--mount-foreground", metavar="ORDNER", help="Online-Laufwerk im Vordergrund ausführen")
    modes.add_argument("--unmount", action="store_true", help="Online-Laufwerk geordnet aushängen")
    modes.add_argument("--pin", metavar="RELATIVER_PFAD", help="Online-Datei für Offline-Zugriff vorhalten")
    modes.add_argument("--evict", metavar="RELATIVER_PFAD", help="Lokalen Cache einer sauberen, geschlossenen Datei freigeben")
    modes.add_argument("--service-status", action="store_true", help="Status des Online-Laufwerks als JSON")
    parser.add_argument("--state-dir", help="Verzeichnis für Hintergrund-Aufträge (keine Änderung des Kontospeichers)")
    parser.add_argument("--remote", default="/my-files", help="Cloud-Ordner des Online-Laufwerks")
    parser.add_argument("--read-write", action="store_true", help="Schreibzugriff im Online-Laufwerk aktivieren")
    parser.add_argument("--offline", action="store_true", help="Online-Laufwerk nur aus vorhandenem Cache öffnen")
    args = parser.parse_args(argv)
    from .backend import Backend, BackendError
    backend = Backend(args.cli)
    from pathlib import Path
    from .backend import data_dir
    try:
        if args.daemon or args.background or args.sync_status or args.stop_background:
            from .background import Controller, run_daemon
            state = Path(args.state_dir) if args.state_dir else data_dir() / "background"
            if args.sync_status:
                print(json.dumps(Controller(state, executable=args.cli).status(), indent=2, ensure_ascii=False))
                return 0
            if args.stop_background:
                Controller(state, executable=args.cli).stop()
                return 0
            if args.background:
                from .desktop_service import DesktopClient
                try:
                    DesktopClient(args.cli).request("restore")
                except BackendError:
                    # A missing optional native FUSE helper must not stop sync.
                    pass
            return run_daemon(state, backend=backend)
        if args.desktop_service:
            from .desktop_service import run_service
            return run_service(backend)
        if args.mount_foreground:
            from .fuse import run_mount
            result = run_mount(backend, Path(args.mount_foreground), data_dir() / "ondemand-foreground",
                               remote_root=args.remote, writable=args.read_write, offline=args.offline)
            return 1 if result.get("error") else 0
        if args.mount or args.unmount or args.pin or args.evict or args.service_status:
            from .desktop_service import DesktopClient
            client = DesktopClient(args.cli)
            if args.mount:
                value = client.request("mount", mountpoint=str(Path(args.mount).absolute()), remote_root=args.remote,
                                       writable=args.read_write, offline=args.offline)
            elif args.unmount:
                value = client.request("unmount")
            elif args.pin or args.evict:
                value = client.request("pin" if args.pin else "evict", path=args.pin or args.evict)
            else:
                value = client.request("status")
            print(json.dumps(value, indent=2, ensure_ascii=False))
            return 0
    except (RuntimeError, OSError, ValueError) as exc:
        print(str(exc), file=sys.stderr)
        return 1
    if args.diagnose:
        result = {"app": __version__, "backend_found": backend.available, "backend": backend.executable}
        try:
            from .rust_core import call
            result["rust_core"] = call("info")
            result["backend_version"] = backend.version()
        except BackendError as exc:
            result["error"] = str(exc)
        print(json.dumps(result, indent=2, ensure_ascii=False))
        return 0 if "error" not in result else 1
    try:
        from .app import Application
    except (ImportError, ValueError) as exc:
        print(f"GTK4/PyGObject fehlt: {exc}\nBitte das Distributionspaket für NeutronDrive installieren.", file=sys.stderr)
        return 1
    from gi.repository import Gtk
    if not Gtk.init_check():
        print("Keine grafische Sitzung erreichbar. NeutronDrive in einer Wayland- oder X11-Desktopsitzung starten.", file=sys.stderr)
        return 1
    return Application(backend, demo=args.demo).run([sys.argv[0]])


if __name__ == "__main__":
    raise SystemExit(main())
