import { afterEach, expect, test } from 'bun:test';
import { BytePacer, installTransferLimits, pacedStream } from './transferLimits';
const original = globalThis.fetch;
afterEach(() => { globalThis.fetch = original; });
test('parallel streams reserve one aggregate budget and unlimited is immediate', async () => {
    const waits: number[] = [];
    const pacer = new BytePacer(1024, { now: () => 0, wait: async ms => { waits.push(ms); } });
    await Promise.all([pacer.take(1024), pacer.take(2048)]);
    expect(waits).toEqual([1000, 3000]);
    await new BytePacer(0, { now: () => 0, wait: async () => { throw new Error('Unexpected wait'); } }).take(999);
    expect(() => new BytePacer(-1)).toThrow();
});
test('stream pacing preserves bytes, bounds chunks and supports cancellation', async () => {
    const bytes = new Uint8Array(65537).fill(42);
    const source = new Response(bytes).body!;
    const sizes: number[] = [];
    const pacer = new BytePacer(1024, { now: () => 0, wait: async ms => { sizes.push(ms); } });
    expect(new Uint8Array(await new Response(pacedStream(source, pacer)).arrayBuffer())).toEqual(bytes);
    expect(sizes.length).toBe(5);
    const abort = new AbortController();
    const pending = new BytePacer(1).take(1000, abort.signal);
    abort.abort(new Error('Cancelled'));
    await expect(pending).rejects.toThrow('Cancelled');
});
test('real HTTP upload and download are paced without changing body or status', async () => {
    const body = new Uint8Array(65536).fill(7);
    const server = Bun.serve({ port: 0, hostname: '127.0.0.1', async fetch(request) {
        const received = new Uint8Array(await request.arrayBuffer());
        expect(received).toEqual(body);
        return new Response(received, { status: 201, headers: { 'x-test': 'kept' } });
    } });
    try {
        installTransferLimits(256, 256);
        const start = performance.now();
        const response = await fetch(server.url, { method: 'POST', body });
        expect(new Uint8Array(await response.arrayBuffer())).toEqual(body);
        expect(response.status).toBe(201);
        expect(response.headers.get('x-test')).toBe('kept');
        expect(response.url).toContain('127.0.0.1');
        expect(performance.now() - start).toBeGreaterThan(400);
    } finally { server.stop(true); }
});
