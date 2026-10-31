import XCTest
@testable import Store

final class MemoryRepositoryTests: XCTestCase {
    func testFindsStoredRow() {
        let repository = MemoryRepository()
        repository.put(Row(id: 1, name: "a"))
        XCTAssertNotNil(repository.find(1))
    }
}
