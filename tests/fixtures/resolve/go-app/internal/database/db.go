package database

import "github.com/lib/pq"

// Client talks to the database.
type Client struct{ url string }

// Connect opens a client.
func Connect(url string) *Client {
	_ = pq.Driver{}
	return &Client{url: url}
}

// Log writes a line.
func (c *Client) Log(line string) {}
