/**
 * Multi-step tool flows, with every side effect injected so tests can run them without a
 * Keychain, a filesystem or a katok binary.
 *
 *  - initialize: the katok_init flow (Keychain key, data directory, first sync, doctor).
 *  - runSyncPlan: katok_sync, optionally followed by a media backfill.
 */

import { existsSync, mkdirSync } from "node:fs";
import { ARCHIVE_KEY_ENV, keychainTarget } from "./archiveKey.js";
import { doctorArgs, globalArgs, type PlanStep, syncArgs } from "./args.js";
import {
  BACKFILL_TIMEOUT_MS,
  DEFAULT_TIMEOUT_MS,
  LONG_TIMEOUT_MS,
  resetArchiveKey,
  runKatok,
  withNote,
} from "./katok.js";
import {
  generateArchiveKey,
  KeychainNotFoundError,
  type KeychainReader,
  type KeychainWriter,
  type KeyGenerator,
  readKeychain,
  writeKeychain,
} from "./keychain.js";
import { archivePath, dataDir, type Env } from "./paths.js";

/** Runs katok with the global flags already in place. */
export type Runner = (args: string[], options: { timeoutMs: number }) => Promise<unknown>;

export interface InitDeps {
  env: Env;
  readKey: KeychainReader;
  writeKey: KeychainWriter;
  generateKey: KeyGenerator;
  archiveExists: (file: string) => boolean;
  makeDir: (dir: string) => void;
  run: Runner;
  resetKey: () => void;
}

export interface InitResult {
  key: {
    source: "env" | "keychain";
    /** True when this call generated and stored a new key. */
    created: boolean;
    service?: string;
    account?: string;
  };
  data_dir: string;
  /** True when the archive already existed, in which case the first sync was skipped. */
  already_initialized: boolean;
  sync: unknown | null;
  doctor: unknown;
}

export function defaultInitDeps(env: Env = process.env): InitDeps {
  return {
    env,
    readKey: readKeychain,
    writeKey: writeKeychain,
    generateKey: generateArchiveKey,
    archiveExists: existsSync,
    makeDir: (dir) => {
      mkdirSync(dir, { recursive: true, mode: 0o700 });
    },
    run: (args, options) => runKatok([...globalArgs(env), ...args], options),
    resetKey: resetArchiveKey,
  };
}

/**
 * Set up the dedicated archive: ensure an archive key exists (env var or Keychain, generating
 * and storing one only when the Keychain definitively has none), create the data directory,
 * run the first sync, and report doctor. Already-initialized setups only get doctor.
 *
 * The key value is never part of the result or of any thrown message.
 */
export async function initialize(deps: InitDeps): Promise<InitResult> {
  const { env } = deps;
  const dir = dataDir(env);
  const archive = archivePath(env);
  const archiveExisted = deps.archiveExists(archive);

  let key: InitResult["key"];
  if (env[ARCHIVE_KEY_ENV]) {
    key = { source: "env", created: false };
  } else {
    const target = keychainTarget(env);
    if (target === null) {
      throw new Error(
        "KATOK_MCP_KEYCHAIN=0 이고 KATOK_ARCHIVE_KEY도 없어 아카이브 키를 둘 곳이 없습니다. KATOK_MCP_KEYCHAIN=0을 지우거나 KATOK_ARCHIVE_KEY를 설정한 뒤 katok_init을 다시 실행하세요.",
      );
    }
    const where = `service ${target.service}, account ${target.account}`;
    let existing: string | null = null;
    try {
      existing = (await deps.readKey(target.service, target.account)).trim();
    } catch (error) {
      if (!(error instanceof KeychainNotFoundError)) {
        const why = error instanceof Error ? error.message : String(error);
        throw new Error(
          `키체인(${where})을 읽지 못해 키를 새로 만들지 않았습니다 (${why}). macOS 키체인 접근 팝업이 떠 있으면 "항상 허용"을 누르고 katok_init을 다시 실행하세요.`,
        );
      }
    }
    if (existing !== null && existing.length === 0) {
      throw new Error(
        `키체인 항목(${where})이 비어 있어 덮어쓰지 않았습니다. 항목을 지우고(security delete-generic-password -s ${target.service} -a ${target.account}) katok_init을 다시 실행하세요.`,
      );
    }
    if (existing !== null) {
      key = { source: "keychain", created: false, ...target };
    } else {
      if (archiveExisted) {
        throw new Error(
          `${archive}에 아카이브가 이미 있는데 키체인(${where})에는 키가 없습니다. 새 키를 만들면 기존 아카이브를 열 수 없거나(암호화됨) 평문 아카이브가 제자리에서 암호화됩니다. 원래 키를 키체인에 복구하거나 KATOK_ARCHIVE_KEY로 주거나, 데이터 폴더를 옮긴 뒤 katok_init을 다시 실행하세요.`,
        );
      }
      const fresh = await deps.generateKey();
      await deps.writeKey(target.service, target.account, fresh);
      // A lookup cached before this point still says "no key"; the sync below must see the new one.
      deps.resetKey();
      key = { source: "keychain", created: true, ...target };
    }
  }

  deps.makeDir(dir);

  let sync: unknown | null = null;
  if (!archiveExisted) {
    try {
      sync = await deps.run(syncArgs({}), { timeoutMs: LONG_TIMEOUT_MS });
    } catch (error) {
      throw withNote(
        error,
        `${key.created ? "아카이브 키는 이미 키체인에 저장됐습니다. " : ""}macOS 권한(Full Disk Access)을 확인한 뒤 katok_sync를 실행하거나 katok_init을 다시 실행하세요`,
      );
    }
  }
  const doctor = await deps.run(doctorArgs({}), { timeoutMs: DEFAULT_TIMEOUT_MS });
  return { key, data_dir: dir, already_initialized: archiveExisted, sync, doctor };
}

export interface SyncOutcome {
  data: { sync: unknown; backfill?: unknown };
  out_dir?: string;
}

/**
 * Run the planned steps in order and stop at the first failure. When a later step fails, the
 * error says that the earlier ones already succeeded, so the caller repeats only the failed one.
 */
export async function runSyncPlan(steps: PlanStep[], run: Runner): Promise<SyncOutcome> {
  const data: { sync: unknown; backfill?: unknown } = { sync: null };
  let outDir: string | undefined;
  const done: string[] = [];
  for (const step of steps) {
    const timeoutMs = step.name === "backfill" ? BACKFILL_TIMEOUT_MS : LONG_TIMEOUT_MS;
    try {
      const result = await run(step.args, { timeoutMs });
      if (step.name === "sync") data.sync = result;
      else data.backfill = result;
    } catch (error) {
      if (done.length === 0) throw error;
      throw withNote(
        error,
        `${done.join(", ")} 단계는 이미 끝났습니다. 실패한 ${step.name}만 다시 실행하세요 (katok_media_backfill)`,
      );
    }
    done.push(step.name);
    if (step.out_dir) outDir = step.out_dir;
  }
  return outDir === undefined ? { data } : { data, out_dir: outDir };
}
