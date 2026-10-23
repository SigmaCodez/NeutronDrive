# Native Qt runtime and replacement

NeutronDrive's own application source remains MIT. This native package includes
Qt libraries, plugins, QML modules and translations extracted from the official
Qt/PySide6 wheels pinned in qt-lock.json. It does not include the PySide6 or
Shiboken Python bindings, and the application does not require Python at runtime.
Selected upstream binaries are not modified.

Matching source archives are recorded in qt-sources-lock.json. Their license
texts and third-party notices are included under share/licenses/neutrondrive/qt.
The LGPL/GPL/commercial alternatives of upstream components apply; no commercial
Qt license is claimed. Qt Quick/QML, Controls and Dialogs come from the pinned
qtdeclarative source archive.

Qt source and license information:
https://download.qt.io/archive/qt/6.10/6.10.2/

The native executable dynamically loads Qt from lib/neutrondrive/qt. Users may
replace those libraries and plugins with compatible modified versions for use
and debugging. Keep the Qt ABI and platform plugins consistent. Build-time wheel
hash checks select the distributed payload; they do not prevent user replacement
of runtime libraries. Native frontend build provenance and architecture-specific
runtime compatibility must be checked separately before publishing a package.
