# Demo app

Stores rows. Entry point: `cmd/app/main.go`.

## Store

`NewStore` opens a store; `Store.Get` reads one row. See [the store](internal/store/store.go)
and [missing](docs/none.md). `Get` alone is ambiguous.
