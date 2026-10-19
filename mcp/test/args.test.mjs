// Unit tests for argv construction and the send gate. Runs against dist/ (pnpm build first).
// Nothing here spawns katok or touches KakaoTalk.

import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  chunkArgs,
  doctorArgs,
  globalArgs,
  indexArgs,
  listWindowsArgs,
  mediaGetArgs,
  messagesArgs,
  POLICY_FILES,
  planSend,
  roomsArgs,
  searchArgs,
  syncArgs,
  transcriptArgs,
} from "../dist/args.js";
import { defaultDataDir } from "../dist/paths.js";

const DELIVERY_FLAGS = ["--text", "--image", "--accept-use-policy", "--draft"];
const OPEN_ENV = {
  KATOK_MCP_ALLOW_SEND: "1",
  KATOK_MCP_ACCEPT_USE_POLICY: "1",
};

describe("read commands", () => {
  it("doctor never adds --macos-probe unless probe is true", () => {
    assert.deepEqual(doctorArgs({}), ["doctor", "--json"]);
    assert.deepEqual(doctorArgs({ probe: false }), ["doctor", "--json"]);
    assert.deepEqual(doctorArgs({ probe: true }), ["doctor", "--macos-probe", "--json"]);
  });

  it("sync always reads the macos source", () => {
    assert.deepEqual(syncArgs({}), ["sync", "--source", "macos", "--json"]);
    assert.deepEqual(syncArgs({ incremental: true }), [
      "sync",
      "--source",
      "macos",
      "--incremental",
      "--json",
    ]);
  });

  it("rooms, messages, transcript, index", () => {
    assert.deepEqual(roomsArgs(), ["rooms", "--json"]);
    assert.deepEqual(messagesArgs({ chat_id: "42", since: "2026-01-01T00:00:00Z", limit: 5 }), [
      "messages",
      "--chat",
      "42",
      "--since",
      "2026-01-01T00:00:00Z",
      "--limit",
      "5",
      "--json",
    ]);
    assert.deepEqual(transcriptArgs({ chat_id: "42", out_dir: "/tmp/x" }), [
      "transcript",
      "--chat",
      "42",
      "--out",
      "/tmp/x",
      "--json",
    ]);
    assert.deepEqual(indexArgs({ full: true }), ["index", "--full", "--json"]);
  });

  it("search puts the query after -- and rejects chat_id for semantic", () => {
    assert.deepEqual(searchArgs({ mode: "bm25", query: "-x", limit: 3, chat_id: "7" }), [
      "search",
      "bm25",
      "--limit",
      "3",
      "--chat",
      "7",
      "--json",
      "--",
      "-x",
    ]);
    assert.throws(() => searchArgs({ mode: "semantic", query: "q", chat_id: "7" }));
  });

  it("chunk flags are get-only", () => {
    assert.deepEqual(chunkArgs({ action: "get", chunk_id: "c1", redact: true }), [
      "chunk",
      "get",
      "--redact",
      "--json",
      "--",
      "c1",
    ]);
    assert.throws(() => chunkArgs({ action: "context", chunk_id: "c1", redact: true }));
  });

  it("media get repeats --kind", () => {
    assert.deepEqual(
      mediaGetArgs({ chat_id: "9", kind: ["photo", "file"], no_cdn: true, out_dir: "/o" }),
      [
        "media",
        "get",
        "--chat",
        "9",
        "--kind",
        "photo",
        "--kind",
        "file",
        "--out",
        "/o",
        "--no-cdn",
        "--json",
      ],
    );
  });

  it("list windows is the read-only send flag", () => {
    assert.deepEqual(listWindowsArgs(), ["send", "--list-windows", "--json"]);
  });

  it("global --data-dir is always explicit: KATOK_MCP_DATA_DIR or the dedicated default", () => {
    assert.deepEqual(globalArgs({}), ["--data-dir", defaultDataDir()]);
    assert.deepEqual(globalArgs({ KATOK_MCP_DATA_DIR: "" }), ["--data-dir", defaultDataDir()]);
    assert.deepEqual(globalArgs({ KATOK_MCP_DATA_DIR: "/d" }), ["--data-dir", "/d"]);
    assert.ok(defaultDataDir().endsWith("/Library/Application Support/katok-mcp"));
  });
});

describe("send gate: dry_run", () => {
  it("defaults to dry_run with --no-open and no delivery flags", () => {
    const plan = planSend({ chat_id: "123" }, {});
    assert.equal(plan.ok, true);
    assert.equal(plan.mode, "dry_run");
    assert.deepEqual(plan.args, ["send", "--chat", "123", "--dry-run", "--no-open", "--json"]);
    assert.equal(plan.stdin, "");
    for (const flag of DELIVERY_FLAGS) assert.ok(!plan.args.includes(flag), flag);
  });

  it("dry_run ignores text and never forwards it", () => {
    const plan = planSend({ chat_id: "123", text: "hello", mode: "dry_run" }, OPEN_ENV);
    assert.equal(plan.ok, true);
    assert.equal(plan.stdin, "");
    for (const flag of DELIVERY_FLAGS) assert.ok(!plan.args.includes(flag), flag);
  });

  it("allow_open: true drops --no-open", () => {
    const plan = planSend({ chat_id: "123", allow_open: true }, {});
    assert.equal(plan.ok, true);
    assert.ok(!plan.args.includes("--no-open"));
  });

  it("falls back to --room when chat_id is absent", () => {
    const plan = planSend({ room: "Room A" }, {});
    assert.deepEqual(plan.args, ["send", "--room", "Room A", "--dry-run", "--no-open", "--json"]);
  });

  it("needs a target", () => {
    assert.equal(planSend({}, {}).ok, false);
  });
});

