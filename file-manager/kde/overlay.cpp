// Minimal Qt/KIO ABI adapter. Cache validation and status stay in Rust.
#include <KOverlayIconPlugin>
#include <KFilePlacesModel>
#include <QLibrary>
#include <QMap>
#include <QTimer>
#include <QUrl>
#ifndef NEUTRONDRIVE_RUST_LIBRARY
#define NEUTRONDRIVE_RUST_LIBRARY "/usr/lib/neutrondrive/bin/libneutrondrive_file_manager.so"
#endif
class NeutronDriveOverlay final : public KOverlayIconPlugin {
    Q_OBJECT
    Q_PLUGIN_METADATA(IID "org.kde.overlayicon.neutrondrive")
    QLibrary library{QStringLiteral(NEUTRONDRIVE_RUST_LIBRARY)};
    using Available = int (*)(const char *);
    Available available = nullptr;
    QMap<QUrl, bool> seen;
    using SyncRoot = size_t (*)(char *, size_t);
    SyncRoot syncRoot = nullptr;
    KFilePlacesModel *places = nullptr;
    void updatePlaces() {
        if (!syncRoot) return;
        char path[4097]; if (!syncRoot(path, sizeof(path))) return;
        QUrl url = QUrl::fromLocalFile(QString::fromUtf8(path));
        bool exists = false;
        for (int row = 0; row < places->rowCount(); ++row) {
            auto index = places->index(row, 0);
            if (places->url(index) != url) continue;
            if (places->isDevice(index)) { if (!places->isHidden(index)) places->setPlaceHidden(index, true); }
            else {
                exists = true;
                if (places->text(index) == QStringLiteral("Proton Drive") &&
                    index.data(KFilePlacesModel::IconNameRole).toString() != QStringLiteral("io.github.SigmaCodez.NeutronDrive"))
                    places->editPlace(index, places->text(index), url, QStringLiteral("io.github.SigmaCodez.NeutronDrive"));
            }
        }
        if (!exists) places->addPlace(QStringLiteral("Proton Drive"), url, QStringLiteral("io.github.SigmaCodez.NeutronDrive"));
    }
public:
    NeutronDriveOverlay() {
        available = reinterpret_cast<Available>(library.resolve("neutrondrive_is_available"));
        syncRoot = reinterpret_cast<SyncRoot>(library.resolve("neutrondrive_sync_root"));
        places = new KFilePlacesModel(this);
        auto timer = new QTimer(this);
        connect(timer, &QTimer::timeout, this, [this] {
            updatePlaces();
            for (auto i = seen.begin(); i != seen.end(); ++i) {
                bool current = check(i.key());
                if (current != i.value()) { i.value() = current; emit overlaysChanged(i.key(), icons(current)); }
            }
        });
        timer->start(2000);
    }
    bool check(const QUrl &url) const { return available && url.isLocalFile() && available(url.toLocalFile().toUtf8().constData()); }
    static QStringList icons(bool value) { return value ? QStringList{QStringLiteral("neutrondrive-available")} : QStringList{}; }
    QStringList getOverlays(const QUrl &url) override {
        bool value = check(url);
        if (seen.size() < 10000 || seen.contains(url)) seen.insert(url, value);
        return icons(value);
    }
};
#include "overlay.moc"
