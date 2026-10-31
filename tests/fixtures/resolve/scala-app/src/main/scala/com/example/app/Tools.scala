package com.example.app

// A wildcard import of an object binds its members.
import com.example.store.Util._

object Tools {
  def clean(key: String): String = normalize(key)
}
