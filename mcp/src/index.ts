#!/usr/bin/env node
/**
 * katok-mcp: a thin stdio MCP server over the local katok KakaoTalk CLI (cli/ in this repo).
 *
 * Each tool maps to `katok ... --json` invocations and wraps katok's JSON in `{ ok, data, out_dir? }`
 * without interpreting it, except katok_index, whose documents[] becomes document_count. katok_init
 * and katok_sync (with_media) chain several invocations. katok_doctor adds binary_build (and a
 * warning when the in-repo binary is older than its source) beside data.
 * This process never opens the network, never reads KakaoTalk databases or key files, and
 * writes nothing to stdout except MCP protocol frames (diagnostics go to stderr).
 */

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { z } from "zod";
import {
  chunkArgs,
  doctorArgs,
  globalArgs,
  indexArgs,
  listWindowsArgs,
  mediaBackfillPlan,
  mediaGetArgs,
  mediaGetOutDir,
  messagesArgs,
  planSend,
  roomsArgs,
  searchArgs,
  syncPlan,
  TIMESTAMP_PATTERN,
  transcriptArgs,
  transcriptOutDir,
} from "./args.js";
import { inspectBinaryBuild } from "./buildStamp.js";
import { createFailureWriter, failureRecord } from "./failureLog.js";
import { sendFailureHint } from "./hints.js";
import {
  BACKFILL_TIMEOUT_MS,
  DEFAULT_TIMEOUT_MS,
  LONG_TIMEOUT_MS,
  resolveBin,
  runKatok,
} from "./katok.js";
import { createLock, type Lock } from "./lock.js";
import { dataDir, REPO_ROOT } from "./paths.js";
import { shrinkIndexResult } from "./results.js";
import { fail, failFrom, ok } from "./toolResult.js";
import { defaultInitDeps, initialize, runSyncPlan } from "./workflows.js";

const VERSION = "0.3.0";

/** Every tool answers `{ ok, data, out_dir? }`; data is katok's JSON, uninterpreted. */
const outputSchema = {
  ok: z.boolean().describe("true when the tool succeeded (failures are returned with isError)"),
  data: z.unknown().describe("katok's JSON output; its shape is documented by the katok CLI"),
  out_dir: z
    .string()
    .optional()
    .describe("Absolute directory on this Mac where files written by the call were saved"),
};

/** katok_doctor's wrapper also reports the binary build, and warns about a stale one. */
const doctorOutputSchema = {
  ...outputSchema,
  warning: z
    .string()
    .optional()
    .describe("Present when the in-repo katok binary looks older than its source; tell the user"),
  binary_build: z
    .object({
      path: z.string().describe("The katok binary this server runs"),
      ref: z.string().nullable().describe("Commit stamped by scripts/install.sh, or null"),
      mtime: z.string().nullable().describe("Binary modification time (ISO 8601), or null"),
    })
    .optional(),
};

const indexOutputSchema = {
  ...outputSchema,
  data: z
    .unknown()
    .describe(
      "katok index JSON. documents[] is replaced by document_count (its length) unless verbose is true",
    ),
};

/**
 * Two process-wide locks. Tools that drive the KakaoTalk UI share one; tools that write the
 * archive or files share the other. Read tools take neither.
 */
const uiLock: Lock = createLock();
const writeLock: Lock = createLock();

interface Prepared {
  args: string[];
  stdin?: string;
  outDir?: string;
}

/**
 * Run one katok command and turn every failure into an MCP tool error. `prepare` runs inside
 * the lock and the error handler, so an invalid out_dir becomes a tool error too.
 */
async function callPrepared(
  prepare: () => Prepared,
  options: {
    timeoutMs?: number;
    lock?: Lock;
    failureHint?: string;
    /** Reshapes katok's JSON before it is wrapped. */
    shape?: (data: unknown) => unknown;
    /** Sees every failure (including prepare's) before it becomes a tool error. */
    onError?: (error: unknown) => void;
  } = {},
): Promise<CallToolResult> {
  const run = async (): Promise<CallToolResult> => {
    try {
      const prepared = prepare();
      const data = await runKatok([...globalArgs(process.env), ...prepared.args], {
        timeoutMs: options.timeoutMs ?? DEFAULT_TIMEOUT_MS,
        ...(prepared.stdin === undefined ? {} : { stdin: prepared.stdin }),
      });
      return ok(options.shape ? options.shape(data) : data, prepared.outDir);
    } catch (error) {
      options.onError?.(error);
      return failFrom(error, options.failureHint);
    }
  };
  return options.lock ? options.lock(run) : run();
}

