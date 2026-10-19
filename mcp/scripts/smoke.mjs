// stdio smoke test: spawn dist/index.js, then
//   initialize -> tools/list -> katok_init -> katok_rooms -> katok_search keyword
//   -> katok_media_backfill dry_run (kind file) -> two katok_send calls the gate must refuse.
//
// It prints counts, keys, numbers and statuses only: never a room name, a sender, a message
// body, a file name or a path inside the archive. katok_init is real (Keychain key + first
// sync); if it fails, the error is printed as is and the run stops (no workaround).
// It never calls draft, a real send, --macos-probe, katok_list_windows or a real backfill.
//
// SMOKE_SKIP_INIT=1 skips katok_init, for QA against a synthetic archive (KATOK_MCP_DATA_DIR).
// SMOKE_BACKFILL_KINDS=file,photo,video widens the dry run. Not the default: katok's dry run
// skips only the CDN download, so for photo and video it still decrypts whatever KakaoTalk has
// in its local cache into <data_dir>/media. kind "file" has no local cache and writes nothing.

import path from "node:path";
import { fileURLToPath } from "node:url";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const env = { ...process.env };
// Prove the gate with send disabled, whatever the caller's shell has.
delete env.KATOK_MCP_ALLOW_SEND;

const transport = new StdioClientTransport({
  command: process.execPath,
  args: [path.join(root, "dist/index.js")],
  env,
  stderr: "pipe",
});
transport.stderr?.on("data", (chunk) => process.stderr.write(`[server] ${chunk}`));

const client = new Client({ name: "katok-mcp-smoke", version: "0.0.0" });

function textOf(result) {
  return result.content?.find((part) => part.type === "text")?.text ?? "";
}

function section(title) {
  console.log(`\n== ${title}`);
}

let failures = 0;
function expect(condition, label) {
  console.log(`${condition ? "PASS" : "FAIL"} ${label}`);
  if (!condition) failures += 1;
}

async function finish() {
  await client.close();
  console.log(`\n${failures === 0 ? "smoke OK" : `smoke FAILED (${failures})`}`);
  process.exit(failures === 0 ? 0 : 1);
}

/** The first array found in a katok payload: the payload itself or its first array property. */
function firstArray(value) {
  if (Array.isArray(value)) return value;
  if (value && typeof value === "object") {
    return Object.values(value).find(Array.isArray) ?? [];
  }
  return [];
}

await client.connect(transport);
section("initialize");
const info = client.getServerVersion();
console.log(`server: ${info?.name} ${info?.version}`);
console.log(`capabilities: ${Object.keys(client.getServerCapabilities() ?? {}).join(", ")}`);

section("tools/list");
const { tools } = await client.listTools();
console.log(`${tools.length} tools: ${tools.map((tool) => tool.name).join(", ")}`);
expect(tools.length === 13, "13 tools registered");
const hints = ["readOnlyHint", "destructiveHint", "idempotentHint", "openWorldHint"];
expect(
  tools.every((tool) => hints.every((hint) => typeof tool.annotations?.[hint] === "boolean")),
  "every tool declares all four annotation hints",
);
expect(
  tools.every((tool) => tool.outputSchema !== undefined),
  "every tool declares an outputSchema",
);
for (const tool of tools) {
  const a = tool.annotations ?? {};
  console.log(
    `  ${tool.name}: readOnly=${a.readOnlyHint} destructive=${a.destructiveHint} idempotent=${a.idempotentHint} openWorld=${a.openWorldHint}`,
  );
}

