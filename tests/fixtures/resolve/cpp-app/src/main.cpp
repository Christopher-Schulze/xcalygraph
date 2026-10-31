#include <cstdlib>
#include <iostream>
#include "store/repository.hpp"

static std::string label(const store::Repository& repository, int id) {
    const store::Row* row = repository.find(id);
    return row ? row->name : "";
}

int main() {
    auto* repository = new store::MemoryRepository();
    repository->put(1, "a");
    const char* url = std::getenv("APP_URL");
    std::cout << label(*repository, 1) << url << std::endl;
    return 0;
}
