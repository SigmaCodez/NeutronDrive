import { MemberRole } from '@protontech/drive-sdk';
import { type ActionArgs, type Command, printObject } from './cli';

export function linkSnapshot(info: any) {
    const link = info?.urlAccess;
    return { uid: link?.uid || '', role: link?.role || 'viewer',
        password: link?.customPassword || '', expiration: link?.expirationTime ? new Date(link.expirationTime).toISOString() : '',
        editorsCanShare: info?.editorsCanShare ?? false };
}

export async function configureSharing(context: ActionArgs, raw: string) {
    if (Buffer.byteLength(raw) > 4096) throw new Error('Link settings exceed limit');
    const value = JSON.parse(raw);
    if (!value || typeof value !== 'object' || Array.isArray(value)
        || ![MemberRole.Viewer, MemberRole.Editor].includes(value.role)
        || typeof value.password !== 'string' || value.password.length > 256
        || typeof value.expiration !== 'string' || typeof value.editorsCanShare !== 'boolean'
        || typeof value.nodeUid !== 'string' || !value.nodeUid || value.nodeUid.length > 1024
        || !value.expected || typeof value.expected !== 'object') throw new Error('Invalid link settings');
    let expiration: Date | undefined;
    if (value.expiration) {
        expiration = new Date(value.expiration);
        if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/.test(value.expiration)
            || !Number.isFinite(expiration.getTime()) || expiration.toISOString() !== value.expiration
            || expiration.getTime() <= Date.now()) throw new Error('Choose a future expiration date');
    }
    const nodePath = context.paths.getPath(context.args[0]);
    const node = await nodePath.getNode();
    if (node.uid !== value.nodeUid) throw new Error('File identity changed. Reopen sharing before saving.');
    const current = linkSnapshot(await nodePath.sdk.getSharingInfo(node));
    if (Object.entries(current).some(([key, item]) => value.expected[key] !== item))
        throw new Error('Sharing settings changed. Refresh before saving.');
    const result = await nodePath.sdk.shareNode(node, {
        urlAccess: { role: value.role, customPassword: value.password || undefined, expiration },
        editorsCanShare: value.editorsCanShare,
    });
    printObject(result, context.options.json);
}

export class CommandSharingConfigure implements Command {
    group = 'sharing'; name = 'configure'; args = ['path'];
    help = 'Configure public link settings from private JSON on stdin.';
    async action(context: ActionArgs) {
        await configureSharing(context, await Bun.stdin.text());
    }
}
