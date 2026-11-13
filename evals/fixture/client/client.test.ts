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
  assert.ok(item.id);
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
  assert.equal(Math.floor(item.createdAt / 1000), Math.floor(Date.now() / 1000));
});
