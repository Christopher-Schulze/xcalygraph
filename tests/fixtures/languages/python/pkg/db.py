"""Database access."""
import os


class Database:
    """Talks to the database."""

    def __init__(self, url: str):
        self.url = url

    def query(self, sql: str) -> list:
        return []


def connect() -> Database:
    return Database(os.environ.get("DB_URL", ""))
