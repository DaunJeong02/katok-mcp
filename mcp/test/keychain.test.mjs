// Unit tests for the Keychain write path. Nothing here runs /usr/bin/security.

import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { addCommandLine, addFailure, KeychainDuplicateError } from "../dist/keychain.js";

const KEY = "ef".repeat(32);

describe("addCommandLine", () => {
  it("builds one add-generic-password command with the key and no -U", () => {
    const line = addCommandLine("com.example.svc", "archive-key", KEY);
    assert.equal(line, `add-generic-password -s "com.example.svc" -a "archive-key" -w "${KEY}"`);
    assert.ok(!/(^| )-U( |$)/.test(line));
  });

  it("quotes values with spaces", () => {
    assert.match(
      addCommandLine("my service", "my account", KEY),
      /-s "my service" -a "my account"/,
    );
  });

  it("refuses characters the interactive parser cannot carry", () => {
    for (const bad of ['a"b', "a\\b", "a\nb", ""]) {
      assert.throws(() => addCommandLine(bad, "acct", KEY), /service must be/);
      assert.throws(() => addCommandLine("svc", bad, KEY), /account must be/);
    }
  });
});

describe("addFailure", () => {
  it("maps exit 45 to a duplicate error", () => {
    const error = addFailure("s", "a", KEY, 45, "security: The specified item already exists");
    assert.ok(error instanceof KeychainDuplicateError);
  });

  it("maps the 'already exists' text to a duplicate error even without the exit code", () => {
    assert.ok(
      addFailure("s", "a", KEY, 1, "item already exists in the keychain") instanceof
        KeychainDuplicateError,
    );
  });

  it("keeps other failures as plain errors and redacts the key", () => {
    const error = addFailure("s", "a", KEY, 1, `denied for ${KEY}`);
    assert.ok(!(error instanceof KeychainDuplicateError));
    assert.match(error.message, /failed: denied for \[redacted\]/);
    assert.ok(!error.message.includes(KEY));
  });

  it("falls back to the exit code when stderr is empty", () => {
    assert.match(addFailure("s", "a", KEY, 36, "").message, /exit code 36/);
  });
});
