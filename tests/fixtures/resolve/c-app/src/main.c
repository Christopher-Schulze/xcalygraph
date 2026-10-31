#include <stdio.h>
#include <stdlib.h>
#include "store/store.h"

typedef int (*putter)(store_t *, struct row);

static int apply(putter put, store_t *store) {
    struct row row = {1, "a"};
    return put(store, row);
}

int main(void) {
    const char *path = getenv("STORE_PATH");
    store_t *store = store_open();
    apply(store_put, store);
    printf("%s %s\n", path, store_get(store, 1)->name);
    return 0;
}
