/**
 * Pure argument builders for every katok invocation this server can make.
 *
 * Nothing here spawns a process or reads `process.env`; the send gate takes the environment
 * as a parameter so tests can pin every refusal without touching KakaoTalk.
 */

import path from "node:path";

/** Repository of the katok fork. Policy files the send gate points at live here. */
export const KATOK_REPO = "/Users/jeongdaun/enternext/01_business/36_katok";
export const POLICY_FILES = [
  `${KATOK_REPO}/ACCEPTABLE_USE_POLICY.md`,
  `${KATOK_REPO}/DISCLAIMER.md`,
] as const;

export type Env = Readonly<Record<string, string | undefined>>;

/** Optional global flags placed before the subcommand. */
export function globalArgs(env: Env): string[] {
  const dataDir = env.KATOK_MCP_DATA_DIR;
  return dataDir ? ["--data-dir", dataDir] : [];
}

export function doctorArgs(input: { probe?: boolean }): string[] {
  const args = ["doctor"];
  // --macos-probe can raise a macOS TCC prompt, so it is only ever an explicit opt-in.
  if (input.probe === true) args.push("--macos-probe");
  args.push("--json");
  return args;
}

export function syncArgs(input: { incremental?: boolean }): string[] {
  const args = ["sync", "--source", "macos"];
  if (input.incremental === true) args.push("--incremental");
  args.push("--json");
  return args;
}

export function roomsArgs(): string[] {
  return ["rooms", "--json"];
}

export function messagesArgs(input: {
  chat_id: string;
  since?: string;
  until?: string;
  limit?: number;
}): string[] {
  const args = ["messages", "--chat", input.chat_id];
  if (input.since) args.push("--since", input.since);
  if (input.until) args.push("--until", input.until);
  if (input.limit !== undefined) args.push("--limit", String(input.limit));
  args.push("--json");
  return args;
}

export type SearchMode = "keyword" | "bm25" | "semantic";

export function searchArgs(input: {
  mode: SearchMode;
  query: string;
  limit?: number;
  chat_id?: string;
}): string[] {
  const args = ["search", input.mode];
  if (input.limit !== undefined) args.push("--limit", String(input.limit));
  if (input.chat_id) {
    if (input.mode === "semantic") {
      throw new Error("chat_id filter is only supported for keyword and bm25 search");
    }
    args.push("--chat", input.chat_id);
  }
  // `--` keeps a query that starts with "-" from being parsed as a flag.
  args.push("--json", "--", input.query);
  return args;
}

export function indexArgs(input: { full?: boolean; dry_run?: boolean }): string[] {
  const args = ["index"];
  if (input.full === true) args.push("--full");
  if (input.dry_run === true) args.push("--dry-run");
  args.push("--json");
  return args;
}

export type ChunkAction = "get" | "context" | "parent";

export function chunkArgs(input: {
  action: ChunkAction;
  chunk_id: string;
  redact?: boolean;
  include_message_ids?: boolean;
}): string[] {
  const args = ["chunk", input.action];
  if (input.action === "get") {
    if (input.redact === true) args.push("--redact");
    if (input.include_message_ids === true) args.push("--include-message-ids");
  } else if (input.redact === true || input.include_message_ids === true) {
    throw new Error("redact and include_message_ids are only supported for action 'get'");
  }
  args.push("--json", "--", input.chunk_id);
  return args;
}

export function transcriptArgs(input: {
  chat_id: string;
  since?: string;
  out_dir?: string;
}): string[] {
  const args = ["transcript", "--chat", input.chat_id];
  if (input.since) args.push("--since", input.since);
  if (input.out_dir) args.push("--out", input.out_dir);
  args.push("--json");
  return args;
}

export type MediaKind = "photo" | "video" | "file";

export function mediaGetArgs(input: {
  chat_id: string;
  kind?: MediaKind[];
  out_dir?: string;
  no_cdn?: boolean;
  log_id?: string;
  limit?: number;
}): string[] {
  const args = ["media", "get", "--chat", input.chat_id];
  if (input.log_id) args.push("--log", input.log_id);
  for (const kind of input.kind ?? []) args.push("--kind", kind);
  if (input.out_dir) args.push("--out", input.out_dir);
  if (input.no_cdn === true) args.push("--no-cdn");
  if (input.limit !== undefined) args.push("--limit", String(input.limit));
  args.push("--json");
  return args;
}

