import io
import json
import os
import tempfile
import unittest
from contextlib import redirect_stdout

import items


class ItemsStoreTest(unittest.TestCase):
    def setUp(self):
        items._ITEMS = []
        self._tmp = tempfile.NamedTemporaryFile(delete=False)
        self._tmp.close()
        os.unlink(self._tmp.name)
        os.environ["ITEMS_STORE"] = self._tmp.name

    def tearDown(self):
        if os.path.exists(self._tmp.name):
            os.unlink(self._tmp.name)

    def test_create_and_get(self):
        item = items.create_item("widget")
        self.assertEqual(item["label"], "widget")
        self.assertEqual(items.get_item(item["id"])["label"], "widget")

    def test_list_first_page(self):
        for name in ("a", "b", "c"):
            items.create_item(name)
        page1 = items.list_items(page=1, page_size=2)
        self.assertEqual([i["label"] for i in page1["items"]], ["a", "b"])

    def test_get_missing(self):
        self.assertIsNone(items.get_item("nope"))

    def test_total_counts_all_items(self):
        for name in ("a", "b", "c"):
            items.create_item(name)
        self.assertEqual(items.list_items(page=1, page_size=2)["total"], 3)

    def test_cli_create_list_get(self):
        buf = io.StringIO()
        with redirect_stdout(buf):
            self.assertEqual(items.main(["create", "widget"]), 0)
            self.assertEqual(items.main(["list", "--page-size", "10"]), 0)
            self.assertEqual(items.main(["get", "1"]), 0)
        lines = buf.getvalue().strip().splitlines()
        created = json.loads(lines[0])
        self.assertEqual(created["label"], "widget")
        self.assertEqual(created["id"], "1")
        listed = json.loads(lines[1])
        self.assertEqual(listed["total"], 1)
        got = json.loads(lines[2])
        self.assertEqual(got["id"], "1")


if __name__ == "__main__":
    unittest.main()
