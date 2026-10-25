#include <QApplication>
#include <QPluginLoader>
#include <QFile>
#include <QTimer>
#include <KOverlayIconPlugin>
#include <KFilePlacesModel>
#include <cstdio>
int main(int argc, char **argv) {
    QApplication app(argc, argv);
    const QUrl file = QUrl::fromLocalFile(argv[2]);
    const QUrl root = file.adjusted(QUrl::RemoveFilename | QUrl::StripTrailingSlash);
    KFilePlacesModel initial;
    initial.addPlace(QStringLiteral("Proton Drive"), root, QStringLiteral("folder-cloud"));
    initial.addPlace(QStringLiteral("Other test favorite"), QUrl::fromLocalFile("/tmp/other-test-favorite"), QStringLiteral("folder"));
    QPluginLoader loader(argv[1]);
    auto plugin = qobject_cast<KOverlayIconPlugin *>(loader.instance());
    if (!plugin) { qCritical("%s", qPrintable(loader.errorString())); return 1; }
    if (plugin->getOverlays(file) != QStringList{"neutrondrive-available"}) return 2;
    QFile index(argv[3]);
    if (!index.open(QIODevice::WriteOnly | QIODevice::Truncate)) return 3;
    index.write("{\"version\":1,\"available\":{}}"); index.close();
    QObject::connect(plugin, &KOverlayIconPlugin::overlaysChanged, &app,
        [&](const QUrl &url, const QStringList &icons) {
            if (url != file || !icons.isEmpty()) return;
            KFilePlacesModel places;
            int matching = 0; bool correct = false, other = false;
            for (int row = 0; row < places.rowCount(); ++row) {
                const auto item = places.index(row, 0);
                const auto icon = item.data(KFilePlacesModel::IconNameRole).toString();
                if (places.url(item) == root && !places.isDevice(item)) {
                    ++matching; correct = icon == QStringLiteral("io.github.SigmaCodez.NeutronDrive");
                }
                if (places.text(item) == QStringLiteral("Other test favorite")) other = icon == QStringLiteral("folder");
            }
            const bool ok = matching == 1 && correct && other;
            puts(ok ? "DOLPHIN_OVERLAY_REFRESH_AND_BOOKMARK_ICON_UPGRADE_OK" : "BOOKMARK_ICON_FAILED");
            app.exit(ok ? 0 : 4);
        });
    QTimer::singleShot(6000, &app, [&] { app.exit(5); });
    return app.exec();
}
