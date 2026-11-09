---
name: dashboard-draft
description: Draft the queries and chart specs for a dashboard from agreed metrics, each one checked for cost. Use when asked to build, plan or prototype a dashboard or report.
---

# Draft a dashboard

## Steps

1. Ask what decision the dashboard serves and who reads it. Pick at most 6 headline numbers and 4 charts.
2. For each number or chart, use a definition from `metrics/` (`metric-define`); define missing ones first.
3. Write one query per chart, returning exactly the rows the chart needs (already aggregated, typically under 1,000 rows).
4. `dataskill guard` every query. A dashboard query runs every time someone opens it, so keep each well under the limit; prefer pre-aggregated tables.
5. Write `dashboards/<name>.md` with, per chart: title, the question it answers, chart type (line for trends, bars for comparing categories), the query, and its estimated scan.
6. If the user has a BI tool (Looker Studio, Metabase, Superset, Grafana), offer the queries in that tool's format.

## Done when

Every chart has a query that passes `dataskill guard`, and the spec file lists the estimated scan for each.
