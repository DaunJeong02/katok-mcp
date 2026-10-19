/**
 * Process boundary: locate the katok binary and run it with `execFile` (no shell).
 *
 * katok prints one JSON document on stdout when `--json` is set, including a
 * `{ "ok": false, "error": {...} }` envelope on failure. Anything else — non-JSON output,
 * a non-zero exit, a timeout, or that failure envelope — becomes a thrown KatokError that
 * carries the exit code, stderr, and the head of stdout.
 */

import { execFile } from "node:child_process";
import { accessSync, constants } from "node:fs";
import os from "node:os";
import path from "node:path";
import { KATOK_REPO } from "./args.js";

export const DEFAULT_TIMEOUT_MS = 120_000;
export const LONG_TIMEOUT_MS = 600_000;
const MAX_BUFFER = 256 * 1024 * 1024;
const STDOUT_HEAD = 4000;

export class KatokError extends Error {
  constructor(
    message: string,
    readonly detail: {
      args: string[];
      exitCode: number | null;
      signal: string | null;
      stderr: string;
      stdoutHead: string;
      katokError?: unknown;
    },
  ) {
    super(message);
    this.name = "KatokError";
  }
}

function isExecutable(file: string): boolean {
  try {
    accessSync(file, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

export function binCandidates(env: NodeJS.ProcessEnv): string[] {
  const list: string[] = [];
  if (env.KATOK_BIN) list.push(env.KATOK_BIN);
  list.push(path.join(KATOK_REPO, "target/release/katok"));
  list.push(path.join(os.homedir(), ".cargo/bin/katok"));
  return list;
}

export function resolveBin(env: NodeJS.ProcessEnv = process.env): string {
  const candidates = binCandidates(env);
  const found = candidates.find(isExecutable);
  if (!found) {
    throw new Error(
      `katok binary not found. Tried: ${candidates.join(", ")}. Set KATOK_BIN to the katok executable.`,
    );
  }
  return found;
}

function isFailureEnvelope(value: unknown): value is { ok: false; error?: unknown } {
  return (
    typeof value === "object" &&
    value !== null &&
    "ok" in value &&
    (value as { ok: unknown }).ok === false
  );
}

function describeKatokError(error: unknown): string {
  if (typeof error !== "object" || error === null) return "katok reported failure";
  const body = error as { code?: unknown; message?: unknown; cause?: unknown };
  const text = (value: unknown) => (typeof value === "string" && value.length > 0 ? value : null);
  const code = text(body.code);
  const message = text(body.message);
  const cause = text(body.cause);
  // katok's cause usually repeats the message as its prefix ("open archive: open archive: ...").
  const tail = cause && message && cause.startsWith(message) ? cause : [message, cause];
  const parts = [code, tail].flat().filter((part): part is string => part !== null);
  return parts.length > 0 ? parts.join(": ") : "katok reported failure";
}

export interface RunOptions {
  timeoutMs?: number;
  stdin?: string;
  env?: NodeJS.ProcessEnv;
}

export function runKatok(args: string[], options: RunOptions = {}): Promise<unknown> {
  const env = options.env ?? process.env;
  const bin = resolveBin(env);
  const timeout = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;

  return new Promise((resolve, reject) => {
    const child = execFile(
      bin,
      args,
      { env, timeout, maxBuffer: MAX_BUFFER, encoding: "utf8", windowsHide: true },
      (error, stdout, stderr) => {
        const stdoutText = String(stdout ?? "");
        const stderrText = String(stderr ?? "").trim();
        const exitCode = error ? (typeof error.code === "number" ? error.code : null) : 0;
        const signal = error?.signal ?? null;
        const detail = {
          args,
          exitCode,
          signal: signal === null ? null : String(signal),
          stderr: stderrText,
          stdoutHead: stdoutText.slice(0, STDOUT_HEAD),
        };

        let parsed: unknown;
        let parseError: string | null = null;
        try {
          parsed = JSON.parse(stdoutText.trim());
        } catch (err) {
          parseError = err instanceof Error ? err.message : String(err);
        }

        if (error?.killed && signal) {
          reject(
            new KatokError(`katok timed out or was killed (${signal}) after ${timeout} ms`, detail),
          );
          return;
        }
        if (parseError !== null) {
          const why = error
            ? `katok exited with code ${exitCode ?? "unknown"} and printed non-JSON output`
            : `katok printed non-JSON output (${parseError})`;
          reject(new KatokError(why, detail));
          return;
        }
        if (isFailureEnvelope(parsed)) {
          reject(
            new KatokError(`katok failed: ${describeKatokError(parsed.error)}`, {
              ...detail,
              katokError: parsed.error,
            }),
          );
          return;
        }
        if (error) {
          reject(new KatokError(`katok exited with code ${exitCode ?? "unknown"}`, detail));
          return;
        }
        resolve(parsed);
      },
    );

    // Always close stdin. `katok send` reads the body from stdin when --text is absent, so an
    // open pipe would hang until the timeout.
    child.stdin?.on("error", () => {
      // The child may exit before reading stdin (EPIPE); its exit status reports the real outcome.
    });
    child.stdin?.end(options.stdin ?? "");
  });
}
