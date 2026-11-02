public struct Codec {
    public func encode(_ value: Int, radix base: Int) -> String { "radix" }
    public func encode(_ value: Int, base radix: Int) -> String { "base" }
    public func run() {
        _ = self.encode(1, base: 2)
        _ = self.encode(1, missing: 2)
    }
}