/** callPrepared for the common case: argv only. */
function call(
  buildArgs: () => string[],
  options: Parameters<typeof callPrepared>[1] = {},
): Promise<CallToolResult> {
  return callPrepared(() => ({ args: buildArgs() }), options);
}

/** Writes <data_dir>/last-<kind>-error.json; the data dir is resolved per call like globalArgs. */
function recordFailure(kind: "sync" | "index", tool: string, error: unknown, step?: string): void {
  createFailureWriter(dataDir(process.env))(kind, failureRecord(tool, error, new Date(), step));
}

const rfc3339 = z
  .string()
  .min(1)
  .regex(TIMESTAMP_PATTERN, {
    message: "Use RFC3339 with an offset (2026-07-20T00:00:00Z) or a date (2026-07-20)",
  })
  .describe(
    "RFC3339 timestamp with offset, e.g. 2026-07-20T00:00:00Z, or a date YYYY-MM-DD (00:00:00 UTC)",
  );
const chatId = z.string().min(1).describe("chat_id as reported by katok_rooms or a search hit");
const limit = z.number().int().positive().max(100_000);

/** Reads the local archive only. */
const READ_ONLY = {
  readOnlyHint: true,
  destructiveHint: false,
  idempotentHint: true,
  openWorldHint: false,
} as const;

/** Writes local files, never deletes, same arguments end in the same state. */
const LOCAL_WRITE = {
  readOnlyHint: false,
  destructiveHint: false,
  idempotentHint: true,
  openWorldHint: false,
} as const;

/** Like LOCAL_WRITE, but may also reach the Kakao CDN or a model download. */
const NETWORK_WRITE = { ...LOCAL_WRITE, openWorldHint: true } as const;

const server = new McpServer({ name: "katok-mcp", version: VERSION });

server.registerTool(
  "katok_init",
  {
    title: "katok init",
    description: [
      "One-time setup of the dedicated katok-mcp archive (default data dir ~/Library/Application Support/katok-mcp, or KATOK_MCP_DATA_DIR).",
      "If neither KATOK_ARCHIVE_KEY nor the Keychain item (service com.enternext.katok-mcp, account archive-key) holds a key, generates a random one with `openssl rand -hex 32` and stores it in the macOS Keychain; the key value is never returned or logged.",
      "Then creates the data directory, runs `katok sync --source macos --json` once and returns `katok doctor --json`.",
      "Already initialized (archive exists and doctor reports a completed sync with messages)? It only returns doctor and syncs nothing. An archive whose first sync never completed is synced again on a re-run.",
      "The first sync is one transaction: a timeout keeps nothing, and a re-run reads everything again. After a timeout, give the user the terminal command from the error (no MCP time limit, key read from the Keychain); re-running katok_init only helps when the delay was one-off, such as a pending permission prompt.",
      "Requires Full Disk Access for the process that launched this server (Terminal / Claude Code / Codex). The first run can raise macOS prompts (TCC 'access data from other apps', Keychain access): ask the user to allow them, choosing 'Always Allow' for Keychain.",
      "Refuses to create a new key when an archive already exists without a Keychain key, because that would orphan or silently re-encrypt it. The first sync can take minutes (600 s timeout).",
    ].join(" "),
    inputSchema: {},
    outputSchema,
    annotations: LOCAL_WRITE,
  },
  async () => {
    try {
      const result = await writeLock(() => initialize(defaultInitDeps(process.env)));
      return ok(result);
    } catch (error) {
      return failFrom(error);
    }
  },
);

