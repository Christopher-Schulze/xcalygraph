package store

import "example.com/app/internal/database"

// Timeout is the default timeout in seconds.
const Timeout = 5

type options struct {
	Timeout int
	mode    string
}

var defaults = options{Timeout: 1, mode: database.Mode}

var labels = map[string]string{database.Mode: "mode"}

var limits = map[int]string{Timeout: "timeout"}

var modes = []string{database.Mode}

const defaultMode = database.Mode
