Name:           neutrondrive
Version:        0.4.0~alpha1
Release:        1%{?dist}
Summary:        Independent GTK client for Proton Drive (alpha)
%if 0%{?neutrondrive_vendor_pyfuse3}
License:        MIT AND LGPL-2.1-or-later
%else
License:        MIT
%endif
URL:            https://github.com/SigmaCodez/NeutronDrive
Source0:        neutrondrive-payload.tar.gz
ExclusiveArch:  x86_64 aarch64
Requires:       python3 >= 3.10
Requires:       python3-gobject
Requires:       xdg-utils
Requires:       ca-certificates
%if 0%{?neutrondrive_vendor_pyfuse3}
Requires:       python(abi) = %{neutrondrive_python_abi}
Provides:       bundled(python3dist(pyfuse3)) = 3.4.2
%else
Requires:       python3dist(pyfuse3)
%endif
Requires:       python3dist(trio)
Requires:       fuse3
%if 0%{?suse_version}
Requires:       python3-gobject-Gdk
Requires:       typelib-1_0-Gtk-4_0 >= 4.6
Requires:       libsecret-1-0
%else
Requires:       gtk4 >= 4.6
Requires:       libsecret
Recommends:     gnome-keyring
%endif

# Proton's Bun executable has appended application data. Stripping or rewriting
# it corrupts the payload, so distribution BRP strip/debug generation is disabled.
%global debug_package %{nil}
%global __os_install_post %{nil}

%description
Browse and transfer files with a native GTK interface and the bundled SDK-based NeutronDrive event bridge. Requires an account, browser and running Secret Service.
NeutronDrive is a community project not affiliated with Proton AG.

%prep
%setup -q -c

%build
# The immutable, checksum-verified upstream executable is included in Source0.

%install
mkdir -p %{buildroot}
cp -a usr %{buildroot}/

%files
/usr/bin/neutrondrive
/usr/share/dbus-1/services/io.github.SigmaCodez.NeutronDrive.Service.service
/usr/lib/systemd/user/neutrondrive-background.service
/usr/lib/neutrondrive/
%license /usr/share/licenses/neutrondrive/
%doc /usr/share/doc/neutrondrive/
/usr/share/applications/io.github.SigmaCodez.NeutronDrive.desktop
/usr/share/metainfo/io.github.SigmaCodez.NeutronDrive.metainfo.xml
/usr/share/icons/hicolor/scalable/apps/io.github.SigmaCodez.NeutronDrive.svg

%changelog
* Tue Oct 20 2026 SigmaCodez <maintainers@users.noreply.github.com> - 0.4.0~alpha1-1
- Background sync, on-demand FUSE files and source-built SDK event bridge.
