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

/** Metadata inventory is separate from thumbnail hydration: scrolling never downloads originals. */
export class CommandPhotoLibrary implements Command {
    group = 'photo'; name = 'library'; args = ['request'];
    async action({ photosSdk, args: [raw] }: ActionArgs) {
        const request = JSON.parse(raw);
        if (!request || typeof request !== 'object' || Array.isArray(request)) throw new Error('Invalid photo request');
        const identity = (value: unknown): string => {
            if (typeof value !== 'string' || !value || value.length > 1024 || /[\x00-\x1f\x7f]/.test(value)) throw new Error('Invalid photo identity');
            return value;
        };
        const name = (value: unknown): string => {
            const result = identity(value).trim();
            if (!result || result.length > 255 || /[/\\]/.test(result)) throw new Error('Invalid album name');
            return result;
        };
        const selection = (): string[] => {
            if (!Array.isArray(request.ids) || !request.ids.length || request.ids.length > 48) throw new Error('Select between 1 and 48 photos');
            const ids: string[] = request.ids.map(identity);
            if (new Set(ids).size !== ids.length) throw new Error('Duplicate photo identity');
            return ids;
        };
        let result: unknown;
        switch (request.action) {
            case 'index': {
                const items: { uid: string }[] = [];
                const seen = new Set<string>();
                const iterator = request.album ? photosSdk.iterateAlbum(identity(request.album)) : photosSdk.iterateTimeline();
                for await (const item of iterator) {
                    if (seen.has(item.nodeUid)) continue;
                    seen.add(item.nodeUid); items.push({ uid: item.nodeUid });
                    if (items.length > 500000) throw new Error('Photo library exceeds the supported metadata limit');
                }
                result = { items, total: items.length }; break;
            }
            case 'details': {
                const ids = selection(); const thumbnails = new Map<string, string>();
                if (request.thumbnails !== false) {
                    try {
                        for await (const t of photosSdk.iterateThumbnails(ids)) {
                            if (t.ok && t.thumbnail.length <= 512 * 1024 && t.thumbnail[0] === 0xff && t.thumbnail[1] === 0xd8)
                                thumbnails.set(t.nodeUid, 'data:image/jpeg;base64,' + Buffer.from(t.thumbnail).toString('base64'));
                        }
                    } catch { /* Metadata and original downloads remain usable without previews. */ }
                }
                const nodes = new Map();
                for await (const node of photosSdk.iterateNodes(ids)) {
                    if ('missingUid' in node) continue;
                    const metadata = node.activeRevision?.claimedAdditionalMetadata as { Location?: { Latitude?: unknown; Longitude?: unknown } } | undefined;
                    const latitude = metadata?.Location?.Latitude, longitude = metadata?.Location?.Longitude;
                    const location = typeof latitude === 'number' && Number.isFinite(latitude) && Math.abs(latitude) <= 90 &&
                        typeof longitude === 'number' && Number.isFinite(longitude) && Math.abs(longitude) <= 180 ? { latitude, longitude } : null;
                    nodes.set(node.uid, { uid: node.uid, name: node.name.ok ? node.name.value : '[Unavailable name]', safe: node.name.ok,
                        modified: node.photo?.captureTime ?? node.activeRevision?.claimedModificationTime, size: node.activeRevision?.claimedSize,
                        thumbnail: thumbnails.get(node.uid) || '', location, albums: node.photo?.albums ?? [] });
                }
                result = { photos: ids.filter(id => nodes.has(id)).map(id => nodes.get(id)), missing: ids.filter(id => !nodes.has(id)) }; break;
            }
            case 'albums': {
                const albums = [];
                for await (const node of photosSdk.iterateAlbums()) {
                    albums.push({ uid: node.uid, name: node.name.ok ? node.name.value : '[Unavailable name]', safe: node.name.ok,
                        count: node.album?.photoCount ?? 0, shared: node.isShared, cover: node.album?.coverPhotoNodeUid ?? '' });
                    if (albums.length > 10000) throw new Error('Album list exceeds the supported limit');
                }
                result = { albums }; break;
            }
            case 'create': result = { uid: (await photosSdk.createAlbum(name(request.name))).uid }; break;
            case 'rename': result = { uid: (await photosSdk.updateAlbum(identity(request.album), { name: name(request.name) })).uid }; break;
            case 'cover': result = { uid: (await photosSdk.updateAlbum(identity(request.album), { coverPhotoNodeUid: identity(request.photo) })).uid }; break;
            case 'delete':
                // Never force-delete photos that only exist in an album.
                await photosSdk.deleteAlbum(identity(request.album), { saveToTimeline: true }); result = { accepted: true }; break;
            case 'add':
            case 'remove': {
                const ids = selection(); const album = identity(request.album); const results = [];
                const iterator = request.action === 'add' ? photosSdk.addPhotosToAlbum(album, ids) : photosSdk.removePhotosFromAlbum(album, ids);
                for await (const entry of iterator) results.push(entry);
                if (results.length !== ids.length || results.some(entry => !entry.ok)) throw new Error('Some album changes failed; refresh to inspect the result');
                result = { accepted: true }; break;
            }
            default: throw new Error('Unsupported photo library action');
        }
        console.log(JSON.stringify(result));
    }
}

/** Read only quota fields through the upstream authenticated API client. */
export class CommandAccountStorage implements Command {
    group = 'account'; name = 'storage';
    async action({ apiClient }: ActionArgs) {
        if (!apiClient) throw new Error('Account API is unavailable');
        const response = await apiClient.authenticatedRequest.get(`${apiClient.baseUrlWithProtocol}/core/v4/users`).json<{ User?: { MaxSpace?: number; UsedSpace?: number } }>();
        const total = response.User?.MaxSpace;
        const used = response.User?.UsedSpace;
        if (!Number.isSafeInteger(total) || !Number.isSafeInteger(used) || total! < 0 || used! < 0) throw new Error('Invalid account storage response');
        console.log(JSON.stringify({ total, used, free: Math.max(0, total! - used!) }));
    }
}