server.registerTool(
  "katok_doctor",
  {
    title: "katok doctor",
    description:
      "Run `katok doctor --json`: archive and index freshness, including freshness.recommendation (sync_before_search, index_before_semantic_search). Never triggers macOS permission prompts unless probe is true. probe: true adds --macos-probe, which may raise a macOS 'access data from other apps' (TCC) prompt — only set it when the user explicitly asks for a permission diagnostic. Beside data, binary_build reports the katok binary (path, the commit scripts/install.sh stamped, mtime); warning appears when the in-repo binary has no stamp or is older than cli/src, and means: re-run scripts/install.sh.",
    inputSchema: {
      probe: z
        .boolean()
        .optional()
        .describe("Add --macos-probe. May raise a macOS TCC prompt. Default false."),
    },
    outputSchema: doctorOutputSchema,
    annotations: READ_ONLY,
  },
  async ({ probe }) => {
    try {
      const data = await runKatok([...globalArgs(process.env), ...doctorArgs({ probe })]);
      let build: ReturnType<typeof inspectBinaryBuild> | null = null;
      try {
        build = inspectBinaryBuild(resolveBin(process.env), REPO_ROOT);
      } catch {
        // Reporting the build is best effort: doctor itself already succeeded.
      }
      return ok(data, undefined, build ?? {});
    } catch (error) {
      return failFrom(error);
    }
  },
);

server.registerTool(
  "katok_sync",
  {
    title: "katok sync",
    description: [
      "Run `katok sync --source macos --json`: copy new messages from the live macOS KakaoTalk database into the local katok archive. Requires Full Disk Access for the process that launches this server (terminal / Claude Code / Codex). Writes only to the local archive; may take minutes on the first run (600 s timeout). Run katok_init first on a fresh setup.",
      "Result data is { sync, backfill? }.",
      "chat_id: [ids] syncs only those rooms (repeated --chat); katok keeps a separate incremental cursor for that exact set, so a later sync without chat_id still reads every room.",
      "with_media: true then runs `katok media backfill` (all kinds, every room even with chat_id, output <data_dir>/media) after a successful sync and returns both results plus out_dir. This reaches the Kakao CDN (the only network access) and can download a lot; use katok_media_backfill with dry_run first when the size is unknown.",
      "A failure is also kept in <data_dir>/last-sync-error.json (time, exit status, katok error code, stderr head; never message content).",
    ].join(" "),
    inputSchema: {
      incremental: z
        .boolean()
        .optional()
        .describe("Add --incremental: read only rows added since the last sync."),
      chat_id: z
        .array(chatId)
        .min(1)
        .max(200)
        .optional()
        .describe(
          "Sync only these chat_ids (strings, as reported by katok_rooms). Default: every room.",
        ),
      with_media: z
        .boolean()
        .optional()
        .describe(
          "After a successful sync, also download attachments (photos, videos, files) whose CDN link is still valid. Default false.",
        ),
    },
    outputSchema,
    annotations: NETWORK_WRITE,
  },
  async (input) => {
    try {
      const steps = syncPlan(input, process.env);
      const outcome = await writeLock(() =>
        runSyncPlan(
          steps,
          (args, options) => runKatok([...globalArgs(process.env), ...args], options),
          (step, error) => recordFailure("sync", "katok_sync", error, step),
        ),
      );
      return ok(outcome.data, outcome.out_dir);
    } catch (error) {
      return failFrom(error);
    }
  },
);

server.registerTool(
  "katok_rooms",
  {
    title: "katok rooms",
    description:
      "Run `katok rooms --json`: every chat in the archive with chat_id, chat_name, chat_type (direct/group/self), message_count and newest message time. Reads the archive, not live KakaoTalk; run katok_sync first when recency matters.",
    inputSchema: {},
    outputSchema,
    annotations: READ_ONLY,
  },
  async () => call(() => roomsArgs()),
);

server.registerTool(
  "katok_messages",
  {
    title: "katok messages",
    description:
      "Run `katok messages --chat <id> [--since] [--until] [--limit] --json`: the newest `limit` archived messages of one chat inside the optional RFC3339 range, ordered oldest first, with sender identity.",
    inputSchema: {
      chat_id: chatId,
      since: rfc3339
        .optional()
        .describe(
          "Only messages at or after this time: RFC3339 with offset, e.g. 2026-07-20T00:00:00Z, or YYYY-MM-DD (00:00:00 UTC)",
        ),
      until: rfc3339
        .optional()
        .describe(
          "Only messages at or before this time: RFC3339 with offset, e.g. 2026-07-20T00:00:00Z, or YYYY-MM-DD (00:00:00 UTC)",
        ),
      limit: limit
        .optional()
        .describe("Max rows from the newest end of the range (katok default 500)"),
    },
    outputSchema,
    annotations: READ_ONLY,
  },
  async (input) => call(() => messagesArgs(input)),
);

