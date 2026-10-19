/**
 * macOS Keychain access through /usr/bin/security, plus archive key generation.
 *
 * The key is a secret: it must never reach an error message, a log line, or a tool result.
 * The write path hands the key to `security -i` on stdin (never argv, so not visible in `ps`) and
 * builds its error messages from stderr, never from `error.message`.
 */

import { execFile } from "node:child_process";

/** The first access can wait on a macOS "allow access" dialog, so give a person time to answer. */
const KEYCHAIN_TIMEOUT_MS = 60_000;
const KEY_PATTERN = /^[0-9a-f]{64}$/;

/** `security` exits 44 and says "could not be found" when the item does not exist. */
const NOT_FOUND_EXIT_CODE = 44;
/** `security add-generic-password` without -U exits 45 when the item already exists. */
const DUPLICATE_EXIT_CODE = 45;

export class KeychainNotFoundError extends Error {
  constructor(service: string, account: string) {
    super(`keychain item (service ${service}, account ${account}) was not found`);
    this.name = "KeychainNotFoundError";
  }
}

/** Thrown by the writer when the item already exists, e.g. a concurrent katok_init stored a key first. */
export class KeychainDuplicateError extends Error {
  constructor(service: string, account: string) {
    super(`keychain item (service ${service}, account ${account}) already exists`);
    this.name = "KeychainDuplicateError";
  }
}

export type KeychainReader = (service: string, account: string) => Promise<string>;
export type KeychainWriter = (service: string, account: string, key: string) => Promise<void>;
export type KeyGenerator = () => Promise<string>;

/** Read one generic password. Rejects without the value on failure. */
export const readKeychain: KeychainReader = (service, account) =>
  new Promise((resolve, reject) => {
    execFile(
      "/usr/bin/security",
      ["find-generic-password", "-s", service, "-a", account, "-w"],
      { timeout: KEYCHAIN_TIMEOUT_MS, encoding: "utf8" },
      (error, stdout, stderr) => {
        if (error) {
          // stderr carries messages such as "could not be found"; stdout would hold the secret.
          const text = String(stderr ?? "").trim();
          const code: unknown = (error as { code?: unknown }).code;
          if (code === NOT_FOUND_EXIT_CODE || /could not be found/i.test(text)) {
            reject(new KeychainNotFoundError(service, account));
            return;
          }
          reject(new Error(`security find-generic-password failed: ${text || error.message}`));
          return;
        }
        resolve(String(stdout));
      },
    );
  });

/** Quote one argument for `security -i`'s command parser. Characters it cannot carry are refused. */
function quoteForSecurity(label: string, value: string): string {
  const unsafe = [...value].some((ch) => ch === '"' || ch === "\\" || ch.charCodeAt(0) < 0x20);
  if (value.length === 0 || unsafe) {
    throw new Error(
      `keychain ${label} must be non-empty and free of quotes, backslashes and control characters`,
    );
  }
  return `"${value}"`;
}

/**
 * The one command line fed to `security -i` on stdin. The key travels over stdin, not argv, so
 * it never shows up in `ps`. No `-U`: an existing item must not be overwritten.
 */
export function addCommandLine(service: string, account: string, key: string): string {
  return [
    "add-generic-password",
    "-s",
    quoteForSecurity("service", service),
    "-a",
    quoteForSecurity("account", account),
    "-w",
    quoteForSecurity("key", key),
  ].join(" ");
}

/** Turn a failed `security -i add-generic-password` into an error that never holds the key. */
export function addFailure(
  service: string,
  account: string,
  key: string,
  code: unknown,
  stderr: string,
): Error {
  const text = stderr.replaceAll(key, "[redacted]").trim();
  if (code === DUPLICATE_EXIT_CODE || /already exists/i.test(text)) {
    return new KeychainDuplicateError(service, account);
  }
  return new Error(
    `security add-generic-password failed: ${text || `exit code ${String(code ?? "unknown")}`}`,
  );
}

/**
 * Create one generic password through `security -i` (commands on stdin). Fails with
 * KeychainDuplicateError when the item exists; it never updates one.
 */
export const writeKeychain: KeychainWriter = (service, account, key) =>
  new Promise((resolve, reject) => {
    let command: string;
    try {
      command = addCommandLine(service, account, key);
    } catch (error) {
      reject(error);
      return;
    }
    const child = execFile(
      "/usr/bin/security",
      ["-i"],
      { timeout: KEYCHAIN_TIMEOUT_MS, encoding: "utf8" },
      (error, _stdout, stderr) => {
        if (error) {
          // Not error.message: it can repeat the command line.
          reject(
            addFailure(
              service,
              account,
              key,
              (error as { code?: unknown }).code,
              String(stderr ?? ""),
            ),
          );
          return;
        }
        resolve();
      },
    );
    child.stdin?.on("error", () => {
      // security may exit before reading stdin; its exit status reports the outcome.
    });
    child.stdin?.end(`${command}\n`);
  });

/** 32 random bytes as 64 lowercase hex characters, from `openssl rand -hex 32`. */
export const generateArchiveKey: KeyGenerator = () =>
  new Promise((resolve, reject) => {
    execFile(
      "/usr/bin/openssl",
      ["rand", "-hex", "32"],
      { timeout: 10_000, encoding: "utf8" },
      (error, stdout) => {
        if (error) {
          reject(new Error(`openssl rand -hex 32 failed (exit code ${error.code ?? "unknown"})`));
          return;
        }
        const key = String(stdout).trim();
        if (!KEY_PATTERN.test(key)) {
          reject(new Error("openssl rand -hex 32 did not print 64 hex characters"));
          return;
        }
        resolve(key);
      },
    );
  });
