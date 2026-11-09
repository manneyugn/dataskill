//! Talks to the warehouse through its own CLI (`bq` or `psql`), so dataskill holds no credentials.

use crate::config::Config;
use crate::schema::{now_iso, Column, Schema, Table};
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
pub struct Estimate {
    /// Bytes the query would scan (BigQuery) or an estimate from the plan (Postgres).
    pub bytes: Option<u64>,
    /// Planner's row estimate (Postgres only).
    pub rows: Option<u64>,
    /// Planner's total cost (Postgres only).
    pub cost: Option<f64>,
    pub note: String,
}

fn strip_semicolons(sql: &str) -> &str {
    sql.trim().trim_end_matches(';').trim_end()
}

fn exec(cmd: &mut Command, what: &str) -> Result<String> {
    let out = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow!("{what}: the command is not installed or not on PATH")
        } else {
            anyhow!("{what}: {e}")
        }
    })?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() {
        let msg = if stderr.trim().is_empty() { stdout } else { stderr };
        bail!("{what} failed: {}", msg.trim());
    }
    Ok(stdout)
}

// ---------- BigQuery ----------

fn bq(cfg: &Config, format: &str) -> Command {
    let mut c = Command::new(std::env::var("DATASKILL_BQ").unwrap_or_else(|_| "bq".into()));
    c.arg("--quiet").arg(format!("--format={format}"));
    if let Some(p) = &cfg.bigquery.project {
        c.arg(format!("--project_id={p}"));
    }
    if let Some(l) = &cfg.bigquery.location {
        c.arg(format!("--location={l}"));
    }
    c
}

fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => m
            .get(key)
            .or_else(|| m.values().find_map(|x| find_key(x, key))),
        Value::Array(a) => a.iter().find_map(|x| find_key(x, key)),
        _ => None,
    }
}

