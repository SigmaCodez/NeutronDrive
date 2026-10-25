// Hash the exact Next.js inline bootstrap scripts; never allow unsafe-inline JS.
import { readdir, readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';
const hashes = new Set();
async function walk(dir) {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) await walk(file);
    else if (entry.name.endsWith('.html')) {
      const html = await readFile(file, 'utf8');
      for (const match of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)) {
        if (!/\bsrc\s*=/i.test(match[1]) && match[2]) hashes.add(`'sha256-${createHash('sha256').update(match[2]).digest('base64')}'`);
      }
    }
  }
}
await walk('out');
const csp = `default-src 'self'; script-src 'self' ${[...hashes].join(' ')}; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'; upgrade-insecure-requests`;
await writeFile('security-headers.conf', `add_header Content-Security-Policy "${csp}" always;\nadd_header X-Content-Type-Options nosniff always;\nadd_header X-Frame-Options DENY always;\nadd_header Referrer-Policy strict-origin-when-cross-origin always;\nadd_header Permissions-Policy "camera=(), microphone=(), geolocation=(), payment=(), usb=()" always;\nadd_header Cross-Origin-Opener-Policy same-origin always;\n`);
console.log(`CSP generated with ${hashes.size} inline script hashes.`);
