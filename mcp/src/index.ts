#!/usr/bin/env node
/**
 * katok-mcp: a thin stdio MCP server over the local katok KakaoTalk CLI.
 *
 * Every tool maps to exactly one `katok ... --json` invocation and returns katok's JSON as is.
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
  mediaGetArgs,
  messagesArgs,
  planSend,
  roomsArgs,
  searchArgs,
  syncArgs,
  transcriptArgs,
} from "./args.js";
import { DEFAULT_TIMEOUT_MS, KatokError, LONG_TIMEOUT_MS, runKatok } from "./katok.js";

const VERSION = "0.1.0";

function ok(value: unknown): CallToolResult {
  return { content: [{ type: "text", text: JSON.stringify(value, null, 2) }] };
}

function fail(message: string, detail?: unknown): CallToolResult {
  const text = detail === undefined ? message : `${message}\n\n${JSON.stringify(detail, null, 2)}`;
  return { isError: true, content: [{ type: "text", text }] };
}

function failFrom(error: unknown): CallToolResult {
  if (error instanceof KatokError) return fail(error.message, error.detail);
  return fail(error instanceof Error ? error.message : String(error));
}

/** Run one katok command and turn every failure into an MCP tool error. */
async function call(
  buildArgs: () => string[],
  options: { timeoutMs?: number; stdin?: string } = {},
): Promise<CallToolResult> {
  try {
    const args = [...globalArgs(process.env), ...buildArgs()];
    return ok(
      await runKatok(args, { timeoutMs: options.timeoutMs ?? DEFAULT_TIMEOUT_MS, ...options }),
    );
  } catch (error) {
    return failFrom(error);
  }
}

const rfc3339 = z.string().min(1).describe("RFC3339 timestamp, e.g. 2026-07-20T00:00:00+09:00");
const chatId = z.string().min(1).describe("chat_id as reported by katok_rooms or a search hit");
const limit = z.number().int().positive().max(100_000);

const READ_ONLY = { readOnlyHint: true, destructiveHint: false, openWorldHint: false } as const;

const server = new McpServer({ name: "katok-mcp", version: VERSION });

server.registerTool(
  "katok_doctor",
  {
    title: "katok doctor",
    description:
      "Run `katok doctor --json`: archive and index freshness, including freshness.recommendation (sync_before_search, index_before_semantic_search). Never triggers macOS permission prompts unless probe is true. probe: true adds --macos-probe, which may raise a macOS 'access data from other apps' (TCC) prompt — only set it when the user explicitly asks for a permission diagnostic.",
    inputSchema: {
      probe: z
        .boolean()
        .optional()
        .describe("Add --macos-probe. May raise a macOS TCC prompt. Default false."),
    },
    annotations: READ_ONLY,
  },
  async ({ probe }) => call(() => doctorArgs({ probe })),
);

server.registerTool(
  "katok_sync",
  {
    title: "katok sync",
    description:
      "Run `katok sync --source macos --json`: copy new messages from the live macOS KakaoTalk database into the local katok archive. Requires Full Disk Access for the process that launches this server (terminal / Claude Code). Writes only to the local katok archive; may take minutes on the first run (600 s timeout).",
    inputSchema: {
      incremental: z
        .boolean()
        .optional()
        .describe("Add --incremental: read only rows added since the last sync."),
    },
    annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: false },
  },
  async ({ incremental }) => call(() => syncArgs({ incremental }), { timeoutMs: LONG_TIMEOUT_MS }),
);