export function listWindowsArgs(): string[] {
  return ["send", "--list-windows", "--json"];
}

export type SendMode = "dry_run" | "draft" | "send";

export interface SendInput {
  chat_id?: string;
  room?: string;
  text?: string;
  image_path?: string;
  mode?: SendMode;
  confirm?: boolean;
  allow_open?: boolean;
  take_focus_now?: boolean;
}

export type SendPlan =
  | { ok: true; mode: SendMode; args: string[]; stdin: string }
  | { ok: false; reason: string };

function reject(reason: string): SendPlan {
  return { ok: false, reason };
}

function allowedChats(env: Env): string[] | null {
  const raw = env.KATOK_MCP_ALLOWED_CHATS;
  if (raw === undefined || raw.trim() === "") return null;
  return raw
    .split(",")
    .map((id) => id.trim())
    .filter((id) => id.length > 0);
}

/**
 * Decide whether a send request may run and build its argv.
 *
 * Every refusal happens here, before any process is spawned. The message body travels over
 * stdin rather than argv so it never shows up in process listings.
 */
export function planSend(input: SendInput, env: Env): SendPlan {
  const mode: SendMode = input.mode ?? "dry_run";
  const hasChat = typeof input.chat_id === "string" && input.chat_id.length > 0;
  const hasRoom = typeof input.room === "string" && input.room.length > 0;
  const hasText = typeof input.text === "string" && input.text.length > 0;
  const hasImage = typeof input.image_path === "string" && input.image_path.length > 0;

  if (!hasChat && !hasRoom) return reject("chat_id or room is required (chat_id is preferred)");
  if (hasText && hasImage) return reject("text and image_path are mutually exclusive");

  const target = hasChat ? ["--chat", input.chat_id as string] : ["--room", input.room as string];
  const screenFlags: string[] = [];
  if (input.allow_open !== true) screenFlags.push("--no-open");
  if (input.take_focus_now === true) screenFlags.push("--take-focus-now");

  if (mode === "dry_run") {
    return {
      ok: true,
      mode,
      args: ["send", ...target, "--dry-run", ...screenFlags, "--json"],
      stdin: "",
    };
  }

  // draft and send both deliver content into KakaoTalk, so they share the remaining gates.
  if (mode === "send") {
    if (input.confirm !== true) {
      return reject(
        "mode 'send' delivers a real message and needs confirm: true. Run mode 'dry_run' first and confirm the exact room and content with the user.",
      );
    }
    if (env.KATOK_MCP_ALLOW_SEND !== "1") {
      return reject(
        "mode 'send' is disabled. The server operator must set KATOK_MCP_ALLOW_SEND=1 to enable real sends.",
      );
    }
  }

  if (mode === "draft" && hasImage) {
    return reject("mode 'draft' only stages text; image_path is not supported in draft mode");
  }
  if (!hasText && !hasImage) {
    return reject(`mode '${mode}' needs text or image_path`);
  }

  const allowed = allowedChats(env);
  if (allowed !== null) {
    if (!hasChat) {
      return reject(
        "KATOK_MCP_ALLOWED_CHATS is set, so draft/send must address the room by chat_id, not room name",
      );
    }
    if (!allowed.includes(input.chat_id as string)) {
      return reject(`chat_id ${input.chat_id} is not in KATOK_MCP_ALLOWED_CHATS`);
    }
  }

  if (env.KATOK_MCP_ACCEPT_USE_POLICY !== "1") {
    return reject(
      `mode '${mode}' needs --accept-use-policy. Read ${POLICY_FILES.join(" and ")}, then the server operator must set KATOK_MCP_ACCEPT_USE_POLICY=1.`,
    );
  }

  const args = ["send", ...target];
  let stdin = "";
  if (hasImage) {
    args.push("--image", path.resolve(input.image_path as string));
  } else {
    stdin = input.text as string;
  }
  if (mode === "draft") args.push("--draft");
  args.push(...screenFlags, "--accept-use-policy", "--json");
  return { ok: true, mode, args, stdin };
}
