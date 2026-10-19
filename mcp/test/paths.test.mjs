// Unit tests for path resolution: the binary search order, the dedicated data directory and the
// repo-relative policy files. Nothing here touches a real archive.

import assert from "node:assert/strict";
import path from "node:path";
import { describe, it } from "node:test";
import { POLICY_FILES } from "../dist/args.js";
import {
  archivePath,
  binCandidates,
  dataDir,
  defaultDataDir,
  mediaDir,
  REPO_ROOT,
  repoRoot,
} from "../dist/paths.js";

describe("repo root", () => {
  it("is two levels above the compiled module (<repo>/mcp/dist/x.js)", () => {
    assert.equal(repoRoot("file:///work/repo/mcp/dist/paths.js"), "/work/repo");
  });

  it("points at the real checkout: cli/ and mcp/ are siblings", () => {
    assert.ok(path.basename(path.dirname(REPO_ROOT)) !== "mcp");
    assert.equal(
      path.resolve(REPO_ROOT, "mcp", "dist"),
      path.resolve(import.meta.dirname, "../dist"),
    );
  });
});

describe("binary search order", () => {
  it("KATOK_BIN, then cli/target/release/katok, then ~/.cargo/bin/katok", () => {
    assert.deepEqual(binCandidates({ KATOK_BIN: "/x/katok" }, "/repo", "/home/u"), [
      "/x/katok",
      "/repo/cli/target/release/katok",
      "/home/u/.cargo/bin/katok",
    ]);
    assert.deepEqual(binCandidates({}, "/repo", "/home/u"), [
      "/repo/cli/target/release/katok",
      "/home/u/.cargo/bin/katok",
    ]);
  });

  it("has no sidecar or enter-cut default", () => {
    for (const candidate of binCandidates({})) {
      assert.ok(!/enter-cut|33_enter|sidecar|36_katok/.test(candidate), candidate);
    }
  });
});

describe("data directory", () => {
  it("defaults to ~/Library/Application Support/katok-mcp", () => {
    assert.equal(defaultDataDir("/home/u"), "/home/u/Library/Application Support/katok-mcp");
    assert.equal(dataDir({}), defaultDataDir());
  });

  it("KATOK_MCP_DATA_DIR overrides it; media and archive live inside", () => {
    const env = { KATOK_MCP_DATA_DIR: "/tmp/katok-mcp-test" };
    assert.equal(dataDir(env), "/tmp/katok-mcp-test");
    assert.equal(mediaDir(env), "/tmp/katok-mcp-test/media");
    assert.equal(archivePath(env), "/tmp/katok-mcp-test/archive.sqlite3");
  });
});

describe("policy files", () => {
  it("live in cli/", () => {
    assert.deepEqual(
      POLICY_FILES.map((file) => path.relative(REPO_ROOT, file)),
      ["cli/ACCEPTABLE_USE_POLICY.md", "cli/DISCLAIMER.md"],
    );
  });
});
