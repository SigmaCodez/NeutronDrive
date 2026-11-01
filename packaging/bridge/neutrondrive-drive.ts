/** NeutronDrive entry point; upstream auth, session, cache and commands retained. */
import { createHash } from 'node:crypto';
import { getConfig } from './config';
import { initCredentials } from './credentials';
import { CommandSharingConfigure } from './sharingCommands';
import { flushOutput } from './flushOutput';
import { FeatureFlags } from '@protontech/drive-sdk';
import { applyDefaultCliOptions, run } from './cli';
import { COMMANDS } from './commands';
import { CommandEventsPoll, CommandEventsWatch } from './eventCommands';

import { CommandComputerCreate, CommandComputerInfo, CommandComputerRemove } from './computerCommands';

import { CommandPhotoImport, CommandVersions, CommandRestoreVersion, CommandPhotoPage, CommandPhotoLibrary, CommandAccountStorage } from './libraryCommands';

declare const APP_VERSION: string;
declare const SDK_VERSION: string | undefined;

// Local-only cache namespace: never initialize the SDK, event manager or network.
// No tokens, passwords or raw session identifiers leave the credential boundary.
if (Bun.argv[2] === 'cache' && Bun.argv[3] === 'identity') {
    try {
        const config = getConfig({ appVersion: APP_VERSION, clientUidPrefix: 'neutrondrive' });
        const silent = { debug() {}, info() {}, warn() {}, error() {} };
        const credentials = initCredentials(config, silent);
        await credentials.load();
        const namespace = credentials.isLoggedIn() && credentials.uid
            ? createHash('sha256').update(JSON.stringify(['neutrondrive-views-v1', config.baseUrl, credentials.uid])).digest('hex') : null;
        console.log(JSON.stringify({ namespace }));
        await flushOutput();
        process.exit(0);
    } catch {
        console.error('Local account cache identity unavailable');
        process.exit(1);
    }
}
const eventMode = Bun.argv[2] === 'events';
await run([...COMMANDS, ...applyDefaultCliOptions([new CommandSharingConfigure(), new CommandEventsPoll(), new CommandEventsWatch(), new CommandComputerCreate(), new CommandComputerInfo(), new CommandComputerRemove(), new CommandVersions(), new CommandRestoreVersion(), new CommandPhotoImport(), new CommandPhotoPage(), new CommandPhotoLibrary(), new CommandAccountStorage()])], {
    appVersion: APP_VERSION,
    sdkVersion: SDK_VERSION,
    clientUidPrefix: 'neutrondrive',
    // Events mode uses SDK iterateEvents with consumer-owned cursors. Starting
    // the legacy manager too would consume events before our listener exists.
    enablePersistedEvents: !eventMode,
    enableMetrics: false,
    flags: { [FeatureFlags.DriveSmallFileUpload]: true },
});
await flushOutput();
process.exit(0);
