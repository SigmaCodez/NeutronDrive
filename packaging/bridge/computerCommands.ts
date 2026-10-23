import { DeviceType } from '@protontech/drive-sdk';
import { type ActionArgs, type Command } from './cli';

export class CommandComputerCreate implements Command {
    group = 'computers';
    name = 'create';
    help = 'Register a Linux computer in Proton Drive.';
    args = ['name'];
    async action({ sdk, args: [name] }: ActionArgs) {
        if (!name || name.length > 200 || /[\x00-\x1f/]/.test(name) || name === '.' || name === '..') throw new Error('Invalid computer name');
        for await (const existing of sdk.iterateDevices()) {
            if (existing.name.ok && existing.name.value === name) throw new Error('Computer name already exists');
        }
        const device = await sdk.createDevice(name, DeviceType.Linux);
        console.log(JSON.stringify({ uid: device.uid, rootFolderUid: device.rootFolderUid, name }));
    }
}

export class CommandComputerInfo implements Command {
    group = 'computers';
    name = 'info';
    help = 'Read the identity of a registered computer.';
    args = ['uid'];
    async action({ sdk, args: [uid] }: ActionArgs) {
        const device = await sdk.getDevice(uid);
        if (!device.name.ok) throw new Error('Computer name is unavailable');
        console.log(JSON.stringify({ uid: device.uid, rootFolderUid: device.rootFolderUid, name: device.name.value }));
    }
}
