---
name: reconcile
description: Compare two tables, two versions of a model, or a table and a source system, and list exactly where and why they differ. Use after a migration or refactor, when two reports disagree, or before replacing a model.
---

# Reconcile

## Steps

1. Agree the key that identifies a row in both sides and the columns that should match. Confirm both sides have the same grain; if not, aggregate one side first.
2. Restrict both sides to the same time window (a recent partition range) to keep the scan small.
3. Run, through `dataskill guard` and `dataskill run`:
   1. **Counts:** rows per side, and distinct keys per side.
   2. **Missing keys:** keys only in A and keys only in B (counts, plus up to 10 examples each).
   3. **Value differences:** for keys in both, per column, how many rows differ (treat NULL = NULL as equal: `IS DISTINCT FROM` in Postgres, `IFNULL`/`COALESCE` pairs in BigQuery).
   4. **Totals:** sums of key numeric columns on each side and the difference.
4. For every difference, find the cause by looking at examples: time zone shift, late-arriving data, filter change, duplicate keys, rounding.

## Output

A short table: check, side A, side B, difference. Then one line per cause found, with the evidence.

## Done when

Every difference is either explained with evidence or listed as unexplained with example keys.
