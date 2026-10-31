#include "store/store.h"

int test_put(void) {
    store_t *store = store_open();
    struct row row = {2, "b"};
    return store_put(store, row);
}
