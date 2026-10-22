// Spawns dist/index.js over stdio with a FAKE katok (a shell script in a temp dir). Checks the
// wiring that unit tests cannot: serialization by tool family, send failure hints, out_dir
// refusal, and the keyless/no-archive guard. No real katok, Keychain or KakaoTalk is involved.

import assert from "node:assert/strict";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import { after, before, describe, it } from "node:test";
import { fileURLToPath } from "node:url";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const tmp = mkdtempSync(path.join(os.tmpdir(), "katok-server-test-"));
const dataDir = path.join(tmp, "data");
const logFile = path.join(tmp, "calls.log");
const fakeBin = path.join(tmp, "katok");
mkdirSync(dataDir);
writeFileSync(path.join(dataDir, "archive.sqlite3"), "x");
writeFileSync(logFile, "");
// argv after --data-dir <dir>: "$3" is the subcommand. `send` fails like a timeout would.
// `index` prints katok's documents[] shape (synthetic entries); `search` records its argv.
writeFileSync(
  fakeBin,
  `#!/bin/bash
echo "start $3" >> "${logFile}"
sleep 0.4
echo "end $3" >> "${logFile}"
if [ "$3" = "send" ]; then echo "boom" >&2; exit 1; fi
if [ "$3" = "index" ]; then echo '{"written_documents":2,"documents":[{"id":"d1"},{"id":"d2"}]}'; exit 0; fi
if [ "$3" = "search" ]; then echo "argv \${*:3}" >> "${logFile}"; fi
echo '{"ok":true}'
`,
);
chmodSync(fakeBin, 0o755);

const lines = () =>
  readFileSync(logFile, "utf8")
    .split("\n")
    .filter((line) => line !== "");

async function connect(extraEnv) {
  const env = { ...process.env, ...extraEnv };
  delete env.KATOK_ARCHIVE_KEY;
  Object.assign(env, extraEnv);
  const client = new Client({ name: "server-test", version: "0.0.0" });
  await client.connect(
    new StdioClientTransport({
      command: process.execPath,
      args: [path.join(root, "dist/index.js")],
      env,
      stderr: "pipe",
    }),
  );
  return client;
}

const textOf = (result) => result.content?.find((part) => part.type === "text")?.text ?? "";

