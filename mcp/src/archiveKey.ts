/**
 * Supplies KATOK_ARCHIVE_KEY to katok child processes.
 *
 * Priority: the KATOK_ARCHIVE_KEY environment variable, then one macOS Keychain lookup through
 * /usr/bin/security, then nothing (katok then reports the encrypted archive itself).
 * The key value never appears in logs, errors, or tool results. Only its source does.
 */

import { closeSync, existsSync, openSync, readSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { KeychainNotFoundError, type KeychainReader, readKeychain } from "./keychain.js";
import type { Env } from "./paths.js";

export const ARCHIVE_KEY_ENV = "KATOK_ARCHIVE_KEY";
export const DEFAULT_KEYCHAIN_SERVICE = "com.enternext.katok-mcp";
export const DEFAULT_KEYCHAIN_ACCOUNT = "archive-key";
export const KEY_HINT =
  "KATOK_ARCHIVE_KEY 환경변수 또는 키체인 항목이 필요합니다. katok_init을 한 번 실행하면 키체인 항목을 만들고 첫 동기화까지 합니다 (run katok_init once to create the Keychain item, or set KATOK_ARCHIVE_KEY).";

export type KeySource = "env" | "keychain" | "none";

export interface KeyResolution {
  source: KeySource;
  /** Present only when source is "keychain". Never log or return it. */
  key?: string;
  /** Why no key is available. Safe to show: it never contains the key. */
  reason?: string;
}

export interface KeychainTarget {
  service: string;
  account: string;
}

/** Where to look in the Keychain, or null when the lookup must not run. */
export function keychainTarget(env: Env): KeychainTarget | null {
  if (env[ARCHIVE_KEY_ENV]) return null;
  if (env.KATOK_MCP_KEYCHAIN === "0") return null;
  return {
    service: env.KATOK_MCP_KEYCHAIN_SERVICE || DEFAULT_KEYCHAIN_SERVICE,
    account: env.KATOK_MCP_KEYCHAIN_ACCOUNT || DEFAULT_KEYCHAIN_ACCOUNT,
  };
}

/** How long "the Keychain has no such item" is remembered, so a missing item does not re-prompt on every call. */
export const NOT_FOUND_TTL_MS = 30_000;

/**
 * Create a resolver for the Keychain key. Only a successful lookup is cached for the life of
 * the process. "Item not found" is remembered for NOT_FOUND_TTL_MS. Every other failure (a
 * denied dialog, a timeout, a locked Keychain) is never cached, so the next call asks again
 * instead of running katok keyless for the rest of the process. Concurrent calls share one
 * lookup. `now` is injectable for tests.
 */
export function createKeyProvider(
  reader: KeychainReader = readKeychain,
  now: () => number = Date.now,
) {
  interface Entry {
    service: string;
    account: string;
    resolution: KeyResolution;
    /** Epoch ms after which the entry is stale, or null for "until reset()". */
    expiresAt: number | null;
  }
  let cached: Entry | null = null;
  let inflight: { service: string; account: string; promise: Promise<KeyResolution> } | null = null;
  let generation = 0;

  async function lookup(target: KeychainTarget): Promise<KeyResolution> {
    const mine = generation;
    const remember = (resolution: KeyResolution, ttlMs: number | null) => {
      if (mine !== generation) return resolution; // reset() ran meanwhile: this answer may be stale
      cached = {
        ...target,
        resolution,
        expiresAt: ttlMs === null ? null : now() + ttlMs,
      };
      return resolution;
    };
    try {
      const key = (await reader(target.service, target.account)).trim();
      if (key.length === 0) {
        return { source: "none", reason: "keychain item is empty" };
      }
      return remember({ source: "keychain", key }, null);
    } catch (error) {
      const why = error instanceof Error ? error.message : String(error);
      const resolution: KeyResolution = {
        source: "none",
        reason: `keychain lookup (service ${target.service}, account ${target.account}) failed: ${why}`,
      };
      return error instanceof KeychainNotFoundError
        ? remember(resolution, NOT_FOUND_TTL_MS)
        : resolution;
    }
  }

  async function resolve(env: Env): Promise<KeyResolution> {
    if (env[ARCHIVE_KEY_ENV]) return { source: "env" };
    const target = keychainTarget(env);
    if (target === null) {
      return { source: "none", reason: "keychain lookup disabled by KATOK_MCP_KEYCHAIN=0" };
    }
    const same = (entry: { service: string; account: string }) =>
      entry.service === target.service && entry.account === target.account;
    if (
      cached !== null &&
      same(cached) &&
      (cached.expiresAt === null || now() < cached.expiresAt)
    ) {
      return cached.resolution;
    }
    if (inflight !== null && same(inflight)) return inflight.promise;
    const promise = lookup(target).finally(() => {
      if (inflight?.promise === promise) inflight = null;
    });
    inflight = { ...target, promise };
    return promise;
  }

  /** Forget everything cached. katok_init calls this once the Keychain holds the key. */
  function reset(): void {
    generation += 1;
    cached = null;
    inflight = null;
  }

  return Object.assign(resolve, { reset });
}

/** Child-process env with the key filled in when it came from the Keychain. */
export function withArchiveKey(
  env: NodeJS.ProcessEnv,
  resolution: KeyResolution,
): NodeJS.ProcessEnv {
  if (resolution.source !== "keychain" || resolution.key === undefined) return env;
  return { ...env, [ARCHIVE_KEY_ENV]: resolution.key };
}

export const WRONG_KEY_HINT =
  "키체인에서 읽은 키로 이 아카이브를 열지 못했습니다 (the Keychain key does not open this archive). 데이터 폴더(KATOK_MCP_DATA_DIR 또는 기본 ~/Library/Application Support/katok-mcp)의 아카이브가 다른 키로 암호화돼 있습니다. 맞는 키를 KATOK_ARCHIVE_KEY로 주거나, 키체인 항목을 원래 키로 복구하거나, 데이터 폴더를 옮긴 뒤 katok_init을 다시 실행하세요.";

/** True when SQLCipher rejected the supplied key ("file is not a database"). */
export function isWrongKeyFailure(text: string): boolean {
  return text.includes("file is not a database");
}

/** True when a katok failure is about the archive key. */
export function isArchiveKeyFailure(text: string): boolean {
  return text.includes(ARCHIVE_KEY_ENV) || /archive .* is encrypted/.test(text);
}

/** katok's own default when no --data-dir is given (cli/src/paths.rs default_data_dir). */
export function katokDefaultDataDir(): string {
  return path.join(os.homedir(), "Library", "Application Support", "katok");
}

/**
 * The data directory katok will use for this exact argv. Global options come before the
 * subcommand, so only a leading `--data-dir <dir>` counts, as katok's own parser sees it.
 */
export function dataDirFromArgs(args: readonly string[]): string {
  const index = args.indexOf("--data-dir");
  const firstNonGlobal = args.findIndex((arg) => !arg.startsWith("--"));
  const value = index >= 0 ? args[index + 1] : undefined;
  const isGlobal = index >= 0 && (firstNonGlobal === -1 || index < firstNonGlobal);
  return isGlobal && value ? path.resolve(value) : katokDefaultDataDir();
}

export const SQLITE_PLAINTEXT_HEADER = Buffer.from("SQLite format 3\0", "latin1");

/** First 16 bytes of a file, or null when it does not exist. Other errors propagate. */
export type HeaderReader = (file: string) => Buffer | null;

export const readHeader: HeaderReader = (file) => {
  let fd: number;
  try {
    fd = openSync(file, "r");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return null;
    throw error;
  }
  try {
    const buffer = Buffer.alloc(SQLITE_PLAINTEXT_HEADER.length);
    const read = readSync(fd, buffer, 0, buffer.length, 0);
    return buffer.subarray(0, read);
  } finally {
    closeSync(fd);
  }
};

export const PLAINTEXT_REFUSAL =
  "이 아카이브는 평문입니다. 키체인 키를 주입하면 katok이 제자리에서 암호화하므로 중단했습니다. 의도한 것이면 KATOK_ARCHIVE_KEY를 직접 설정하세요.";

/**
 * Refuse to hand a Keychain key to katok for a plaintext archive: katok would encrypt it in
 * place (cli/src/archive.rs encrypt_in_place). An explicit KATOK_ARCHIVE_KEY env var is
 * the operator's own choice and is not checked. A missing archive is fine: the first sync
 * creates it with that key on purpose.
 *
 * Returns null when the call may proceed, or the refusal message.
 */
export function plaintextGuard(
  resolution: KeyResolution,
  args: readonly string[],
  read: HeaderReader = readHeader,
): string | null {
  if (resolution.source !== "keychain") return null;
  const archive = path.join(dataDirFromArgs(args), "archive.sqlite3");
  let header: Buffer | null;
  try {
    header = read(archive);
  } catch (error) {
    const why = error instanceof Error ? error.message : String(error);
    return `${archive} 헤더를 확인하지 못해 키체인 키 주입을 중단했습니다 (could not read the archive header: ${why}).`;
  }
  if (header?.equals(SQLITE_PLAINTEXT_HEADER)) {
    return `${PLAINTEXT_REFUSAL} (${archive})`;
  }
  return null;
}

/** Subcommand words after katok's leading global flags, e.g. ["send", "--list-windows", "--json"]. */
export function commandArgs(args: readonly string[]): string[] {
  let index = 0;
  while (index < args.length && (args[index] ?? "").startsWith("--")) {
    // Only --data-dir takes a separate value among the global flags this server passes.
    index += args[index] === "--data-dir" ? 2 : 1;
  }
  return args.slice(index);
}

/** Commands that never open (and so never create) the archive. */
export function opensArchive(args: readonly string[]): boolean {
  const [command, ...rest] = commandArgs(args);
  if (command === "doctor" || command === "permissions") return false;
  if (command === "send" && (rest.includes("--list-windows") || rest.includes("--list-rooms"))) {
    return false;
  }
  return true;
}

export const NO_KEY_NO_ARCHIVE =
  "아카이브(archive.sqlite3)가 아직 없고 아카이브 키도 없어 실행하지 않았습니다. katok은 파일이 없으면 평문 아카이브를 새로 만듭니다. katok_init을 먼저 실행하세요 (run katok_init first; it creates the Keychain key and the encrypted archive, or set KATOK_ARCHIVE_KEY).";

/**
 * Refuse to start a command that would make katok create a brand-new plaintext archive: no key
 * to encrypt it with (neither KATOK_ARCHIVE_KEY nor a readable Keychain item) and no archive
 * file yet. Commands that never open the archive pass. An explicit KATOK_ARCHIVE_KEY passes.
 *
 * Returns null when the call may proceed, or the refusal message (never containing a key).
 */
export function missingArchiveGuard(
  resolution: KeyResolution,
  args: readonly string[],
  exists: (file: string) => boolean = existsSync,
): string | null {
  if (resolution.source !== "none" || !opensArchive(args)) return null;
  const archive = path.join(dataDirFromArgs(args), "archive.sqlite3");
  if (exists(archive)) return null;
  const why = resolution.reason ? ` (${resolution.reason})` : "";
  return `${NO_KEY_NO_ARCHIVE}${why} [${archive}]`;
}
