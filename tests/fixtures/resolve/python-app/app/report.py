from app.facade import Database, open_database


def report():
    return open_database().query("x"), Database
