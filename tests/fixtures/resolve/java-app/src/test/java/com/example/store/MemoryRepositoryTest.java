package com.example.store;

import org.junit.jupiter.api.Test;

class MemoryRepositoryTest {
    @Test
    void findsStoredRow() {
        MemoryRepository repository = new MemoryRepository();
        repository.put(new Row(1, "a"));
        repository.find(1);
    }
}
