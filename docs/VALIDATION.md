# Validation record — 0.6.0 alpha3

Previous evidence: [0.6.0 alpha2](VALIDATION-0.6-alpha2.md).

The GUI exposes automatic synchronization only; the separate manual page,
actions and controller fields were removed. Existing automatic jobs, Rust sync
policy, guarded execution and saved formats are unchanged.

The appearance follows the Linux Settings portal (`org.freedesktop.appearance`,
`color-scheme`) with Qt color-scheme/palette fallback. Reads are asynchronous,
changes update the running window, and stale initial replies cannot overwrite
newer notifications. No desktop setting is written. Six pages are rendered in
both light and dark mode, with a scrollable 900×640 layout.

Local regression: **304 tests passed without skips**, including 22 Qt UI tests
and a real private-D-Bus portal test covering nested initial variants, live
changes, reset, initial-read races and subscription cleanup. No real desktop
appearance preference is changed by these tests.

Final package/CI verification is pending. The 11 live-account checks from alpha2
remain evidence for the unchanged sync implementation; they are not presented
as a new account test for this UI-only update.
