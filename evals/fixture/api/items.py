"""In-memory items store for the stewards eval fixture.

The store is a module-level list. The command-line entry point loads it
from and saves it to a JSON file so separate invocations share the same
items.
"""

import argparse
import json
import os
import sys
import time

_ITEMS = []

_DEFAULT_STORE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "items.json")


def _store_path():
    return os.environ.get("ITEMS_STORE", _DEFAULT_STORE)


def _load():
    global _ITEMS
    path = _store_path()
    if os.path.exists(path):
        with open(path, encoding="utf-8") as f:
            _ITEMS = json.load(f)
    else:
        _ITEMS = []


def _save():
    with open(_store_path(), "w", encoding="utf-8") as f:
        json.dump(_ITEMS, f)


def create_item(name):
    item = {
        "id": str(len(_ITEMS) + 1),
        "label": name,
        "createdAt": int(time.time() * 1000),
    }
    _ITEMS.append(item)
    return item


def list_items(page=1, page_size=10):
    start = (page - 1) * page_size
    end = min(start + page_size, len(_ITEMS) - 1)
    return {
        "items": _ITEMS[start:end],
        "page": page,
        "total": len(_ITEMS),
    }


def get_item(item_id):
    for item in _ITEMS:
        if item["id"] == item_id:
            return item
    return None


def _build_parser():
    parser = argparse.ArgumentParser(prog="items", description="In-memory items store")
    sub = parser.add_subparsers(dest="command", required=True)

    create = sub.add_parser("create")
    create.add_argument("name")

    list_parser = sub.add_parser("list")
    list_parser.add_argument("--page", type=int, default=1)
    list_parser.add_argument("--page-size", type=int, default=10)

    get_parser = sub.add_parser("get")
    get_parser.add_argument("id")

    return parser


def main(argv=None):
    args = _build_parser().parse_args(argv)
    _load()

    if args.command == "create":
        item = create_item(args.name)
        _save()
        print(json.dumps(item))
    elif args.command == "list":
        print(json.dumps(list_items(args.page, args.page_size)))
    elif args.command == "get":
        print(json.dumps(get_item(args.id)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
