---
name: data-quality
description: Write and run data quality checks (nulls, duplicates, freshness, ranges, referential integrity) for a table and report pass or fail. Use when asked to validate a table, investigate suspicious numbers, or add tests before a pipeline change.
---

# Data quality

## Steps

1. `dataskill describe <table>` to get columns, types and the partition column.
2. Ask the user (or infer from names, marked as an assumption) which columns are keys, which must never be NULL, and how fresh the table should be.
3. Write one query per check. Each returns a single row with `check_name`, `failed_rows` and `total_rows`. Always restrict to a recent partition window unless the user asks for a full scan.

| Check | Fails when |
| --- | --- |
| not_null | a required column is NULL |
| unique | a key value appears more than once |
| freshness | the newest timestamp is older than the agreed lag |
| accepted_values | a status-like column has a value outside the list |
| range | a number or date falls outside a sane range (negative amounts, future dates) |
| relationships | a foreign key has no match in its parent table |

4. `dataskill guard` each query, then `dataskill run` it.
5. Report a table of checks with pass or fail and the failing row count. For failures, show up to 5 example keys (never whole rows with personal data).
6. If the project uses dbt, offer to turn the passing checks into dbt tests (`dbt-model` skill).

## Done when

Every check has run and reported pass or fail with a count, and the scan window used is stated.
