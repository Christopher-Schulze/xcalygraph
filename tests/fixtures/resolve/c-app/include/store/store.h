#ifndef STORE_H
#define STORE_H

#define MAX_ROWS 64

/* A stored row. */
struct row {
    int id;
    const char *name;
};

typedef struct store {
    struct row rows[MAX_ROWS];
    int count;
} store_t;

/* Opens an empty store. */
store_t *store_open(void);
int store_put(store_t *store, struct row row);
const struct row *store_get(const store_t *store, int id);

#endif
