// Unit tests for the katok_init flow. Every side effect (Keychain, openssl, filesystem, katok)
// is replaced by a fake, so no real Keychain item or archive is touched.

import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { KeychainDuplicateError, KeychainNotFoundError } from "../dist/keychain.js";
import { initialize } from "../dist/workflows.js";

const SECRET = "ab".repeat(32);
const DIR = "/tmp/katok-mcp-init-test";
const ENV = { KATOK_MCP_DATA_DIR: DIR };

function harness({
  env = ENV,
  stored = null,
  readError = null,
  archive = false,
  syncError = null,
  raceWinner = null,
} = {}) {
  const log = [];
  const state = { stored, resets: 0 };
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
      return { ran: args[0] };
    },
    resetKey: () => {
      state.resets += 1;
      log.push(["reset"]);
    },
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
    assert.deepEqual(result.doctor, { ran: "doctor" });
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
