// Unit tests for the out_dir allow-list. Only the temp dir below is touched.

import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, realpathSync, symlinkSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";
import {
  mediaBackfillPlan,
  mediaGetArgs,
  mediaGetOutDir,
  transcriptArgs,
  transcriptOutDir,
} from "../dist/args.js";
import {
  allowedOutRoots,
  expandHome,
  OUT_ROOTS_ENV,
  OutDirError,
  resolveOutDir,
} from "../dist/outDir.js";

const base = realpathSync(mkdtempSync(path.join(os.tmpdir(), "katok-outdir-")));
const DATA = path.join(base, "data");
const EXTRA = path.join(base, "export");
const HOME = path.join(base, "home");
mkdirSync(DATA);
mkdirSync(EXTRA);
mkdirSync(HOME);
const ENV = { KATOK_MCP_DATA_DIR: DATA };

describe("resolveOutDir", () => {
  it("allows the data dir and anything below it", () => {
    assert.equal(resolveOutDir(DATA, ENV, HOME), DATA);
    assert.equal(resolveOutDir(`${DATA}/media/42`, ENV, HOME), `${DATA}/media/42`);
  });

  it("rejects everything else and names the allowed locations", () => {
    for (const bad of ["/etc", "/", path.join(base, "other"), `${DATA}/../other`, "relative/dir"]) {
      assert.throws(
        () => resolveOutDir(bad, ENV, HOME),
        (error) => {
          assert.ok(error instanceof OutDirError, bad);
          assert.ok(error.message.includes(DATA), "lists the data dir");
          assert.ok(error.message.includes(OUT_ROOTS_ENV), "names the env var");
          return true;
        },
        bad,
      );
    }
  });

  it("does not treat a sibling that shares a prefix as inside", () => {
    assert.throws(() => resolveOutDir(`${DATA}-evil`, ENV, HOME), OutDirError);
    assert.throws(() => resolveOutDir(`${DATA}/..hidden/../../x`, ENV, HOME), OutDirError);
  });

  it("allows roots from KATOK_MCP_OUT_ROOTS (colon separated)", () => {
    const env = { ...ENV, [OUT_ROOTS_ENV]: `/nonexistent-root:${EXTRA}` };
    assert.equal(resolveOutDir(`${EXTRA}/sub/dir`, env, HOME), `${EXTRA}/sub/dir`);
    assert.throws(() => resolveOutDir(path.join(base, "other"), env, HOME), OutDirError);
    assert.deepEqual(allowedOutRoots(env, HOME), [DATA, "/nonexistent-root", EXTRA]);
  });

  it("expands ~ in the candidate and in the roots", () => {
    const env = { ...ENV, [OUT_ROOTS_ENV]: "~/exports" };
    assert.deepEqual(allowedOutRoots(env, HOME), [DATA, path.join(HOME, "exports")]);
    assert.equal(resolveOutDir("~/exports/a", env, HOME), path.join(HOME, "exports", "a"));
    assert.throws(() => resolveOutDir("~/elsewhere", env, HOME), OutDirError);
    assert.equal(expandHome("~", HOME), HOME);
    assert.equal(expandHome("~/x", HOME), path.join(HOME, "x"));
    assert.equal(expandHome("~x/y", HOME), "~x/y");
  });

  it("ignores relative and empty entries in KATOK_MCP_OUT_ROOTS", () => {
    const env = { ...ENV, [OUT_ROOTS_ENV]: ":relative/root::  " };
    assert.deepEqual(allowedOutRoots(env, HOME), [DATA]);
  });

  it("does not let a symlink inside the data dir lead outside", () => {
    symlinkSync("/etc", path.join(DATA, "link"));
    assert.throws(() => resolveOutDir(path.join(DATA, "link", "x"), ENV, HOME), OutDirError);
  });
});

describe("tools reject an out_dir outside the allowed roots", () => {
  it("transcript, media get and media backfill", () => {
    assert.throws(() => transcriptArgs({ chat_id: "1", out_dir: "/etc" }, ENV), OutDirError);
    assert.throws(() => transcriptOutDir({ out_dir: "/etc" }, ENV), OutDirError);
    assert.throws(() => mediaGetArgs({ chat_id: "1", out_dir: "/etc" }, ENV), OutDirError);
    assert.throws(() => mediaGetOutDir({ chat_id: "1", out_dir: "/etc" }, ENV), OutDirError);
    assert.throws(() => mediaBackfillPlan({ out_dir: "/etc" }, ENV), OutDirError);
  });

  it("and pass the resolved path to --out when allowed", () => {
    const env = { ...ENV, [OUT_ROOTS_ENV]: EXTRA };
    assert.deepEqual(transcriptArgs({ chat_id: "1", out_dir: `${EXTRA}/t` }, env), [
      "transcript",
      "--chat",
      "1",
      "--out",
      `${EXTRA}/t`,
      "--json",
    ]);
    assert.equal(mediaBackfillPlan({ out_dir: `${EXTRA}/m` }, env).out_dir, `${EXTRA}/m`);
  });

  it("defaults need no out_dir check", () => {
    assert.equal(transcriptOutDir({}, ENV), `${DATA}/transcripts`);
    assert.equal(mediaGetOutDir({ chat_id: "7" }, ENV), `${DATA}/media/7`);
    assert.equal(mediaBackfillPlan({}, ENV).out_dir, `${DATA}/media`);
  });
});
