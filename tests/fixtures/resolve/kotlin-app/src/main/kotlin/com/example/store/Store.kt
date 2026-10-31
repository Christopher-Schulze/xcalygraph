package com.example.store

/** Reads rows by id. */
interface Repository {
    fun find(id: Int): Row?
}

data class Row(val id: Int, val name: String)

const val MAX_ID = 1000

open class BaseRepository {
    protected fun check(id: Int) {
        require(id <= MAX_ID)
    }
}

class MemoryRepository : BaseRepository(), Repository {
    private val rows = mutableMapOf<Int, Row>()

    override fun find(id: Int): Row? {
        check(id)
        return rows[id]
    }

    fun put(row: Row) {
        rows[row.id] = row
    }

    fun put(id: Int, name: String) = put(Row(id, name))

    companion object {
        fun empty(): MemoryRepository = MemoryRepository()
    }
}
