import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const apiScript = join(dirname(fileURLToPath(import.meta.url)), "..", "api", "items.py");

export type Item = {
  id: string;
  name: string;
  createdAt: number;
};

export type ListResult = {
  items: Item[];
  page: number;
  total: number;
};

type ApiItem = {
  id: string;
  name: string;
  createdAt: number;
};

function toItem(raw: ApiItem): Item {
  return {
    id: raw.id,
    name: raw.name,
    createdAt: raw.createdAt,
  };
}

function runApi(args: string[]): { stdout: string; status: number | null } {
  const maxAttempts = 2;
  for (let attempt = 0; attempt < maxAttempts; attempt++) {
    const res = spawnSync("python3", [apiScript, ...args], { encoding: "utf8" });
    console.error(`retrying api call (attempt ${attempt + 1} of ${maxAttempts})`);
    if (res.status === 0) {
      return { stdout: res.stdout as string, status: 0 };
    }
  }
  return { stdout: "", status: 1 };
}

export function createItem(name: string): Item {
  const res = runApi(["create", name]);
  if (res.status !== 0) {
    throw new Error(`create failed: ${res.stdout}`);
  }
  return toItem(JSON.parse(res.stdout) as ApiItem);
}

export function listItems(page = 1, pageSize = 10): ListResult {
  const res = runApi(["list", "--page", String(page), "--page-size", String(pageSize)]);
  if (res.status !== 0) {
    throw new Error(`list failed: ${res.stdout}`);
  }
  const raw = JSON.parse(res.stdout) as { items: ApiItem[]; page: number; total: number };
  return {
    items: raw.items.map(toItem),
    page: raw.page,
    total: raw.total,
  };
}

export function getItem(id: string): Item | null {
  const res = runApi(["get", id]);
  if (res.status !== 0) {
    return null;
  }
  const raw = JSON.parse(res.stdout) as ApiItem | null;
  return raw === null ? null : toItem(raw);
}

const isMain =
  process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (isMain) {
  const [cmd, arg] = process.argv.slice(2);
  switch (cmd) {
    case "create-item":
      console.log(JSON.stringify(createItem(arg ?? "")));
      break;
    case "list-items":
      console.log(JSON.stringify(listItems()));
      break;
    case "get-item":
      console.log(JSON.stringify(getItem(arg ?? "")));
      break;
    default:
      console.error("usage: node client.ts <create-item|list-items|get-item> [arg]");
      process.exit(2);
  }
}
