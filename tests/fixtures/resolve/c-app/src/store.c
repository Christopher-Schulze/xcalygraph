#include <stdlib.h>
#include "store/store.h"

static int valid(int id) {
    return id >= 0 && id < MAX_ROWS;
}

store_t *store_open(void) {
    store_t *store = calloc(1, sizeof(store_t));
    return store;
}

int store_put(store_t *store, struct row row) {
    if (!valid(row.id)) {
        return -1;
    }
    store->rows[store->count++] = row;
    return 0;
}

const struct row *store_get(const store_t *store, int id) {
    return valid(id) ? &store->rows[id] : NULL;
}