server.registerTool(
  "katok_rooms",
  {
    title: "katok rooms",
    description:
      "Run `katok rooms --json`: every chat in the archive with chat_id, chat_name, chat_type (direct/group/self), message_count and newest message time. Reads the archive, not live KakaoTalk; run katok_sync first when recency matters.",
    inputSchema: {},
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
      since: rfc3339.optional().describe("Only messages at or after this RFC3339 timestamp"),
      until: rfc3339.optional().describe("Only messages at or before this RFC3339 timestamp"),
      limit: limit
        .optional()
        .describe("Max rows from the newest end of the range (katok default 500)"),
    },
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
      "Run `katok search <keyword|bm25|semantic> <query> [--limit N] --json`. Returns a global top-k across rooms (default 10) with chunk_id, chat_id, chat_name, sender_nickname, started_at and a short snippet. keyword = exact substring, bm25 = ranked FTS, semantic = local EmbeddingGemma vectors. Semantic search first checks katok doctor and refuses when the index is missing or stale (run katok_index). chat_id filtering works for keyword and bm25 only.",
    inputSchema: {
      mode: z.enum(["keyword", "bm25", "semantic"]),
      query: z.string().min(1),
      limit: limit.optional().describe("Number of hits (katok default 10)"),
      chat_id: chatId.optional().describe("Restrict hits to one chat_id (keyword/bm25 only)"),
    },
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
      "Run `katok index [--full] --json`: build or refresh the local EmbeddingGemma vector index used by semantic search. The first run downloads the embedding model into the local cache; can take minutes (600 s timeout). Writes only to the local katok data directory.",
    inputSchema: {
      full: z.boolean().optional().describe("Add --full: rebuild every vector"),
      dry_run: z.boolean().optional().describe("Add --dry-run"),
    },
    annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: false },
  },
  async (input) => call(() => indexArgs(input), { timeoutMs: LONG_TIMEOUT_MS }),
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
    annotations: READ_ONLY,
  },
  async (input) => call(() => chunkArgs(input)),
);

server.registerTool(
  "katok_transcript",
  {
    title: "katok transcript",
    description:
      "Run `katok transcript --chat <id> [--since] [--out <dir>] --json`: export one chat's messages as a Markdown file and report its path. Reads the archive; a range with no messages writes no file.",
    inputSchema: {
      chat_id: chatId,
      since: rfc3339.optional(),
      out_dir: z
        .string()
        .min(1)
        .optional()
        .describe("Directory to write into (default: katok data dir)"),
    },
    annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: false },
  },
  async (input) => call(() => transcriptArgs(input)),
);

server.registerTool(
  "katok_media_get",
  {
    title: "katok media get",
    description:
      "Run `katok media get --chat <id> [--kind ...] [--out <dir>] [--no-cdn] --json`: extract photos, albums, videos and file attachments of one chat. Without no_cdn, katok may GET the attachment's presigned Kakao CDN URL (the only network access in katok); set no_cdn: true to stay fully local. File attachments have no local cache, so no_cdn returns nothing for them.",
    inputSchema: {
      chat_id: chatId,
      kind: z
        .array(z.enum(["photo", "video", "file"]))
        .optional()
        .describe("Default: every kind"),
      out_dir: z.string().min(1).optional(),
      no_cdn: z.boolean().optional(),
      log_id: z.string().min(1).optional().describe("Extract one media message by logId"),
      limit: limit.optional(),
    },
    annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: true },
  },
  async (input) => call(() => mediaGetArgs(input)),
);

server.registerTool(
  "katok_list_windows",
  {
    title: "katok list windows",
    description:
      "Run `katok send --list-windows --json`: list the KakaoTalk chat windows currently open. Read only; needs Accessibility permission. Sends nothing.",
    inputSchema: {},
    annotations: READ_ONLY,
  },
  async () => call(() => listWindowsArgs()),
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
      "draft and send are refused unless the server env has KATOK_MCP_ACCEPT_USE_POLICY=1 (the user accepted 36_katok/ACCEPTABLE_USE_POLICY.md and DISCLAIMER.md), and, when KATOK_MCP_ALLOWED_CHATS is set, unless chat_id is in that list.",
      "allow_open defaults to false, which adds --no-open so a closed room is never opened on screen.",
      "Never infer send permission from a search or summary request; confirm the exact room and final text with the user first. No loops, bulk or scheduled sends.",
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
    annotations: { readOnlyHint: false, destructiveHint: true, openWorldHint: true },
  },
  async (input) => {
    const plan = planSend(input, process.env);
    if (!plan.ok) return fail(`katok_send refused: ${plan.reason}`);
    return call(() => plan.args, { stdin: plan.stdin });
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
