// Unit tests for the katok build stamp check behind katok_doctor's binary_build and warning.
// The file system is a fake; no binary or source tree is read.

import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  binaryBuildStatus,
  inspectBinaryBuild,
  STALE_WARNING,
  UNSTAMPED_WARNING,
} from "../dist/buildStamp.js";

const REPO = "/repo";
const IN_REPO = "/repo/cli/target/release/katok";
const T = Date.parse("2026-10-22T00:00:00Z");

const facts = (overrides = {}) => ({
  bin: IN_REPO,
  inRepo: true,
  binMtimeMs: T,
  ref: "abc123",
  stampMtimeMs: T,
  newestSourceMtimeMs: T - 1000,
  ...overrides,
});

describe("binaryBuildStatus", () => {
  it("reports path, ref and mtime and no warning for a current stamped build", () => {
    assert.deepEqual(binaryBuildStatus(facts()), {
      binary_build: { path: IN_REPO, ref: "abc123", mtime: "2026-10-22T00:00:00.000Z" },
    });
  });

  it("warns when the source is newer than both the binary and the stamp", () => {
    const status = binaryBuildStatus(facts({ newestSourceMtimeMs: T + 1000 }));
    assert.equal(status.warning, STALE_WARNING);
    assert.match(status.warning, /scripts\/install\.sh 를 다시 실행하세요/);
  });

  it("trusts a stamp newer than the source even when cargo left the binary untouched", () => {
    const status = binaryBuildStatus(
      facts({ binMtimeMs: T - 5000, stampMtimeMs: T + 2000, newestSourceMtimeMs: T + 1000 }),
    );
    assert.equal(status.warning, undefined);
  });

  it("warns when the in-repo binary has no stamp", () => {
    const status = binaryBuildStatus(facts({ ref: null, stampMtimeMs: null }));
    assert.equal(status.warning, UNSTAMPED_WARNING);
    assert.equal(status.binary_build.ref, null);
  });

  it("never warns about a binary outside the repo (KATOK_BIN, ~/.cargo/bin)", () => {
    const status = binaryBuildStatus(
      facts({ bin: "/elsewhere/katok", inRepo: false, ref: null, newestSourceMtimeMs: T + 1000 }),
    );
    assert.equal(status.warning, undefined);
    assert.equal(status.binary_build.path, "/elsewhere/katok");
  });

  it("does not warn when times are unknown", () => {
    assert.equal(
      binaryBuildStatus(facts({ binMtimeMs: null, stampMtimeMs: null })).warning,
      undefined,
    );
    assert.equal(binaryBuildStatus(facts({ newestSourceMtimeMs: null })).warning, undefined);
  });
});

describe("inspectBinaryBuild", () => {
  const fakeFs = (files, newest, links = {}) => {
    const walked = [];
    return {
      walked,
      fs: {
        mtimeMs: (file) => files[file]?.mtime ?? null,
        readText: (file) => files[file]?.text ?? null,
        newestMtimeMs: (dir) => {
          walked.push(dir);
          return newest;
        },
        realpath: (file) => links[file] ?? file,
      },
    };
  };

  it("reads <binary>.ref, trims it, and walks cli/src for the in-repo binary", () => {
    const { fs, walked } = fakeFs(
      { [IN_REPO]: { mtime: T }, [`${IN_REPO}.ref`]: { mtime: T, text: "abc123\n" } },
      T + 1,
    );
    const status = inspectBinaryBuild(IN_REPO, REPO, fs);
    assert.deepEqual(walked, ["/repo/cli/src"]);
    assert.equal(status.binary_build.ref, "abc123");
    assert.equal(status.warning, STALE_WARNING);
  });

  it("does not walk the source tree for another binary", () => {
    const { fs, walked } = fakeFs({ "/x/katok": { mtime: T } }, T + 1);
    const status = inspectBinaryBuild("/x/katok", REPO, fs);
    assert.deepEqual(walked, []);
    assert.equal(status.binary_build.ref, null);
    assert.equal(status.warning, undefined);
  });

  it("treats a symlink to the in-repo binary as in-repo and reads the real stamp", () => {
    const link = "/usr/local/bin/katok";
    const { fs, walked } = fakeFs(
      { [IN_REPO]: { mtime: T }, [`${IN_REPO}.ref`]: { mtime: T, text: "abc123" } },
      T + 1,
      { [link]: IN_REPO },
    );
    const status = inspectBinaryBuild(link, REPO, fs);
    assert.deepEqual(walked, ["/repo/cli/src"]);
    assert.equal(status.binary_build.path, link);
    assert.equal(status.binary_build.ref, "abc123");
    assert.equal(status.warning, STALE_WARNING);
  });

  it("treats a symlinked repo path that resolves elsewhere as not in-repo", () => {
    const { fs, walked } = fakeFs({ "/opt/katok": { mtime: T } }, T + 1, {
      [IN_REPO]: "/real/repo/cli/target/release/katok",
      "/opt/katok": "/opt/katok",
    });
    assert.equal(inspectBinaryBuild("/opt/katok", REPO, fs).warning, undefined);
    assert.deepEqual(walked, []);
  });

  it("treats an empty stamp file as no stamp", () => {
    const { fs } = fakeFs({ [IN_REPO]: { mtime: T }, [`${IN_REPO}.ref`]: { text: " \n" } }, T - 1);
    assert.equal(inspectBinaryBuild(IN_REPO, REPO, fs).warning, UNSTAMPED_WARNING);
  });
});
