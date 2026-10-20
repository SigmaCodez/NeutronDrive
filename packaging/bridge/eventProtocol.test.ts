import { describe, expect, test } from 'bun:test';
import { parseCursors, pollEvents, retryDelay, type DriveEvent, type EventSDK } from './eventProtocol';

const roots = [{ path: '/my-files', uid: 'root-uid', scopeId: 'scope' }];
function sdk(events: DriveEvent[], fail = false): EventSDK {
    return { async *iterateEvents() { yield* events; if (fail) throw new Error('private upstream response'); } };
}
const changed = { type: 'node_updated', treeEventScopeId: 'scope', eventId: 'next', nodeUid: 'node', parentNodeUid: 'parent' };

describe('SDK event bridge contract', () => {
    test('bootstrap fast-forward requests one reconciliation and exposes cursor', async () => {
        const batch = await pollEvents(sdk([{ type: 'fast_forward', treeEventScopeId: 'scope', eventId: 'initial' }]), roots, {});
        expect(batch.resets).toEqual(['scope']);
        expect(batch.cursors).toEqual({ scope: 'initial' });
        expect(batch.roots).toEqual(roots);
    });
    test('saved cursor reaches SDK; duplicate scope is queried only once', async () => {
        let calls = 0;
        const source: EventSDK = { async *iterateEvents(scope, cursor) {
            calls++; expect(scope).toBe('scope'); expect(cursor).toBe('old'); yield changed;
        } };
        const batch = await pollEvents(source, [...roots, { ...roots[0], uid: 'other-root' }], { scope: 'old' });
        expect(calls).toBe(1);
        expect(batch.events).toEqual([changed]);
        expect(batch.cursors).toEqual({ scope: 'next' });
        expect(batch.resets).toEqual([]);
    });
    test('partial failed page is never returned as an acknowledged batch', async () => {
        await expect(pollEvents(sdk([changed], true), roots, { scope: 'old' })).rejects.toThrow();
    });
    test('refresh requests full recovery; removed scope is not persisted', async () => {
        const refresh = await pollEvents(sdk([{ ...changed, type: 'tree_refresh' }]), roots, { scope: 'old' });
        expect(refresh.resets).toEqual(['scope']);
        const removed = await pollEvents(sdk([{ ...changed, type: 'tree_remove', eventId: 'none' }], true), roots, { scope: 'old' });
        expect(removed.cursors).toEqual({});
        expect(removed.resets).toEqual(['scope']);
    });
    test('unknown event/scope is rejected without persisting a cursor', async () => {
        await expect(pollEvents(sdk([{ ...changed, type: 'new-unknown-event' }]), roots, {})).rejects.toThrow();
        await expect(pollEvents(sdk([{ ...changed, treeEventScopeId: 'wrong' }]), roots, {})).rejects.toThrow();
    });
    test('unrelated fields are excluded from output', async () => {
        const event = { ...changed, credentials: 'never-export' };
        const batch = await pollEvents(sdk([event]), roots, { scope: 'old' });
        expect(JSON.stringify(batch)).not.toContain('never-export');
    });
    test('aborted query never acknowledges events', async () => {
        const controller = new AbortController(); controller.abort();
        await expect(pollEvents(sdk([changed]), roots, {}, controller.signal)).rejects.toThrow();
        await expect(pollEvents(sdk([]), roots, { scope: 'old' }, controller.signal)).rejects.toThrow();
    });
    test('cursor map validates shape and resource limits', () => {
        expect(parseCursors('{"scope":"old"}')).toEqual({ scope: 'old' });
        for (const input of ['null', '[]', '{"scope":2}', '{"scope":""}', 'x'.repeat(65537)]) {
            expect(() => parseCursors(input)).toThrow();
        }
    });
    test('failure backoff is bounded and never polls faster than foreground SDK', () => {
        expect(retryDelay(0, () => 0)).toBe(30000);
        expect(retryDelay(2, () => 0)).toBe(120000);
        expect(retryDelay(100, () => 0)).toBe(600000);
    });
});
