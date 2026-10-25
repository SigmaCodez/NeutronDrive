# Server-side release content updates

The systemd timer `neutrondrive-content.timer` runs on **161.97.140.89** every
15 minutes (plus up to 45 seconds jitter). It catches up after server downtime.
No local computer, Codex session, AI API, GitHub token or Docker socket is needed.
The former local Codex heartbeat is removed.

`./deploy.sh` installs/updates the service and timer as part of deployment.
The updater runs as the unprivileged `neutrondrive-content` system user, with a
read-only system, no home access, no capabilities, and CPU/memory/time limits.
It can write only its state directory `/var/lib/neutrondrive-content` (and its
private temporary directory). The website mounts that directory read-only.
Updater code and seed data are root-owned in `/usr/local/lib/neutrondrive-content`.

## What updates automatically

- Newest published non-draft GitHub release, including previews, ordered by date.
- Current version, release status and original release notes; corrections to
  existing release notes are picked up as well. Direct package downloads keep
  their existing five-minute refresh.
- The `neutrondrive-website.json` release asset has priority and bundles all five
  locales with fresh PNGs, hashes and the exact release commit. It is produced by
  `.github/workflows/release-website.yml`.
- For releases without a bundle, homepage descriptions, features, FAQ, trust notes and gallery captions come from an
  optional `docs/website-content.json` in the published release's commit.
- Screenshots from that commit's `docs/GUI-PREVIEWS.json` and `docs/previews`.
  The manifest must name the exact release; PNG hashes, dimensions and sizes are
  checked. Old captures keep their real version label until a new set exists.

The server consumes published content; it does not invent or AI-translate text.
The website interface remains available in EN/DE/FR/ES/IT. Release-specific
translations come from the manifest, falling back to English. Without a matching
manifest, original English release notes still update automatically, the feature
overview keeps its reviewed version label, and old beta-specific FAQ and status
claims are replaced with general guidance to the current release notes.

## Publishing content with a future release

Before tagging, update `docs/website-content.json`:

```json
{
  "schema": 1,
  "release": "v0.2.0-beta1",
  "locales": {
    "en": {"description": "Release-specific introduction", "releaseNotes": "Translated or original notes"},
    "de": {"description": "Versionsspezifische Einleitung", "releaseNotes": "Deutsche Release-Hinweise"}
  }
}
```

The checked-in file contains the current five-language content as an example.
English is required. Optional fields: `description`, `releaseNotes`, `features`
(exactly four title/text pairs), `faq` (1–20 question/answer pairs), `trust`
(three title/text pairs), `views` (five tab/title/caption triples in the existing
files/light/photos/computer/sharing order). Only plain text is accepted; legal
pages, scripts, URLs and deployment settings cannot be changed through it.
An inherited manifest with an older release tag is ignored, not relabelled.

Capture actual application screenshots with synthetic data, update the existing
`docs/GUI-PREVIEWS.json` with `published_release` and SHA256 per image, and include
both manifests in the release tag. No homepage deployment is then necessary. For automatic capture and bundling,
see `docs/RELEASING.md`: the release workflow requires all five complete locales,
including `releaseNotes`, and generates screenshots from the published DEB.

## Atomic publication and failures

All downloads use fixed GitHub hosts and the resolved release commit. No remote
code or packages are executed. Validated JSON and PNGs are staged into immutable
content-addressed snapshots; a symlink switches the complete set atomically.
Unchanged content causes no publication or rebuild. Ten snapshots are retained.
Network/rate-limit/invalid-image failures leave the last complete snapshot intact
and mark the service failed; the timer retries next time. Missing or stale capture
manifests preserve previous images with `screenshots_pending: true` and are retried.

The live data is `/live/current/content.json`. `/live/status.json` exposes the last
check time, outcome and pending-image flag. The historical `/content-version.json`
remains the seed provenance, not the live content status. Browsers refresh content
every five minutes, so propagation is normally within about 20 minutes.

Inspect or run on the server:

```sh
systemctl status neutrondrive-content.timer
systemctl start neutrondrive-content.service
journalctl -u neutrondrive-content.service -n 30
cat /var/lib/neutrondrive-content/status.json
```

Validation: Python unit tests cover source failures, stale manifests, image
verification and atomic publication; JavaScript tests cover language fallbacks,
removal of old beta claims and local screenshot URLs. Deployment also runs the
website tests, production build and container health checks.