function semanticNeedsIndex(doctor: unknown): boolean {
  const rec = (
    doctor as { freshness?: { recommendation?: { index_before_semantic_search?: unknown } } }
  )?.freshness?.recommendation;
  return rec?.index_before_semantic_search === true;
}

server.registerTool(
  "katok_search",
  {
    title: "katok search",
    description:
      "Run `katok search <keyword|bm25|semantic> <query> [--limit N] --json`. Returns a global top-k across rooms (default 10) with chunk_id, chat_id, chat_name, sender_nickname, started_at and a short snippet. keyword = exact substring, newest hits first (--order newest; order: oldest reverses), bm25 = ranked FTS, semantic = local EmbeddingGemma vectors. Semantic search first checks katok doctor and refuses when the index is missing or stale (run katok_index). chat_id filtering works for keyword and bm25 only.",
    inputSchema: {
      mode: z.enum(["keyword", "bm25", "semantic"]),
      query: z.string().min(1),
      limit: limit.optional().describe("Number of hits (katok default 10)"),
      chat_id: chatId.optional().describe("Restrict hits to one chat_id (keyword/bm25 only)"),
      order: z
        .enum(["newest", "oldest"])
        .optional()
        .describe("keyword only: hit order by time (default newest). bm25/semantic rank by score"),
    },
    outputSchema,
    annotations: READ_ONLY,
  },
  async (input) => {
    if (input.mode === "semantic") {
      try {
        const doctor = await runKatok([...globalArgs(process.env), ...doctorArgs({})]);
        if (semanticNeedsIndex(doctor)) {
          const reason = (doctor as { freshness?: { recommendation?: { reason?: unknown } } })
            .freshness?.recommendation?.reason;
          return fail(
            "Semantic search needs an up-to-date index first: katok doctor reports freshness.recommendation.index_before_semantic_search = true. Run katok_index (katok index --json), then retry. keyword and bm25 search work without the index.",
            { reason },
          );
        }
      } catch (error) {
        return failFrom(error);
      }
    }
    return call(() => searchArgs(input));
  },
);

server.registerTool(
  "katok_index",
  {
    title: "katok index",
    description:
      "Run `katok index [--full] --json`: build or refresh the local EmbeddingGemma vector index used by semantic search. The first run downloads the embedding model into the local cache; can take minutes (600 s timeout). Writes only to the local katok data directory. katok lists every indexed window in documents[] (thousands of entries); this tool returns document_count instead unless verbose is true. A failure is also kept in <data_dir>/last-index-error.json.",
    inputSchema: {
      full: z.boolean().optional().describe("Add --full: rebuild every vector"),
      dry_run: z.boolean().optional().describe("Add --dry-run"),
      verbose: z
        .boolean()
        .optional()
        .describe("Keep katok's full documents[] list (can be megabytes). Default false"),
    },
    outputSchema: indexOutputSchema,
    annotations: NETWORK_WRITE,
  },
  async ({ verbose, ...input }) =>
    call(() => indexArgs(input), {
      timeoutMs: LONG_TIMEOUT_MS,
      lock: writeLock,
      shape: (data) => shrinkIndexResult(data, verbose === true),
      onError: (error) => recordFailure("index", "katok_index", error),
    }),
);

server.registerTool(
  "katok_chunk",
  {
    title: "katok chunk",
    description:
      "Run `katok chunk <get|context|parent> <chunk-id> --json`. get returns the full chunk text (use only when the user asks to open a result or gives a chunk id); context returns the neighbouring chunks; parent returns the 5-minute parent window. redact (get only) masks the entire text.",
    inputSchema: {
      action: z.enum(["get", "context", "parent"]),
      chunk_id: z.string().min(1),
      redact: z.boolean().optional().describe("Add --redact (get only)"),
      include_message_ids: z.boolean().optional().describe("Add --include-message-ids (get only)"),
    },
    outputSchema,
    annotations: READ_ONLY,
  },
  async (input) => call(() => chunkArgs(input)),
);

