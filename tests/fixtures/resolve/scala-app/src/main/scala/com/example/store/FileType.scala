package com.example.store

// A sealed trait and its companion object share the name; only the trait can be extended.
sealed trait FileType

object FileType {
  case object File extends FileType
  case object Dir extends FileType
}
