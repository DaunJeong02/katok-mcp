// Unit tests for result shaping (katok_index's documents[] → document_count). Pure.

import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { shrinkIndexResult } from "../dist/results.js";

// Shape of `katok index --json` (cli/src/commands/index_commands.rs), synthetic values.
const payload = () => ({
  full: false,
  dry_run: false,
  candidate_chunks: 3,
  written_documents: 3,
  embedding_calls: 1,
  documents: [{ id: "doc-1" }, { id: "doc-2" }, { id: "doc-3" }],
  embedder: "synthetic-embedder",
  semantic_units: "parent_windows",
});

describe("shrinkIndexResult", () => {
  it("replaces documents[] with document_count and keeps every other field", () => {
    const { documents, ...rest } = payload();
    assert.deepEqual(shrinkIndexResult(payload(), false), { ...rest, document_count: 3 });
    assert.equal(documents.length, 3);
  });

  it("keeps the full payload with verbose", () => {
    assert.deepEqual(shrinkIndexResult(payload(), true), payload());
  });

  it("leaves data without a documents array untouched", () => {
    const data = { written_documents: 0 };
    assert.equal(shrinkIndexResult(data, false), data);
    assert.equal(shrinkIndexResult(null, false), null);
    assert.deepEqual(shrinkIndexResult([1, 2], false), [1, 2]);
    assert.deepEqual(shrinkIndexResult({ documents: "x" }, false), { documents: "x" });
  });
});
