# Changelog

## Unreleased

### CLI

- `katok --version` (and `-V`) prints `katok <version>` from `Cargo.toml` and exits 0.

### Sync

- `katok sync --chat <chat_id>` (repeatable) reads and archives only the named chats. The
  macOS reader filters in its query, so other rooms' bodies are never read. A filtered sync
  keeps an incremental cursor of its own per set of chats and never moves the unfiltered one,
  so a later sync without `--chat` still reads the rows of rooms the filtered one skipped. A
  new set reads its chats in full once. Refused with `--prune-preview`/`--prune-deleted`.

### Messages

- `katok messages --json` adds `mentioned_user_ids` (decimal strings) and `mentions_all` to
  each message, read from the top-level `mentions[].user_id` and `all_mention` of KakaoTalk's
  `attachment` JSON; a reply's `src_mentions` is not this message's mention. Stored in two new
  `messages` columns added on open; a row without mentions keeps NULL, so the first sync after
  the upgrade rewrites only rows that do have one.

### Rooms

- `katok rooms --json` adds `last_seen_log_id`, `new_message_count` and
  `my_watermark_log_id`: the room's `lastSeenLogId`, `countOfNewMessage` and the account
  owner's entry in the `watermarkKeys`/`watermarkValues` binary plists. Which of them tracks
  what the owner has read is not settled, so all three are reported; log ids are decimal
  strings, missing values `null`. Every macOS sync, incremental or not, re-reads them for all
  rooms it covers and stores them on existing `chats` rows only.
- `katok rooms --json` adds `member_count` (`activeMembersCount`, owner included),
  `member_user_ids` (the `displayMemberIds` preview without the owner, as decimal strings; a
  direct chat without a preview gives its peer) and `is_open_chat` (`linkId != 0`), stored and
  refreshed the same way. `ChatReadState` is renamed `ChatRoomState` to carry them.

### Doctor

- `doctor --json` adds `kakao_app.status` (`running`/`not_running` by `pgrep -x KakaoTalk`,
  `unknown` if pgrep fails) and `source_access`. With `--macos-probe`, `source_access` holds
  the outcome of a real `opendir` of the KakaoTalk container and `open(O_RDONLY)` of one
  database file: `status` `ok`/`denied`/`missing`/`error`, the step that failed and its
  errno. Without the flag it is `not_checked`: opening the container is what raises the macOS
  prompt, and a plain `doctor` stays prompt-free.

### Sync range

- `katok sync --chat <chat_id> --since <unix_ms>` stores a chat the archive does not hold yet
  (no message, no floor) only from that send time on. The time is kept as the chat's floor in a
  new `chat_floors` table and every later sync drops rows before it, whatever the read path:
  cursors keep advancing to the newest row, and the re-read margin, replaced rows and late
  arrivals cannot bring older rows back. A chat already in the archive ignores `--since`;
  `forget` removes the floor, so changing a range is forget then sync with the new `--since`.

### Forget

- `katok forget --chat <chat_id> [--yes]` deletes one chat's rows from every archive table
  (messages, chats, chunks, chunk FTS, chunk/parent links, parent windows, reply edges) in one
  transaction, plus its media and transcripts in their default locations. Without `--yes` it
  only counts. A reply from another chat into the forgotten one keeps its row and loses its
  parent. Semantic vectors cannot be dropped per chat, so a `REBUILD_REQUIRED` marker is left
  in the semantic directory and reported as `doctor --json` `semantic_rebuild_required` until
  an index run that started after it commits. It also clears every incremental source cursor,
  plain and per chat set: they do not record which chats they passed, and a surviving one made
  the next `--incremental` sync of the forgotten chat skip its history while reporting
  `incremental: true`. The next incremental sync reads the source in full once.

### Index

- `katok index` now validates the staged generation against the archive snapshot it
  started from instead of re-reading the live archive at commit time. A `katok sync`
  that lands during a long embedding pass used to fail the commit with
  `semantic_index_stale` and discard all the staged vectors. The generation now
  commits at the start-time revision, search reports it stale until the next
  `katok index`, and that run reuses every unchanged vector and embeds only the
  windows that changed. Search keeps rejecting a stale index as before.

### Send

- `send --draft --image` is refused at argument parsing with a sentence that says
  why (an image cannot be left as a draft) and what to do instead, rather than
  clap's generic conflict wording; `--draft --dry-run` gets the same treatment.
  Under `--json` any argument error now comes back as the usual
  `{"ok": false, "error": {...}}` document with code `invalid_arguments`
  (exit status 2) instead of prose on stderr. The send path also checks the
  pair itself, since its image branch ran ahead of the draft branch.
