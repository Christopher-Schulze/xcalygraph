/// Reads rows by id.
public protocol Repository {
    func find(_ id: Int) -> Row?
}

public struct Row {
    public let id: Int
    public let name: String
}

public final class MemoryRepository: Repository {
    private var rows: [Int: Row] = [:]

    public init() {}

    public func find(_ id: Int) -> Row? {
        guard valid(id) else { return nil }
        return rows[id]
    }

    func valid(_ id: Int) -> Bool {
        id >= 0
    }
}