describe("katok-mcp server (fake katok)", () => {
  let client;
  before(async () => {
    client = await connect({
      KATOK_BIN: fakeBin,
      KATOK_MCP_DATA_DIR: dataDir,
      KATOK_ARCHIVE_KEY: "test-key-not-a-secret",
    });
  });
  after(async () => {
    await client.close();
  });

  it("runs two write-family calls strictly one after the other", async () => {
    writeFileSync(logFile, "");
    await Promise.all([
      client.callTool({ name: "katok_sync", arguments: {} }),
      client.callTool({ name: "katok_index", arguments: {} }),
    ]);
    assert.deepEqual(lines(), ["start sync", "end sync", "start index", "end index"]);
  });

  it("does not make a read tool wait for a write tool", async () => {
    writeFileSync(logFile, "");
    await Promise.all([
      client.callTool({ name: "katok_sync", arguments: {} }),
      client.callTool({ name: "katok_rooms", arguments: {} }),
    ]);
    const log = lines();
    assert.ok(log.indexOf("start rooms") < log.indexOf("end sync"), log.join(" | "));
  });

  it("serializes the UI family (send, list_windows) apart from the write family", async () => {
    writeFileSync(logFile, "");
    await Promise.all([
      client.callTool({ name: "katok_list_windows", arguments: {} }),
      client.callTool({ name: "katok_send", arguments: { chat_id: "1", mode: "dry_run" } }),
      client.callTool({ name: "katok_sync", arguments: {} }),
    ]);
    const log = lines();
    // first two are both `send`; the second must start after the first ended.
    const sends = log.filter((line) => line.endsWith(" send"));
    assert.deepEqual(sends, ["start send", "end send", "start send", "end send"]);
    const firstEndSend = log.indexOf("end send");
    assert.ok(log.indexOf("start sync") < firstEndSend, "sync ran beside the UI lock");
  });

  it("a failed dry_run says nothing was sent and does not suggest narrowing", async () => {
    const dry = await client.callTool({
      name: "katok_send",
      arguments: { chat_id: "1", mode: "dry_run" },
    });
    assert.equal(dry.isError, true);
    const text = textOf(dry);
    assert.ok(!text.includes("범위를 줄여"), text);
    assert.match(text, /다음 행동: dry_run은 아무것도 보내지 않습니다/);
  });

  it("refuses an out_dir outside the allowed roots without running katok", async () => {
    writeFileSync(logFile, "");
    for (const [name, args] of [
      ["katok_transcript", { chat_id: "1", out_dir: "/etc" }],
      ["katok_media_get", { chat_id: "1", out_dir: "/etc" }],
      ["katok_media_backfill", { out_dir: "/etc" }],
    ]) {
      const result = await client.callTool({ name, arguments: args });
      assert.equal(result.isError, true, name);
      assert.match(textOf(result), /허용 범위 밖/, name);
      assert.match(textOf(result), /KATOK_MCP_OUT_ROOTS/, name);
    }
    assert.deepEqual(lines(), []);
  });

  it("publishes the 0.3.0 inputs and wrapper fields in the tool schemas", async () => {
    const { tools } = await client.listTools();
    const byName = Object.fromEntries(tools.map((tool) => [tool.name, tool]));
    const since = byName.katok_messages.inputSchema.properties.since;
    assert.match(since.description, /2026-07-20T00:00:00Z/);
    assert.ok(!since.description.includes("+09:00"));
    assert.equal(typeof since.pattern, "string");
    assert.equal(byName.katok_sync.inputSchema.properties.chat_id.type, "array");
    assert.equal(byName.katok_sync.inputSchema.properties.chat_id.items.type, "string");
    assert.deepEqual(byName.katok_search.inputSchema.properties.order.enum, ["newest", "oldest"]);
    assert.equal(byName.katok_index.inputSchema.properties.verbose.type, "boolean");
    assert.ok(byName.katok_doctor.outputSchema.properties.warning);
    assert.ok(byName.katok_doctor.outputSchema.properties.binary_build);
  });

  it("katok_index returns document_count instead of documents[] unless verbose", async () => {
    const short = await client.callTool({ name: "katok_index", arguments: {} });
    assert.notEqual(short.isError, true, textOf(short));
    assert.deepEqual(short.structuredContent.data, { written_documents: 2, document_count: 2 });
    const full = await client.callTool({ name: "katok_index", arguments: { verbose: true } });
    assert.deepEqual(full.structuredContent.data.documents, [{ id: "d1" }, { id: "d2" }]);
  });

  it("katok_doctor reports binary_build beside data, with no warning for a non-repo binary", async () => {
    const result = await client.callTool({ name: "katok_doctor", arguments: {} });
    assert.notEqual(result.isError, true, textOf(result));
    assert.deepEqual(result.structuredContent.data, { ok: true });
    assert.equal(result.structuredContent.binary_build.path, fakeBin);
    assert.equal(result.structuredContent.binary_build.ref, null);
    assert.equal(typeof result.structuredContent.binary_build.mtime, "string");
    assert.equal(result.structuredContent.warning, undefined);
  });

  it("keyword search passes --order newest; bm25 does not", async () => {
    writeFileSync(logFile, "");
    await client.callTool({ name: "katok_search", arguments: { mode: "keyword", query: "q" } });
    await client.callTool({ name: "katok_search", arguments: { mode: "bm25", query: "q" } });
    const argv = lines().filter((line) => line.startsWith("argv "));
    assert.deepEqual(argv, [
      "argv search keyword --order newest --json -- q",
      "argv search bm25 --json -- q",
    ]);
  });

  it("refuses a since without an offset before katok starts", async () => {
    writeFileSync(logFile, "");
    const result = await client.callTool({
      name: "katok_messages",
      arguments: { chat_id: "1", since: "2026-07-20T00:00:00" },
    });
    assert.equal(result.isError, true);
    assert.deepEqual(lines(), []);
  });

  it("accepts an out_dir inside the data dir", async () => {
    const out = path.join(dataDir, "exports");
    const result = await client.callTool({
      name: "katok_transcript",
      arguments: { chat_id: "1", out_dir: out },
    });
    assert.notEqual(result.isError, true, textOf(result));
    assert.equal(result.structuredContent.out_dir, out);
  });
});

