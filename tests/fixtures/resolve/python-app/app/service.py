from app.store.db import connect, Database
from . import util
import app.store.db as dbmod


class Base:
    def describe(self):
        return "base"


class Service(Base):
    def __init__(self):
        self.db = Database("x")

    def run(self, name: str):
        rows = self.db.query("select")
        util.log(name)
        other = connect()
        other.query("again")
        dbmod.connect()
        return self.describe()


def main():
    Service().run("x")


if __name__ == "__main__":
    main()
