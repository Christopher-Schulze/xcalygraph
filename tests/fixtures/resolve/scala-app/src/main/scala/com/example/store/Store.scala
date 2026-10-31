package com.example.store

/** Reads rows by id. */
trait Repository {
  def find(id: Int): Option[Row]
}

case class Row(id: Int, name: String)

object Limits {
  val MaxId = 1000
}

abstract class BaseRepository {
  protected def check(id: Int): Unit = require(id <= Limits.MaxId)
}

class MemoryRepository extends BaseRepository with Repository {
  private val rows = scala.collection.mutable.Map[Int, Row]()

  override def find(id: Int): Option[Row] = {
    check(id)
    rows.get(id)
  }

  def put(row: Row): Unit = rows(row.id) = row

  def put(id: Int, name: String): Unit = put(Row(id, name))
}
