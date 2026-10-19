/**
 * Where things live: the katok binary, the dedicated data directory, and the policy files.
 *
 * Everything is derived from the location of this compiled file (`<repo>/mcp/dist/paths.js`)
 * or from an explicit environment variable. Nothing here spawns a process.
 */

import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

export type Env = Readonly<Record<string, string | undefined>>;

/** The repository root, computed from the compiled module location (`<repo>/mcp/dist/*.js`). */
export function repoRoot(moduleUrl: string = import.meta.url): string {
  return path.resolve(path.dirname(fileURLToPath(moduleUrl)), "..", "..");
}

export const REPO_ROOT = repoRoot();
export const CLI_DIR = path.join(REPO_ROOT, "cli");

/** Policy files the send gate points at. They ship inside the cli/ subtree. */
export const POLICY_FILES = [
  path.join(CLI_DIR, "ACCEPTABLE_USE_POLICY.md"),
  path.join(CLI_DIR, "DISCLAIMER.md"),
] as const;

/** The dedicated data directory: its own archive, its own auth cache, its own media. */
export function defaultDataDir(home: string = os.homedir()): string {
  return path.join(home, "Library", "Application Support", "katok-mcp");
}

/** `KATOK_MCP_DATA_DIR` when set and non-empty, otherwise the dedicated default. */
export function dataDir(env: Env): string {
  const override = env.KATOK_MCP_DATA_DIR;
  return override ? path.resolve(override) : defaultDataDir();
}

/** Default root for downloaded attachments; katok adds a `<chat_id>` folder per room. */
export function mediaDir(env: Env): string {
  return path.join(dataDir(env), "media");
}

/** Default folder for transcript exports (katok's own default under the data directory). */
export function transcriptDir(env: Env): string {
  return path.join(dataDir(env), "transcripts");
}

export function archivePath(env: Env): string {
  return path.join(dataDir(env), "archive.sqlite3");
}

/**
 * katok binary candidates, first match wins: KATOK_BIN, the repo's own release build, then
 * a cargo-installed binary.
 */
export function binCandidates(
  env: Env,
  repo: string = REPO_ROOT,
  home: string = os.homedir(),
): string[] {
  const list: string[] = [];
  if (env.KATOK_BIN) list.push(env.KATOK_BIN);
  list.push(path.join(repo, "cli", "target", "release", "katok"));
  list.push(path.join(home, ".cargo", "bin", "katok"));
  return list;
}
