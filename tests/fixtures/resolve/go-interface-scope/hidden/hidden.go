package hidden

// closer is unexported and this package imports nothing: no other package's type can reach it.
type closer interface {
	Shut() string
}

type local struct{}

func (local) Shut() string { return "local" }
