# Dolphin integration addon

This thin KIO adapter supplies green availability overlays and a **Proton Drive**
Places bookmark. It hides the matching device entry without changing other
volumes. Cache decisions and path validation remain in the Rust library.
The existing Dolphin service menu supplies Keep on device, Free up space and Share.

Build against the **host's** Qt and KDE Frameworks, not NeutronDrive's bundled Qt.
Install your distribution's CMake, C++ compiler, Extra CMake Modules and KIO
 development packages. For Dolphin 6:

```sh
cmake -S /usr/share/neutrondrive/integrations/dolphin -B /tmp/neutrondrive-kde \
  -DNEUTRONDRIVE_KF_MAJOR=6 -DCMAKE_INSTALL_PREFIX=/usr
cmake --build /tmp/neutrondrive-kde
sudo cmake --install /tmp/neutrondrive-kde
```

Use `-DNEUTRONDRIVE_KF_MAJOR=5` for Dolphin 5. Distribution packages can install
with `DESTDIR` instead of using sudo. Restart Dolphin after installation.
The native NeutronDrive host package must provide
`/usr/lib/neutrondrive/bin/libneutrondrive_file_manager.so` and the emblem icon.
This is an optional host addon, not a plugin injected from Flatpak/Snap/AppImage.

KDE 6 has been compiled and exercised with a real KIO plugin loader. KDE 5 is a
build target but has not yet been runtime-verified in this development change.
