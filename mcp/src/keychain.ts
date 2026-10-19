/**
 * macOS Keychain access through /usr/bin/security, plus archive key generation.
 *
 * The key is a secret: it must never reach an error message, a log line, or a tool result.
 * `execFile` errors embed the whole command line (including `-w <key>`), so the write path
 * builds its own messages from stderr and never from `error.message`.
 */

import { execFile } from "node:child_process";

/** The first access can wait on a macOS "allow access" dialog, so give a person time to answer. */
const KEYCHAIN_TIMEOUT_MS = 60_000;
const KEY_PATTERN = /^[0-9a-f]{64}$/;

/** `security` exits 44 and says "could not be found" when the item does not exist. */
const NOT_FOUND_EXIT_CODE = 44;

export class KeychainNotFoundError extends Error {
  constructor(service: string, account: string) {
    super(`keychain item (service ${service}, account ${account}) was not found`);
    this.name = "KeychainNotFoundError";
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

/** Store (or update, `-U`) one generic password. */
export const writeKeychain: KeychainWriter = (service, account, key) =>
  new Promise((resolve, reject) => {
    execFile(
      "/usr/bin/security",
      ["add-generic-password", "-s", service, "-a", account, "-w", key, "-U"],
      { timeout: KEYCHAIN_TIMEOUT_MS, encoding: "utf8" },
      (error, _stdout, stderr) => {
        if (error) {
          // Not error.message: it repeats the argv, which contains the key.
          const text = String(stderr ?? "")
            .replaceAll(key, "[redacted]")
            .trim();
          const code: unknown = (error as { code?: unknown }).code;
          reject(
            new Error(
              `security add-generic-password failed: ${text || `exit code ${String(code ?? "unknown")}`}`,
            ),
          );
          return;
        }
        resolve();
      },
    );
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