describe("send gate: send", () => {
  it("refuses without confirm: true", () => {
    const plan = planSend({ chat_id: "1", text: "hi", mode: "send", confirm: false }, OPEN_ENV);
    assert.equal(plan.ok, false);
    assert.match(plan.reason, /confirm: true/);
  });

  it("refuses confirm: true without KATOK_MCP_ALLOW_SEND=1", () => {
    const plan = planSend(
      { chat_id: "1", text: "hi", mode: "send", confirm: true },
      { KATOK_MCP_ACCEPT_USE_POLICY: "1" },
    );
    assert.equal(plan.ok, false);
    assert.match(plan.reason, /KATOK_MCP_ALLOW_SEND=1/);
  });

  it("refuses without KATOK_MCP_ACCEPT_USE_POLICY=1 and names both policy files", () => {
    const plan = planSend(
      { chat_id: "1", text: "hi", mode: "send", confirm: true },
      { KATOK_MCP_ALLOW_SEND: "1" },
    );
    assert.equal(plan.ok, false);
    for (const file of POLICY_FILES) assert.ok(plan.reason.includes(file), file);
  });

  it("builds text send with body on stdin, --no-open and --accept-use-policy", () => {
    const plan = planSend({ chat_id: "1", text: "hi", mode: "send", confirm: true }, OPEN_ENV);
    assert.equal(plan.ok, true);
    assert.deepEqual(plan.args, [
      "send",
      "--chat",
      "1",
      "--no-open",
      "--accept-use-policy",
      "--json",
    ]);
    assert.equal(plan.stdin, "hi");
    assert.ok(!plan.args.includes("hi"), "body must not be in argv");
  });

  it("builds image send with an absolute path", () => {
    const plan = planSend(
      { chat_id: "1", image_path: "/tmp/a.png", mode: "send", confirm: true, take_focus_now: true },
      OPEN_ENV,
    );
    assert.equal(plan.ok, true);
    assert.deepEqual(plan.args, [
      "send",
      "--chat",
      "1",
      "--image",
      "/tmp/a.png",
      "--no-open",
      "--take-focus-now",
      "--accept-use-policy",
      "--json",
    ]);
  });

  it("refuses text and image together, and empty content", () => {
    assert.equal(
      planSend(
        { chat_id: "1", text: "a", image_path: "/a.png", mode: "send", confirm: true },
        OPEN_ENV,
      ).ok,
      false,
    );
    assert.equal(planSend({ chat_id: "1", mode: "send", confirm: true }, OPEN_ENV).ok, false);
  });
});

describe("send gate: draft", () => {
  it("refuses without KATOK_MCP_ACCEPT_USE_POLICY=1", () => {
    const plan = planSend({ chat_id: "1", text: "hi", mode: "draft" }, {});
    assert.equal(plan.ok, false);
    assert.match(plan.reason, /KATOK_MCP_ACCEPT_USE_POLICY=1/);
  });

  it("does not need confirm or KATOK_MCP_ALLOW_SEND", () => {
    const plan = planSend(
      { chat_id: "1", text: "hi", mode: "draft" },
      { KATOK_MCP_ACCEPT_USE_POLICY: "1" },
    );
    assert.equal(plan.ok, true);
    assert.deepEqual(plan.args, [
      "send",
      "--chat",
      "1",
      "--draft",
      "--no-open",
      "--accept-use-policy",
      "--json",
    ]);
    assert.equal(plan.stdin, "hi");
  });

  it("refuses images, because katok would send an image even with --draft", () => {
    const plan = planSend(
      { chat_id: "1", image_path: "/a.png", mode: "draft" },
      { KATOK_MCP_ACCEPT_USE_POLICY: "1" },
    );
    assert.equal(plan.ok, false);
  });
});

describe("send gate: KATOK_MCP_ALLOWED_CHATS", () => {
  const env = { ...OPEN_ENV, KATOK_MCP_ALLOWED_CHATS: " 10, 20 " };

  it("allows listed chats", () => {
    assert.equal(planSend({ chat_id: "20", text: "hi", mode: "draft" }, env).ok, true);
    assert.equal(
      planSend({ chat_id: "10", text: "hi", mode: "send", confirm: true }, env).ok,
      true,
    );
  });

  it("refuses unlisted chats for draft and send", () => {
    assert.equal(planSend({ chat_id: "30", text: "hi", mode: "draft" }, env).ok, false);
    assert.equal(
      planSend({ chat_id: "30", text: "hi", mode: "send", confirm: true }, env).ok,
      false,
    );
  });

  it("refuses room-name addressing for draft and send", () => {
    assert.equal(planSend({ room: "Room A", text: "hi", mode: "draft" }, env).ok, false);
  });

  it("does not restrict dry_run", () => {
    assert.equal(planSend({ chat_id: "30" }, env).ok, true);
  });
});
