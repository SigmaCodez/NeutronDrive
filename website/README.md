# NeutronDrive homepage

Multilingual promotional website built with Next.js 16.4.3 and React 19.3.1.
The app is statically exported; only unprivileged Nginx runs in production.

## Local development

Use Node.js 24 and pnpm 11.25.0:

```sh
cd website
pnpm install --frozen-lockfile
pnpm dev
pnpm build
pnpm audit --prod
pnpm test
```

The static output is `website/out`. `next start` does not serve static exports;
use a static server for the exported directory or build the Docker image.

## Deploy

Run `./deploy.sh` from the repository root. The destination is intentionally
fixed to **161.97.140.89**, domain **neutrondrive.sigmacodez.dev**.
Defaults: SSH user `root`, key `~/.ssh/SaaS01`. Override only authentication with
`NEUTRONDRIVE_SSH_USER` and `NEUTRONDRIVE_SSH_KEY`. Host-key verification is strict;
the server must already be trusted in `~/.ssh/known_hosts`.

The script transfers only `website/` (without dependencies, build output or
secrets) to `/opt/neutrondrive/releases/<timestamp>-<pid>`. The server builds a
versioned image, tests a candidate container without host ports, then replaces
`neutrondrive-web` with **31219:31219**. It checks the host-port health endpoint
and homepage. Occupied ports belonging to another service stop deployment.
A deployment lock prevents concurrent replacement. Previous images and source
releases are retained; failed activation restores the previous application image.

By default the proxy is managed manually and no NPM authentication is required.
In Nginx Proxy Manager configure:

- Domain: `neutrondrive.sigmacodez.dev`
- Scheme: `http`
- Forward Hostname/IP: `neutrondrive-web` (container on the existing proxy network)
- Forward Port: `31219`
- SSL: Let's Encrypt, Force SSL and HSTS for this hostname.

The container and host both use port 31219. The proxy connects directly over
the existing Docker network; no firewall changes are needed.
Host port 31218 belongs to `sigmacodez-web` and must not be reused.

Optional: `./deploy.sh --configure-proxy` also configures NPM and verifies live
HTTPS. This requires valid credentials in the existing server-side file
`/opt/codezwerk-proxy/admin-credentials.json`. They stay on the server. Failed
proxy authentication stops this optional mode before replacing the container.

Prerequisites: Docker, Python 3, curl, flock, ss, and the existing Docker network
`codezwerk-proxy_default`. Optional automatic proxy setup also requires
`codezwerk_proxy` and its API on 127.0.0.1:81. DNS must point to this server.
The host port serves HTTP; HTTPS terminates at the existing proxy on 443.

## Security and operation

- Read-only container, UID/GID 101, all capabilities dropped, no-new-privileges.
- Writable temporary data limited to a 32 MiB noexec/nosuid tmpfs.
- CPU, memory, process and Docker log limits; healthcheck and restart policy.
- HTTPS redirects and HSTS are enabled in NPM when setting up the domain.
- Build-generated CSP hashes for Next.js inline scripts; no unsafe-inline JS.
- No Server Actions, login, forms, cookies or analytics. The only API route
  proxies a fixed public GitHub release feed with verified TLS and a five-minute
  cache. Visitor headers and credentials are not forwarded.
- Only GET/HEAD; request-size and timeout limits; hidden files denied.
- HTML is not cached long-term; hashed Next assets can be cached for a year.
- Dependency versions and lockfile are committed. Docker base images refresh on
  deployment; rerun deployment regularly to pick up base-image security updates.
- Access logging is disabled in the app; optional proxy setup also disables it in the proxy entry. Proxy error
  logs may still contain request metadata. Existing server-wide log rotation
  remains unchanged. Release content is maintained by the server-side systemd timer
  described in `RELEASE-AUTOMATION.md`; OS patching remains separate.

These measures apply to this website. They do not certify the whole server or
replace ongoing dependency and OS patching. NPM admin exposure and unrelated
applications are not modified.

## Content maintenance

`lib/translations.js` contains complete English, German, French, Spanish and
Italian content, including legal pages. `app/language.js` uses the primary browser
language (including regional variants); unsupported languages fall back to English.
A manual selection persists in localStorage. Automatic clears that preference
and follows browser language changes. The initial static HTML is English.

