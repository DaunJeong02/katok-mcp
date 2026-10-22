// Unit tests for the next-step hints added in 0.3.0 and for the outdated-binary mapping in
// katok.ts. Pure text in, text out; nothing spawns katok.

import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { nextStepHint, sendFailureHint } from "../dist/hints.js";
import { KatokError, markOutdated, outdatedFlag } from "../dist/katok.js";

const detail = (overrides = {}) => ({
  args: ["--data-dir", "/d", "search", "keyword", "--order", "newest", "--json", "--", "q"],
  exitCode: 2,
  signal: null,
  stderr: "",
  stdoutHead: "",
  ...overrides,
});

// What a current katok prints on stdout for a clap parse error under --json (cli/src/main.rs).
const clapEnvelope = (flag) =>
  JSON.stringify({
    ok: false,
    error: {
      code: "invalid_arguments",
      message: `unexpected argument '${flag}' found`,
      cause: `error: unexpected argument '${flag}' found\n\nUsage: katok search keyword [OPTIONS] <QUERY>\n\nFor more information, try '--help'.`,
    },
  });

describe("outdated katok binary", () => {
  it("detects clap's unexpected argument for the recent flags only", () => {
    assert.equal(outdatedFlag("error: unexpected argument '--order' found"), "--order");
    assert.equal(outdatedFlag("error: unexpected argument '--chat' found"), "--chat");
    assert.equal(outdatedFlag("error: unexpected argument '--frobnicate' found"), null);
    assert.equal(outdatedFlag("invalid value 'x' for '--since <SINCE>'"), null);
  });

  it("rewrites the error from the stdout envelope (current builds)", () => {
    const error = new KatokError("katok failed: invalid_arguments: unexpected argument", {
      ...detail({ stdoutHead: clapEnvelope("--order") }),
      katokError: JSON.parse(clapEnvelope("--order")).error,
    });
    const mapped = markOutdated(error);
    assert.ok(mapped instanceof KatokError);
    assert.match(mapped.message, /katok binary is outdated.*--order.*scripts\/install\.sh/);
    assert.deepEqual(mapped.detail, error.detail);
    assert.match(nextStepHint(mapped.message), /install\.sh를 다시 실행/);
  });

  it("rewrites the error from stderr (builds without the JSON parse-error envelope)", () => {
    const error = new KatokError("katok exited with code 2 and printed non-JSON output", {
      ...detail({ stderr: "error: unexpected argument '--chat' found" }),
    });
    assert.match(markOutdated(error).message, /outdated.*--chat/);
  });

  it("leaves other errors and non-katok errors alone", () => {
    const other = new KatokError("katok exited with code 1", detail({ stderr: "boom" }));
    assert.equal(markOutdated(other), other);
    const plain = new Error("unexpected argument '--order'");
    assert.equal(markOutdated(plain), plain);
  });

  it("the outdated message never gets the since/until hint", () => {
    const text = `katok binary is outdated: it does not accept --order\n${clapEnvelope("--order")}`;
    assert.doesNotMatch(nextStepHint(text), /since/);
  });
});

// Messages captured by impl-cli from the built katok (branch improve/critique-v2).
const TIME_TAIL =
  "expected an RFC3339 time such as 2026-07-20T00:00:00Z or 2026-07-20T09:00:00+09:00, or a date such as 2026-07-20 (00:00:00 UTC)";
const badTime = (flag, value) => {
  const message = `invalid value '${value}' for '${flag} <TIME>': ${TIME_TAIL}`;
  const katokError = {
    code: "invalid_arguments",
    message,
    cause: `error: ${message}\n\nUsage: katok messages [OPTIONS] --chat <CHAT>`,
  };
  // The text index.ts hands to nextStepHint: the KatokError message, then its detail as JSON.
  return `katok failed: invalid_arguments: ${message}: error: ${message}\n\nUsage: katok messages\n${JSON.stringify({ katokError })}`;
};

describe("since/until hint", () => {
  it("fires for katok's real invalid --since and --until messages", () => {
    for (const [flag, value] of [
      ["--until", "2026-13-01"],
      ["--since", "soon"],
    ]) {
      assert.match(
        nextStepHint(badTime(flag, value)),
        /RFC3339\(예 2026-07-20T00:00:00Z\) 또는 YYYY-MM-DD/,
      );
    }
  });

  it("does not fire for an invalid --order value or a rejected --order flag", () => {
    const order = "katok failed: invalid_arguments: invalid value 'latest' for '--order <ORDER>'";
    assert.equal(nextStepHint(order), null);
    const rejected = "katok failed: invalid_arguments: unexpected argument '--order' found";
    assert.ok(!(nextStepHint(rejected) ?? "").includes("RFC3339"));
  });

  it("does not fire when --since only appears in argv of another invalid_arguments error", () => {
    const message = "katok failed: invalid_arguments: unexpected argument '--frobnicate' found";
    const json = JSON.stringify({ args: ["messages", "--since", "2026-07-20"] });
    const hint = nextStepHint(`${message}\n${json}`);
    assert.ok(hint === null || !hint.includes("RFC3339"), hint);
  });
});

describe("timeout hint", () => {
  it("names the real inputs: incremental and chat_id for sync, chat_id and limit for reads", () => {
    const hint = nextStepHint("katok timed out or was killed (SIGTERM) after 1 ms");
    assert.match(hint, /katok_sync는 incremental: true 또는 chat_id/);
    assert.match(hint, /chat_id와 limit/);
  });
});

describe("send failure hint", () => {
  it("adds the archive check without suggesting a retry from the archive alone", () => {
    const hint = sendFailureHint("send");
    assert.match(hint, /먼저 확인하고, 가지 않은 것을 확인한 뒤에만/);
    assert.match(hint, /katok_sync\(incremental: true\) 뒤 katok_messages/);
    assert.match(hint, /아카이브에 보이지 않아도 재전송하지 마십시오/);
    assert.match(hint, /이미지 전송은 아카이브에 나타나지 않습니다/);
    assert.ok(!hint.includes("\n"), "one paragraph");
    assert.ok(!hint.includes("재시도하세요"));
    assert.ok(!hint.includes("범위를 줄여"));
  });
});
