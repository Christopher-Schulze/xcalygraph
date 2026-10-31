package com.example.app

import com.example.store.MemoryRepository
import com.example.store.Repository
import com.example.store.Row as StoredRow

class Service(private val repository: Repository) {
    fun name(id: Int): String {
        val row: StoredRow? = repository.find(id)
        return format(row)
    }

    private fun format(row: StoredRow?): String = row?.name ?: ""
}

fun main() {
    val service = Service(MemoryRepository.empty())
    val url = System.getenv("APP_URL")
    println(service.name(1) + url)
}
