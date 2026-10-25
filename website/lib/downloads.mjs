const extensions = { DEB: /\.deb$/i, RPM: /\.rpm$/i, AppImage: /\.appimage$/i, Flatpak: /\.flatpak$/i, Snap: /\.snap$/i, Arch: /\.pkg\.tar\.(?:zst|xz)$/i };
export function packageAssets(release, format) {
  if (!release || !Array.isArray(release.assets) || !extensions[format]) return [];
  return release.assets.filter(asset => {
    if (!asset || asset.state !== 'uploaded' || !(asset.size > 0) || typeof asset.name !== 'string' || !/^neutrondrive[-_]/i.test(asset.name) || !extensions[format].test(asset.name)) return false;
    try {
      const url = new URL(asset.browser_download_url);
      return url.protocol === 'https:' && url.host === 'github.com' && !url.username && !url.password && !url.search && !url.hash &&
        decodeURIComponent(url.pathname) === `/SigmaCodez/NeutronDrive/releases/download/${release.tag_name}/${asset.name}`;
    } catch { return false; }
  }).sort((a,b) => {
    const rank = name => /(?:amd64|x86_64|x64)/i.test(name) ? 0 : 1;
    return rank(a.name)-rank(b.name) || a.name.localeCompare(b.name,'en');
  });
}
