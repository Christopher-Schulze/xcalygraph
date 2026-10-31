extension MemoryRepository {
    public func put(_ row: Row) {
        rows[row.id] = row
    }

    public func put(id: Int, name: String) {
        put(Row(id: id, name: name))
    }
}
