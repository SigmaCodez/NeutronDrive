Name:           neutrondrive
Epoch:          1
Version:        1.0.11
Release:        1%{?dist}
Summary:        Independent Qt client for Proton Drive
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
# Persist the RPM build family; never select a different distribution's asset.
%if 0%{?suse_version}
printf 'rpm-opensuse\n' > %{buildroot}/usr/lib/neutrondrive/package-format
%else
printf 'rpm-fedora\n' > %{buildroot}/usr/lib/neutrondrive/package-format
%endif

%files
/usr/bin/neutrondrive
/usr/share/dbus-1/services/io.github.SigmaCodez.NeutronDrive.Service.service
/usr/lib/systemd/user/neutrondrive-background.service
/usr/lib/neutrondrive/
%license /usr/share/licenses/neutrondrive/
%doc /usr/share/doc/neutrondrive/
/usr/lib/nautilus/
/usr/lib/nemo/
/usr/lib/caja/
/usr/lib/thunarx-3/
/usr/lib64/nautilus/
/usr/lib64/nemo/
/usr/lib64/caja/
/usr/lib64/thunarx-3/
/usr/lib/*-linux-gnu/nautilus/
/usr/lib/*-linux-gnu/nemo/
/usr/lib/*-linux-gnu/caja/
/usr/lib/*-linux-gnu/thunarx-3/
/usr/share/file-manager/actions/neutrondrive-*.desktop
/usr/share/kio/servicemenus/neutrondrive.desktop
/usr/share/applications/io.github.SigmaCodez.NeutronDrive.desktop
/usr/share/metainfo/io.github.SigmaCodez.NeutronDrive.metainfo.xml
/usr/share/icons/hicolor/scalable/apps/io.github.SigmaCodez.NeutronDrive.svg
/usr/share/icons/hicolor/scalable/emblems/neutrondrive-available.svg
/usr/share/icons/hicolor/scalable/emblems/neutrondrive-shared.svg
/usr/share/icons/hicolor/scalable/emblems/neutrondrive-syncing.svg
/usr/share/neutrondrive/integrations/dolphin/

%changelog
* Sun Oct 25 2026 SigmaCodez <maintainers@users.noreply.github.com> - 0.6.0~alpha11-1
- Cached metadata browsing, tray startup and native availability emblems

* Sat Oct 24 2026 SigmaCodez <maintainers@users.noreply.github.com> - 0.6.0~alpha10-1
- Guided placeholder sync, recursive offline folders, Photos albums and sharing.

* Sat Oct 24 2026 SigmaCodez <maintainers@users.noreply.github.com> - 0.6.0~alpha8-1
- Complete native Rust application runtime with retained Qt Quick UI and Proton SDK.

* Tue Oct 20 2026 SigmaCodez <maintainers@users.noreply.github.com> - 0.6.0~alpha6-1
- Background sync, on-demand FUSE files and source-built SDK event bridge.
