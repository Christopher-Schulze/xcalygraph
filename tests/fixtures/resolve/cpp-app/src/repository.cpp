#include "store/repository.hpp"

namespace store {

const Row* MemoryRepository::find(int id) const {
    return valid(id) ? &rows_[id] : nullptr;
}

void MemoryRepository::put(const Row& row) {
    if (valid(row.id)) {
        rows_[row.id] = row;
    }
}

void MemoryRepository::put(int id, const std::string& name) {
    put(Row{id, name});
}

bool MemoryRepository::valid(int id) const {
    return id >= 0 && id < 16;
}

}  // namespace store
