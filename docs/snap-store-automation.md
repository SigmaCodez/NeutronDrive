# Automatic Snap Store beta releases

The **Publish Snap Store beta** workflow uploads the existing amd64 Snap asset of a
published GitHub beta release to `latest/beta`. It does not build a different
package and never promotes releases to stable. Normal pushes and pull requests
cannot publish to the Store.

## Release procedure

1. Finish package CI and test the Snap under confinement.
2. Create a draft GitHub release with a tag such as `v0.1.0-beta2`.
3. Attach exactly one `*_amd64.snap` and `SHA256SUMS`, including the Snap's SHA-256.
4. Publish the release **after all assets have finished uploading**.
5. Check **Actions → Publish Snap Store beta**. A successful upload requires the
   Store's review to pass; upload or review errors fail the workflow visibly.

The publisher checks that the release is published, its tag matches the package
version, and the package is `neutrondrive`, strict, core24, amd64 and grade devel.
It requires the summary `Unofficial Proton Drive client for Linux`.
The default-branch validator runs before the Store credential is exposed to the
upload step. The downloaded Snap is inspected, not executed.

If assets were attached late or the Store was temporarily unavailable, select
**Run workflow** and enter the existing published release tag. Inspect the Store
revision state before retrying an upload accepted by the Store but not released.
This workflow does not bypass manual reviews or repair inconsistent Store states.

A release created with GitHub's built-in `GITHUB_TOKEN` does not trigger another
workflow through its release event. Such a release-producing workflow must call
`.github/workflows/release-snap-store.yml` explicitly after attaching all assets,
passing `tag` and the `SNAPCRAFT_STORE_CREDENTIALS` secret. Releases published by
the maintainer through the website or authenticated CLI trigger automatically.

## Credential maintenance

Repository: `SigmaCodez/NeutronDrive`.
Secret: `SNAPCRAFT_STORE_CREDENTIALS` (GitHub Actions repository secret).
Scope: `neutrondrive`, `latest/beta`, ACLs `package_access,package_push,package_release`.
Current credential expires **2027-10-26 at 00:00 UTC**. Renew it before expiry.

Generate the replacement interactively without printing its contents:

```bash
umask 077
snapcraft export-login --snaps=neutrondrive --channels=latest/beta \
  --acls=package_access,package_push,package_release \
  --expires=2027-10-26T00:00:00Z store-login
# Store directly as an encrypted repository secret; never commit the file.
gh secret set SNAPCRAFT_STORE_CREDENTIALS --repo SigmaCodez/NeutronDrive < store-login
rm store-login
```

Choose a new expiry when renewing. Keep branch protection enabled and restrict
who can modify workflows or publish releases. No Store credentials belong in
source code, release assets, screenshots or logs.

The automation does not replace the Store listing edited in the dashboard.
Keyring and mount-observe connections remain separate installation requirements.