describe("katok-mcp server with sending enabled (fake katok that fails)", () => {
  it("a failed send says to check KakaoTalk first, never to retry or narrow", async () => {
    const client = await connect({
      KATOK_BIN: fakeBin,
      KATOK_MCP_DATA_DIR: dataDir,
      KATOK_ARCHIVE_KEY: "test-key-not-a-secret",
      KATOK_MCP_ALLOW_SEND: "1",
      KATOK_MCP_ACCEPT_USE_POLICY: "1",
    });
    try {
      const result = await client.callTool({
        name: "katok_send",
        arguments: { chat_id: "1", text: "synthetic", mode: "send", confirm: true },
      });
      assert.equal(result.isError, true);
      const text = textOf(result);
      assert.match(text, /카카오톡에서 해당 방에 메시지가 실제로 갔는지 먼저 확인/);
      assert.match(text, /확인한 뒤에만/);
      assert.ok(!text.includes("범위를 줄여"), text);
      assert.ok(!text.includes("재시도하세요"), text);
    } finally {
      await client.close();
    }
  });
});

describe("katok-mcp server keeps the last sync/index failure (fake katok that fails)", () => {
  it("writes last-sync-error.json and last-index-error.json without stdout or argv", async () => {
    const failDir = path.join(tmp, "fail-data");
    mkdirSync(failDir);
    writeFileSync(path.join(failDir, "archive.sqlite3"), "x");
    const failBin = path.join(tmp, "katok-fail");
    writeFileSync(
      failBin,
      `#!/bin/bash
echo "synthetic stderr" >&2
echo '{"ok":false,"error":{"code":"synthetic_failure","message":"synthetic stdout body"}}'
exit 1
`,
    );
    chmodSync(failBin, 0o755);
    const client = await connect({
      KATOK_BIN: failBin,
      KATOK_MCP_DATA_DIR: failDir,
      KATOK_ARCHIVE_KEY: "test-key-not-a-secret",
    });
    try {
      const sync = await client.callTool({
        name: "katok_sync",
        arguments: { chat_id: ["11", "22"] },
      });
      assert.equal(sync.isError, true);
      const index = await client.callTool({ name: "katok_index", arguments: {} });
      assert.equal(index.isError, true);
    } finally {
      await client.close();
    }
    for (const [file, tool] of [
      ["last-sync-error.json", "katok_sync"],
      ["last-index-error.json", "katok_index"],
    ]) {
      assert.equal(statSync(path.join(failDir, file)).mode & 0o777, 0o600, file);
      const text = readFileSync(path.join(failDir, file), "utf8");
      const record = JSON.parse(text);
      assert.equal(record.tool, tool, file);
      assert.equal(record.exitCode, 1, file);
      assert.equal(record.errorCode, "synthetic_failure", file);
      assert.equal(record.stderrHead, "synthetic stderr", file);
      assert.ok(!text.includes("synthetic stdout body"), file);
      assert.ok(!text.includes("test-key-not-a-secret"), file);
      assert.ok(!text.includes("--data-dir"), file);
    }
    assert.equal(JSON.parse(readFileSync(path.join(failDir, "last-sync-error.json"))).step, "sync");
  });
});

describe("katok-mcp server before katok_init (no key, no archive)", () => {
  it("refuses read tools and never starts katok, so no archive appears", async () => {
    const emptyDir = path.join(tmp, "empty-data");
    mkdirSync(emptyDir);
    writeFileSync(logFile, "");
    const client = await connect({
      KATOK_BIN: fakeBin,
      KATOK_MCP_DATA_DIR: emptyDir,
      KATOK_MCP_KEYCHAIN: "0",
    });
    try {
      for (const [name, args] of [
        ["katok_rooms", {}],
        ["katok_search", { mode: "keyword", query: "x" }],
        ["katok_sync", {}],
      ]) {
        const result = await client.callTool({ name, arguments: args });
        assert.equal(result.isError, true, name);
        assert.match(textOf(result), /katok_init/, name);
      }
      // doctor never opens the archive, so it still runs.
      const doctor = await client.callTool({ name: "katok_doctor", arguments: {} });
      assert.notEqual(doctor.isError, true, textOf(doctor));
      assert.deepEqual(lines(), ["start doctor", "end doctor"]);
      assert.equal(existsSync(path.join(emptyDir, "archive.sqlite3")), false);
    } finally {
      await client.close();
    }
  });
});
