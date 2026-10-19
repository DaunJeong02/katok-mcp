// Unit tests for archive key resolution. A fake reader stands in for /usr/bin/security,
// so these tests never touch the real Keychain.

import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  createKeyProvider,
  DEFAULT_KEYCHAIN_ACCOUNT,
  DEFAULT_KEYCHAIN_SERVICE,
  isArchiveKeyFailure,
  isWrongKeyFailure,
  keychainTarget,
  withArchiveKey,
} from "../dist/archiveKey.js";

const SECRET = "k".repeat(64);

function fakeReader(result) {
  const calls = [];
  const reader = async (service, account) => {
    calls.push({ service, account });
    if (result instanceof Error) throw result;
    return result;
  };
  return { reader, calls };
}

describe("keychainTarget", () => {
  it("uses the default service and account", () => {
    assert.deepEqual(keychainTarget({}), {
      service: DEFAULT_KEYCHAIN_SERVICE,
      account: DEFAULT_KEYCHAIN_ACCOUNT,
    });
  });

  it("honours KATOK_MCP_KEYCHAIN_SERVICE / ACCOUNT", () => {
    assert.deepEqual(
      keychainTarget({ KATOK_MCP_KEYCHAIN_SERVICE: "s", KATOK_MCP_KEYCHAIN_ACCOUNT: "a" }),
      { service: "s", account: "a" },
    );
  });

  it("is skipped when the env var is set or KATOK_MCP_KEYCHAIN=0", () => {
    assert.equal(keychainTarget({ KATOK_ARCHIVE_KEY: "x" }), null);
    assert.equal(keychainTarget({ KATOK_MCP_KEYCHAIN: "0" }), null);
  });
});

describe("createKeyProvider", () => {
  it("prefers KATOK_ARCHIVE_KEY and never calls the Keychain", async () => {
    const { reader, calls } = fakeReader(SECRET);
    const resolve = createKeyProvider(reader);
    const result = await resolve({ KATOK_ARCHIVE_KEY: "from-env" });
    assert.deepEqual(result, { source: "env" });
    assert.equal(calls.length, 0);
    const env = { KATOK_ARCHIVE_KEY: "from-env" };
    assert.equal(withArchiveKey(env, result), env);
  });

  it("KATOK_MCP_KEYCHAIN=0 disables the lookup", async () => {
    const { reader, calls } = fakeReader(SECRET);
    const result = await createKeyProvider(reader)({ KATOK_MCP_KEYCHAIN: "0" });
    assert.equal(result.source, "none");
    assert.match(result.reason, /KATOK_MCP_KEYCHAIN=0/);
    assert.equal(calls.length, 0);
    assert.equal(withArchiveKey({}, result).KATOK_ARCHIVE_KEY, undefined);
  });

  it("reads the Keychain once, trims, and caches", async () => {
    const { reader, calls } = fakeReader(`${SECRET}\n`);
    const resolve = createKeyProvider(reader);
    const first = await resolve({});
    const second = await resolve({});
    assert.equal(first.source, "keychain");
    assert.equal(first.key, SECRET);
    assert.equal(second, first);
    assert.equal(calls.length, 1);
    assert.deepEqual(calls[0], {
      service: DEFAULT_KEYCHAIN_SERVICE,
      account: DEFAULT_KEYCHAIN_ACCOUNT,
    });
    assert.equal(withArchiveKey({ A: "1" }, first).KATOK_ARCHIVE_KEY, SECRET);
  });

  it("caches a failure as 'none' with a reason that holds no key", async () => {
    const { reader, calls } = fakeReader(new Error("item could not be found"));
    const resolve = createKeyProvider(reader);
    const result = await resolve({});
    await resolve({});
    assert.equal(result.source, "none");
    assert.equal(result.key, undefined);
    assert.match(result.reason, /could not be found/);
    assert.equal(calls.length, 1);
  });

  it("treats an empty item as 'none'", async () => {
    const result = await createKeyProvider(fakeReader("  \n").reader)({});
    assert.equal(result.source, "none");
  });
});

describe("isArchiveKeyFailure", () => {
  it("matches katok's encrypted-archive error only", () => {
    assert.ok(
      isArchiveKeyFailure("archive at /x/archive.sqlite3 is encrypted; set KATOK_ARCHIVE_KEY"),
    );
    assert.ok(!isArchiveKeyFailure("no chat 1 in the archive; run sync first"));
  });
});

describe("isWrongKeyFailure", () => {
  it("matches SQLCipher's wrong-key error", () => {
    assert.ok(isWrongKeyFailure("open archive: SQLite error: file is not a database"));
    assert.ok(!isWrongKeyFailure("archive at /x is encrypted; set KATOK_ARCHIVE_KEY"));
  });
});
