Name:           neutrondrive
Version:        0.1.0~alpha1
Release:        1%{?dist}
Summary:        Independent GTK client for Proton Drive (alpha)
License:        MIT
URL:            https://github.com/SigmaCodez/NeutronDrive
Source0:        neutrondrive-payload.tar.gz
ExclusiveArch:  x86_64 aarch64
Requires:       python3 >= 3.10
Requires:       python3-gobject
Requires:       xdg-utils
Requires:       ca-certificates
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
Browse and transfer files with a native GTK interface and the bundled official
Proton Drive CLI. Requires an account, browser and running Secret Service.
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
/usr/lib/neutrondrive/
%license /usr/share/licenses/neutrondrive/
%doc /usr/share/doc/neutrondrive/
/usr/share/applications/io.github.SigmaCodez.NeutronDrive.desktop
/usr/share/metainfo/io.github.SigmaCodez.NeutronDrive.metainfo.xml
/usr/share/icons/hicolor/scalable/apps/io.github.SigmaCodez.NeutronDrive.svg

%changelog
* Tue Oct 20 2026 SigmaCodez <maintainers@users.noreply.github.com> - 0.1.0~alpha1-1
- Initial alpha packaging; bundled upstream CLI 0.9.0.
