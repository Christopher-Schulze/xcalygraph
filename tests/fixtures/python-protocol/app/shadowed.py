from third_party import Protocol


def local_import():
    from typing import Protocol
    return Protocol


class NotAProtocol(Protocol):
    def read(self, count: int) -> bytes: ...

    def close(self) -> None: ...
