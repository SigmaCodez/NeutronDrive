/** Additive event contract. No authentication or HTTP implementation lives here. */
export type EventCursor = Record<string, string>;
export type Root = { path: string; uid: string; scopeId: string };
export type DriveEvent = {
    type: string;
    treeEventScopeId: string;
    eventId: string;
    nodeUid?: string;
    parentNodeUid?: string;
    isTrashed?: boolean;
    isShared?: boolean;
};
export type EventSDK = {
    iterateEvents(scope: string, cursor?: string, signal?: AbortSignal): AsyncIterable<DriveEvent>;
};
export type Batch = {
    protocol: 1;
    type: 'batch';
    roots: Root[];
    events: DriveEvent[];
    cursors: EventCursor;
    resets: string[];
};

const EVENT_TYPES = new Set([
    'node_created', 'node_updated', 'node_deleted', 'tree_refresh', 'tree_remove',
    'fast_forward', 'shared_with_me_updated',
]);

export function parseCursors(raw: string): EventCursor {
    if (raw.length > 64 * 1024) throw new Error('Event cursor input exceeds 64 KiB');
    const value = JSON.parse(raw);
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Invalid event cursor map');
    const entries = Object.entries(value);
    if (entries.length > 64) throw new Error('Too many event scopes');
    for (const [scope, cursor] of entries) {
        if (!scope || scope.length > 512 || typeof cursor !== 'string' || !cursor || cursor.length > 4096) {
            throw new Error('Invalid event cursor');
        }
    }
    return Object.fromEntries(entries) as EventCursor;
}

/** Return a complete batch only. A failed page never acknowledges partial data. */
export async function pollEvents(
    sdk: EventSDK, roots: Root[], saved: EventCursor, signal?: AbortSignal,
): Promise<Batch> {
    if (roots.length === 0 || roots.length > 64) throw new Error('One to 64 roots are required');
    if (signal?.aborted) throw new Error('Event query aborted');
    const batch: Batch = { protocol: 1, type: 'batch', roots, events: [], cursors: {}, resets: [] };
    for (const scope of new Set(roots.map(root => root.scopeId))) {
        if (!scope) throw new Error('Root has no event scope');
        let cursor = saved[scope];
        if (!cursor) batch.resets.push(scope);
        let removed = false;
        for await (const event of sdk.iterateEvents(scope, cursor, signal)) {
            if (signal?.aborted) throw new Error('Event query aborted');
            if (event.treeEventScopeId !== scope || !EVENT_TYPES.has(event.type) || !event.eventId) {
                throw new Error('Unexpected SDK event contract');
            }
            // The public event shape contains identifiers only, never names,
            // credentials, file contents or raw server error responses.
            batch.events.push({
                type: event.type, treeEventScopeId: scope, eventId: event.eventId,
                ...(event.nodeUid ? { nodeUid: event.nodeUid } : {}),
                ...(event.parentNodeUid ? { parentNodeUid: event.parentNodeUid } : {}),
                ...(typeof event.isTrashed === 'boolean' ? { isTrashed: event.isTrashed } : {}),
                ...(typeof event.isShared === 'boolean' ? { isShared: event.isShared } : {}),
            });
            if (batch.events.length > 100000) throw new Error('Event batch too large; full recovery is required');
            cursor = event.eventId;
            if (event.type === 'tree_refresh' || event.type === 'tree_remove') batch.resets.push(scope);
            if (event.type === 'tree_remove') { removed = true; break; }
        }
        // The pinned SDK yields tree_remove then throws its NotFound error.
        // Breaking the generator above retains the explicit removal signal.
        if (signal?.aborted) throw new Error('Event query aborted');
        if (!removed && cursor) batch.cursors[scope] = cursor;
    }
    batch.resets = [...new Set(batch.resets)];
    return batch;
}

export function retryDelay(attempt: number, random = Math.random): number {
    return Math.min(600000, 30000 * 2 ** Math.min(Math.max(attempt, 0), 5)) + Math.floor(random() * 1000);
}
