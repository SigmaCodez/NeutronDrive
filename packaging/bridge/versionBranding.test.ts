// Copied into the pinned CLI's src/ directory by scripts/build-backend.
import { expect, spyOn, test } from 'bun:test';
import { printVersions } from './cli/version';

test('third-party bridge version never queries or recommends official CLI updates', async () => {
    const fetch = spyOn(globalThis, 'fetch').mockRejectedValue(new Error('No network allowed'));
    const log = spyOn(console, 'log').mockImplementation(() => {});
    try {
        await printVersions({ appVersion: 'external-drive-neutrondrive@0.2.0-alpha', sdkVersion: 'js@test', clientUidPrefix: 'test' });
        expect(fetch).not.toHaveBeenCalled();
        expect(log.mock.calls).toEqual([
            ['Proton Drive CLI external-drive-neutrondrive@0.2.0-alpha'],
            ['Proton Drive SDK js@test'],
        ]);
    } finally {
        fetch.mockRestore();
        log.mockRestore();
    }
});
