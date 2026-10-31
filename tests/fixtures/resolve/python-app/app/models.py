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
