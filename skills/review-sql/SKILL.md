---
name: review-sql
description: Review SQL files or a pull request for correctness, cost and safety, and report findings that each cite a line and a fix. Use when asked to review queries, dbt models, or SQL changes in a PR.
---

# Review SQL

## Steps

1. Collect the SQL under review: the files named, or the `.sql` files changed in the branch (`git diff --name-only origin/main...HEAD -- '*.sql'`).
2. Run `dataskill check --json <files>` for the mechanical findings.
3. Read each query yourself for the problems a parser cannot see:
   - **Grain:** does the result have one row per what the author intends? Look for joins that multiply rows (one-to-many joins before a SUM), and DISTINCT used to hide them.
   - **Filters:** time windows off by one (`<` vs `<=`, time zones, `DATE(timestamp)` in UTC vs local); filters in `WHERE` that turn a `LEFT JOIN` into an inner join.
   - **NULLs:** `COUNT(col)` vs `COUNT(*)`, averages over NULLs, `NOT IN` with NULLs, string concatenation with NULL.
   - **Cost:** missing partition filters, `SELECT *`, repeated scans of the same large table that a CTE could share.
   - **Names:** every table and column exists (`dataskill lookup`).
4. For queries that changed, run `dataskill guard` on the new version to report the bytes scanned.

## Output

One finding per item, most severe first:

```
file.sql:12  [fan-out] Joining orders to order_items before SUM(orders.amount) counts each order once per item.
             Fix: aggregate order_items first, or sum order_items.price instead.
```

Use labels `block`, `bug`, `cost`, `style`. Do not report style issues unless asked. If nothing is wrong, say so plainly.

## Done when

Every finding names a file and line and gives a concrete fix, and `dataskill check` was run on every file reviewed.
