package com.example.app

// A selective import of an object member binds that member.
import com.example.store.Util.{normalize}

object Keys {
  def key(raw: String): String = normalize(raw)
}
