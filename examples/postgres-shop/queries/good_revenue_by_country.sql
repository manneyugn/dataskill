-- Paid revenue by country. One row per country.
SELECT u.country, COUNT(*) AS orders, ROUND(SUM(o.amount), 2) AS revenue
FROM shop.orders o
JOIN shop.users u ON u.id = o.user_id
WHERE o.status = 'paid'
GROUP BY u.country
ORDER BY revenue DESC;
