// The promise-chain mutex behind the uiLock / writeLock in index.ts.

import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { createLock } from "../dist/lock.js";

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

describe("createLock", () => {
  it("runs overlapping tasks one after another, in call order", async () => {
    const lock = createLock();
    const events = [];
    const task = (name, ms) => async () => {
      events.push(`start ${name}`);
      await sleep(ms);
      events.push(`end ${name}`);
      return name;
    };
    const results = await Promise.all([
      lock(task("a", 30)),
      lock(task("b", 5)),
      lock(task("c", 1)),
    ]);
    assert.deepEqual(results, ["a", "b", "c"]);
    assert.deepEqual(events, ["start a", "end a", "start b", "end b", "start c", "end c"]);
  });

  it("a failing task rejects only its own caller and releases the lock", async () => {
    const lock = createLock();
    const first = lock(async () => {
      await sleep(5);
      throw new Error("boom");
    });
    const second = lock(async () => "ok");
    await assert.rejects(first, /boom/);
    assert.equal(await second, "ok");
    assert.equal(await lock(async () => "later"), "later");
  });

  it("two locks do not block each other", async () => {
    const ui = createLock();
    const write = createLock();
    const events = [];
    const slow = ui(async () => {
      events.push("ui start");
      await sleep(30);
      events.push("ui end");
    });
    const fast = write(async () => {
      events.push("write");
    });
    await Promise.all([slow, fast]);
    assert.deepEqual(events, ["ui start", "write", "ui end"]);
  });

  it("a task that throws synchronously still releases the lock", async () => {
    const lock = createLock();
    await assert.rejects(
      lock(() => {
        throw new Error("sync boom");
      }),
      /sync boom/,
    );
    assert.equal(await lock(async () => 1), 1);
  });
});
