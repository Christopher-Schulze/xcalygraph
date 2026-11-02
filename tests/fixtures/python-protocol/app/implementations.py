class FileReader:
    def read(self, count: int) -> bytes:
        return b""

    def close(self) -> None:
        pass


class MissingClose:
    def read(self, count: int) -> bytes:
        return b""


class WrongShape:
    def read(self, count: int, encoding: str) -> bytes:
        return b""

    def close(self) -> None:
        pass


class DecoratedReader:
    @staticmethod
    def read(self, count: int) -> bytes:
        return b""

    def close(self) -> None:
        pass


class LongWrong:
    def long_read(self, value: ThisIsAnExtremelyLongTypeNameThisIsAnExtremelyLongTypeNameThisIsAnExtremelyLongTypeNameThisIsAnExtremelyLongTypeNameThisIsAnExtremelyLongTypeNameBeta) -> bytes:
        return b""


class LiteralWrong:
    def literal_read(self, value: Literal["a b"]) -> bytes:
        return b""
