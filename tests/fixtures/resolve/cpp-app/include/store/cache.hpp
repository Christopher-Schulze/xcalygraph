#pragma once

#include "store/repository.hpp"

#define STORE_EXPORT __attribute__((visibility("default")))

namespace store {

// Export macros sit between `class` and the name in shared-library headers.
class STORE_EXPORT CachedRepository : public MemoryRepository {
public:
    STORE_EXPORT int hits() const;
};

// Same namespace, other file: no qualifier needed.
inline int cache_slot(int id) { return clamp_id(id) % 4; }

}  // namespace store
