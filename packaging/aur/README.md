# Arch Linux / Manjaro package

`neutrondrive-bin` repackages the NeutronDrive 1.0.7 DEBs with fixed SHA256
checksums, preserving the bundled Qt runtime, Rust core, bridge, language catalogs
and native D-Bus/systemd files. Host dependencies are resolved by pacman. It does
not run account, autostart or synchronization commands during installation.

The recipe declares x86_64 and aarch64. Native ARM64 requires GLIBC 2.39; the Arch
CI installation/runtime test currently covers x86_64 only. The installed
application uses Rust/Qt and has no Python runtime dependency.

## Install from this repository

Install `base-devel` and `git`, review the PKGBUILD, then run as a regular user:

```sh
git clone https://github.com/SigmaCodez/NeutronDrive.git
cd NeutronDrive/packaging/aur/neutrondrive-bin
makepkg -si
```

AUR hosts build recipes, not binary packages. The recipe is provided here and in
the GitHub release, but **an AUR entry has not yet been published** because an AUR
account/SSH key has not been configured. Do not use `yay -S neutrondrive-bin` until
an actual AUR listing is confirmed. The GitHub release may also provide the tested
x86_64 `.pkg.tar.zst`; install it with `sudo pacman -U ./neutrondrive-bin-*.pkg.tar.zst`.

## Publish to AUR

After registering an AUR account and adding your public SSH key, check that the
package name is available and clone `ssh://aur@aur.archlinux.org/neutrondrive-bin.git`.
Copy `PKGBUILD` and the generated `.SRCINFO` into that checkout, inspect the changes,
commit and push. Do not include binary packages or downloaded source files.
Regenerate metadata with `makepkg --printsrcinfo > .SRCINFO` after each recipe edit.

The dedicated CI job builds as an unprivileged user in Arch Linux, installs the
package with pacman, checks the native Rust/Qt runtime, opens all supported locales, and saves
the generated `.SRCINFO` alongside the binary. `namcap` provides additional package
linting; binary-bundled Qt can produce dependency suggestions which need review.
