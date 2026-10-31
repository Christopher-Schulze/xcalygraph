"""Facade re-exporting the store API: importers name it, the store defines it."""

from app.store import Database
from app.store.db import connect as open_database
