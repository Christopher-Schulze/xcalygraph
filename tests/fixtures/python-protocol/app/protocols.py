from typing import Protocol


class Reader(Protocol):
    def read(self, count: int) -> bytes: ...

    def close(self) -> None: ...


class NeedsFlag(Protocol):
    flag: bool

    def read(self, count: int) -> bytes: ...


class DefaultValue(Protocol):
    flag = True

    def read(self, count: int) -> bytes: ...


class LongReader(Protocol):
    def long_read(self, value: ThisIsAnExtremelyLongTypeNameThisIsAnExtremelyLongTypeNameThisIsAnExtremelyLongTypeNameThisIsAnExtremelyLongTypeNameThisIsAnExtremelyLongTypeNameAlpha) -> bytes: ...


class LiteralReader(Protocol):
    def literal_read(self, value: Literal["a  b"]) -> bytes: ...
