import { type ActionArgs, type Command, type Options, PathType } from './cli';
import { parseCursors, pollEvents, retryDelay, type Root } from './eventProtocol';

const options: Options = {
    'cursor-json': { type: 'string', default: '{}', help: 'Previously acknowledged per-scope cursor map.' },
};

async function query({ sdk, paths, args, options }: ActionArgs, signal: AbortSignal) {
    const roots: Root[] = [];
    for (const path of args) {
        // v1 deliberately covers My files; Photos/core/shared scopes are not
        // silently treated as interchangeable event sources.
        const node = await paths.getNode(path, [PathType.MyFiles]);
        if (node.type !== 'folder') throw new Error('Event roots must be folders');
        roots.push({ path, uid: node.uid, scopeId: node.treeEventScopeId });
    }
    return pollEvents(sdk, roots, parseCursors(options['cursor-json']), signal);
}

export class CommandEventsPoll implements Command {
    group = 'events';
    name = 'poll';
    help = 'Read SDK Drive events since a cursor; one JSON batch, no recursive tree scan.';
    args = ['path...'];
    options = options;
    async action(args: ActionArgs) {
        const controller = new AbortController();
        const stop = () => controller.abort();
        process.on('SIGTERM', stop);
        process.on('SIGINT', stop);
        try {
            console.log(JSON.stringify(await query(args, controller.signal)));
        } finally {
            process.off('SIGTERM', stop);
            process.off('SIGINT', stop);
        }
    }
}

/** Diagnostic NDJSON stream. The Python scheduler uses poll for atomic ACKs. */
export class CommandEventsWatch implements Command {
    group = 'events';
    name = 'watch';
    help = 'Stream SDK event batches as NDJSON. Delivery cursors are not persistent acknowledgements.';
    args = ['path...'];
    options = options;
    async action(args: ActionArgs) {
        const controller = new AbortController();
        let timer: ReturnType<typeof setTimeout> | undefined;
        let wake: (() => void) | undefined;
        const stop = () => { controller.abort(); if (timer) clearTimeout(timer); wake?.(); };
        process.on('SIGTERM', stop);
        process.on('SIGINT', stop);
        let failures = 0;
        try {
            while (!controller.signal.aborted) {
                let delay = 30000;
                try {
                    const batch = await query(args, controller.signal);
                    console.log(JSON.stringify(batch));
                    args.options['cursor-json'] = JSON.stringify(batch.cursors);
                    failures = 0;
                } catch {
                    if (controller.signal.aborted) break;
                    delay = retryDelay(failures++);
                    console.log(JSON.stringify({ protocol: 1, type: 'error', code: 'event_query_failed', retryMs: delay }));
                }
                if (!controller.signal.aborted) await new Promise<void>(resolve => { wake = resolve; timer = setTimeout(resolve, delay); });
            }
        } finally {
            stop();
            process.off('SIGTERM', stop);
            process.off('SIGINT', stop);
        }
    }
}
