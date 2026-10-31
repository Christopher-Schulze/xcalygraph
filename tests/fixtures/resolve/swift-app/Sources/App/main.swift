import Store

@main
struct App {
    static func main() {
        let repository = MemoryRepository()
        repository.put(id: 1, name: "a")
        print(label(repository, 1))
        print(render(1, root: 2, header: "h"))
    }

    static func label(_ repository: Repository, _ id: Int) -> String {
        repository.find(id)?.name ?? ""
    }
}
