/** Additive public-SDK operations; account and cryptography stay upstream. */
import { type ActionArgs, type Command } from './cli';

export class CommandVersions implements Command {
    group = 'versions'; name = 'list'; args = ['path'];
    async action({ sdk, paths, args: [path] }: ActionArgs) {
        const node = await paths.getPath(path).getNode();
        if (node.type !== 'file' || !node.activeRevision) throw new Error('Version history requires a file');
        const revisions = [];
        for await (const revision of sdk.iterateRevisions(node.uid)) {
            revisions.push({ uid: revision.uid, state: revision.state, created: revision.creationTime, size: revision.claimedSize });
            if (revisions.length > 10000) throw new Error('Version history exceeds the supported limit');
        }
        console.log(JSON.stringify({ uid: node.uid, active: node.activeRevision.uid, revisions }));
    }
}
export class CommandRestoreVersion implements Command {
    group = 'versions'; name = 'restore'; args = ['path', 'revision', 'expected'];
    async action({ sdk, paths, args: [path, revision, expected] }: ActionArgs) {
        const node = await paths.getPath(path).getNode();
        if (node.type !== 'file' || !expected || node.activeRevision?.uid !== expected) throw new Error('File changed; reload version history before restoring');
        let found = false;
        for await (const entry of sdk.iterateRevisions(node.uid)) if (entry.uid === revision && entry.state === 'superseded') found = true;
        if (!found) throw new Error('Selected historical version is no longer available');
        const current = await sdk.getNode(node.uid);
        if (current.activeRevision?.uid !== expected) throw new Error('File changed while loading history; reload before restoring');
        await sdk.restoreRevision(revision);
        // The SDK explicitly documents eventual server application of restores.
        console.log(JSON.stringify({ accepted: true }));
    }
}
export class CommandPhotoPage implements Command {
    group = 'photo'; name = 'page'; args = ['offset'];
    async action({ photosSdk, args: [raw] }: ActionArgs) {
        if (!/^\d+$/.test(raw || '')) throw new Error('Invalid photo offset');
        const offset = Number(raw);
        if (!Number.isSafeInteger(offset)) throw new Error('Invalid photo offset');
        const ids: string[] = []; let index = 0; let more = false;
        for await (const item of photosSdk.iterateTimeline()) {
            if (index++ < offset) continue;
            if (ids.length === 48) { more = true; break; }
            ids.push(item.nodeUid);
        }
        const thumbnails = new Map<string, string>();
        // Missing thumbnails never make the original photo unavailable.
        try {
            for await (const thumb of photosSdk.iterateThumbnails(ids)) {
                if (thumb.ok && thumb.thumbnail.length <= 512 * 1024 && thumb.thumbnail[0] === 0xff && thumb.thumbnail[1] === 0xd8)
                    thumbnails.set(thumb.nodeUid, 'data:image/jpeg;base64,' + Buffer.from(thumb.thumbnail).toString('base64'));
            }
        } catch { /* Display placeholders while originals remain downloadable. */ }
        const nodes = new Map();
        for await (const node of photosSdk.iterateNodes(ids)) {
            if ('missingUid' in node) continue;
            nodes.set(node.uid, { uid: node.uid, name: node.name.ok ? node.name.value : '[Unavailable name]', safe: node.name.ok,
                modified: node.activeRevision?.claimedModificationTime, size: node.activeRevision?.claimedSize,
                thumbnail: thumbnails.get(node.uid) || '' });
        }
        console.log(JSON.stringify({ offset, more, photos: ids.filter(id => nodes.has(id)).map(id => nodes.get(id)) }));
    }
}
