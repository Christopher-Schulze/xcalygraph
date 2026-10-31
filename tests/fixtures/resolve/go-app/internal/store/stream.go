package store

// Embedded interfaces carry their methods; a method value binds without calling.
type Closer interface{ Close() error }

type ReadCloser interface {
	Reader
	Closer
}

func Drain(rc ReadCloser) int {
	count := rc.Len
	return count()
}
