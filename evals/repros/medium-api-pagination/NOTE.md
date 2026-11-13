Command (run from `evals/fixture/api`):

    python3 - <<'PY'
    import items
    items._ITEMS = []
    for name in ("a", "b", "c"):
        items.create_item(name)
    print(items.list_items(page=2, page_size=2))
    PY

Expected failing output: page 2 has `"items": []` while `"total": 3`, so
the last item is skipped. After the fix, page 2 contains the third item.
