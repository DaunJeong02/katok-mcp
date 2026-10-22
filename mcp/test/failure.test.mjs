// Unit tests for the last-failure record (<data_dir>/last-sync-error.json, last-index-error.json)
// and for runSyncPlan's failure callback. The file system is injected; nothing is written.

import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { syncPlan } from "../dist/args.js";
import { createFailureWriter, FAILURE_FILES, failureRecord } from "../dist/failureLog.js";
import { KatokError } from "../dist/katok.js";
import { runSyncPlan } from "../dist/workflows.js";

const NOW = new Date("2026-10-22T01:02:03.000Z");
const STDOUT_BODY = "synthetic stdout body that must not be kept";

const katokError = () =>
  new KatokError("katok failed: sync_failed: synthetic", {
    args: ["--data-dir", "/d", "sync", "--source", "macos", "--json"],
    exitCode: 1,
    signal: null,
    stderr: `synthetic stderr line\n${"x".repeat(5000)}`,
    stdoutHead: STDOUT_BODY,
    katokError: { code: "sync_failed", message: "synthetic" },
  });

describe("failureRecord", () => {
  it("keeps time, tool, exit status, katok's code and the stderr head only", () => {
    const record = failureRecord("katok_sync", katokError(), NOW, "sync");
    assert.deepEqual(Object.keys(record).sort(), [
      "at",
      "errorCode",
      "exitCode",
      "signal",
      "stderrHead",
      "step",
      "tool",
    ]);
    assert.equal(record.at, "2026-10-22T01:02:03.000Z");
    assert.equal(record.tool, "katok_sync");
    assert.equal(record.step, "sync");
    assert.equal(record.exitCode, 1);
    assert.equal(record.errorCode, "sync_failed");
    assert.ok(record.stderrHead.startsWith("synthetic stderr line"));
    assert.ok(record.stderrHead.length <= 2000);
    const text = JSON.stringify(record);
    assert.ok(!text.includes(STDOUT_BODY));
    assert.ok(!text.includes("--data-dir"));
  });

  it("records a timeout by its signal", () => {
    const error = new KatokError("katok timed out or was killed (SIGTERM) after 1 ms", {
      args: [],
      exitCode: null,
      signal: "SIGTERM",
      stderr: "",
      stdoutHead: STDOUT_BODY,
    });
    const record = failureRecord("katok_index", error, NOW);
    assert.equal(record.signal, "SIGTERM");
    assert.equal(record.errorCode, null);
    assert.equal(record.step, undefined);
  });

  it("keeps the server's own message for a failure before katok ran", () => {
    const record = failureRecord("katok_sync", new Error("katok binary not found."), NOW);
    assert.equal(record.message, "katok binary not found.");
    assert.equal(record.exitCode, null);
  });
});

describe("createFailureWriter", () => {
  it("writes a per-process temp file, then renames it over last-<kind>-error.json", () => {
    const calls = [];
    const write = createFailureWriter(
      "/d",
      {
        writeFile: (file, text) => calls.push(["write", file, JSON.parse(text)]),
        rename: (from, to) => calls.push(["rename", from, to]),
      },
      () => {},
      4242,
    );
    const record = failureRecord("katok_index", katokError(), NOW);
    write("index", record);
    assert.deepEqual(calls, [
      ["write", "/d/last-index-error.json.4242.tmp", record],
      ["rename", "/d/last-index-error.json.4242.tmp", "/d/last-index-error.json"],
    ]);
    assert.equal(FAILURE_FILES.sync, "last-sync-error.json");
  });

  it("never throws and warns without the record's content", () => {
    const warnings = [];
    const write = createFailureWriter(
      "/missing",
      {
        writeFile: () => {
          throw Object.assign(new Error("ENOENT: no such file"), { code: "ENOENT" });
        },
        rename: () => {},
      },
      (line) => warnings.push(line),
    );
    write("sync", failureRecord("katok_sync", katokError(), NOW));
    assert.equal(warnings.length, 1);
    assert.match(warnings[0], /last-sync-error\.json \(ENOENT\)/);
    assert.ok(!warnings[0].includes("synthetic"));
  });
});

describe("runSyncPlan failure callback", () => {
  const ENV = { KATOK_MCP_DATA_DIR: "/d" };

  it("reports the failed step with the original error, then rethrows", async () => {
    const seen = [];
    const error = katokError();
    await assert.rejects(
      runSyncPlan(
        syncPlan({}, ENV),
        async () => {
          throw error;
        },
        (step, err) => seen.push([step, err]),
      ),
      /synthetic/,
    );
    assert.deepEqual(seen, [["sync", error]]);
  });

  it("reports a backfill failure as step backfill", async () => {
    const seen = [];
    await assert.rejects(
      runSyncPlan(
        syncPlan({ with_media: true }, ENV),
        async (args) => {
          if (args[0] === "media") throw new Error("cdn down");
          return {};
        },
        (step) => seen.push(step),
      ),
      /cdn down/,
    );
    assert.deepEqual(seen, ["backfill"]);
  });

  it("is not called on success", async () => {
    let called = false;
    await runSyncPlan(
      syncPlan({}, ENV),
      async () => ({}),
      () => {
        called = true;
      },
    );
    assert.equal(called, false);
  });
});
