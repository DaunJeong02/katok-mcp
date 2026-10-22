// Unit tests for the katok_init flow. Every side effect (Keychain, openssl, filesystem, katok)
// is replaced by a fake, so no real Keychain item or archive is touched.

import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { HintedError, KatokError } from "../dist/katok.js";
import { KeychainDuplicateError, KeychainNotFoundError } from "../dist/keychain.js";
import { failFrom } from "../dist/toolResult.js";
import { initialize, syncCompleted, terminalSyncCommand } from "../dist/workflows.js";

const SECRET = "ab".repeat(32);
const DIR = "/tmp/katok-mcp-init-test";
const ENV = { KATOK_MCP_DATA_DIR: DIR };
// doctor after a finished first sync; `freshness.last_sync` is null until one completes.
const COMPLETED = { ran: "doctor", freshness: { last_sync: { total_messages: 3 } } };
const NEVER_SYNCED = { ran: "doctor", freshness: { last_sync: null } };

function harness({
  env = ENV,
  stored = null,
  readError = null,
  archive = false,
  syncError = null,
  raceWinner = null,
  doctor = COMPLETED,
} = {}) {
  const log = [];
  const state = { stored, resets: 0, failures: [] };
  const deps = {
    env,
    readKey: async (service, account) => {
      log.push(["read", service, account]);
      if (readError) throw readError;
      if (state.stored === null) throw new KeychainNotFoundError(service, account);
      return `${state.stored}\n`;
    },
    writeKey: async (service, account, key) => {
      log.push(["write", service, account]);
      if (raceWinner !== null) {
        // Another katok_init stored its key between our read and our write.
        state.stored = raceWinner;
        throw new KeychainDuplicateError(service, account);
      }
      state.stored = key;
      state.writtenKey = key;
    },
    generateKey: async () => {
      log.push(["generate"]);
      return SECRET;
    },
    archiveExists: (file) => {
      log.push(["exists", file]);
      return archive;
    },
    makeDir: (dir) => log.push(["mkdir", dir]),
    run: async (args) => {
      log.push(["katok", ...args]);
      if (args[0] === "sync" && syncError) throw syncError;
      if (args[0] === "doctor") return doctor;
      return { ran: args[0] };
    },
    resetKey: () => {
      state.resets += 1;
      log.push(["reset"]);
    },
    recordFailure: (kind, record) => state.failures.push([kind, record]),
    binPath: () => "/repo/cli/target/release/katok",
    now: () => new Date("2026-10-22T00:00:00Z"),
  };
  return { deps, log, state };
}

const names = (log) => log.map((entry) => entry[0]);

