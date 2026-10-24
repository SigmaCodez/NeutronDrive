/** NeutronDrive entry point; upstream auth, session, cache and commands retained. */
import { FeatureFlags } from '@protontech/drive-sdk';
import { applyDefaultCliOptions, run } from './cli';
import { COMMANDS } from './commands';
import { CommandEventsPoll, CommandEventsWatch } from './eventCommands';

import { CommandComputerCreate, CommandComputerInfo } from './computerCommands';

import { CommandVersions, CommandRestoreVersion, CommandPhotoPage, CommandPhotoLibrary, CommandAccountStorage } from './libraryCommands';

declare const APP_VERSION: string;
declare const SDK_VERSION: string | undefined;

const eventMode = Bun.argv[2] === 'events';
await run([...COMMANDS, ...applyDefaultCliOptions([new CommandEventsPoll(), new CommandEventsWatch(), new CommandComputerCreate(), new CommandComputerInfo(), new CommandVersions(), new CommandRestoreVersion(), new CommandPhotoPage(), new CommandPhotoLibrary(), new CommandAccountStorage()])], {
    appVersion: APP_VERSION,
    sdkVersion: SDK_VERSION,
    clientUidPrefix: 'neutrondrive',
    // Events mode uses SDK iterateEvents with consumer-owned cursors. Starting
    // the legacy manager too would consume events before our listener exists.
    enablePersistedEvents: !eventMode,
    enableMetrics: false,
    flags: { [FeatureFlags.DriveSmallFileUpload]: true },
});
process.exit(0);
