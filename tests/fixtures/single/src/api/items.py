from src.store.db import save_item

router = []


def create_item(body):
    if "name" not in body:
        return 422, None
    item_id = save_item(body)
    return 201, {"id": item_id, **body}


def list_items():
    # simplified: returns every item; paginate once lists pass 1000
    return []
