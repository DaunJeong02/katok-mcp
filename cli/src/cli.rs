use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// What to tell someone who passes `--draft` with `--image`.
///
/// A draft is pasted text left in the compose box for review; there is no path that stages an
/// image the same way, and falling back to sending it would deliver something the caller asked
/// to hold back. So the pair is refused instead of being resolved in favour of either flag.
pub(crate) const DRAFT_WITH_IMAGE: &str = "--draft cannot be combined with --image: an image \
    cannot be left as a draft, only text can. Drop --draft to send the image, or drop --image \
    to stage a text draft.";

/// What to tell someone who passes `--draft` with `--dry-run`.
pub(crate) const DRAFT_WITH_DRY_RUN: &str = "--draft cannot be combined with --dry-run: a dry run \
    stages and sends nothing, so there would be no draft to leave. Use --dry-run alone to check \
    which room would be targeted.";

/// A plain-language replacement for clap's wording when the flags in conflict are ones this
/// CLI has a specific reason to refuse, or `None` to leave clap's message as it is.
pub(crate) fn conflict_message(error: &clap::Error) -> Option<String> {
    use clap::error::{ContextKind, ErrorKind};

    if error.kind() != ErrorKind::ArgumentConflict {
        return None;
    }
    let text = |kind| error.get(kind).map(|value| value.to_string());
    let pair = [text(ContextKind::InvalidArg)?, text(ContextKind::PriorArg)?];
    let names = |flag: &str| pair.iter().any(|arg| arg.starts_with(flag));
    if names("--draft") && names("--image") {
        Some(DRAFT_WITH_IMAGE.to_string())
    } else if names("--draft") && names("--dry-run") {
        Some(DRAFT_WITH_DRY_RUN.to_string())
    } else {
        None
    }
}

