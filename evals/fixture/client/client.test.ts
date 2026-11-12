import assert from "node:assert/strict";
import { rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { beforeEach, test } from "node:test";

import { createItem, getItem, listItems } from "./client.ts";

const store = join(tmpdir(), "items-client-test.json");

beforeEach(() => {
  process.env.ITEMS_STORE = store;
  rmSync(store, { force: true });
});

test("createItem returns the created item", () => {
  const item = createItem("widget");
  assert.equal(item.name, "widget");
  assert.ok(item.id);
});

test("successful create does not log a retry message", (t) => {
  const err = t.mock.method(console, "error", () => {});
  createItem("widget");
  const text = err.mock.calls.map((call) => call.arguments.join(" ")).join("\n");
  assert.ok(!text.includes("retrying"), `misleading retry log: ${text}`);
});

test("getItem returns null for a missing id", () => {
  assert.equal(getItem("nope"), null);
});

test("listItems reports the store total", () => {
  createItem("a");
  const result = listItems(1, 10);
  assert.equal(result.total, 1);
});

test("createItem stamps the current time", () => {
  const item = createItem("clock");
  assert.equal(item.createdAt, Date.now());
});
