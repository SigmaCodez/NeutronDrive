import test from 'node:test';
import assert from 'node:assert/strict';
import { packageAssets } from '../lib/downloads.mjs';
const tag='v2-alpha';
const asset=name=>({name,state:'uploaded',size:100,browser_download_url:`https://github.com/SigmaCodez/NeutronDrive/releases/download/${tag}/${name}`});
const release={tag_name:tag,assets:['neutrondrive_2_arm64.deb','neutrondrive_2_amd64.deb','neutrondrive-2-fedora-x86_64.rpm','neutrondrive-2-opensuse-x86_64.rpm','NeutronDrive-2-x86_64.AppImage','neutrondrive-2-x86_64.flatpak','neutrondrive_2_amd64.snap','neutrondrive-bin-2-x86_64.pkg.tar.zst','neutrondrive-bin-2-aur.tar.gz','SHA256SUMS','other_2_amd64.deb'].map(asset)};
test('selects installable assets for every format without source archives or dependencies',()=>{
 for(const [format,count] of [['DEB',2],['RPM',2],['AppImage',1],['Flatpak',1],['Snap',1],['Arch',1]])assert.equal(packageAssets(release,format).length,count,format);
 assert.match(packageAssets(release,'DEB')[0].name,/amd64/);
});
test('does not substitute old releases, external URLs, missing or incomplete packages',()=>{
 const good=asset('neutrondrive_2_amd64.deb');
 const bad=[{...good,state:'new'},{...good,size:0},{...good,browser_download_url:good.browser_download_url.replace('v2-alpha','v1')},{...good,browser_download_url:'https://example.com/file.deb'},{...good,browser_download_url:good.browser_download_url+'?token=bad'}];
 assert.deepEqual(packageAssets({...release,assets:bad},'DEB'),[]);
 assert.deepEqual(packageAssets(null,'DEB'),[]);
 assert.deepEqual(packageAssets({tag_name:tag,assets:[]},'RPM'),[]);
 assert.deepEqual(packageAssets(release,'unknown'),[]);
});
test('accepts URL-encoded valid filenames',()=>{
 const a=asset('neutrondrive_2+build_amd64.deb');
 a.browser_download_url=a.browser_download_url.replace('+','%2B');
 assert.equal(packageAssets({tag_name:tag,assets:[a]},'DEB').length,1);
});
