CREATE TABLE users (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL
);

CREATE TABLE orders (
  id INTEGER PRIMARY KEY,
  user_id INTEGER REFERENCES users(id),
  total INTEGER
);

CREATE VIEW big_orders AS
SELECT * FROM orders WHERE total > 100;
