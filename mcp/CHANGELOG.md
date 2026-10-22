# Changelog

## 0.3.0 (2026-10-22)

Needs a katok build that has `search keyword --order` and `sync --chat`. Re-run
`scripts/install.sh` after updating; an older binary is reported as outdated instead of failing
with a bare argument error.

- `katok_search`: new `order` input (`newest` | `oldest`, keyword only). Keyword search now
  passes `--order newest` by default. bm25 and semantic never get `--order`, and refuse it.
- `since` / `until` (katok_messages, katok_transcript): validated before katok runs. Accepts
  RFC3339 with an offset (`2026-07-20T00:00:00Z`) or a date (`2026-07-20`). A katok
  `invalid_arguments` error about `--since`/`--until` gets a format hint.
- `katok_index`: `data.documents[]` is replaced by `data.document_count` unless the new
  `verbose: true` input is set. The full list can be thousands of entries and megabytes.
- `katok_sync`: new `chat_id: string[]` input, passed as repeated `--chat`. katok keeps a
  separate incremental cursor for that exact set. `with_media` still backfills every room.
  The timeout hint now names the real inputs (`incremental`, `chat_id`, `limit`).
- `katok_send`: the "send may have gone through" hint also says how to check through the
  archive, not to resend because the archive does not show it, and that images never show up.
- `katok_init`: re-running after an interrupted first sync (archive present, doctor reports no
  completed sync or zero messages) runs the sync again instead of only returning doctor.
  `already_initialized` is true only when nothing had to be synced. The first sync is one
  transaction, so a timeout keeps nothing: its error now carries its own hint, led by a terminal
  command with no MCP time limit (Keychain lookup included, key value never shown), and says a
  re-run only helps for a one-off delay. It never gets the generic "narrow the request" hint.
- Failures of katok_init's first sync, katok_sync and katok_index are kept in
  `<data_dir>/last-sync-error.json` / `last-index-error.json` (0600, written through a per-pid
  temp file): time, tool, step, exit code, signal, katok error code, stderr head. Never stdout,
  argv or key. A later success keeps the file.
- `katok_doctor`: the wrapper gains `binary_build` (`path`, `ref`, `mtime`) and, for the in-repo
  binary only (compared by real path, so a symlink to it counts), a `warning` when it has no
  build stamp or is older than `cli/src`.
- An error from clap about `--order` or `--chat` ("unexpected argument") becomes
  "katok binary is outdated". It says to re-run scripts/install.sh for the repository's build,
  and that a KATOK_BIN or ~/.cargo/bin binary has to be updated where it came from.
- `scripts/install.sh` writes the build stamp `cli/target/release/katok.ref` (the commit, or
  `unknown`) after `cargo build --release`.
