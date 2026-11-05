/** Shared byte pacing for SDK HTTP streams. No credentials or payloads are inspected. */
export type Clock = { now(): number; wait(ms: number, signal?: AbortSignal): Promise<void> };
const clock: Clock = {
    now: () => performance.now(),
    wait: (ms, signal) => new Promise((resolve, reject) => {
        signal?.throwIfAborted();
        const abort = () => { clearTimeout(timer); reject(signal?.reason); };
        const timer = setTimeout(() => { signal?.removeEventListener('abort', abort); resolve(); }, ms);
        signal?.addEventListener('abort', abort, { once: true });
    }),
};
export class BytePacer {
    private next = 0;
    constructor(readonly bytesPerSecond: number, private readonly time: Clock = clock) {
        if (!Number.isSafeInteger(bytesPerSecond) || bytesPerSecond < 0 || bytesPerSecond > 1073741824) {
            throw new Error('Invalid transfer limit');
        }
    }
    async take(bytes: number, signal?: AbortSignal): Promise<void> {
        signal?.throwIfAborted();
        if (!this.bytesPerSecond) return;
        // Reserve synchronously: concurrent streams share one aggregate budget.
        const now = this.time.now();
        this.next = Math.max(now, this.next) + bytes * 1000 / this.bytesPerSecond;
        await this.time.wait(Math.max(0, this.next - now), signal);
        signal?.throwIfAborted();
    }
}
export function pacedStream(source: ReadableStream<Uint8Array>, pacer: BytePacer, signal?: AbortSignal): ReadableStream<Uint8Array> {
    if (!pacer.bytesPerSecond) return source;
    const reader = source.getReader();
    let pending: Uint8Array | undefined;
    let offset = 0;
    const stop = new AbortController();
    const combined = signal ? AbortSignal.any([signal, stop.signal]) : stop.signal;
    return new ReadableStream<Uint8Array>({
        async pull(controller) {
            try {
                combined.throwIfAborted();
                if (!pending || offset === pending.byteLength) {
                    const result = await reader.read();
                    if (result.done) { controller.close(); reader.releaseLock(); return; }
                    pending = result.value; offset = 0;
                }
                const chunk = pending.subarray(offset, offset + 16384);
                await pacer.take(chunk.byteLength, combined);
                offset += chunk.byteLength;
                controller.enqueue(chunk);
            } catch (error) {
                controller.error(error);
                await reader.cancel(error).catch(() => {});
            }
        },
        async cancel(reason) { stop.abort(reason); await reader.cancel(reason); },
    }, { highWaterMark: 0 });
}
export function installTransferLimits(uploadKib: number, downloadKib: number): void {
    const upload = new BytePacer(uploadKib * 1024);
    const download = new BytePacer(downloadKib * 1024);
    if (!upload.bytesPerSecond && !download.bytesPerSecond) return;
    const original = globalThis.fetch;
    globalThis.fetch = Object.assign(async (input: Parameters<typeof fetch>[0], init?: Parameters<typeof fetch>[1]): Promise<Response> => {
        let request = new Request(input, init);
        if (upload.bytesPerSecond && request.body) {
            request = new Request(request, { body: pacedStream(request.body, upload, request.signal), duplex: 'half' } as RequestInit);
        }
        const response = await original(request);
        if (!download.bytesPerSecond || !response.body) return response;
        const result = new Response(pacedStream(response.body, download, request.signal), {
            status: response.status, statusText: response.statusText, headers: response.headers,
        });
        for (const key of ['url', 'redirected', 'type'] as const) Object.defineProperty(result, key, { value: response[key] });
        return result;
    }, original);
}