server.registerTool(
  "katok_transcript",
  {
    title: "katok transcript",
    description:
      "Run `katok transcript --chat <id> [--since] [--out <dir>] --json`: export one chat's messages as a Markdown file and report its path. Reads the archive; a range with no messages writes no file. Default folder <data_dir>/transcripts, reported as out_dir.",
    inputSchema: {
      chat_id: chatId,
      since: rfc3339.optional(),
      out_dir: z
        .string()
        .min(1)
        .optional()
        .describe(
          "Directory to write into (default: <data_dir>/transcripts). Must be inside the data dir or a root listed in the server env KATOK_MCP_OUT_ROOTS (colon separated); `~` is the home directory.",
        ),
    },
    outputSchema,
    annotations: LOCAL_WRITE,
  },
  async (input) =>
    callPrepared(
      () => ({
        args: transcriptArgs(input, process.env),
        outDir: transcriptOutDir(input, process.env),
      }),
      { lock: writeLock },
    ),
);

server.registerTool(
  "katok_media_get",
  {
    title: "katok media get",
    description:
      "Run `katok media get --chat <id> [--log <logId>] [--kind ...] [--out <dir>] [--no-cdn] [--limit N] --json`: extract photos, albums, videos and file attachments of ONE chat (use katok_media_backfill for every room). Without no_cdn, katok may GET the attachment's presigned Kakao CDN URL (the only network access in katok); set no_cdn: true to stay fully local. File attachments have no local cache, so no_cdn returns nothing for them. Files land in out_dir (default <data_dir>/media/<chat_id>).",
    inputSchema: {
      chat_id: chatId,
      kind: z
        .array(z.enum(["photo", "video", "file"]))
        .optional()
        .describe("Default: every kind"),
      out_dir: z
        .string()
        .min(1)
        .optional()
        .describe(
          "Directory to write into (default: <data_dir>/media/<chat_id>). Must be inside the data dir or a root listed in the server env KATOK_MCP_OUT_ROOTS (colon separated); `~` is the home directory.",
        ),
      no_cdn: z.boolean().optional(),
      log_id: z
        .string()
        .regex(/^\d+$/, "log_id is the numeric logId of one media message")
        .optional()
        .describe("Extract one media message by logId (--log)"),
      limit: limit
        .optional()
        .describe("Max media messages read from the room (katok default 5000)"),
    },
    outputSchema,
    annotations: NETWORK_WRITE,
  },
  async (input) =>
    callPrepared(
      () => ({
        args: mediaGetArgs(input, process.env),
        outDir: mediaGetOutDir(input, process.env),
      }),
      { lock: writeLock },
    ),
);

server.registerTool(
  "katok_media_backfill",
  {
    title: "katok media backfill",
    description: [
      "Run `katok media backfill [--chat] [--kind ...] [--dry-run] [--max-bytes] [--limit] --out <dir> --json`: save every attachment whose download link is still valid, across all rooms (or one with chat_id).",
      "File attachments have no local cache and their CDN signed link expires (about 14 days, then HTTP 410 and the file is gone for good), so they must be fetched soon after they are received. Photos and videos have only a lazily filled local cache and the same expiring link.",
      "Kinds default to all three (file, photo, video). Files already saved are skipped without a network call, so re-running is free and resumes an interrupted run.",
      "Use dry_run: true first: it sends no network request and reports tier_totals (planned = would be downloaded from the CDN, stub = expired or unavailable, existing = already saved) and bytes (the CDN download size) so the user can judge the size before anything is downloaded.",
      "Caveat (katok behaviour): dry_run still decrypts photos and videos that are already in KakaoTalk's local cache (tiers full/thumb) into out_dir; only the CDN download is skipped. kind: ['file'] has no local cache, so a file-only dry run writes nothing.",
      "Output goes to out_dir (default <data_dir>/media, one <chat_id> folder per room), also returned as out_dir. The only network access is the Kakao CDN download. Timeout 30 minutes.",
    ].join(" "),
    inputSchema: {
      chat_id: chatId.optional().describe("Limit to one room (default: every room with media)"),
      kind: z
        .array(z.enum(["photo", "video", "file"]))
        .optional()
        .describe("Kinds to save (default: file, photo and video)"),
      dry_run: z
        .boolean()
        .optional()
        .describe(
          "Send no network request and report what would be downloaded. Photos/videos already in KakaoTalk's local cache are still written to out_dir.",
        ),
      max_bytes: z
        .number()
        .int()
        .positive()
        .optional()
        .describe("Refuse attachments larger than this many bytes (katok default 512 MiB)"),
      limit: limit
        .max(1_000_000)
        .optional()
        .describe("Max media messages read per room (katok default 100000)"),
      out_dir: z
        .string()
        .min(1)
        .optional()
        .describe(
          "Root folder (default: <data_dir>/media). Must be inside the data dir or a root listed in the server env KATOK_MCP_OUT_ROOTS (colon separated); `~` is the home directory.",
        ),
    },
    outputSchema,
    annotations: NETWORK_WRITE,
  },
  async (input) =>
    callPrepared(
      () => {
        const plan = mediaBackfillPlan(input, process.env);
        return { args: plan.args, outDir: plan.out_dir };
      },
      { timeoutMs: BACKFILL_TIMEOUT_MS, lock: writeLock },
    ),
);

