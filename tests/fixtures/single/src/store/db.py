_items = []


def save_item(item):
    _items.append(item)
    return len(_items)


def count_items():
    return len(_items)
