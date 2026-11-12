from src.api.items import create_item


def test_create_ok():
    assert create_item({"name": "a"})[0] == 201
