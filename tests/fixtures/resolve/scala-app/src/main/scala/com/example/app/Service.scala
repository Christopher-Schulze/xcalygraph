package com.example.app

import com.example.store.{MemoryRepository, Repository, Row => StoredRow}

class Service(repository: Repository) {
  def name(id: Int): String = {
    val row: Option[StoredRow] = repository.find(id)
    format(row)
  }

  private def format(row: Option[StoredRow]): String = row.map(_.name).getOrElse("")
}

object Main {
  def main(args: Array[String]): Unit = {
    val service = new Service(new MemoryRepository())
    val url = sys.env("APP_URL")
    println(service.name(1) + url)
  }
}