- Pasting an image or a draft, and the global-Enter fallback for text, now
  raise the target chat window (`AXRaise`, plus `AXMain` where settable) and
  confirm it is KakaoTalk's `AXFocusedWindow` immediately before the
  keystroke. Activating the app alone left the paste to whichever window was
  key last, so with several chats open it could land in another room. A
  mismatch fails with `WindowFocusLost` and posts nothing.
- Opening a closed room by chat-list search now raises the chat list and
  types the query only once it is KakaoTalk's focused window. With a chat
  window key, the query could go into that room's compose box.
- Raising a window before a keystroke now waits until it reads back as
  KakaoTalk's focused window (up to the old fixed 200 ms) instead of always
  sleeping 200 ms. Before a chat-list row is clicked, the list waits until it
  is focused plus a 150 ms settle, never more than the old fixed 300 ms.
  Clearing the chat search afterwards reads the box back every 30 ms.
- Opening a closed room by search no longer falls through to the slow
  list scans when the chat list still shows the results of an earlier
  search for the same text. KakaoTalk filters only on typed input, so the
  box cleared through AX leaves those results up, typing the same text
  changes nothing, and the "filter has not taken effect" wait used to run
  out. A list of the pre-query length is now taken when every row reads
  back as a room, so no unrendered row can hide a second room of that name.

### Reading

- An unreadable KakaoTalk container (permission denied, interrupted call, ...) is
  reported as "container directory could not be read: <OS error>" instead of
  "no KakaoTalk database files found". Only a missing directory still counts as
  having no database files.
- `media backfill` resolves the KakaoTalk databases once per run and reuses them for
  the room list and every room, instead of listing the container again per room.
- The self chat (the memo room whose only member is the account owner) is
  now typed `self` and named after the owner's nickname, which is how
  KakaoTalk titles its window. It used to come out as a `chat-<id>` group,
  which `send --chat` could not match to a window.
- `katok sync --incremental` reads each KakaoTalk database only past the
  cursors the previous sync stored: its eight newest local `NTChatMessage`
  rows, each as rowid plus `(chatId, logId)`. The read starts after the
  newest of them that still holds the same message. The rowid is KakaoTalk's
  insertion order, so a message the Mac receives late still lands after it.
  When none holds (deleted, replaced, renumbered by a vacuum), or a database
  has no cursor, that database is read in full. Every sync
  stores the cursor inside its transaction; the report gains `incremental`
  only with the flag. A message KakaoTalk edits in place, and the names of
  rooms without new messages, wait for the next sync without the flag. It
  cannot be combined with `--prune-preview`/`--prune-deleted`.
- Auth hands the database key it already derived to the reader, the media
  reader and the room-id lookup instead of deriving it again, derives the
  database file name (used only to order probes) only when there is more
  than one file, reads the katok auth cache before running `ioreg`, and runs
  the two derivations side by side.
- The account's newest-message time in `sync_cursors` no longer moves back
  when a sync carries only an older message.

### Media

- `media backfill --dry-run` no longer writes anything. It used to skip only the
  CDN request and still decrypted every local-cache `full`/`thumb` photo and
  video into `--out`. The local tiers are now resolved in memory only, so the
  tier counts are unchanged, and their records carry no `path`.
- A cached video that decrypts but has no complete top-level `moov` box is no
  longer reported as `full`. It is recorded as `full-incomplete`, nothing is
  written for it, and the CDN and thumbnail tiers are tried instead.

### Build

- New default-on `semantic` feature gates `fastembed` and its ONNX Runtime
  download. `--no-default-features --features private-send` builds a binary
  for read, sync, media and send without them; `index` and `search semantic`
  then fail up front with `semantic_feature_disabled`.
- `tokio` is now part of the `semantic` feature too. A slim build drives the
  test embedder's futures with a single poll instead, so it no longer links a
  runtime (-258 KB).
- On Apple Silicon `sha2` is built with its `asm` feature, which uses the
  ARMv8 SHA-256 instructions (runtime-detected). A KakaoTalk key derivation
  (100k PBKDF2-HMAC-SHA256 rounds) went from about 114 ms to 27 ms, and every
  command that opens a KakaoTalk database does one or two.