if (process.env.SMOKE_SKIP_INIT === "1") {
  section("tools/call katok_init (skipped: SMOKE_SKIP_INIT=1)");
} else {
  section("tools/call katok_init (real: Keychain key + first sync)");
  const init = await client.callTool({ name: "katok_init", arguments: {} });
  if (init.isError) {
    console.log("isError: true");
    console.log(textOf(init));
    console.log("\nstopping: katok_init failed, nothing else was run (no workaround)");
    failures += 1;
    await finish();
  }
  const body = init.structuredContent;
  console.log(
    `isError: false, ok: ${body.ok}, key.source: ${body.data.key.source}, key.created: ${body.data.key.created}`,
  );
  console.log(`already_initialized: ${body.data.already_initialized}`);
  console.log(`sync keys: ${Object.keys(body.data.sync ?? {}).join(", ")}`);
  console.log(`doctor keys: ${Object.keys(body.data.doctor ?? {}).join(", ")}`);
  console.log(
    `freshness.recommendation: ${JSON.stringify(body.data.doctor?.freshness?.recommendation)}`,
  );
  expect(
    !JSON.stringify(init).includes(process.env.KATOK_ARCHIVE_KEY ?? "\u0000"),
    "no env key in result",
  );
  expect(!/\b[0-9a-f]{64}\b/.test(JSON.stringify(init)), "no 64-hex string in the init result");
}

section("tools/call katok_rooms");
const rooms = await client.callTool({ name: "katok_rooms", arguments: {} });
if (rooms.isError) {
  console.log(`isError: true\n${textOf(rooms).split("\n")[0]}`);
  failures += 1;
} else {
  const list = firstArray(rooms.structuredContent.data);
  const byType = {};
  for (const room of list) byType[room.chat_type] = (byType[room.chat_type] ?? 0) + 1;
  console.log(`isError: false, rooms: ${list.length}`);
  console.log(`chat_type distribution: ${JSON.stringify(byType)}`);
}

section("tools/call katok_search mode=keyword");
const query = process.env.SMOKE_QUERY ?? "네";
const search = await client.callTool({
  name: "katok_search",
  arguments: { mode: "keyword", query, limit: 10 },
});
if (search.isError) {
  console.log(`isError: true\n${textOf(search).split("\n")[0]}`);
  failures += 1;
} else {
  const hits = firstArray(search.structuredContent.data);
  console.log(`isError: false, hits (limit 10): ${hits.length}`);
}

section("tools/call katok_media_backfill dry_run (nothing is downloaded)");
const totalsByKind = {};
const kinds = (process.env.SMOKE_BACKFILL_KINDS ?? "file").split(",");
for (const kind of kinds) {
  const result = await client.callTool({
    name: "katok_media_backfill",
    arguments: { kind: [kind], dry_run: true },
  });
  if (result.isError) {
    console.log(`${kind}: isError: true\n${textOf(result).split("\n")[0]}`);
    failures += 1;
    continue;
  }
  const { data, out_dir: outDir } = result.structuredContent;
  totalsByKind[kind] = { rooms: data.room_count, tier_totals: data.tier_totals, bytes: data.bytes };
  console.log(
    `${kind}: dry_run=${data.dry_run} rooms_with_media=${data.room_count} tier_totals=${JSON.stringify(data.tier_totals)} bytes=${data.bytes} errors=${(data.errors ?? []).length}`,
  );
  expect(typeof outDir === "string" && outDir.endsWith("/media"), `${kind}: out_dir is reported`);
}
const sumBytes = Object.values(totalsByKind).reduce((sum, entry) => sum + (entry.bytes ?? 0), 0);
console.log(`total bytes across kinds: ${sumBytes}`);

section("tools/call katok_send mode=send confirm=false");
const sendNoConfirm = await client.callTool({
  name: "katok_send",
  arguments: { chat_id: "0", text: "smoke", mode: "send", confirm: false },
});
console.log(`isError: ${sendNoConfirm.isError}\n${textOf(sendNoConfirm)}`);
expect(sendNoConfirm.isError === true && /confirm: true/.test(textOf(sendNoConfirm)), "refused");

section("tools/call katok_send mode=send confirm=true (KATOK_MCP_ALLOW_SEND unset)");
const sendNoEnv = await client.callTool({
  name: "katok_send",
  arguments: { chat_id: "0", text: "smoke", mode: "send", confirm: true },
});
console.log(`isError: ${sendNoEnv.isError}\n${textOf(sendNoEnv)}`);
expect(sendNoEnv.isError === true && /KATOK_MCP_ALLOW_SEND=1/.test(textOf(sendNoEnv)), "refused");

await finish();
