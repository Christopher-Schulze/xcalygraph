CREATE TABLE users (
  id bigint PRIMARY KEY,
  email text NOT NULL
);

CREATE TABLE orders (
  id bigint PRIMARY KEY,
  user_id bigint REFERENCES users (id)
);

CREATE VIEW order_counts AS
  SELECT user_id, count(*) FROM orders GROUP BY user_id;
