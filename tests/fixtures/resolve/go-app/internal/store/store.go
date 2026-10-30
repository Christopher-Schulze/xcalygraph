package store

import (
	"errors"
	"os"

	db "example.com/app/internal/database"
)

type Reader interface {
	Get(id int) (*Row, error)
	Len() int
}

type Row struct{ ID int }

type Store struct {
	rows   map[int]*Row
	client *db.Client
}

func NewStore(path string) (*Store, error) {
	if path == "" {
		return nil, errors.New("empty")
	}
	client := db.Connect(os.Getenv("DB_URL"))
	return &Store{client: client}, nil
}

func (s *Store) Get(id int) (*Row, error) {
	row := s.rows[id]
	s.client.Log("get")
	return row, nil
}

func (s *Store) Len() int { return len(s.rows) }

func count(r Reader) int {
	return r.Len()
}
