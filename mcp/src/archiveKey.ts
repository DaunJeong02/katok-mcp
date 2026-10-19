/**
 * Supplies KATOK_ARCHIVE_KEY to katok child processes.
 *
 * Priority: the KATOK_ARCHIVE_KEY environment variable, then one macOS Keychain lookup through
 * /usr/bin/security, then nothing (katok then reports the encrypted archive itself).
 * The key value never appears in logs, errors, or tool results. Only its source does.
 */

import { closeSync, openSync, readSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { type KeychainReader, readKeychain } from "./keychain.js";
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

/**
 * Create a resolver that reads the Keychain at most once per process and caches the outcome,
 * success or failure, so a denied dialog is not raised again on every tool call.
 */
export function createKeyProvider(reader: KeychainReader = readKeychain) {
  let cached: Promise<KeyResolution> | null = null;

  async function lookup(target: KeychainTarget): Promise<KeyResolution> {
    try {
      const key = (await reader(target.service, target.account)).trim();
      if (key.length === 0) {
        return { source: "none", reason: "keychain item is empty" };
      }
      return { source: "keychain", key };
    } catch (error) {
      const why = error instanceof Error ? error.message : String(error);
      return {
        source: "none",
        reason: `keychain lookup (service ${target.service}, account ${target.account}) failed: ${why}`,
      };
    }
  }

  async function resolve(env: Env): Promise<KeyResolution> {
    if (env[ARCHIVE_KEY_ENV]) return { source: "env" };
    const target = keychainTarget(env);
    if (target === null) {
      return { source: "none", reason: "keychain lookup disabled by KATOK_MCP_KEYCHAIN=0" };
    }
    cached ??= lookup(target);
    return cached;
  }

  /** Forget the cached outcome. katok_init calls this after it stores a new key. */
  function reset(): void {
    cached = null;
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
