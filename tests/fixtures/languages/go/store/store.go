// Package store keeps rows in memory.
package store

import (
	"errors"
	"os"

	db "example.com/app/internal/database"
)

// MaxRows limits the table size.
const MaxRows = 100

var defaultName = "rows"

// Reader reads rows.
type Reader interface {
	Get(id int) (*Row, error)
	Len() int
}

// Row is one record. It has an id.
type Row struct {
	ID   int
	Name string
}

// Store holds rows.
type Store struct {
	rows   map[int]*Row
	client *db.Client
	Reader
}

type ID = int

// NewStore opens a store.
func NewStore(path string) (*Store, error) {
	if path == "" {
		return nil, errors.New("empty path")
	}
	client := db.Connect(os.Getenv("DB_URL"))
	return &Store{client: client}, nil
}

// Get returns the row with the id.
func (s *Store) Get(id int) (*Row, error) {
	row, ok := s.rows[id]
	if !ok {
		return nil, errors.New("missing")
	}
	s.client.Log(row.Name)
	return row, nil
}

func (s Store) Len() int { return len(s.rows) }

func load() {
	st, _ := NewStore(defaultName)
	r := &Row{ID: 1}
	var reader Reader = st
	reader.Len()
	st.Get(r.ID)
	_, _ = os.LookupEnv("STORE_MODE")
}
