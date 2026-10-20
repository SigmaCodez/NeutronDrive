"""Read-only, explicitly labelled UI fixtures. Never used for real accounts."""
from .backend import BackendError, Node


class DemoBackend:
    available = True
    executable = "Demo — keine Cloud-Verbindung"

    def reset_cancel(self):
        pass

    def cancel(self):
        pass

    def list_nodes(self, path):
        if path != "/my-files":
            return []
        return [
            Node("demo-docs", "Dokumente", "folder", path="/my-files/Dokumente"),
            Node("demo-photos", "Fotografie", "folder", path="/my-files/Fotografie"),
            Node("demo-project", "Projekte", "folder", path="/my-files/Projekte"),
            Node("demo-one", "Projektübersicht.pdf", "file", 2483200, "2026-10-20T12:00:00Z", path="/my-files/Projektübersicht.pdf"),
            Node("demo-two", "Bergsee.jpg", "file", 8514432, "2026-10-18T09:30:00Z", path="/my-files/Bergsee.jpg"),
            Node("demo-three", "Notizen.md", "file", 4096, "2026-10-17T15:45:00Z", path="/my-files/Notizen.md"),
        ]

    def version(self):
        return "Demo-Modus · keine Verbindung und keine Übertragungen"

    def __getattr__(self, name):
        def readonly(*args, **kwargs):
            raise BackendError("Die Demo zeigt Beispieldaten. Für echte Aktionen NeutronDrive ohne --demo starten.")
        return readonly
