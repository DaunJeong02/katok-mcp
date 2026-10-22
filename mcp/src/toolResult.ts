/**
 * MCP tool results: the `{ ok, data, out_dir? }` success wrapper and the error text with its
 * next-step hint. Pure, so the exact text a tool error shows can be tested.
 */

import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { nextStepHint } from "./hints.js";
import { HintedError, KatokError } from "./katok.js";

/** `extras` sit beside data at the wrapper level; `warning` is placed before data so it is seen. */
export function ok(
  data: unknown,
  outDir?: string,
  extras: Record<string, unknown> = {},
): CallToolResult {
  const { warning, ...rest } = extras;
  const body = {
    ok: true,
    ...(warning === undefined ? {} : { warning }),
    data,
    ...(outDir === undefined ? {} : { out_dir: outDir }),
    ...rest,
  };
  return {
    content: [{ type: "text", text: JSON.stringify(body, null, 2) }],
    structuredContent: body,
  };
}

/**
 * A tool error. `hintOverride` replaces the generic next-step hint (katok_send must never be
 * told to narrow or repeat the call).
 */
export function fail(message: string, detail?: unknown, hintOverride?: string): CallToolResult {
  const hint =
    hintOverride ??
    nextStepHint(`${message}\n${detail === undefined ? "" : JSON.stringify(detail)}`);
  const head = hint === null ? message : `${message}\n\n${hint}`;
  const text = detail === undefined ? head : `${head}\n\n${JSON.stringify(detail, null, 2)}`;
  return { isError: true, content: [{ type: "text", text }] };
}

export function failFrom(error: unknown, hintOverride?: string): CallToolResult {
  // An error that brings its own hint (katok_init's first-sync timeout) keeps it.
  if (error instanceof HintedError) return failFrom(error.inner, hintOverride ?? error.hint);
  if (error instanceof KatokError) return fail(error.message, error.detail, hintOverride);
  return fail(error instanceof Error ? error.message : String(error), undefined, hintOverride);
}
