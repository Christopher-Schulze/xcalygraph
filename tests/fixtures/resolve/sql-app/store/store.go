package store

import "database/sql"

// Emails lists every user email.
func Emails(db *sql.DB) error {
	_, err := db.Query("SELECT email FROM users")
	return err
}

func Counts(db *sql.DB) error {
	_, err := db.Query(`
		SELECT user_id FROM order_counts WHERE user_id = $1`, 1)
	return err
}
