#pragma once
// Direct Qt API bindings only; application policy stays in Rust.
#include <QSettings>
#include <QLockFile>
#include <QTranslator>
#include <QLibraryInfo>
#include <QGuiApplication>
#include <QClipboard>
#include <QQuickWindow>
#include <QKeyEvent>
#include <QStandardPaths>
#include <QDesktopServices>
#include <QUrl>
#include <QLocale>
#include <QDateTime>
#include <QStringList>
#include <memory>
// Qt Test's event injection entry point is exported by the pinned Qt Gui
// runtime. Use the window-system path so Qt Quick gets spontaneous pointer
// events; sendEvent(QMouseEvent) does not reproduce native pointer delivery.
QT_BEGIN_NAMESPACE
Q_GUI_EXPORT void qt_handleMouseEvent(QWindow *, const QPointF &, const QPointF &,
                                      Qt::MouseButtons, Qt::MouseButton,
                                      QEvent::Type, Qt::KeyboardModifiers, int);
QT_END_NAMESPACE
namespace ndqt {
inline std::unique_ptr<QTranslator> newTranslator() { return std::make_unique<QTranslator>(); }
inline bool loadTranslation(QTranslator &translator, const QString &language, const QString &domain) {
    return translator.load(QLocale(language), domain, "_", QLibraryInfo::path(QLibraryInfo::TranslationsPath));
}
inline bool installTranslation(QTranslator &translator) { return QCoreApplication::installTranslator(&translator); }
inline void removeTranslation(QTranslator &translator) { QCoreApplication::removeTranslator(&translator); }
inline std::unique_ptr<QLockFile> instanceLock(const QString &path) {
    auto lock = std::make_unique<QLockFile>(path);
    lock->setStaleLockTime(0);
    return lock;
}
inline bool claimLock(QLockFile &lock) { return lock.tryLock(0); }
inline bool snapshotWindow(const QString &name, const QString &path) {
    for (auto *window : QGuiApplication::allWindows()) {
        if (window->objectName() == name) {
            auto *quick = qobject_cast<QQuickWindow *>(window);
            return quick && quick->grabWindow().save(path);
        }
    }
    return false;
}
inline void setClipboard(const QString &text) { QGuiApplication::clipboard()->setText(text); }
inline QString clipboardText() { return QGuiApplication::clipboard()->text(); }
// Native UI regression input: deliver real Qt events to the same window as a user.
inline bool windowKey(const QString &name, int key, bool control) {
    for (auto *window : QGuiApplication::allWindows()) {
        if (window->objectName() != name) continue;
        auto modifiers = control ? Qt::ControlModifier : Qt::NoModifier;
        QKeyEvent press(QEvent::KeyPress, key, modifiers);
        QKeyEvent release(QEvent::KeyRelease, key, modifiers);
        QCoreApplication::sendEvent(window, &press);
        QCoreApplication::sendEvent(window, &release);
        return true;
    }
    return false;
}
inline bool windowClick(const QString &name, double x, double y, bool doubleClick) {
    for (auto *window : QGuiApplication::allWindows()) {
        if (window->objectName() != name) continue;
        QPointF local(x, y);
        QPointF global = window->mapToGlobal(local.toPoint());
        static int timestamp = 0;
        timestamp += 1000;
        for (int click = 0; click < (doubleClick ? 2 : 1); ++click) {
            qt_handleMouseEvent(window, local, global, Qt::LeftButton, Qt::LeftButton,
                                QEvent::MouseButtonPress, Qt::NoModifier, ++timestamp);
            qt_handleMouseEvent(window, local, global, Qt::NoButton, Qt::LeftButton,
                                QEvent::MouseButtonRelease, Qt::NoModifier, ++timestamp);
        }
        return true;
    }
    return false;
}
inline QString cacheLocation() { return QStandardPaths::writableLocation(QStandardPaths::CacheLocation); }
inline bool openUrl(const QString &url) { return QDesktopServices::openUrl(QUrl(url)); }
inline std::unique_ptr<QSettings> interfaceSettings() {
    return std::make_unique<QSettings>(QSettings::IniFormat, QSettings::UserScope,
                                      "NeutronDrive", "interface");
}
inline std::unique_ptr<QSettings> fileSettings(const QString &path) {
    return std::make_unique<QSettings>(path, QSettings::IniFormat);
}
inline QString setting(const QSettings &settings, const QString &key, const QString &fallback) {
    return settings.value(key, fallback).toString();
}
inline void setSetting(QSettings &settings, const QString &key, const QString &value) {
    settings.setValue(key, value);
}
inline bool syncSettings(QSettings &settings) {
    settings.sync();
    return settings.status() == QSettings::NoError;
}
inline QStringList systemLanguages() { return QLocale::system().uiLanguages(); }
inline QString nativeLanguageName(const QString &locale) { return QLocale(locale).nativeLanguageName(); }
inline QString numberText(const QString &locale, double value, int precision) {
    return QLocale(locale).toString(value, 'f', precision);
}
inline QString dateText(const QString &locale, const QString &value) {
    return QLocale(locale).toString(QDateTime::fromString(value, Qt::ISODate).toLocalTime().date(), QLocale::ShortFormat);
}
inline QString timeText() { return QTime::currentTime().toString("HH:mm:ss"); }
}
