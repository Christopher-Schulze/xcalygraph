package com.example.store

import org.scalatest.flatspec.AnyFlatSpec

class MemoryRepositorySpec extends AnyFlatSpec {
  "find" should "return stored rows" in {
    val repository = new MemoryRepository()
    repository.put(Row(1, "a"))
    repository.find(1)
  }
}