- New `sidecar` Cargo profile for the binary apps bundle: fat LTO, one codegen
  unit, `panic = "abort"`, symbols stripped. The slim build drops from 11.2 MB
  to 8.0 MB with `sync` and `rooms` as fast as before. `sidecar-symbols` keeps
  symbols and line tables for symbolicating a crash.

## 0.3.2 - 2026-08-12

### Release

- Fixed release validation after the repository privacy rewrite by scanning
  commits from the merge-base of the previous version tag through the current
  release, rather than rescanning unrelated pre-remediation ancestors.

## 0.3.1 - 2026-08-12

### Privacy

- Replaced live-derived KakaoTalk identity fixtures with explicit synthetic
  values and hardened the publication privacy checks.
- Added a full-history CI and release gate that rejects non-synthetic UUIDs,
  plausible KakaoTalk user identifiers, derived database-name oracles, and
  personal home paths in every newly introduced commit, including values
  removed by a later commit.

### Search and indexing

- Search indexes now recover atomically from orphaned, stale, or corrupt
  generations while preserving the last committed generation when rebuilding
  fails.
- Semantic parent-window loading and bulk index preparation now avoid
  repeated per-result archive reads.

## 0.3.0 - 2026-08-02

### Sending

- The existing macOS Accessibility-based `katok send` command is enabled in
  default builds. Read-only installs remain available with
  `cargo install katok --no-default-features`.
- Text, image, and draft modes fail before stdin, archive, Accessibility, or
  KakaoTalk UI access unless `--accept-use-policy` is supplied. Read-only
  listing and non-delivery `--dry-run` remain available without acceptance.
- `katok send --list-windows` and `--list-rooms` no longer require a dummy
  `--room` or `--chat` target.
- `ACCEPTABLE_USE_POLICY.md` and `DISCLAIMER.md` prohibit spam, impersonation,
  account theft, stalking, harassment, post-refusal contact, bulk/repetitive
  sending, privacy violations, and access-control evasion.
- The documented network boundary is scoped to `katok send`: it drives the
  local official KakaoTalk app and makes no direct Kakao remote private
  protocol/API or HTTP/socket call. Other subcommands may use documented CDN
  or model-download paths.

## 0.2.0 - 2026-07-29

### Sending

- The private `katok send` source is excluded from default builds and requires the explicit `private-send` Cargo feature; no sending skill is shipped in the public 0.2.0 surface.
- `katok send --chat <chat-id>` addresses a room by id. Display names are not identifiers and may be shared by multiple rooms, so an ambiguous name is refused outright rather than guessed at; `--chat` resolves it through the room's position among same-named chats.
- Rooms are matched by member set rather than by string. KakaoTalk and the archive may list an unnamed group's members in different orders, so set comparison keeps the room reachable.
- The chat search box is now typed into with real keystrokes. Setting its accessibility value painted the text but told the app nothing had happened, so the list never filtered and the search strategy silently did nothing — leaving the unfiltered list, where a room far enough down sits outside the scroll area and was clicked at coordinates nobody could see.
- `--draft` leaves a message in the compose box for review. It pastes rather than writing the accessibility value, because writing that value *is* sending: KakaoTalk delivers on the change with no keystroke involved.
- A send that needs the screen now runs behind a full-screen curtain that blocks keyboard and mouse input, so it cannot collide with whoever is using the machine. Blocked input is dropped rather than queued, which is why the block is always visible. Esc or the cancel button stops the run.
- The previously active application and the clipboard are restored on every exit path, including failures, and a global keystroke is never posted unless KakaoTalk is confirmed frontmost at that instant — a collision now produces a clean failure instead of a paste into someone else's document.
- `--room` rejects control characters and invisible formatting marks, naming the codepoint, since those are quoting accidents upstream rather than missing rooms.
- `katok send` waits for a gap in the user's typing before taking focus, restores the previously active application when it is done, and no longer overwrites the clipboard: the existing contents are saved and put back. Added `--take-focus-now` to skip the wait and `--focus-wait` to bound it.
- Resolving a room takes focus once for the whole attempt sequence rather than once per attempt.

### Reading

