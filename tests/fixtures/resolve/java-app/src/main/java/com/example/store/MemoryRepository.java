package com.example.store;

import java.util.HashMap;
import java.util.Map;

/** In-memory repository. */
public class MemoryRepository extends BaseRepository implements Repository {
    private final Map<Integer, Row> rows = new HashMap<>();

    @Override
    public Row find(int id) {
        check(id);
        return rows.get(id);
    }

    void put(Row row) {
        rows.put(row.id(), row);
    }

    void put(int id, String name) {
        put(new Row(id, name));
    }
}

abstract class BaseRepository {
    protected void check(int id) {
        if (id > Row.MAX_ID) {
            throw new IllegalArgumentException("id");
        }
    }
}
