#include <gtest/gtest.h>
#include "store/repository.hpp"

TEST(MemoryRepository, FindsStoredRow) {
    store::MemoryRepository repository;
    repository.put(store::Row{1, "a"});
    EXPECT_NE(repository.find(1), nullptr);
}
