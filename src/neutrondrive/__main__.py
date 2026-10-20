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
    parser.add_argument("--cli", help="Pfad zur offiziellen proton-drive CLI")
    args = parser.parse_args(argv)
    from .backend import Backend, BackendError
    backend = Backend(args.cli)
    if args.diagnose:
        result = {"app": __version__, "backend_found": backend.available, "backend": backend.executable}
        try:
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
