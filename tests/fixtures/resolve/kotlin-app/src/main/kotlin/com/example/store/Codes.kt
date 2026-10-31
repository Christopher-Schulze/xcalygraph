package com.example.store

interface Tagged {
  fun tag(): String
}

class Tags : Tagged {
  override fun tag(): String = "t"
}

// `Tagged by tags`: the class implements the interface through a delegate.
class Labeled(tags: Tags) : Tagged by tags

// Top-level overloads tell apart by argument count, also with a trailing lambda.
internal inline fun encode(action: StringBuilder.() -> Unit): String = buildString(action)

internal fun encode(prefix: String, action: StringBuilder.() -> Unit): String = prefix + encode(action)

fun label(): String = encode { append("x") }
