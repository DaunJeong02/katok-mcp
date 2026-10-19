/**
 * Supplies KATOK_ARCHIVE_KEY to katok child processes.
 *
 * Priority: the KATOK_ARCHIVE_KEY environment variable, then one macOS Keychain lookup through
 * /usr/bin/security, then nothing (katok then reports the encrypted archive itself).
 * The key value never appears in logs, errors, or tool results. Only its source does.
 */

import { execFile } from "node:child_process";

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
