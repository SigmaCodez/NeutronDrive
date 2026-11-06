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
test('abort interrupts a paced stream while its source read is pending', async () => {
    let cancelled: unknown;
    let started!: () => void;
    const reading = new Promise<void>(resolve => { started = resolve; });
    const source = new ReadableStream<Uint8Array>({ pull() { started(); }, cancel(reason) { cancelled = reason; } }, { highWaterMark: 0 });
    const abort = new AbortController();
    const reader = pacedStream(source, new BytePacer(1024), abort.signal).getReader();
    const reason = new Error('Transfer stopped');
    const pending = reader.read().then(() => 'unexpected completion', error => error);
    await reading;
    abort.abort(reason);
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
        const outcome = await Promise.race([pending, new Promise(resolve => { timer = setTimeout(() => resolve('timeout'), 100); })]);
        expect(outcome).toBe(reason);
        expect(cancelled).toBe(reason);
    } finally {
        clearTimeout(timer);
        await reader.cancel().catch(() => {});
    }
});
test('paced stream cleans up abort handling after completion and consumer cancellation', async () => {
    const immediate = () => new BytePacer(1024, { now: () => 0, wait: async () => {} });
    const reason = new Error('Stopped');
    for (const alreadyAborted of [false, true]) {
        let cancelled: unknown;
        const abort = new AbortController();
        if (alreadyAborted) abort.abort(reason);
        const source = new ReadableStream<Uint8Array>({ cancel(value) { cancelled = value; } });
        const reader = pacedStream(source, immediate(), abort.signal).getReader();
        if (alreadyAborted) await expect(reader.read()).rejects.toBe(reason);
        else await reader.cancel(reason);
        expect(cancelled).toBe(reason);
        abort.abort(reason);
    }
    const abort = new AbortController();
    const bytes = new Uint8Array([1, 2, 3]);
    const result = await new Response(pacedStream(new Response(bytes).body!, immediate(), abort.signal)).arrayBuffer();
    expect(new Uint8Array(result)).toEqual(bytes);
    // A completed stream has released its source lock and must not be cancelled again.
    abort.abort(reason);
});
