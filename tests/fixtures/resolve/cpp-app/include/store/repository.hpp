#pragma once
#include <string>

namespace store {

/// A stored row.
struct Row {
    int id;
    std::string name;
};

/// Reads rows by id.
class Repository {
public:
    virtual ~Repository() = default;
    virtual const Row* find(int id) const = 0;
};

class MemoryRepository : public Repository {
public:
    const Row* find(int id) const override;
    void put(const Row& row);
    void put(int id, const std::string& name);

private:
    bool valid(int id) const;
    Row rows_[16];
};

inline int clamp_id(int id) { return id < 0 ? 0 : id; }

}  // namespace store
