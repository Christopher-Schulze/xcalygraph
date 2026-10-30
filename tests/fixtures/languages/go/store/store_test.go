package store

import "testing"

func TestGet(t *testing.T) {
	s, _ := NewStore("x")
	s.Get(1)
}

func helper() {}