describe("katok_init", () => {
  it("generates and stores a key when the Keychain has none, then syncs and runs doctor", async () => {
    const { deps, log, state } = harness();
    const result = await initialize(deps);

    assert.deepEqual(
      names(log).filter((name) => name !== "exists"),
      ["read", "generate", "write", "reset", "mkdir", "katok", "katok"],
    );
    assert.deepEqual(
      log.find((entry) => entry[0] === "write"),
      ["write", "com.enternext.katok-mcp", "archive-key"],
    );
    assert.equal(state.writtenKey, SECRET);
    assert.deepEqual(
      log.filter((entry) => entry[0] === "katok").map((entry) => entry[1]),
      ["sync", "doctor"],
    );
    assert.deepEqual(log.find((entry) => entry[0] === "katok").slice(1), [
      "sync",
      "--source",
      "macos",
      "--json",
    ]);
    assert.equal(result.key.created, true);
    assert.equal(result.key.source, "keychain");
    assert.equal(result.already_initialized, false);
    assert.equal(result.data_dir, DIR);
    assert.deepEqual(result.sync, { ran: "sync" });
    assert.deepEqual(result.doctor, COMPLETED);
  });

  it("never puts the key value in the result", async () => {
    const { deps } = harness();
    const result = await initialize(deps);
    assert.ok(!JSON.stringify(result).includes(SECRET));
  });

  it("resets the key cache before the first sync so the new key is used", async () => {
    const { deps, log } = harness();
    await initialize(deps);
    assert.ok(names(log).indexOf("reset") < names(log).indexOf("katok"));
  });

  it("keeps an existing Keychain key and does not generate or write", async () => {
    const { deps, log } = harness({ stored: SECRET });
    const result = await initialize(deps);
    assert.ok(!names(log).includes("generate"));
    assert.ok(!names(log).includes("write"));
    assert.equal(result.key.created, false);
    assert.equal(result.sync !== null, true);
  });

  it("resets the key cache on the existing-key branch too, before the first sync", async () => {
    const { deps, log, state } = harness({ stored: SECRET });
    await initialize(deps);
    assert.equal(state.resets, 1);
    assert.ok(names(log).indexOf("reset") < names(log).indexOf("katok"));
  });

  it("resets the key cache even when the archive already exists", async () => {
    const { deps, state } = harness({ stored: SECRET, archive: true });
    await initialize(deps);
    assert.equal(state.resets, 1);
  });

  it("uses the key a concurrent init stored first instead of its own", async () => {
    const winner = "cd".repeat(32);
    const { deps, log, state } = harness({ raceWinner: winner });
    const result = await initialize(deps);
    assert.equal(result.key.created, false);
    assert.equal(result.key.source, "keychain");
    assert.equal(state.stored, winner);
    // read (not found), generate, write (duplicate), read again, reset, then the sync.
    assert.deepEqual(
      names(log).filter((name) => name !== "exists"),
      ["read", "generate", "write", "read", "reset", "mkdir", "katok", "katok"],
    );
    assert.ok(!JSON.stringify(result).includes(winner));
    assert.ok(!JSON.stringify(result).includes(SECRET));
  });

  it("passes a failed write other than a duplicate through", async () => {
    const { deps, log } = harness();
    deps.writeKey = async () => {
      throw new Error("security add-generic-password failed: User canceled the operation");
    };
    await assert.rejects(initialize(deps), /User canceled/);
    assert.ok(!names(log).includes("katok"));
  });

  it("only returns doctor when the archive already exists", async () => {
    const { deps, log } = harness({ stored: SECRET, archive: true });
    const result = await initialize(deps);
    assert.deepEqual(
      log.filter((entry) => entry[0] === "katok").map((entry) => entry[1]),
      ["doctor"],
    );
    assert.equal(result.already_initialized, true);
    assert.equal(result.sync, null);
  });

  it("is idempotent: a second run keeps the first run's key", async () => {
    const first = harness();
    await initialize(first.deps);
    const second = harness({ stored: first.state.stored, archive: true });
    const result = await initialize(second.deps);
    assert.equal(result.key.created, false);
    assert.equal(first.state.stored, SECRET);
  });

  it("refuses to create a key next to an existing archive", async () => {
    const { deps, log } = harness({ archive: true });
    await assert.rejects(initialize(deps), /아카이브가 이미 있는데 키체인/);
    assert.ok(!names(log).includes("generate"));
    assert.ok(!names(log).includes("write"));
  });

  it("does not overwrite the Keychain when the read fails for another reason", async () => {
    const { deps, log } = harness({ readError: new Error("User interaction is not allowed") });
    await assert.rejects(initialize(deps), /읽지 못해 키를 새로 만들지 않았습니다/);
    assert.ok(!names(log).includes("write"));
  });

  it("uses KATOK_ARCHIVE_KEY without touching the Keychain", async () => {
    const { deps, log } = harness({ env: { ...ENV, KATOK_ARCHIVE_KEY: "from-env-0123456789" } });
    const result = await initialize(deps);
    assert.ok(!names(log).includes("read"));
    assert.ok(!names(log).includes("write"));
    assert.equal(result.key.source, "env");
  });

  it("explains KATOK_MCP_KEYCHAIN=0 without a key", async () => {
    const { deps } = harness({ env: { ...ENV, KATOK_MCP_KEYCHAIN: "0" } });
    await assert.rejects(initialize(deps), /KATOK_MCP_KEYCHAIN=0/);
  });

  it("says the key is already stored when the first sync fails", async () => {
    const { deps, state } = harness({ syncError: new Error("Operation not permitted") });
    await assert.rejects(
      initialize(deps),
      /Operation not permitted.*이미 키체인에 저장됐습니다.*Full Disk Access/,
    );
    assert.equal(state.stored, SECRET);
  });
});

const timeout = () =>
  new KatokError("katok timed out or was killed (SIGTERM) after 600000 ms", {
    args: [],
    exitCode: null,
    signal: "SIGTERM",
    stderr: "",
    stdoutHead: "",
  });