fn as_u64(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

pub fn parse_bq_dry_run(out: &str) -> Option<u64> {
    if let Ok(v) = serde_json::from_str::<Value>(out)
        && let Some(b) = find_key(&v, "totalBytesProcessed").and_then(as_u64) {
            return Some(b);
        }
    // Plain-text form: "... this query will process 12345 bytes of data."
    let i = out.find("will process ")?;
    out[i + 13..]
        .split_whitespace()
        .next()
        .and_then(|n| n.replace(',', "").parse().ok())
}

fn bq_dry_run(cfg: &Config, sql: &str) -> Result<Estimate> {
    let out = exec(
        bq(cfg, "json")
            .arg("query")
            .arg("--nouse_legacy_sql")
            .arg("--dry_run")
            .arg(strip_semicolons(sql)),
        "bq dry run",
    )?;
    let bytes = parse_bq_dry_run(&out)
        .ok_or_else(|| anyhow!("could not read bytes processed from bq output: {}", out.trim()))?;
    Ok(Estimate { bytes: Some(bytes), rows: None, cost: None, note: "BigQuery dry run".into() })
}

fn bq_run(cfg: &Config, sql: &str, max_rows: usize, max_bytes: u64) -> Result<String> {
    exec(
        bq(cfg, "csv")
            .arg("query")
            .arg("--nouse_legacy_sql")
            .arg(format!("--max_rows={max_rows}"))
            // Enforced by BigQuery itself: the job fails rather than bill more than this.
            .arg(format!("--maximum_bytes_billed={max_bytes}"))
            .arg(strip_semicolons(sql)),
        "bq query",
    )
}

fn bq_rows(cfg: &Config, sql: &str) -> Result<Vec<Value>> {
    let out = exec(
        bq(cfg, "json")
            .arg("query")
            .arg("--nouse_legacy_sql")
            .arg("--max_rows=1000000")
            .arg(sql),
        "bq query",
    )?;
    let t = out.trim();
    if t.is_empty() {
        return Ok(vec![]);
    }
    let v: Value = serde_json::from_str(t).context("bq returned output that is not JSON")?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

fn safe_name(s: &str) -> Result<&str> {
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || "_-.:".contains(c)) {
        bail!("'{s}' is not a valid project or dataset name");
    }
    Ok(s)
}

fn bq_map(cfg: &Config) -> Result<Schema> {
    if cfg.bigquery.datasets.is_empty() {
        bail!("list the datasets to map under [bigquery] datasets = [...] in dataskill.toml");
    }
    let mut tables: Vec<Table> = Vec::new();
    for ds in &cfg.bigquery.datasets {
        let ds = safe_name(ds)?;
        let full = if ds.contains('.') {
            ds.to_string()
        } else {
            let p = cfg
                .bigquery
                .project
                .as_deref()
                .ok_or_else(|| anyhow!("set [bigquery] project, or write datasets as project.dataset"))?;
            format!("{}.{ds}", safe_name(p)?)
        };
        let cols = bq_rows(
            cfg,
            &format!(
                "SELECT table_name, column_name, data_type, is_partitioning_column \
                 FROM `{full}.INFORMATION_SCHEMA.COLUMNS` ORDER BY table_name, ordinal_position"
            ),
        )?;
        let mut by_table: BTreeMap<String, Table> = BTreeMap::new();
        for row in cols {
            let get = |k: &str| row.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            let tname = format!("{full}.{}", get("table_name"));
            let t = by_table.entry(tname.clone()).or_insert_with(|| Table { name: tname, ..Default::default() });
            if get("is_partitioning_column") == "YES" {
                t.partitioned_by = Some(get("column_name"));
            }
            t.columns.push(Column { name: get("column_name"), data_type: get("data_type") });
        }
        // Sizes are optional: not every dataset exposes __TABLES__.
        if let Ok(sizes) = bq_rows(cfg, &format!("SELECT table_id, row_count, size_bytes FROM `{full}.__TABLES__`")) {
            for row in sizes {
                let id = row.get("table_id").and_then(Value::as_str).unwrap_or_default();
                if let Some(t) = by_table.get_mut(&format!("{full}.{id}")) {
                    t.rows = row.get("row_count").and_then(as_u64);
                    t.bytes = row.get("size_bytes").and_then(as_u64);
                }
            }
        }
        tables.extend(by_table.into_values());
    }
    Ok(Schema { warehouse: "bigquery".into(), generated_at: now_iso(), tables })
}

// ---------- Postgres ----------

fn psql(cfg: &Config) -> Result<Command> {
    let var = cfg.postgres.url_env.clone().unwrap_or_else(|| "DATABASE_URL".into());
    let url = std::env::var(&var)
        .map_err(|_| anyhow!("set the {var} environment variable to your Postgres connection URL"))?;
    let mut c = Command::new(std::env::var("DATASKILL_PSQL").unwrap_or_else(|_| "psql".into()));
    c.arg(url).args(["-X", "-q", "-v", "ON_ERROR_STOP=1"]);
    // Every session dataskill opens is read-only and time-limited, whatever the SQL says.
    c.env("PGOPTIONS", "-c default_transaction_read_only=on -c statement_timeout=60000");
    Ok(c)
}

pub fn parse_pg_explain(out: &str) -> Option<(f64, u64, u64)> {
    let v: Value = serde_json::from_str(out.trim()).ok()?;
    let plan = v.get(0)?.get("Plan")?;
    let cost = plan.get("Total Cost")?.as_f64()?;
    let rows = plan.get("Plan Rows")?.as_f64()? as u64;
    let width = plan.get("Plan Width")?.as_f64()? as u64;
    Some((cost, rows, width))
}

/// Walks a plan: full-table scans (by relation) and the estimated bytes of every other scan node.
pub fn pg_scans(plan: &Value, seq: &mut Vec<String>, other_bytes: &mut u64) {
    let node = plan.get("Node Type").and_then(Value::as_str).unwrap_or("");
    if node == "Seq Scan" {
        if let Some(rel) = plan.get("Relation Name").and_then(Value::as_str) {
            let schema = plan.get("Schema").and_then(Value::as_str).unwrap_or("public");
            seq.push(format!("{schema}.{rel}"));
        }
    } else if node.contains("Scan") {
        let rows = plan.get("Plan Rows").and_then(Value::as_f64).unwrap_or(0.0) as u64;
        let width = plan.get("Plan Width").and_then(Value::as_f64).unwrap_or(0.0) as u64;
        *other_bytes = other_bytes.saturating_add(rows.saturating_mul(width));
    }
    if let Some(children) = plan.get("Plans").and_then(Value::as_array) {
        for c in children {
            pg_scans(c, seq, other_bytes);
        }
    }
}

fn pg_dry_run(cfg: &Config, sql: &str) -> Result<Estimate> {
    let out = exec(
        psql(cfg)?.args(["-A", "-t", "-c"]).arg(format!("EXPLAIN (VERBOSE, FORMAT JSON) {}", strip_semicolons(sql))),
        "psql EXPLAIN",
    )?;
    let (cost, rows, _) =
        parse_pg_explain(&out).ok_or_else(|| anyhow!("could not read the EXPLAIN plan: {}", out.trim()))?;
    let v: Value = serde_json::from_str(out.trim())?;
    let mut seq = Vec::new();
    let mut other = 0u64;
    if let Some(plan) = v.get(0).and_then(|p| p.get("Plan")) {
        pg_scans(plan, &mut seq, &mut other);
    }
    // A full scan reads the whole table, so count its size on disk.
    let mut seq_bytes = 0u64;
    if !seq.is_empty() {
        let list = seq
            .iter()
            .map(|r| format!("'{}'", r.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");
        let sizes = exec(
            psql(cfg)?.args(["-A", "-t", "-c"]).arg(format!(
                "SELECT COALESCE(SUM(pg_total_relation_size(to_regclass(x))), 0) FROM unnest(ARRAY[{list}]) AS x"
            )),
            "psql table size",
        )?;
        seq_bytes = sizes.trim().parse().unwrap_or(0);
    }
    let note = if seq.is_empty() {
        "Postgres plan estimate; no full-table scans".to_string()
    } else {
        format!("Postgres plan; full scan of {}", seq.join(", "))
    };
    Ok(Estimate { bytes: Some(seq_bytes.saturating_add(other)), rows: Some(rows), cost: Some(cost), note })
}

fn pg_run(cfg: &Config, sql: &str, max_rows: usize) -> Result<String> {
    exec(
        psql(cfg)?
            .arg("--csv")
            .arg("-c")
            .arg(format!("SELECT * FROM (\n{}\n) AS dataskill_q LIMIT {max_rows}", strip_semicolons(sql))),
        "psql",
    )
}

fn pg_map(cfg: &Config) -> Result<Schema> {
    let schemas = if cfg.postgres.schemas.is_empty() { vec!["public".to_string()] } else { cfg.postgres.schemas.clone() };
    let list = schemas
        .iter()
        .map(|s| safe_name(s).map(|s| format!("'{s}'")))
        .collect::<Result<Vec<_>>>()?
        .join(",");
    let sql = format!(
        "SELECT c.table_schema || '.' || c.table_name AS t, c.column_name, c.data_type, \
         COALESCE(GREATEST(k.reltuples, 0), 0)::bigint AS n \
         FROM information_schema.columns c \
         LEFT JOIN pg_namespace ns ON ns.nspname = c.table_schema \
         LEFT JOIN pg_class k ON k.relname = c.table_name AND k.relnamespace = ns.oid \
         WHERE c.table_schema IN ({list}) ORDER BY 1, c.ordinal_position"
    );
    let out = exec(psql(cfg)?.args(["--csv", "-t", "-c"]).arg(sql), "psql")?;
    let mut by_table: BTreeMap<String, Table> = BTreeMap::new();
    for line in out.lines().filter(|l| !l.trim().is_empty()) {
        let f = split_csv(line);
        if f.len() < 4 {
            continue;
        }
        let t = by_table.entry(f[0].clone()).or_insert_with(|| Table { name: f[0].clone(), ..Default::default() });
        t.rows = f[3].parse().ok();
        t.columns.push(Column { name: f[1].clone(), data_type: f[2].clone() });
    }
    Ok(Schema { warehouse: "postgres".into(), generated_at: now_iso(), tables: by_table.into_values().collect() })
}

/// Minimal CSV field splitter that handles quoted fields.
pub fn split_csv(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    out.push(cur);
    out
}

// ---------- dispatch ----------

pub fn dry_run(cfg: &Config, sql: &str) -> Result<Estimate> {
    match cfg.dialect() {
        "bigquery" => bq_dry_run(cfg, sql),
        "postgres" => pg_dry_run(cfg, sql),
        other => bail!("no dry run for warehouse '{other}'; use bigquery or postgres"),
    }
}

pub fn run(cfg: &Config, sql: &str, max_rows: usize) -> Result<String> {
    match cfg.dialect() {
        "bigquery" => bq_run(cfg, sql, max_rows, cfg.max_bytes()?),
        "postgres" => pg_run(cfg, sql, max_rows),
        other => bail!("cannot run queries on warehouse '{other}'; use bigquery or postgres"),
    }
}

pub fn map(cfg: &Config) -> Result<Schema> {
    match cfg.dialect() {
        "bigquery" => bq_map(cfg),
        "postgres" => pg_map(cfg),
        other => bail!("cannot map warehouse '{other}'; use bigquery or postgres"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_bq_dry_run_json_and_text() {
        let json = r#"{"statistics":{"query":{"totalBytesProcessed":"1048576"},"totalBytesProcessed":"1048576"}}"#;
        assert_eq!(parse_bq_dry_run(json), Some(1_048_576));
        let text = "Query successfully validated. Assuming the tables are not modified, running this query will process 2048 bytes of data.";
        assert_eq!(parse_bq_dry_run(text), Some(2048));
        assert_eq!(parse_bq_dry_run("nothing"), None);
    }

    #[test]
    fn reads_pg_explain() {
        let out = r#"[{"Plan":{"Node Type":"Seq Scan","Total Cost":431.5,"Plan Rows":10000,"Plan Width":36}}]"#;
        assert_eq!(parse_pg_explain(out), Some((431.5, 10000, 36)));
    }

    #[test]
    fn finds_full_scans_in_plan() {
        let plan: Value = serde_json::from_str(r#"{"Node Type":"Hash Join","Plans":[
            {"Node Type":"Seq Scan","Relation Name":"orders","Schema":"shop","Plan Rows":50000,"Plan Width":20},
            {"Node Type":"Index Scan","Relation Name":"users","Schema":"shop","Plan Rows":10,"Plan Width":40}]}"#).unwrap();
        let (mut seq, mut other) = (Vec::new(), 0);
        pg_scans(&plan, &mut seq, &mut other);
        assert_eq!(seq, vec!["shop.orders"]);
        assert_eq!(other, 400);
    }

    #[test]
    fn csv_split() {
        assert_eq!(split_csv(r#"a,"b,c","d""e""#), vec!["a", "b,c", "d\"e"]);
    }

    #[test]
    fn rejects_bad_names() {
        assert!(safe_name("proj.ds").is_ok());
        assert!(safe_name("ds`; DROP").is_err());
    }
}
