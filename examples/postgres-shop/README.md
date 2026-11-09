# Try dataskill on a local Postgres

```bash
createdb shopdemo
psql shopdemo -f seed.sql
export DATABASE_URL=postgresql://localhost/shopdemo
cd examples/postgres-shop

dataskill map                                   # build schema memory
dataskill lookup country                        # find a column
dataskill check queries/bad_mistakes.sql        # five warnings
dataskill guard queries/bad_write.sql           # blocked
dataskill guard "SELECT a.id, b.id AS other_id FROM shop.orders a JOIN shop.orders b ON a.user_id = b.user_id LIMIT 10"   # refused: 8 MB scan, over the 6 MB demo limit
dataskill run queries/good_revenue_by_country.sql                 # runs, read-only
```