`app/page.js` retrieves `/api/releases` on load, every five minutes and when the
page becomes visible again. Nginx proxies a fixed public GitHub endpoint and
caches it for five minutes (up to roughly ten minutes on a continuously open tab).
`newestRelease` selects the newest published non-draft from the 100 returned
releases, including prereleases, by `published_at`. It validates the GitHub URL.
Header and hero download buttons lead to the package picker. Package cards use
validated `browser_download_url` assets from that release for direct downloads.
Multiple assets (e.g. DEB architectures or Fedora/openSUSE RPMs) get a filename
selector. Only uploaded, nonempty NeutronDrive installable files are offered;
source archives, checksums, dependencies and foreign/older-release URLs are
excluded. Missing assets and fetch failures have a translated unavailable state,
never an automatic fallback to an old package. The separate version link opens
release notes (or the release overview if the API is unavailable).
No GitHub token is used. GitHub serves the download files directly.
The screenshots are verified against the beta 0.1 capture manifest and explicitly
labelled as 0.1.0-beta1, independent of the current
download version. The server checks new releases every 15 minutes and atomically publishes notes,
verified captures and available translated content manifests without rebuilding.
No local computer or Codex is required. See `RELEASE-AUTOMATION.md` for the source
manifest format and English fallback. `/live/current/content.json` records the
active release; `public/content-version.json` preserves the seed provenance.

Screenshots in `public/screenshots` are unedited real GUI captures copied from
`docs/previews`, using synthetic demo data. Provenance: `docs/GUI-PREVIEWS.json`.
Privacy and imprint live in `app/datenschutz` and `app/impressum`; the operator
requested a private-person attribution with town only, without a street address.

Validation: production build, dependency audit, shell syntax, Python compilation,
container health/security headers, browser gallery/FAQ interaction.

Deployment now uses host port 31219. Proxy and certificate setup are managed
manually unless `--configure-proxy` is explicitly passed.

### Release FAQ source

`docs/website-content.json` contains five general FAQ rows per language: four
project/platform questions followed by the contribution question. Do not keep a
second copy of release-specific answers there. The publisher and server updater
insert the three Q&A sections from `releaseNotes` between those rows.

For each of en/de/fr/es/it, `releaseNotes` must contain exactly three sections
separated by a blank line. Each starts with a question on its own line, followed
by its answer: changes, upgrade instructions, and current limitations. The first
two questions must name the exact release version without the leading `v`.
Use actual newlines; legacy escaped newlines are normalized on ingestion.

Before tagging, update the release identifier and review all five translations
against the actual changes, current package availability and documented limits.
CI and the release publisher reject missing, malformed or mismatched sections.
The server also upgrades existing eight-row manifests and drops outdated release
FAQ rows when their notes cannot be verified. New facts/translations still need
review; this automation never invents them or merely changes version numbers.

To correct copy for an already published release, run **Publish release website
content** with its `tag` and `refresh_text: true` after merging the correction.
This uses reviewed text from main only if it names that exact release, records
`content_source_commit`, and still captures the GUI from the actual release
package using the tagged fixtures. Normal new releases continue using tagged
copy. The server's 15-minute timer picks up the resulting manifest automatically.

If main already targets a newer release, put a reviewed correction in
`docs/website-content-corrections/<tag>.json`. Only the explicit
`refresh_text` mode selects it; normal tagged releases are unaffected.

### Immediate update after a release

After uploading `neutrondrive-website.json`, the release workflow runs
`website/scripts/trigger-release-sync.sh`. It starts the existing server updater
and verifies the live release, source commit, all five localized text sets and
screenshot hashes against the published bundle. Three attempts cover transient
failures and overlap with a running timer. A failed immediate update fails the
workflow visibly; the independent 15-minute server timer remains as fallback.
The trigger waits for the new manifest and screenshots to be ready, rather than
showing an incomplete release at the initial release-created event.

One-time server setup (explicit administrator authorization required):
create a dedicated Ed25519 key, then run
`bash website/scripts/install-release-trigger.sh PUBLIC_KEY_FILE` as root on
161.97.140.89. This creates the system identity `neutrondrive-release` with a
root-owned authorized key using an OpenSSH forced command and `restrict`.
Its only sudo permission is `/usr/bin/systemctl start neutrondrive-content.service`;
the service itself still runs as the sandboxed `neutrondrive-content` user.
No shell commands, forwarding, passwords or private keys are installed on the
server. No deployment or general administrator SSH key is used by Actions.

Store the dedicated private key in repository secret `ND_CONTENT_SYNC_SSH_KEY`
and the server host-key line (verified through the existing trusted connection)
in `ND_CONTENT_SYNC_KNOWN_HOSTS`. SSH host verification is mandatory. Private
material is written to a runner temporary directory with restrictive permissions
and removed on exit. Reusable-workflow callers must pass these two secrets.
To revoke, remove the dedicated authorized key and these repository secrets;
the fallback timer remains usable.
