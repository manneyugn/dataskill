---
name: metric-define
description: Record a business metric's definition once (SQL, grain, filters, owner) so every later query reuses it and gets the same number. Use when a metric is defined, disputed, or used in more than one place.
---

# Define a metric

Metrics live in `metrics/<metric_name>.md` in the repo, one file each.

## Steps

1. Agree the definition with the user in words: what is counted, what is excluded, the time basis (event time or processing time, which time zone), and the grain.
2. Write the canonical query and check it: `dataskill guard`, then `dataskill run` for one recent week.
3. Cross-check: compute the same number a second way (a different table, or a dashboard the user trusts). If the two disagree, investigate before saving; explain the difference.
4. Save `metrics/<metric_name>.md`:

~~~markdown
# weekly_active_users

Owner: <person>
Definition: Distinct users with at least one session event in the ISO week (Monday start, UTC).
Excludes: internal and test accounts (users.is_internal = TRUE).
Grain: one row per week.
Checked: 2026-11-09, matches the Product dashboard within 0.3%.

```sql
-- canonical query
```
~~~

5. When any later query uses this metric, copy the canonical SQL rather than rewriting it.

## Done when

The metric file exists, its query passes `dataskill guard`, and two independent query paths agree (or the gap is explained in the file).
