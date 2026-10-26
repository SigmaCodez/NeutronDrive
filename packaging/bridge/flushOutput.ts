/** Finish queued pipe writes before the CLI's explicit successful exit.
 * Bun's write callback/drain event alone can fire before its pipe buffer is
 * emptied. Ending the writable stream waits for the pending bytes to finish.
 * Call only after the command and SDK session disposal have completed.
 */
export async function flushOutput(): Promise<void> {
    await Promise.all([process.stdout, process.stderr].map((stream) =>
        new Promise<void>((resolve, reject) => {
            stream.once('error', reject);
            stream.end(() => resolve());
        }),
    ));
}