server.registerTool(
  "katok_list_windows",
  {
    title: "katok list windows",
    description:
      "Run `katok send --list-windows --json`: list the KakaoTalk chat windows currently open. Read only; needs Accessibility permission (`katok permissions macos --accessibility` opens the settings). Sends nothing.",
    inputSchema: {},
    outputSchema,
    annotations: { ...READ_ONLY, openWorldHint: true },
  },
  async () => call(() => listWindowsArgs(), { lock: uiLock }),
);

server.registerTool(
  "katok_send",
  {
    title: "katok send (gated)",
    description: [
      "Address a KakaoTalk chat through the official macOS app UI. Sending is irreversible and reaches other people.",
      "mode dry_run (default): `katok send --chat <id> --dry-run --no-open --json` — resolves the room only.",
      "mode draft: pastes text into the compose box without sending (--draft --accept-use-policy). Text only.",
      "mode send: delivers the message. Refused unless confirm is true AND the server env has KATOK_MCP_ALLOW_SEND=1.",
      "draft and send are refused unless the server env has KATOK_MCP_ACCEPT_USE_POLICY=1 (the user accepted cli/ACCEPTABLE_USE_POLICY.md and cli/DISCLAIMER.md), and, when KATOK_MCP_ALLOWED_CHATS is set, unless chat_id is in that list.",
      "allow_open defaults to false, which adds --no-open so a closed room is never opened on screen.",
      "Never infer send permission from a search or summary request; confirm the exact room and final text with the user first. No loops, bulk or scheduled sends.",
      "If this tool fails (even by timeout) the message may still have been delivered: check KakaoTalk before trying again, never retry blindly.",
    ].join(" "),
    inputSchema: {
      chat_id: chatId.optional().describe("Preferred: unambiguous room id"),
      room: z
        .string()
        .min(1)
        .optional()
        .describe("Room title; names are not unique, prefer chat_id"),
      text: z.string().min(1).optional().describe("Message body (passed to katok via stdin)"),
      image_path: z.string().min(1).optional().describe("Image file to send (mode send only)"),
      mode: z.enum(["dry_run", "draft", "send"]).optional().describe("Default dry_run"),
      confirm: z.boolean().optional().describe("Must be true for mode send"),
      allow_open: z
        .boolean()
        .optional()
        .describe("Allow katok to open a closed room window (default false → --no-open)"),
      take_focus_now: z.boolean().optional().describe("Add --take-focus-now"),
    },
    outputSchema,
    annotations: {
      readOnlyHint: false,
      destructiveHint: true,
      idempotentHint: false,
      openWorldHint: true,
    },
  },
  async (input) => {
    const plan = planSend(input, process.env);
    if (!plan.ok) return fail(`katok_send refused: ${plan.reason}`);
    // A failed send may still have reached the room: no retry or narrowing hint, only "check first".
    return callPrepared(() => ({ args: plan.args, stdin: plan.stdin }), {
      lock: uiLock,
      failureHint: sendFailureHint(plan.mode),
    });
  },
);

async function main(): Promise<void> {
  const transport = new StdioServerTransport();
  await server.connect(transport);
  process.stderr.write(`katok-mcp ${VERSION} ready on stdio\n`);
}

main().catch((error: unknown) => {
  process.stderr.write(
    `katok-mcp failed to start: ${error instanceof Error ? error.stack : String(error)}\n`,
  );
  process.exit(1);
});