describe("katok_init after an interrupted first sync", () => {
  it("syncs again when the archive exists but doctor reports no completed sync", async () => {
    const { deps, log } = harness({ stored: SECRET, archive: true, doctor: NEVER_SYNCED });
    const result = await initialize(deps);
    assert.deepEqual(
      log.filter((entry) => entry[0] === "katok").map((entry) => entry[1]),
      ["doctor", "sync", "doctor"],
    );
    assert.equal(result.already_initialized, false);
    assert.deepEqual(result.sync, { ran: "sync" });
  });

  it("syncs again when the last sync stored no messages", async () => {
    const doctor = { freshness: { last_sync: { total_messages: 0 } } };
    const { deps, log } = harness({ stored: SECRET, archive: true, doctor });
    await initialize(deps);
    assert.ok(log.some((entry) => entry[0] === "katok" && entry[1] === "sync"));
  });

  it("syncCompleted needs a last_sync with at least one message", () => {
    assert.equal(syncCompleted(COMPLETED), true);
    assert.equal(syncCompleted(NEVER_SYNCED), false);
    assert.equal(syncCompleted({ freshness: { last_sync: { total_messages: 0 } } }), false);
    assert.equal(syncCompleted({}), false);
    assert.equal(syncCompleted(null), false);
  });

  it("on a timeout, records the failure and leads the hint with the terminal command", async () => {
    const { deps, state } = harness({ syncError: timeout() });
    const error = await initialize(deps).then(
      () => assert.fail("expected a rejection"),
      (err) => err,
    );
    assert.ok(error instanceof HintedError);
    assert.match(
      error.hint,
      /^다음 행동: 터미널에서 첫 동기화를 직접 실행하세요\. MCP의 600초 제한이 없습니다/,
    );
    assert.ok(
      error.hint.includes(
        `KATOK_ARCHIVE_KEY="$(/usr/bin/security find-generic-password -s com.enternext.katok-mcp -a archive-key -w)" /repo/cli/target/release/katok --data-dir ${DIR} sync --source macos`,
      ),
      error.hint,
    );
    assert.match(error.hint, /권한 팝업 대기처럼 한 번만 생긴 지연이었을 때만/);
    // A key created by this call is still reported.
    assert.match(error.message, /아카이브 키는 이미 키체인에 저장됐습니다/);
    assert.ok(!error.message.includes(SECRET));
    assert.ok(!error.hint.includes(SECRET));
    assert.equal(state.failures.length, 1);
    const [kind, record] = state.failures[0];
    assert.equal(kind, "sync");
    assert.equal(record.tool, "katok_init");
    assert.equal(record.signal, "SIGTERM");
    assert.ok(!JSON.stringify(record).includes(SECRET));
  });

  it("the katok_init timeout error text never advises narrowing or katok_sync", async () => {
    const { deps } = harness({ syncError: timeout() });
    const error = await initialize(deps).then(
      () => assert.fail("expected a rejection"),
      (err) => err,
    );
    // The exact text the katok_init tool returns (index.ts passes the error to failFrom).
    const result = failFrom(error);
    assert.equal(result.isError, true);
    const text = result.content[0].text;
    assert.match(text, /터미널에서 첫 동기화를 직접 실행하세요/);
    for (const banned of ["범위를 줄여", "incremental", "chat_id", "limit", "katok_sync"]) {
      assert.ok(!text.includes(banned), `${banned} in: ${text}`);
    }
    assert.ok(!text.includes(SECRET));
  });

  it("a non-timeout first-sync failure keeps the generic note and hint", async () => {
    const { deps } = harness({ syncError: new Error("Operation not permitted") });
    const error = await initialize(deps).then(
      () => assert.fail("expected a rejection"),
      (err) => err,
    );
    assert.ok(!(error instanceof HintedError));
    assert.match(
      error.message,
      /katok_init은 첫 동기화가 끝나지 않은 아카이브면 동기화를 다시 합니다/,
    );
    assert.match(failFrom(error).content[0].text, /전체 디스크 접근/);
  });

  it("does not offer the terminal command for a failure other than a timeout", async () => {
    const { deps, state } = harness({ syncError: new Error("Operation not permitted") });
    await assert.rejects(initialize(deps), (err) => !err.message.includes("터미널"));
    assert.equal(state.failures.length, 1);
  });

  it("with KATOK_ARCHIVE_KEY the terminal command has no Keychain lookup", async () => {
    const { deps } = harness({
      env: { ...ENV, KATOK_ARCHIVE_KEY: "from-env-0123456789" },
      syncError: timeout(),
    });
    const error = await initialize(deps).then(
      () => assert.fail("expected a rejection"),
      (err) => err,
    );
    assert.ok(error instanceof HintedError);
    assert.ok(!error.hint.includes("find-generic-password"));
    assert.match(error.hint, /서버와 같은 KATOK_ARCHIVE_KEY를 설정한 셸에서/);
    assert.ok(!error.hint.includes("from-env-0123456789"));
    assert.ok(!error.message.includes("아카이브 키는 이미"), "no key was created");
  });

  it("terminalSyncCommand quotes paths with spaces", () => {
    assert.equal(
      terminalSyncCommand({
        bin: null,
        dataDir: "/tmp/katok-mcp-test/Library/Application Support/katok-mcp",
        keychain: null,
      }),
      "katok --data-dir '/tmp/katok-mcp-test/Library/Application Support/katok-mcp' sync --source macos",
    );
  });
});
