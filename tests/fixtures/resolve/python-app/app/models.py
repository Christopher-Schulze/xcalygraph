"""Decorators, dataclass fields and properties."""

from dataclasses import dataclass


def logged(fn):
    return fn


@dataclass
class Account:
    id: int
    owner: str = ""

    @property
    def label(self) -> str:
        return self.owner.upper()


@logged
def describe(account: Account) -> str:
    return account.label


class AdminAccount(Account):
    def __init__(self, id: int) -> None:
        super().__init__(id, "admin")
        super().label


class Saver:
    def save(self) -> int:
        return 1


class AuditedSaver(Saver):
    def save(self) -> int:
        return super().save() + 1
