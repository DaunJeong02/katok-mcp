// Unit tests for archive key resolution. A fake reader stands in for /usr/bin/security,
// so these tests never touch the real Keychain.

import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  commandArgs,
  createKeyProvider,
  DEFAULT_KEYCHAIN_ACCOUNT,
  DEFAULT_KEYCHAIN_SERVICE,
  dataDirFromArgs,
  isArchiveKeyFailure,
  isWrongKeyFailure,
  katokDefaultDataDir,
  keychainTarget,
  missingArchiveGuard,
  NOT_FOUND_TTL_MS,
  opensArchive,
  plaintextGuard,
  withArchiveKey,
} from "../dist/archiveKey.js";
import { KeychainNotFoundError } from "../dist/keychain.js";

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
  it("defaults to the dedicated katok-mcp item", () => {
    assert.equal(DEFAULT_KEYCHAIN_SERVICE, "com.enternext.katok-mcp");
    assert.equal(DEFAULT_KEYCHAIN_ACCOUNT, "archive-key");
  });

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

  it("never caches a failure other than 'not found': the next call asks again", async () => {
    let attempt = 0;
    const calls = [];
    const reader = async (service, account) => {
      calls.push({ service, account });
      attempt += 1;
      if (attempt === 1) throw new Error("User interaction is not allowed");
      return SECRET;
    };
    const resolve = createKeyProvider(reader);
    const first = await resolve({});
    assert.equal(first.source, "none");
    assert.equal(first.key, undefined);
    assert.match(first.reason, /User interaction is not allowed/);
    assert.ok(!first.reason.includes(SECRET));
    const second = await resolve({});
    assert.equal(second.source, "keychain");
    assert.equal(second.key, SECRET);
    assert.equal(calls.length, 2);
    // ...and the success is cached for good.
    await resolve({});
    assert.equal(calls.length, 2);
  });

  it("does not cache an empty item", async () => {
    const { reader, calls } = fakeReader("  \n");
    const resolve = createKeyProvider(reader);
    await resolve({});
    await resolve({});
    assert.equal(calls.length, 2);
  });

  it("remembers 'not found' for 30 seconds only", async () => {
    let clock = 1_000;
    const calls = [];
    const reader = async (service, account) => {
      calls.push(1);
      throw new KeychainNotFoundError(service, account);
    };
    const resolve = createKeyProvider(reader, () => clock);
    assert.equal(NOT_FOUND_TTL_MS, 30_000);
    const first = await resolve({});
    assert.equal(first.source, "none");
    assert.match(first.reason, /was not found/);
    clock += NOT_FOUND_TTL_MS - 1;
    assert.equal(await resolve({}), first);
    assert.equal(calls.length, 1);
    clock += 1;
    await resolve({});
    assert.equal(calls.length, 2);
  });

  it("shares one lookup between concurrent calls", async () => {
    let release;
    const gate = new Promise((resolveGate) => {
      release = resolveGate;
    });
    let count = 0;
    const resolve = createKeyProvider(async () => {
      count += 1;
      await gate;
      return SECRET;
    });
    const pending = [resolve({}), resolve({})];
    release();
    const [a, b] = await Promise.all(pending);
    assert.equal(count, 1);
    assert.equal(a.key, SECRET);
    assert.equal(b.key, SECRET);
  });

  it("an answer that arrives after reset() is not cached", async () => {
    let release;
    const gate = new Promise((resolveGate) => {
      release = resolveGate;
    });
    let count = 0;
    const resolve = createKeyProvider(async (service, account) => {
      count += 1;
      if (count === 1) {
        await gate;
        throw new KeychainNotFoundError(service, account);
      }
      return SECRET;
    });
    const stale = resolve({});
    resolve.reset();
    release();
    assert.equal((await stale).source, "none");
    assert.equal((await resolve({})).key, SECRET);
    assert.equal(count, 2);
  });

  it("reset() makes the next call read the Keychain again", async () => {
    const { reader, calls } = fakeReader(SECRET);
    const resolve = createKeyProvider(reader);
    assert.equal((await resolve({})).source, "keychain");
    resolve.reset();
    await resolve({});
    assert.equal(calls.length, 2);
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

describe("plaintextGuard", () => {
  const plaintext = Buffer.from("SQLite format 3\0", "latin1");
  const encrypted = Buffer.from([
    0x8f, 0x12, 0xa0, 0x33, 0x01, 0xfe, 0x7c, 0x44, 0x9b, 0x00, 0x51, 0xe2, 0x6d, 0x18, 0xc7, 0x2a,
  ]);
  const keychain = { source: "keychain", key: SECRET };
  const args = ["--data-dir", "/tmp/katok-guard-test", "rooms", "--json"];

  function reader(result) {
    const seen = [];
    return {
      seen,
      read: (file) => {
        seen.push(file);
        return result;
      },
    };
  }

  it("refuses a Keychain key for a plaintext archive", () => {
    const { read, seen } = reader(plaintext);
    const refusal = plaintextGuard(keychain, args, read);
    assert.ok(refusal?.startsWith("이 아카이브는 평문입니다."));
    assert.ok(!refusal.includes(SECRET));
    assert.deepEqual(seen, ["/tmp/katok-guard-test/archive.sqlite3"]);
  });

  it("injects for an encrypted header", () => {
    assert.equal(plaintextGuard(keychain, args, reader(encrypted).read), null);
  });

  it("injects when the archive does not exist yet", () => {
    assert.equal(plaintextGuard(keychain, args, reader(null).read), null);
  });

  it("does not check when the key comes from KATOK_ARCHIVE_KEY", () => {
    const { read, seen } = reader(plaintext);
    assert.equal(plaintextGuard({ source: "env" }, args, read), null);
    assert.equal(seen.length, 0);
  });

  it("refuses when the header cannot be read", () => {
    const read = () => {
      throw new Error("EACCES");
    };
    assert.match(plaintextGuard(keychain, args, read), /EACCES/);
  });

  it("checks the same directory katok receives", () => {
    assert.equal(dataDirFromArgs(args), "/tmp/katok-guard-test");
    // Without a --data-dir katok would use its own default, so that is what the guard checks.
    assert.equal(dataDirFromArgs(["rooms", "--json"]), katokDefaultDataDir());
    assert.ok(katokDefaultDataDir().endsWith("/Library/Application Support/katok"));
  });
});

describe("missingArchiveGuard", () => {
  const none = {
    source: "none",
    reason: "keychain lookup failed: User interaction is not allowed",
  };
  const args = (...rest) => ["--data-dir", "/tmp/katok-guard-test", ...rest];
  const missing = () => false;
  const present = () => true;

  it("refuses an archive-opening command when there is no key and no archive file", () => {
    for (const command of [
      ["rooms", "--json"],
      ["sync", "--source", "macos", "--json"],
      ["search", "keyword", "--json", "--", "x"],
      ["media", "backfill", "--out", "/tmp/o", "--json"],
      ["send", "--chat", "1", "--dry-run", "--json"],
    ]) {
      const refusal = missingArchiveGuard(none, args(...command), missing);
      assert.match(refusal, /katok_init/, command.join(" "));
      assert.match(refusal, /User interaction is not allowed/);
      assert.match(refusal, /\/tmp\/katok-guard-test\/archive\.sqlite3/);
    }
  });

  it("lets doctor, permissions and the send listings through", () => {
    for (const command of [
      ["doctor", "--json"],
      ["permissions", "macos"],
      ["send", "--list-windows", "--json"],
      ["send", "--list-rooms", "--json"],
    ]) {
      assert.equal(missingArchiveGuard(none, args(...command), missing), null, command.join(" "));
    }
  });

  it("passes when the archive file exists, whatever the key", () => {
    assert.equal(missingArchiveGuard(none, args("rooms", "--json"), present), null);
  });

  it("passes with a Keychain key or an explicit KATOK_ARCHIVE_KEY", () => {
    assert.equal(missingArchiveGuard({ source: "env" }, args("rooms"), missing), null);
    assert.equal(
      missingArchiveGuard({ source: "keychain", key: SECRET }, args("rooms"), missing),
      null,
    );
  });

  it("checks the file in the data dir katok receives", () => {
    const seen = [];
    missingArchiveGuard(none, args("rooms"), (file) => {
      seen.push(file);
      return true;
    });
    assert.deepEqual(seen, ["/tmp/katok-guard-test/archive.sqlite3"]);
  });

  it("finds the subcommand after the global flags", () => {
    assert.deepEqual(commandArgs(["--data-dir", "/x", "rooms", "--json"]), ["rooms", "--json"]);
    assert.deepEqual(commandArgs(["rooms"]), ["rooms"]);
    assert.equal(opensArchive(["--data-dir", "/x", "doctor", "--json"]), false);
    assert.equal(opensArchive(["--data-dir", "/x", "rooms", "--json"]), true);
    assert.equal(opensArchive([]), true);
  });
});
