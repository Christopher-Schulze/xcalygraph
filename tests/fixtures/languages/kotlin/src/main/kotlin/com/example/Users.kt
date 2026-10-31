package com.example

import kotlinx.coroutines.flow.Flow
import com.example.db.Database as Db

/** User records. */
data class User(val id: Long, val name: String)

enum class Role { ADMIN, MEMBER }

object Registry {
    const val LIMIT = 10
    private val users = mutableListOf<User>()

    fun add(user: User) {
        users.add(user)
    }
}

@Service
class UserService(private val db: Db) {
    fun all(limit: Int = Registry.LIMIT): List<User> = db.query(limit).map { User(it.id, it.name) }

    internal fun count(): Int {
        val list = all()
        return list.size
    }

    private fun unused() {}
}

fun String.shout(): String = uppercase()

fun main(args: Array<String>) {
    val token = System.getenv("TOKEN")
    UserService(Db()).all()
    Registry.add(User(1, token ?: ""))
}