#[derive(Parser)]
#[command(name = "katok", about = "katok: local KakaoTalk search CLI")]
pub(crate) struct Cli {
    #[arg(long)]
    pub(crate) data_dir: Option<PathBuf>,
    #[arg(long)]
    pub(crate) config: Option<PathBuf>,
    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Report archive and index freshness, whether KakaoTalk is running, and setup state.
    ///
    /// Never raises a macOS permission prompt unless `--macos-probe` is given.
    Doctor {
        /// Also try reading the KakaoTalk container: app, container and database presence,
        /// and `source_access` with the errno of a real `opendir`/`open`. This can raise the
        /// macOS "access data from other apps" prompt and waits until it is answered.
        #[arg(long)]
        macos_probe: bool,
        #[arg(long)]
        json: bool,
    },
    Sync {
        #[arg(long)]
        source: Option<String>,
        path: Option<PathBuf>,
        #[arg(long)]
        json: bool,
        /// Report messages the source no longer has, without deleting anything.
        ///
        /// Only the time range the source still covers is considered, so history
        /// KakaoTalk has since pruned is never listed.
        #[arg(long)]
        prune_preview: bool,
        /// Actually delete what `--prune-preview` reports.
        ///
        /// This is the only command that removes archived messages. Preview
        /// first; a deletion cannot be undone from within katok.
        #[arg(long, conflicts_with = "prune_preview")]
        prune_deleted: bool,
        /// Include per-chat earliest-change keys (`touched_chats`) in the report.
        ///
        /// Opt-in: without this flag the JSON shape matches historical consumers exactly.
        /// Each entry is `{chat_id, earliest_changed_timestamp, earliest_changed_message_id}`.
        #[arg(long)]
        touched: bool,
        /// Read only what KakaoTalk added since the last sync of this archive.
        ///
        /// Each source database is read past the cursor the previous sync stored (its newest
        /// local row), or in full when it has none or the cursor no longer matches. New and
        /// replaced messages are seen; a message KakaoTalk edits in place, and room names of
        /// rooms without new messages, wait for the next sync without this flag. Every sync
        /// stores the cursor, so the first one after a full sync is already incremental.
        #[arg(long, conflicts_with_all = ["prune_preview", "prune_deleted"])]
        incremental: bool,
        /// Sync only this chat_id; repeat for more. Other rooms are neither read nor archived.
        ///
        /// A filtered sync keeps its own `--incremental` cursor for this exact set of chats, so
        /// a later sync without `--chat` still reads every room's new rows. A different set
        /// starts over: its first sync reads those chats in full. Cannot be combined with
        /// `--prune-preview`/`--prune-deleted`.
        #[arg(
            long = "chat",
            value_name = "CHAT_ID",
            conflicts_with_all = ["prune_preview", "prune_deleted"]
        )]
        chats: Vec<String>,
        /// Store a room the archive does not hold yet only from this time on (unix
        /// milliseconds, compared with the send time). Needs `--chat`.
        ///
        /// The time becomes that room's floor and applies to every later sync, so older rows
        /// never arrive by any read path. A room already in the archive keeps what it has: to
        /// change its range, `forget` it and sync it again with another `--since`.
        #[arg(long, value_name = "UNIX_MS", requires = "chats", value_parser = clap::value_parser!(i64).range(0..))]
        since: Option<i64>,
    },
    Index {
        /// Rebuild every vector instead of reusing unchanged vectors from the committed generation.
        #[arg(long)]
        full: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    Search {
        #[command(subcommand)]
        command: SearchCommand,
    },
    Chunk {
        #[command(subcommand)]
        command: ChunkCommand,
    },
    Source {
        #[command(subcommand)]
        command: SourceCommand,
    },
    Media {
        #[command(subcommand)]
        command: MediaCommand,
    },
    Permissions {
        #[command(subcommand)]
        command: PermissionsCommand,
    },
    WipeIndex {
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        json: bool,
    },
    /// Delete chats from the archive: messages, chunks, search rows, saved media, transcripts.
    ///
    /// Without `--yes` nothing is deleted and the report counts what would be. Semantic
    /// vectors cannot be removed one chat at a time, so a deletion leaves a rebuild marker in
    /// the semantic index directory, reported by `doctor`, until the next `katok index` (run
    /// `--full`). KakaoTalk itself is untouched: a later sync that reads the chat archives it
    /// again, whole, so drop it from `sync --chat` first. Deleting resets every incremental
    /// cursor, so the next `--incremental` sync reads the source in full once.
    Forget {
        /// chat_id to delete; repeat for more.
        #[arg(long = "chat", value_name = "CHAT_ID", required = true)]
        chats: Vec<String>,
        /// Delete. Without it, only report what would be deleted.
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        json: bool,
    },
    Chunks {
        #[arg(long)]
        chat: String,
        #[arg(long)]
        json: bool,
    },
    /// Export one chat's raw messages over a time range as a Markdown transcript.
    ///
    /// Reads the archive, not the live KakaoTalk database, so run `sync` first when the tail
    /// matters. A range holding no messages writes no file.
    Transcript {
        /// chat_id to export, as reported by `source chats` or a search hit.
        #[arg(long)]
        chat: String,
        /// Only include messages at or after this RFC3339 timestamp.
        #[arg(long)]
        since: Option<String>,
        /// Directory to write the transcript into.
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// List one chat's archived messages as JSON with sender identity (talk2build branch).
    ///
    /// Reads the archive, not the live KakaoTalk database. Returns the newest `--limit` rows
    /// inside the optional `--since` / `--until` RFC3339 range, ordered oldest first, so a
    /// caller can page a mirror view or read a delta after a cursor. Each message carries
    /// `mentioned_user_ids` (decimal strings) and `mentions_all` from KakaoTalk's mention
    /// metadata, never from the body text.
    Messages {
        /// chat_id as reported by `rooms` or `source chats`.
        #[arg(long)]
        chat: String,
        /// Only include messages at or after this RFC3339 timestamp.
        #[arg(long)]
        since: Option<String>,
        /// Only include messages at or before this RFC3339 timestamp.
        #[arg(long)]
        until: Option<String>,
        /// Maximum rows, taken from the newest end of the range.
        #[arg(long, default_value_t = 500, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..=100_000))]
        limit: usize,
        /// Force which sender_id counts as "me" (synthetic fixtures carry no account hash).
        #[arg(long)]
        me: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// List every chat in the archive with message count and newest message (talk2build branch).
    ///
    /// `chat_type` is `direct`, `group`, or `self` (the memo chat whose only member is you,
    /// named after your own nickname as KakaoTalk titles its window). `last_seen_log_id`,
    /// `new_message_count` and `my_watermark_log_id` are the read position KakaoTalk keeps for
    /// the room, as of the last macOS sync; `null` when KakaoTalk has none.
    Rooms {
        #[arg(long)]
        json: bool,
    },
    /// Send, stage, or inspect a KakaoTalk chat through its macOS UI.
    ///
    /// Unlike every other subcommand this writes rather than reads, and it does so by driving
    /// the running app's UI — there is no supported write path into the local archive. Opening
    /// a closed room, staging a draft, or sending an image can bring KakaoTalk forward briefly.
    // This drives the local KakaoTalk UI and is not a Kakao-approved API.
    // Message-affecting modes require an explicit acceptable-use acknowledgement.
    #[cfg(all(target_os = "macos", feature = "private-send"))]
    Send {
        /// Title of the chat as the chat list shows it. Note the self-chat window is titled
        /// with your own nickname, not "나와의 채팅".
        ///
        /// Names are not unique — several rooms can share one. When they do, the send is
        /// refused rather than guessed at; use `--chat` instead.
        #[arg(
            long,
            required_unless_present_any = ["chat", "list_windows", "list_rooms"]
        )]
        room: Option<String>,
        /// Address the room by its `chat_id`, as `search` and `chunks` report it.
        ///
        /// Unambiguous: the name and the last-message time are both read from the archive, and
        /// together they pick the right row even when two rooms share a name.
        #[arg(long, conflicts_with = "room")]
        chat: Option<String>,
        /// Message body. Reads stdin when omitted.
        #[arg(long)]
        text: Option<String>,
        /// Send an image file instead of text. Mutually exclusive with --text and --draft.
        #[arg(long, conflicts_with_all = ["text", "draft"])]
        image: Option<PathBuf>,
        /// List the chat windows currently open and exit without sending.
        #[arg(long)]
        list_windows: bool,
        /// List room names from the chat list (newest first) and exit without sending.
        #[arg(long)]
        list_rooms: bool,
        /// Cap for --list-rooms.
        #[arg(long, default_value_t = 40)]
        limit: usize,
        /// Resolve (and open) the room window but do not send. For verifying targeting safely.
        #[arg(long)]
        dry_run: bool,
        /// Fail instead of opening the room when its window is closed. Use for automation that
        /// must never touch the screen: opening a room briefly moves KakaoTalk's own windows.
        #[arg(long)]
        no_open: bool,
        /// Leave the message in the compose box for review instead of sending it.
        ///
        /// Pasted rather than typed, so nothing is delivered until a person presses Enter.
        /// Needs the screen for a moment, like sending an image does. Text only: it cannot be
        /// combined with --image, and an image is never staged as a draft.
        #[arg(long, conflicts_with_all = ["image", "dry_run"])]
        draft: bool,
        /// Take focus immediately instead of waiting for a gap in your typing.
        ///
        /// Only steps that cannot run in the background wait at all — sending an image, and
        /// opening a closed room. Use this when nobody is at the keyboard.
        #[arg(long)]
        take_focus_now: bool,
        /// Seconds to wait for that gap before giving up and sending nothing.
        #[arg(long, default_value_t = 15, conflicts_with = "take_focus_now")]
        focus_wait: u64,
        /// Confirm that you read and accept ACCEPTABLE_USE_POLICY.md and DISCLAIMER.md.
        ///
        /// Required for text, image, and draft modes. It does not legalize spam,
        /// harassment, impersonation, stalking, or any other prohibited use.
        #[arg(long)]
        accept_use_policy: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum SearchCommand {
    Keyword {
        query: String,
        /// Maximum number of results to return.
        #[arg(long, default_value_t = 10, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..=100_000))]
        limit: usize,
        /// Restrict hits to one chat_id (talk2build branch).
        #[arg(long)]
        chat: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Bm25 {
        query: String,
        /// Maximum number of results to return.
        #[arg(long, default_value_t = 10, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..=100_000))]
        limit: usize,
        /// Restrict hits to one chat_id (talk2build branch).
        #[arg(long)]
        chat: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Semantic {
        query: String,
        /// Maximum number of results to return.
        #[arg(long, default_value_t = 10, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..=100_000))]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ChunkCommand {
    Get {
        chunk_id: String,
        #[arg(long)]
        include_message_ids: bool,
        #[arg(long)]
        redact: bool,
        #[arg(long)]
        json: bool,
    },
    Context {
        chunk_id: String,
        #[arg(long)]
        json: bool,
    },
    Parent {
        chunk_id: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum SourceCommand {
    Chats {
        #[arg(long)]
        source: Option<String>,
        path: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum MediaCommand {
    /// Extract media from one room: photos, albums, videos, and file attachments.
    Get {
        /// KakaoTalk chatId to read media messages from.
        #[arg(long)]
        chat: i64,
        /// Optional KakaoTalk logId to extract one media message.
        #[arg(long)]
        log: Option<i64>,
        /// Output directory for decrypted/fetched media files.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Disable CDN downloads and use only local cache/thumbnail/stub tiers.
        /// File attachments have no local tier, so they resolve to nothing here.
        #[arg(long)]
        no_cdn: bool,
        /// Media kinds to extract; repeatable. Defaults to every kind.
        #[arg(long = "kind", value_name = "KIND")]
        kinds: Vec<String>,
        /// Maximum number of media messages to read from the room.
        #[arg(long, default_value_t = 5000, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..=100_000))]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Save every attachment whose CDN link is still valid, across all rooms.
    ///
    /// KakaoTalk presigns an attachment URL for roughly 14 days and keeps no
    /// local copy of file attachments, so anything not fetched inside that
    /// window is gone for good. Run this on a schedule to keep the window from
    /// closing on files you have not opened.
    ///
    /// Re-running is free: a frame whose output already exists is skipped
    /// without a network call.
    Backfill {
        /// Limit to one room instead of every room that holds media.
        #[arg(long)]
        chat: Option<i64>,
        /// Root output directory; each room gets a `<chatId>` subdirectory.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Media kinds to preserve; repeatable. Defaults to `file`, the only
        /// kind with no local cache to fall back on.
        #[arg(long = "kind", value_name = "KIND")]
        kinds: Vec<String>,
        /// Report which tier each frame would reach and what would be fetched, without
        /// downloading or writing anything: no request is sent and no file is written, local
        /// cache copies included.
        #[arg(long)]
        dry_run: bool,
        /// Refuse to download an attachment larger than this many bytes.
        #[arg(long, default_value_t = 512 * 1024 * 1024)]
        max_bytes: u64,
        /// Maximum number of media messages to read per room.
        #[arg(long, default_value_t = 100_000, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..=1_000_000))]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum PermissionsCommand {
    Macos {
        #[arg(long)]
        accessibility: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
}

#[cfg(all(test, target_os = "macos", feature = "private-send"))]
mod tests {
    use super::*;

    fn parse_error(args: &[&str]) -> clap::Error {
        match Cli::try_parse_from(args) {
            Ok(_) => panic!("{args:?} should have been rejected at parse time"),
            Err(error) => error,
        }
    }

    #[test]
    fn send_draft_with_image_is_rejected_at_parse_time() {
        // Either order: the conflict is symmetric, so the image can never be sent by a command
        // that also asked for a draft.
        for args in [
            [
                "katok", "send", "--room", "r", "--draft", "--image", "x.png",
            ],
            [
                "katok", "send", "--room", "r", "--image", "x.png", "--draft",
            ],
        ] {
            let error = parse_error(&args);
            assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
            assert_eq!(
                conflict_message(&error).as_deref(),
                Some(DRAFT_WITH_IMAGE),
                "{args:?}"
            );
        }
    }

    #[test]
    fn send_draft_with_dry_run_is_rejected_with_its_own_message() {
        let error = parse_error(&["katok", "send", "--room", "r", "--draft", "--dry-run"]);
        assert_eq!(
            conflict_message(&error).as_deref(),
            Some(DRAFT_WITH_DRY_RUN)
        );
    }

    #[test]
    fn send_draft_and_image_each_parse_alone() {
        for args in [
            ["katok", "send", "--room", "r", "--draft", "--text", "hi"],
            ["katok", "send", "--room", "r", "--image", "x.png", "--json"],
        ] {
            Cli::try_parse_from(args).unwrap_or_else(|error| panic!("{args:?}: {error}"));
        }
    }

    #[test]
    fn send_listing_modes_parse_without_target() {
        for flag in ["--list-windows", "--list-rooms"] {
            let cli = Cli::try_parse_from(["katok", "send", flag])
                .unwrap_or_else(|error| panic!("{flag} should not require --room: {error}"));

            assert!(matches!(
                cli.command,
                Commands::Send {
                    list_windows: true,
                    ..
                } | Commands::Send {
                    list_rooms: true,
                    ..
                }
            ));
        }
    }
}
