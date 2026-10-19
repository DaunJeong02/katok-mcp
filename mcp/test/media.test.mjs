// Unit tests for the media flows: backfill argv, output directories, and the order of the
// steps behind katok_sync with_media. Nothing here spawns katok.

import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  mediaBackfillPlan,
  mediaGetArgs,
  mediaGetOutDir,
  syncPlan,
  transcriptOutDir,
} from "../dist/args.js";
import { nextStepHint } from "../dist/hints.js";
import { runSyncPlan } from "../dist/workflows.js";

const ENV = { KATOK_MCP_DATA_DIR: "/tmp/katok-mcp-test" };

describe("mediaBackfillPlan", () => {
  it("defaults to every kind and an explicit --out under the data dir", () => {
    assert.deepEqual(mediaBackfillPlan({}, ENV), {
      args: [
        "media",
        "backfill",
        "--kind",
        "file",
        "--kind",
        "photo",
        "--kind",
        "video",
        "--out",
        "/tmp/katok-mcp-test/media",
        "--json",
      ],
      out_dir: "/tmp/katok-mcp-test/media",
    });
  });

  it("maps every option and resolves a relative out_dir", () => {
    const plan = mediaBackfillPlan(
      {
        chat_id: "42",
        kind: ["video"],
        dry_run: true,
        max_bytes: 1000,
        limit: 7,
        out_dir: "/tmp/elsewhere",
      },
      ENV,
    );
    assert.deepEqual(plan.args, [
      "media",
      "backfill",
      "--chat",
      "42",
      "--kind",
      "video",
      "--dry-run",
      "--max-bytes",
      "1000",
      "--limit",
      "7",
      "--out",
      "/tmp/elsewhere",
      "--json",
    ]);
    assert.equal(plan.out_dir, "/tmp/elsewhere");
    assert.ok(mediaBackfillPlan({ out_dir: "rel/dir" }, ENV).out_dir.startsWith("/"));
  });

  it("an empty kind list falls back to every kind", () => {
    const { args } = mediaBackfillPlan({ kind: [] }, ENV);
    assert.equal(args.filter((arg) => arg === "--kind").length, 3);
  });

  it("only adds --dry-run when asked", () => {
    assert.ok(!mediaBackfillPlan({ dry_run: false }, ENV).args.includes("--dry-run"));
  });
});

describe("media get / transcript output directories", () => {
  it("media get exposes --log and --limit and reports katok's default folder", () => {
    const args = mediaGetArgs({ chat_id: "42", log_id: "99", limit: 3 });
    assert.deepEqual(args, [
      "media",
      "get",
      "--chat",
      "42",
      "--log",
      "99",
      "--limit",
      "3",
      "--json",
    ]);
    assert.equal(mediaGetOutDir({ chat_id: "42" }, ENV), "/tmp/katok-mcp-test/media/42");
    assert.equal(mediaGetOutDir({ chat_id: "42", out_dir: "/o" }, ENV), "/o");
  });

  it("transcript defaults to <data_dir>/transcripts", () => {
    assert.equal(transcriptOutDir({}, ENV), "/tmp/katok-mcp-test/transcripts");
    assert.equal(transcriptOutDir({ out_dir: "/o" }, ENV), "/o");
  });
});

describe("katok_sync with_media ordering", () => {
  it("is sync only by default", () => {
    assert.deepEqual(
      syncPlan({}, ENV).map((step) => step.name),
      ["sync"],
    );
    assert.deepEqual(syncPlan({}, ENV)[0].args, ["sync", "--source", "macos", "--json"]);
  });

  it("with_media runs sync first, then backfill into <data_dir>/media", () => {
    const steps = syncPlan({ with_media: true, incremental: true }, ENV);
    assert.deepEqual(
      steps.map((step) => step.name),
      ["sync", "backfill"],
    );
    assert.deepEqual(steps[0].args, ["sync", "--source", "macos", "--incremental", "--json"]);
    assert.equal(steps[1].args[0], "media");
    assert.equal(steps[1].out_dir, "/tmp/katok-mcp-test/media");
  });

  it("runs the steps in order and returns both results with out_dir", async () => {
    const seen = [];
    const run = async (args) => {
      seen.push(args[0]);
      return { step: args[0] };
    };
    const outcome = await runSyncPlan(syncPlan({ with_media: true }, ENV), run);
    assert.deepEqual(seen, ["sync", "media"]);
    assert.deepEqual(outcome.data, { sync: { step: "sync" }, backfill: { step: "media" } });
    assert.equal(outcome.out_dir, "/tmp/katok-mcp-test/media");
  });

  it("does not backfill when the sync fails", async () => {
    const seen = [];
    const run = async (args) => {
      seen.push(args[0]);
      throw new Error("sync exploded");
    };
    await assert.rejects(runSyncPlan(syncPlan({ with_media: true }, ENV), run), /sync exploded/);
    assert.deepEqual(seen, ["sync"]);
  });

  it("says the sync succeeded when only the backfill fails", async () => {
    const run = async (args) => {
      if (args[0] === "media") throw new Error("cdn down");
      return {};
    };
    await assert.rejects(
      runSyncPlan(syncPlan({ with_media: true }, ENV), run),
      /cdn down.*sync 단계는 이미 끝났습니다/,
    );
  });

  it("returns no out_dir without media", async () => {
    const outcome = await runSyncPlan(syncPlan({}, ENV), async () => ({}));
    assert.equal(outcome.out_dir, undefined);
  });
});

describe("nextStepHint", () => {
  it("points a permission failure at Full Disk Access", () => {
    assert.match(nextStepHint("open db: Operation not permitted (os error 1)"), /전체 디스크 접근/);
  });

  it("points a missing binary at install.sh", () => {
    assert.match(nextStepHint("katok binary not found. Tried: /x"), /install\.sh/);
  });

  it("has nothing to say about unrelated text", () => {
    assert.equal(nextStepHint("something else entirely"), null);
  });
});
