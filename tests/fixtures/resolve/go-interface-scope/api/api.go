package api

// Reader is exported: any package's type with the method can implement it.
type Reader interface {
	Read() string
}
