-- Sample data for trying dataskill locally. Creates schema "shop" with 2,000 users and 50,000 orders.
CREATE SCHEMA IF NOT EXISTS shop;
CREATE TABLE shop.users (id int PRIMARY KEY, email text, country text, created_at timestamptz);
CREATE TABLE shop.orders (id int PRIMARY KEY, user_id int REFERENCES shop.users(id), status text, amount numeric, ordered_at timestamptz);
INSERT INTO shop.users
SELECT g, 'user' || g || '@example.com', (ARRAY['VN','US','SG','JP'])[1 + g % 4], now() - g * interval '1 day'
FROM generate_series(1, 2000) g;
INSERT INTO shop.orders
SELECT g, 1 + (g % 2000), (ARRAY['paid','refunded','pending'])[1 + g % 3], (g % 500) + 0.99, now() - (g % 365) * interval '1 day'
FROM generate_series(1, 50000) g;
ANALYZE;
