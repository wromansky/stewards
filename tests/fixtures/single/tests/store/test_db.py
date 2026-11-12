from src.store.db import save_item


def test_save_returns_id():
    assert save_item({"name": "a"}) == 1
