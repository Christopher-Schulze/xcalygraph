package com.example.store

import org.junit.jupiter.api.Test

class MemoryRepositoryTest {
    @Test
    fun findsStoredRow() {
        val repository = MemoryRepository()
        repository.put(Row(1, "a"))
        repository.find(1)
    }
}
