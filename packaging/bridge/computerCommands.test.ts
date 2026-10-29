import { describe, expect, test } from 'bun:test';
import { CommandComputerCreate, CommandComputerInfo, CommandComputerRemove } from './computerCommands';

function context(name: string, existing: string[] = []) {
    const calls: unknown[] = [];
    return {
        calls,
        context: {
            args: [name],
            sdk: {
                async *iterateDevices() { for (const value of existing) yield { name: { ok: true, value } }; },
                async createDevice(value: string, type: string) { calls.push([value, type]); return { uid: 'device', rootFolderUid: 'root' }; },
                async getDevice(uid: string) { calls.push(uid); return { uid, rootFolderUid: 'root', name: { ok: true, value: 'Linux' } }; },
            },
        } as any,
    };
}
describe('computer registration', () => {
    test('Linux device is created using the SDK', async () => {
        const { calls, context: ctx } = context('Linux');
        await new CommandComputerCreate().action(ctx);
        expect(calls).toEqual([['Linux', 'Linux']]);
    });
    test('duplicates and malformed names never create a device', async () => {
        for (const name of ['', '../x', '..', 'a\0b', 'Linux']) {
            const { calls, context: ctx } = context(name, ['Linux']);
            await expect(new CommandComputerCreate().action(ctx)).rejects.toThrow();
            expect(calls).toEqual([]);
        }
    });
    test('device identity is read via SDK UID', async () => {
        const { calls, context: ctx } = context('device');
        await new CommandComputerInfo().action(ctx);
        expect(calls).toEqual(['device']);
    });
});

test('computer removal verifies the exact root before SDK deletion', async () => {
    for (const root of ['root', 'replacement']) {
        const calls: string[] = [];
        const ctx: any = {args:['device','root'],sdk:{getDevice:async()=>({uid:'device',rootFolderUid:root}),deleteDevice:async(uid:string)=>{calls.push(uid);}}};
        if (root === 'root') await new CommandComputerRemove().action(ctx);
        else await expect(new CommandComputerRemove().action(ctx)).rejects.toThrow('identity changed');
        expect(calls).toEqual(root === 'root' ? ['device'] : []);
    }
});
