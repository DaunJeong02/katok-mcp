// stdio handshake smoke test: spawn dist/index.js, then initialize -> tools/list ->
// katok_doctor -> katok_rooms -> semantic pre-check -> two katok_send calls that the gate must refuse.
//
// It prints counts, keys and statuses only, never room names or message bodies.
// It never calls sync, index, draft, a real send, --macos-probe or katok_list_windows.

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

function firstLine(text) {
  return text.split("\n")[0];
}

function section(title) {
  console.log(`\n== ${title}`);
}

let failures = 0;
function expect(condition, label) {
  console.log(`${condition ? "PASS" : "FAIL"} ${label}`);
  if (!condition) failures += 1;
}

await client.connect(transport);
section("initialize");
const info = client.getServerVersion();
console.log(`server: ${info?.name} ${info?.version}`);
console.log(`capabilities: ${Object.keys(client.getServerCapabilities() ?? {}).join(", ")}`);

section("tools/list");
const { tools } = await client.listTools();
console.log(`${tools.length} tools: ${tools.map((tool) => tool.name).join(", ")}`);
expect(tools.length === 11, "11 tools registered");

section("tools/call katok_doctor");
const doctor = await client.callTool({ name: "katok_doctor", arguments: {} });
if (doctor.isError) {
  console.log(`isError: true\n${firstLine(textOf(doctor))}`);
} else {
  const body = JSON.parse(textOf(doctor));
  console.log(`isError: false, keys: ${Object.keys(body).join(", ")}`);
  console.log(`freshness.recommendation: ${JSON.stringify(body.freshness?.recommendation)}`);
  console.log(`semantic_index present: ${body.semantic_index !== undefined}`);
}

section("tools/call katok_rooms");
const rooms = await client.callTool({ name: "katok_rooms", arguments: {} });
if (rooms.isError) {
  const text = textOf(rooms);
  console.log(`isError: true\n${firstLine(text)}`);
  const detail = text.slice(text.indexOf("\n\n") + 2);
  try {
    const parsed = JSON.parse(detail);
    console.log(`exitCode: ${parsed.exitCode}, katokError.code: ${parsed.katokError?.code}`);
  } catch {
    // detail is not JSON; the first line above already describes the failure
  }
} else {
  const body = JSON.parse(textOf(rooms));
  const list = Array.isArray(body) ? body : (body.rooms ?? []);
  console.log(`isError: false, rooms: ${list.length}`);
  if (list[0]) console.log(`row fields: ${Object.keys(list[0]).join(", ")}`);
}

section("tools/call katok_search mode=semantic (index freshness pre-check)");
const semantic = await client.callTool({
  name: "katok_search",
  arguments: { mode: "semantic", query: "smoke" },
});
console.log(`isError: ${semantic.isError}\n${firstLine(textOf(semantic))}`);

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

await client.close();
console.log(`\n${failures === 0 ? "smoke OK" : `smoke FAILED (${failures})`}`);
process.exit(failures === 0 ? 0 : 1);
