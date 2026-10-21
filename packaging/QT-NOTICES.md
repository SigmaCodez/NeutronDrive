# Qt runtime and replacement

NeutronDrive's own source remains MIT. The private, dynamically linked Qt for
Python runtime is redistributed from official PySide6 Essentials and Shiboken wheels pinned in qt-lock.json.
Only the required Qt modules and desktop plugins are selected; included binaries
are not modified. The wheels omit license files, so upstream license texts and
third-party notices are collected from the matching source archives and included
under share/licenses/neutrondrive/qt. qt-sources-lock.json records the archives. LGPL/GPL/commercial alternatives apply to
upstream components; no commercial Qt license is claimed.

Qt for Python source: https://code.qt.io/pyside/pyside-setup.git (tag v6.10.2)
Qt source and license information: https://download.qt.io/archive/qt/6.10/6.10.2/
https://doc.qt.io/qtforpython-6/licenses.html

The runtime is dynamically loaded from lib/neutrondrive/qt. It is not embedded
into an executable. Users may replace it with compatible modified PySide6,
Shiboken and Qt libraries for debugging and use; keep ABI and platform plugins
consistent. No technical check forbids modified replacement libraries. Build-time
wheel checks pin the distributed payload, not user runtime modifications.

The x86_64 wheels require GLIBC 2.34; ARM64 wheels require GLIBC 2.39. Native
ARM64 Qt packages therefore require Ubuntu 24.04+ or an equivalent runtime.
Python 3.10–3.14 is supported by this application/runtime combination.