- `katok media get` now extracts videos (message type 3) alongside photos and albums. Video bodies live in the `.vid` cache under a `v`-prefixed key stem, and the output extension is sniffed so ISO-BMFF bodies land as `.mp4` instead of `.bin`.
- Video resolution reuses the existing tier order unchanged, so an uncached video still comes from the SHA-1 verified presigned CDN URL and `--no-cdn` still keeps a run local-only.
- `katok media get` extracts generic file attachments (message type 18) alongside photos, albums and videos — one message type covering zip, pdf, xlsx, hwp and every other extension. `--kind photo|video|file` narrows a run.
- File attachments resolve through the CDN alone: KakaoTalk keeps no local copy of them, so `--no-cdn` returns nothing for a file and a stub reads `unavailable` rather than `not-cached`. Output keeps the attachment's original name, sanitised so it cannot escape the output directory, disguise its own extension through a bidi override, or differ in bytes from a visually identical name.
- Added `katok media backfill`, which saves every attachment whose presigned link is still valid across all rooms. Presigned URLs expire after roughly two weeks and a file has no local copy, so anything not fetched inside that window is lost. Re-running is free: an already-saved frame is skipped with no network call.
- Fixed a limit that silently failed every CDN body over 10 MB — most videos and many files — by passing the fetch cap explicitly rather than relying on the HTTP client default.

### Sync performance

- A sync commits as one transaction and reuses prepared statements on the per-row write path, instead of a transaction per row.
- Only chats whose messages actually changed are re-chunked. `sync` reports `touched_chats`, and `--touched` exposes the list in the JSON payload.
- A touched chat rebuilds only its tail rather than its whole history. The rebuild starts at the last stable time-gap boundary preceding the earliest change, so ordinary append cost tracks the changed tail rather than total room size. A mid-history edit still widens the scope to whatever the change reaches, up to a full rebuild at the chat head.
- The reply and parent-reference pass is scoped to touched chats. Both endpoints of every `reply_edges` and `chunk_parent_refs` row are provably inside one chat, because message ids are chat-prefixed, so the whole-archive scan was never necessary. The remaining cost no longer tracks archive size.
- The messages tail is sought by `(chat_id, timestamp)` and the FTS tail delete by rowid, rather than scanning. Added indexes on the chunk tables by `(chat_id, started_at)`.
- A gap-settings change or a `CHUNKER_VERSION` bump still forces a full rebuild.

### Fixes to behaviour inherited from upstream

- Replies are resolved again. The reader looked for `src_logId` only in `supplement`, while KakaoTalk replies store it at the top level of `attachment`; the query now selects and checks that column first, with synthetic regression coverage.
- A wall of repeated long messages no longer makes an archive permanently unsyncable. `parent_id` was hashed from `(account, chat, first child, last child, text)` with no segment ordinal, so two windows cut from one oversized chunk collided whenever the text repeated — nine pasted 999-character messages is enough. The insert violated the primary key and, because a sync is a single transaction, rolled the whole sync back and failed identically on every later run. `CHUNKER_VERSION` is bumped so existing archives rebuild.
- Search snippets are no longer empty in Korean. `str::find` returns a byte offset and `Iterator::skip` counts chars; passing one to the other overshot by the encoding width, so a match late in a long Hangul chunk skipped past the end of the text.
- An edited message now reaches the archive, its chunk, and the index. The upsert updated only `chat_name`, `chat_type`, `sender_nickname` and `reply_to_message_id`, so a corrected body kept its original wording forever and the chat was reported unchanged, keeping it out of the rebuild. `timestamp` and `message_type` are worse than cosmetic: both feed chunk boundary computation.
- One room ingested under two accounts no longer crashes the sync. The chunk boundary test compared chat and sender but not `account_hash`, so both accounts' copies of a message landed in one chunk and violated `chunk_messages`' primary key.
- Parent windows no longer over-report `message_count`. A chunk spilling across three windows was counted by all three.
- An open chat is named by its link rather than by listing its members. `NTChatRoom.chatName` is NULL for these, so the link name prevents rooms from being filed under a member list that appears nowhere in KakaoTalk.
- Added `katok sync --prune-preview` and `--prune-deleted`. Sync otherwise only upserts, so a message deleted upstream stayed in the archive, its chunk text and the embedded index indefinitely. Only the time range the source still covers is reconciled: KakaoTalk prunes its own database and outliving that is what this archive is for, so anything older than the source's reach is left alone, and a chat the source did not mention is skipped entirely. Preview reports without deleting; deletion is never the default.


## 0.1.3 - 2026-07-18

- Added `katok media get` for KakaoTalk image extraction with local Pkv2 `.img`, CDN SHA-1 verified fetch, `.thm` fallback, and stub records.
- Documented that the CDN presigned GET is the only network tier in image extraction, and that `--no-cdn` disables it for local-only runs.
- Added synthetic SQLCipher and media-cache tests for full, CDN, thumbnail, stub, no-cdn, SHA-1 mismatch, and album type 27 paths.
