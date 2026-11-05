package queries

// "SELECT id FROM users"
func Load() string { return "SELECT id FROM users" }
func Unowned() string { return "SELECT id FROM accounts" }
func Migrated() string { return "SELECT id FROM events" }
