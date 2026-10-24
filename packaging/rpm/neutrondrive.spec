Name:           neutrondrive
Version:        0.6.0~alpha8
Release:        1%{?dist}
Summary:        Independent Qt client for Proton Drive (alpha)
License:        MIT
URL:            https://github.com/SigmaCodez/NeutronDrive
Source0:        neutrondrive-payload.tar.gz
ExclusiveArch:  x86_64 aarch64
Requires:       xdg-utils
Requires:       ca-certificates
Requires:       libEGL.so.1()(64bit)
Requires:       libGL.so.1()(64bit)
Requires:       libOpenGL.so.0()(64bit)
Requires:       libxkbcommon-x11.so.0()(64bit)
Requires:       libxcb-cursor.so.0()(64bit)
Requires:       libxcb-icccm.so.4()(64bit)
Requires:       libxcb-image.so.0()(64bit)
Requires:       libxcb-keysyms.so.1()(64bit)
Requires:       libxcb-render-util.so.0()(64bit)
Requires:       libfontconfig.so.1()(64bit)
Requires:       libgssapi_krb5.so.2()(64bit)
Requires:       libbrotlidec.so.1()(64bit)
Requires:       libdbus-1.so.3()(64bit)
Requires:       libSM.so.6()(64bit)
Requires:       libICE.so.6()(64bit)
%ifarch aarch64
Requires:       libc.so.6(GLIBC_2.39)(64bit)
%else
Requires:       libc.so.6(GLIBC_2.34)(64bit)
%endif
Requires:       fuse3
Requires:       libsecret-1.so.0()(64bit)
Recommends:     gnome-keyring

# Proton's Bun executable has appended application data. Stripping or rewriting
# it corrupts the payload, so distribution BRP strip/debug generation is disabled.
%global debug_package %{nil}
%global __os_install_post %{nil}

# Bundled libraries are private to NeutronDrive's RUNPATH, not system Qt/ICU
# providers. Keep automatic requirements on external system libraries only.
%global __provides_exclude_from ^/usr/lib/neutrondrive/.*$
%global __requires_exclude ^(libQt6.*|libicu(data|i18n|uc)\\.so\\.73).*$

%description
Browse and transfer files with a native Qt interface and the bundled SDK-based NeutronDrive event bridge. Requires an account, browser and running Secret Service.
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
* Sat Oct 24 2026 SigmaCodez <maintainers@users.noreply.github.com> - 0.6.0~alpha8-1
- Complete native Rust application runtime with retained Qt Quick UI and Proton SDK.

* Tue Oct 20 2026 SigmaCodez <maintainers@users.noreply.github.com> - 0.6.0~alpha6-1
- Background sync, on-demand FUSE files and source-built SDK event bridge.
