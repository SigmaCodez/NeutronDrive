# Architecture and next milestones

```text
GTK4 Application (main thread)
  └─ single worker executor
       ├─ Backend → serialized official proton-drive 0.9.0 process
       │             └─ official account + Drive SDK → Proton HTTPS / Secret Service
       └─ SyncEngine → explicit tree snapshot → reviewed plan → checked execution
```

`backend.py` adapts the released CLI's JSON contract and never uses a shell.
Operations use UID-based remote paths where supported, explicit conflict
strategies, a cross-process file lock, cancellation and timeouts. Login reads the
first JSON line before the process exits. Batch operations inspect every `ok`
result; exit code zero alone is insufficient. Transfer summaries retain skipped
counts. Name-based virtual roots are blocked when ambiguous.

`sync.py` is a manual reconciliation engine. A plan captures local inode/content
identity, remote revision/content identity and the baseline generation. Execute
checks root/account identity and refuses stale or reused plans. It uses private
staging, descriptor-relative no-follow file access, no-replace installation and
backup recovery. Baselines are bound to the remote root UID and local root, not
just a display path. Every confirmed converged file can become a new baseline;
failed and conflicted entries cannot be silently accepted as synchronized.

The UI does not periodically crawl the remote tree. The CLI maintains its own
SDK event/cache behavior, but does not expose an event stream to this adapter.
The UI is not a browser wrapper: GTK widgets and native file choosers display
real backend results. `--demo` is an explicit separate read-only data source.

## Remaining work before desktop parity

1. A maintained SDK sidecar with long-lived authenticated event subscriptions,
   durable operation journal, recovery and server-aware conflict handling.
2. Background scheduling, local change notifications and tray/session lifecycle.
3. Explicit deletion and rename semantics, transactional reconciliation and
   large-tree/offline/network-partition tests.
4. A filesystem integration for on-demand content and pinning, independently
   tested on relevant kernel/FUSE/filesystem combinations.
5. Revision management, Photos/Docs/Sheets, multi-account UX, accessibility and
   translations beyond the initial German interface.
6. Reproducible source backend builds, artifact signing, broader architecture and
   desktop integration tests, followed by Flathub/Snap Store review if desired.

Windows Cloud Files and macOS FileProvider are platform APIs and cannot simply
be copied into a Linux application. The current integration uses the public
cross-platform SDK through its CLI. No binary reverse engineering was needed.
