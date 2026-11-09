# dataskill

**Agent skills for data work, with a guard that stops bad queries before they reach your warehouse.**

Coding agents are good at code and bad at data. They invent column names, run full-table scans that cost real money, write SQL that runs but returns the wrong number, and will happily `DELETE` if asked nicely. dataskill fixes that with two parts:

- **10 skills** (plain `SKILL.md` files) that teach Claude Code, Codex, Cursor, OpenCode and any agent that reads skills how to do data work properly, each ending in a check it must pass.
- **`dataskill`**, a single Rust binary (CLI + MCP server) that every query goes through: read-only, dry-run priced against your limit, and checked against a local map of your schema.

```
$ dataskill guard "SELECT user_id, COUNT(*) FROM shop.events GROUP BY user_id"
warn  query: [partition-filter] `proj.shop.events` is partitioned by `event_date`, but the query never uses `event_date`, so it scans the whole table.
      fix: Filter on the partition column, e.g. WHERE <partition column> >= <start date>.
Would scan 50.0 GB, over the 10.0 GB limit. Narrow it: filter on the partition column, select fewer columns, or query a smaller pre-aggregated table.
```

Works with **BigQuery** and **Postgres**. No account, no API key, no data leaves your machine: dataskill drives your own `bq` or `psql`, with your own credentials.

## Quick start

```bash
# 1. The guard
cargo install --git https://github.com/your-org/dataskill

# 2. The skills: any agent
npx skills add your-org/dataskill
#    or as a Claude Code plugin (skills + MCP server)
/plugin marketplace add your-org/dataskill
/plugin install dataskill@dataskill

# 3. In your data project
dataskill init        # writes dataskill.toml: warehouse, project, datasets, cost limit
dataskill map         # builds .dataskill/schema.json and SCHEMA.md from metadata only
```

Then ask your agent things like "what was paid revenue by country last month?" and watch it look up names, price the query and check its answer.

## The guard

| Command | What it does |
| --- | --- |
| `dataskill check <sql or files>` | Static checks, no warehouse access. Exit 2 on a block; `--strict` also fails on warnings (for CI). |
| `dataskill guard <sql or file>` | Checks plus a dry run: bytes scanned against `max_bytes`. Unknown columns surface here too. |
| `dataskill run <sql or file>` | Guard, then run **read-only** and return at most `max_rows` rows as CSV. |
| `dataskill map` | Build schema memory from warehouse metadata. |
| `dataskill lookup <term>` / `describe <table>` | Search names; show a table's columns, partition column and size. |
| `dataskill mcp` | The same tools over MCP: `check_sql`, `guard_sql`, `run_query`, `lookup_schema`, `describe_table`. |

What the checks catch:

| Rule | Severity |
| --- | --- |
| Writes and DDL (`INSERT`, `UPDATE`, `DELETE`, `MERGE`, `CREATE`, `DROP` ...) | block |
| SQL that does not parse | block |
| Query would scan more than `max_bytes` | block (guard) |
| Table not in schema memory | warn |
| Partitioned table queried without its partition column | warn |
| `SELECT *` | warn |
| Raw rows with no `LIMIT` | warn |
| Join with no condition (cartesian) | warn |
| `SELECT DISTINCT` over a join (hidden fan-out) | warn |
| `= NULL` / `<> NULL` | warn |
| `NOT IN (subquery)` (empty result if the subquery has a NULL) | warn |

**Safety model.** `run` executes only plain `SELECT` queries, whatever the config says. Postgres sessions are opened read-only with a 60-second statement timeout. BigQuery jobs carry `--maximum_bytes_billed`, so BigQuery itself refuses to bill past your limit even if an estimate was wrong. Use a read-only service account or database role anyway.

## The skills

| Skill | The agent... | ...and must prove |
| --- | --- | --- |
| [explore-warehouse](skills/explore-warehouse/SKILL.md) | maps datasets, sizes, partitions, likely joins | every table it names is in the map |
| [write-sql](skills/write-sql/SKILL.md) | answers a question with real names | guard passes; answer states grain, window, checks |
| [review-sql](skills/review-sql/SKILL.md) | reviews SQL or a PR for grain, NULL, cost bugs | each finding cites a line and a fix |
| [cost-guard](skills/cost-guard/SKILL.md) | makes expensive queries cheaper | before/after scan estimates |
| [data-quality](skills/data-quality/SKILL.md) | writes null, unique, freshness, range checks | every check reports pass/fail with counts |
| [dbt-model](skills/dbt-model/SKILL.md) | adds a dbt model with tests and docs | `dbt build` green |
| [metric-define](skills/metric-define/SKILL.md) | records a metric once, reused everywhere | two query paths agree |
| [reconcile](skills/reconcile/SKILL.md) | compares two tables or versions | every difference explained |
| [explain-pipeline](skills/explain-pipeline/SKILL.md) | draws lineage from SQL and dbt files | every node links to a file |
| [dashboard-draft](skills/dashboard-draft/SKILL.md) | drafts dashboard queries and chart specs | every query passes the guard |

Just want the rules? Copy [RULES.md](RULES.md), five lines for any agent touching data, into your `CLAUDE.md` or `AGENTS.md`.

## In CI

```yaml
- uses: actions/checkout@v4
  with: { fetch-depth: 0 }
- uses: your-org/dataskill/action@v0.1.0
  with:
    strict: "true"
```

Checks every `.sql` file changed in the pull request.

## Configuration

`dataskill.toml`, found in the current folder or any parent:

```toml
warehouse = "bigquery"      # or "postgres"
max_bytes = "10GB"          # refuse queries that would scan more
max_rows = 200              # most rows `run` returns to the agent

[bigquery]
project = "my-project"      # runs and pays for queries
datasets = ["analytics", "other-project.shared"]

[postgres]
url_env = "DATABASE_URL"    # env var with the connection URL
schemas = ["public"]
```

## Try it

[examples/postgres-shop](examples/postgres-shop) seeds a small shop database and walks through every command in two minutes.

## Roadmap

- Snowflake, Databricks and DuckDB
- Column-level checks against schema memory (today, unknown columns are caught by the dry run)
- `dataskill verify`: lint third-party skill packs for risky commands and data exfiltration
- More skills: incident triage, backfill planning, privacy review

## Contributing

New skills are the easiest way in: copy a folder in `skills/`, keep it under 80 lines, and end it with a **Done when** check the agent can actually run. `cargo test` must pass for code changes.

MIT licensed.
