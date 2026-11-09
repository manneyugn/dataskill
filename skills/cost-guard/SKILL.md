---
name: cost-guard
description: Estimate what a query will scan and cost before it runs, and make expensive queries cheaper. Use before running any query on a large table, when a query was refused as over the limit, or when asked to cut warehouse spend.
---

# Cost guard

## Steps

1. Run `dataskill guard <query or file>` (MCP: `guard_sql`). It reports the bytes the query would scan and the project limit (`max_bytes` in `dataskill.toml`).
2. Report the estimate in plain terms. For BigQuery on-demand pricing, cost is roughly bytes scanned × the price per TiB for the user's region; say "roughly" and name the assumption rather than quoting a price from memory.
3. If it is over the limit, or more than the user expects, make it cheaper in this order:
   1. filter on the partition column (`dataskill describe <table>` shows it);
   2. select only the columns needed (BigQuery bills by column);
   3. use a smaller pre-aggregated or sampled table if one exists (`dataskill lookup daily`, `lookup summary`);
   4. split a long time window into one cheap test run on a short window first.
4. Re-run `dataskill guard` after each change and show before and after.

## Never

- Raise `max_bytes` yourself. If the user needs a bigger scan, show the estimate and let them change the limit.
- Note that `LIMIT` does not reduce bytes scanned in BigQuery; do not offer it as a cost fix there.

## Done when

The query passes `dataskill guard`, and you have told the user the estimated scan before and after your changes.
