import { expect, test } from 'bun:test';
import { join } from 'node:path';

test('explicit exit preserves a large JSON response with a delayed pipe reader', async () => {
    const payload = 'x'.repeat(4 * 1024 * 1024);
    const modulePath = join(import.meta.dir, 'flushOutput.ts');
    const child = Bun.spawn([process.execPath, '--eval', `
        import { flushOutput } from ${JSON.stringify(modulePath)};
        process.stdout.write(JSON.stringify({payload: 'x'.repeat(4 * 1024 * 1024)}));
        process.stderr.write('diagnostic\\n');
        await flushOutput();
        process.exit(0);
    `], { stdout: 'pipe', stderr: 'pipe' });
    try {
        await Bun.sleep(150);
        const [output, error, status] = await Promise.all([
            new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited,
        ]);
        expect(status).toBe(0);
        expect(error).toBe('diagnostic\n');
        expect(JSON.parse(output)).toEqual({payload});
    } finally {
        child.kill();
        await child.exited;
    }
}, 15000);
