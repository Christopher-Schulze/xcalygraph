package main

import "example.com/app/internal/store"

func main() {
	s, err := store.NewStore("data")
	if err != nil {
		return
	}
	s.Len()
}
