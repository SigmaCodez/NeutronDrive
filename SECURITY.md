# Security and privacy

NeutronDrive is alpha software, not an independently audited security product.
Please report a vulnerability through GitHub's private vulnerability reporting
when available. Otherwise open an issue containing only a minimal description
and ask for a private channel; do not post credentials, tokens, private filenames,
CLI logs or share URLs in public issues.

The desktop app calls a SHA512-pinned, unmodified official Proton CLI. Proton's
SDK performs encryption, metadata handling, caching, retries and rate limiting.
Authentication happens in a browser at `https://account.proton.me`. NeutronDrive
does not receive a password through a form or store session tokens itself.
The official CLI's Secret Service entry is shared with other uses of that CLI.

The adapter rejects a custom Proton API endpoint and plaintext credential mode,
forces encrypted CLI caches, uses subprocess argument lists without a shell,
validates login URLs, caps captured output and inspects per-item failures even
when the CLI exits with status zero. Raw backend output is not written into the
activity log. The upstream CLI has its own diagnostic and telemetry behavior;
NeutronDrive does not promise to disable all Proton telemetry.

Synchronization rejects symlink paths and special files, checks file content and
identity around copies, retains replaced local data, never propagates deletions,
and stores baselines only for verified converged entries. A remote compare-and-swap
operation is unavailable through the CLI; remote concurrent-write races remain
possible. The normal folder uploader checks for links before invoking the CLI,
but a malicious local process changing that tree after validation is outside its
protection. Select trusted, stable folders for uploads.

Local synchronized data and retained backup files are plaintext, as in any local
filesystem. Use operating-system disk encryption if that is required. The sync
state directory is private (`0700`); it contains filenames and content hashes.
Session keys remain in the operating-system keyring or `pass`.

Flatpak requests home-directory and `org.freedesktop.secrets` access. Snap needs
the explicitly connected `password-manager-service` interface. These permissions
are necessary for the selected file-transfer/keyring design and are broader than
an application-private credential vault. Packages are unsigned until a release
signing process is established; SHA256 sums detect accidental corruption but do
not substitute for a trusted signature.
