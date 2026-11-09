---
name: write-sql
description: Write a SQL query that answers a business question, using only real table and column names, and prove it is safe and affordable before running it. Use for any ad hoc analysis or query request against the warehouse.
---

# Write SQL

## Steps

1. Restate the question as: the metric, the grain (one row per what?), the filters and the time window. If any of these is ambiguous, ask one short question before writing SQL.
2. Find every table and column with `dataskill lookup` / `dataskill describe` (MCP: `lookup_schema`, `describe_table`). If `.dataskill/` does not exist, use the `explore-warehouse` skill first.
3. If the metric is defined in `metrics/` (see `metric-define`), reuse that definition exactly.
4. Write the query:
   - name columns explicitly; no `SELECT *`;
   - filter on the partition column of every partitioned table;
   - aggregate the "many" side before joining to avoid fan-out;
   - use `IS NULL`, never `= NULL`; prefer `NOT EXISTS` to `NOT IN (subquery)`;
   - comment the grain and any assumption at the top of the query.
5. Run `dataskill guard <file.sql>` (MCP: `guard_sql`). Fix every `BLOCK` and every `warn`, or state in one line why a warning does not apply.
6. Run it with `dataskill run <file.sql>` (MCP: `run_query`), which only executes read-only queries under the cost limit.
7. Sanity-check the result before reporting: row count plausible, no unexpected NULLs in key columns, totals in the expected order of magnitude. Say what you checked.

## Never

- Invent a table or column name.
- Run a query that `guard` refused, by any other route.
- Raise the cost limit in `dataskill.toml` without the user asking.

## Done when

`dataskill guard` passes, the query has run, and your answer states the number, the grain, the time window and the checks you made.
