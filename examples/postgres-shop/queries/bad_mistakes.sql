-- Five common mistakes in one query; run `dataskill check` on it.
SELECT DISTINCT *
FROM shop.orders o
JOIN shop.users u ON u.id = o.user_id
WHERE u.country = NULL
  AND o.user_id NOT IN (SELECT id FROM shop.users WHERE email LIKE '%test%');
