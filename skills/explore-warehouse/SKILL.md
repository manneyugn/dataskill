---
name: explore-warehouse
description: Map a data warehouse (BigQuery or Postgres) into local schema memory before any SQL work. Use when starting data work in a repo, when a table or column name is unknown, or when the schema has changed.
---

# Explore the warehouse

Build the schema memory once, then look names up instead of guessing them.

## Steps

1. If there is no `dataskill.toml` in the project, run `dataskill init` and ask the user for the warehouse, the billing project and the datasets or schemas to map. Do not guess these.
2. Run `dataskill map`. It writes `.dataskill/schema.json` and `.dataskill/SCHEMA.md`.
3. Read `.dataskill/SCHEMA.md`. Summarize for the user in a few lines:
   - the largest tables and their size;
   - which tables are partitioned, and on which column;
   - likely join keys (columns such as `user_id` or `order_id` that appear in several tables), marked as guesses until confirmed.
4. Suggest adding `.dataskill/schema.json` to version control so the whole team shares it, or to `.gitignore` if table names are sensitive. Let the user decide.

## Rules

- Never read row-level data to learn the schema; `dataskill map` only reads metadata.
- When a name is unclear later, run `dataskill lookup <part of name>` or `dataskill describe <table>` (MCP: `lookup_schema`, `describe_table`).

## Done when

- `dataskill lookup` returns results for at least one table the user named, and
- every table you mention to the user appears in `.dataskill/SCHEMA.md`.
