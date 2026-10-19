/**
 * Supplies KATOK_ARCHIVE_KEY to katok child processes.
 *
 * Priority: the KATOK_ARCHIVE_KEY environment variable, then one macOS Keychain lookup through
 * /usr/bin/security, then nothing (katok then reports the encrypted archive itself).
 * The key value never appears in logs, errors, or tool results. Only its source does.
 */

import { execFile } from "node:child_process";
import { closeSync, openSync, readSync } from "node:fs";
import os from "node:os";
import path from "node:path";

export const ARCHIVE_KEY_ENV = "KATOK_ARCHIVE_KEY";
export const DEFAULT_KEYCHAIN_SERVICE = "com.enternext.enter-cut";
export const DEFAULT_KEYCHAIN_ACCOUNT = "katok-archive-key";
export const KEY_HINT =
  "KATOK_ARCHIVE_KEY 환경변수 또는 키체인 항목이 필요합니다 (KATOK_ARCHIVE_KEY env var or a Keychain item is required).";

/** The first lookup can wait on a macOS "allow access" dialog, so give a person time to answer. */
const KEYCHAIN_TIMEOUT_MS = 60_000;

export type Env = Readonly<Record<string, string | undefined>>;

export type KeySource = "env" | "keychain" | "none";

export interface KeyResolution {
  source: KeySource;
  /** Present only when source is "keychain". Never log or return it. */
  key?: string;
  /** Why no key is available. Safe to show: it never contains the key. */
  reason?: string;
}

export type KeychainReader = (service: string, account: string) => Promise<string>;

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

/** Read one generic password with /usr/bin/security. Rejects without the value on failure. */
export const readKeychain: KeychainReader = (service, account) =>
  new Promise((resolve, reject) => {
    execFile(
      "/usr/bin/security",
      ["find-generic-password", "-s", service, "-a", account, "-w"],
      { timeout: KEYCHAIN_TIMEOUT_MS, encoding: "utf8" },
      (error, stdout, stderr) => {
        if (error) {
          // stderr carries messages such as "could not be found"; stdout would hold the secret.
          const why = String(stderr ?? "").trim() || error.message;
          reject(new Error(`security find-generic-password failed: ${why}`));
          return;
        }
        resolve(String(stdout));
      },
    );
  });

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

  return async function resolve(env: Env): Promise<KeyResolution> {
    if (env[ARCHIVE_KEY_ENV]) return { source: "env" };
    const target = keychainTarget(env);
    if (target === null) {
      return { source: "none", reason: "keychain lookup disabled by KATOK_MCP_KEYCHAIN=0" };
    }
    cached ??= lookup(target);
    return cached;
  };
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
  "키체인에서 읽은 키로 이 아카이브를 열지 못했습니다 (the Keychain key does not open this archive). The default Keychain item belongs to the enter-cut archive at ~/Library/Application Support/enter-cut/katok: set KATOK_MCP_DATA_DIR to that directory, or supply the matching KATOK_ARCHIVE_KEY.";

/** True when SQLCipher rejected the supplied key ("file is not a database"). */
export function isWrongKeyFailure(text: string): boolean {
  return text.includes("file is not a database");
}

/** True when a katok failure is about the archive key. */
export function isArchiveKeyFailure(text: string): boolean {
  return text.includes(ARCHIVE_KEY_ENV) || /archive .* is encrypted/.test(text);
}

/** katok's own default when no --data-dir is given (36_katok src/paths.rs default_data_dir). */
export function defaultDataDir(): string {
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
  return isGlobal && value ? path.resolve(value) : defaultDataDir();
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
 * place (36_katok src/archive.rs encrypt_in_place). An explicit KATOK_ARCHIVE_KEY env var is
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
