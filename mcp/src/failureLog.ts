/**
 * Keeps the most recent sync or index failure on disk, so it survives the MCP session that saw
 * it: `<data_dir>/last-sync-error.json` and `<data_dir>/last-index-error.json`.
 *
 * The record holds only where and how the call failed: the time, the tool, the exit status,
 * katok's error code and the head of stderr. katok's stdout (message bodies, the full error
 * envelope) and the argv are never written, and nothing here sees the archive key. A later
 * success leaves the file in place: it is "the last failure", not "the current state".
 */

import { renameSync, writeFileSync } from "node:fs";
import path from "node:path";
import { KatokError } from "./katok.js";

export type FailureKind = "sync" | "index";

export const FAILURE_FILES: Readonly<Record<FailureKind, string>> = {
  sync: "last-sync-error.json",
  index: "last-index-error.json",
};

const STDERR_HEAD = 2000;
const MESSAGE_HEAD = 500;

export interface FailureRecord {
  at: string;
  tool: string;
  /** The failed step of a multi-step call (katok_sync with_media: "sync" or "backfill"). */
  step?: string;
  exitCode: number | null;
  signal: string | null;
  /** katok's `error.code` from its JSON failure envelope, when it printed one. */
  errorCode: string | null;
  stderrHead: string;
  /**
   * Only for failures before katok ran (a refusal, a missing binary). Those messages are built
   * by this server. katok's own messages are left out because they can quote its stdout.
   */
  message?: string;
}

/** Build the record for one failure. Pure: `now` is passed in. */
export function failureRecord(
  tool: string,
  error: unknown,
  now: Date,
  step?: string,
): FailureRecord {
  const base =
    step === undefined ? { at: now.toISOString(), tool } : { at: now.toISOString(), tool, step };
  if (error instanceof KatokError) {
    const code = (error.detail.katokError as { code?: unknown } | undefined)?.code;
    return {
      ...base,
      exitCode: error.detail.exitCode,
      signal: error.detail.signal,
      errorCode: typeof code === "string" ? code : null,
      stderrHead: error.detail.stderr.slice(0, STDERR_HEAD),
    };
  }
  const message = error instanceof Error ? error.message : String(error);
  return {
    ...base,
    exitCode: null,
    signal: null,
    errorCode: null,
    stderrHead: "",
    message: message.slice(0, MESSAGE_HEAD),
  };
}

export type FailureWriter = (kind: FailureKind, record: FailureRecord) => void;

export interface FailureFs {
  writeFile: (file: string, text: string) => void;
  rename: (from: string, to: string) => void;
}

const nodeFs: FailureFs = {
  writeFile: (file, text) => writeFileSync(file, text, { encoding: "utf8", mode: 0o600 }),
  rename: renameSync,
};

/**
 * A writer into `dir`. It never throws: a failure to record must not hide the failure being
 * recorded, so it only prints a one-line note (no record content) to stderr. It does not
 * create `dir`; with no data directory there is no archive whose failures matter.
 */
export function createFailureWriter(
  dir: string,
  fs: FailureFs = nodeFs,
  warn: (line: string) => void = (line) => process.stderr.write(`${line}\n`),
  pid: number = process.pid,
): FailureWriter {
  return (kind, record) => {
    const file = path.join(dir, FAILURE_FILES[kind]);
    // Per process: two servers (two Claude Code sessions) may record into one data dir at once.
    const tmp = `${file}.${pid}.tmp`;
    try {
      fs.writeFile(tmp, `${JSON.stringify(record, null, 2)}\n`);
      fs.rename(tmp, file);
    } catch (error) {
      const code = (error as { code?: unknown }).code;
      warn(`katok-mcp: could not write ${file} (${typeof code === "string" ? code : "error"})`);
    }
  };
}
