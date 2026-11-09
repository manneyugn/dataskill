//! A minimal MCP server over stdio (newline-delimited JSON-RPC 2.0), so any agent can call the guard.

use crate::check::check_sql;
use crate::config::Config;
use crate::guard::{guard, guard_run};
use crate::schema::Schema;
use crate::warehouse;
use anyhow::Result;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

const PROTOCOL: &str = "2025-06-18";

fn tools() -> Value {
    let sql = json!({"type": "object", "properties": {"sql": {"type": "string", "description": "One SQL query"}}, "required": ["sql"]});
    json!([
        {
            "name": "check_sql",
            "description": "Static checks on a SQL query without touching the warehouse: blocks writes, flags SELECT *, missing LIMIT, missing partition filters, unknown tables, NULL traps, cartesian joins and join fan-out. Call this on every query before running it.",
            "inputSchema": sql
        },
        {
            "name": "guard_sql",
            "description": "check_sql plus a dry run in the warehouse: returns bytes the query would scan and whether it is under the project's cost limit. A dry run also catches unknown columns.",
            "inputSchema": sql
        },
        {
            "name": "run_query",
            "description": "Runs a read-only query only if guard_sql passes, and returns at most max_rows rows as CSV. Refuses writes and queries over the cost limit.",
            "inputSchema": {"type": "object", "properties": {
                "sql": {"type": "string"},
                "max_rows": {"type": "integer", "description": "Rows to return; capped by the project setting"}
            }, "required": ["sql"]}
        },
        {
            "name": "lookup_schema",
            "description": "Searches the local schema memory for tables and columns whose names contain the term. Use it instead of guessing names.",
            "inputSchema": {"type": "object", "properties": {"term": {"type": "string"}}, "required": ["term"]}
        },
        {
            "name": "describe_table",
            "description": "Lists a table's columns, types, partition column and size from the schema memory.",
            "inputSchema": {"type": "object", "properties": {"table": {"type": "string"}}, "required": ["table"]}
        }
    ])
}

fn text(s: String, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": s}], "isError": is_error})
}

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn call(cfg: &Config, name: &str, args: &Value) -> Value {
    let schema = Schema::load(&cfg.schema_path()).ok().flatten();
    let missing = |k: &str| text(format!("Missing argument '{k}'."), true);
    match name {
        "check_sql" => match arg(args, "sql") {
            None => missing("sql"),
            Some(sql) => {
                let r = check_sql(sql, cfg.dialect(), schema.as_ref(), cfg.allow_writes);
                text(serde_json::to_string_pretty(&r).unwrap_or_default(), r.blocked())
            }
        },
        "guard_sql" => match arg(args, "sql") {
            None => missing("sql"),
            Some(sql) => {
                let g = guard(cfg, schema.as_ref(), sql);
                text(serde_json::to_string_pretty(&g).unwrap_or_default(), !g.may_run())
            }
        },
        "run_query" => match arg(args, "sql") {
            None => missing("sql"),
            Some(sql) => {
                let g = guard_run(cfg, schema.as_ref(), sql);
                if !g.may_run() {
                    return text(serde_json::to_string_pretty(&g).unwrap_or_default(), true);
                }
                let cap = cfg.max_rows();
                let n = args.get("max_rows").and_then(Value::as_u64).map(|n| (n as usize).min(cap)).unwrap_or(cap);
                match warehouse::run(cfg, sql, n) {
                    Ok(csv) => text(format!("{}\n\n{csv}", g.summary), false),
                    Err(e) => text(format!("Query failed: {e}"), true),
                }
            }
        },
        "lookup_schema" => match (arg(args, "term"), &schema) {
            (None, _) => missing("term"),
            (_, None) => text("No schema memory yet. Run `dataskill map` in the project first.".into(), true),
            (Some(t), Some(s)) => {
                let hits = s.lookup(t);
                if hits.is_empty() {
                    text(format!("Nothing matches '{t}'. Try a shorter part of the name."), false)
                } else {
                    text(hits.join("\n"), false)
                }
            }
        },
        "describe_table" => match (arg(args, "table"), &schema) {
            (None, _) => missing("table"),
            (_, None) => text("No schema memory yet. Run `dataskill map` in the project first.".into(), true),
            (Some(t), Some(s)) => match s.describe(t) {
                Some(d) => text(d, false),
                None => text(format!("No table matches '{t}'. Use lookup_schema to find it."), true),
            },
        },
        other => text(format!("Unknown tool '{other}'."), true),
    }
}

/// Handles one JSON-RPC message; returns the response, or None for notifications.
pub fn handle(cfg: &Config, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let id = id?; // notifications (no id) get no response
    let result = match method {
        "initialize" => {
            let version = msg
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL);
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "dataskill", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Use lookup_schema before writing SQL, and guard_sql or run_query before any query reaches the warehouse."
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let name = msg.pointer("/params/name").and_then(Value::as_str).unwrap_or("");
            let empty = json!({});
            let args = msg.pointer("/params/arguments").unwrap_or(&empty);
            call(cfg, name, args)
        }
        _ => {
            return Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("Method not found: {method}")}}))
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

pub fn serve(cfg: &Config) -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(cfg, &msg),
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("Parse error: {e}")}})),
        };
        if let Some(r) = response {
            writeln!(stdout, "{r}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_tools_and_checks() {
        let cfg = Config::default();
        let init = handle(&cfg, &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}})).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(handle(&cfg, &json!({"jsonrpc":"2.0","method":"notifications/initialized"})).is_none());
        let list = handle(&cfg, &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).unwrap();
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 5);
        let r = handle(&cfg, &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"check_sql","arguments":{"sql":"DROP TABLE x"}}})).unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("read-only"));
    }

    #[test]
    fn run_query_refuses_writes_before_touching_warehouse() {
        let cfg = Config::default();
        let r = handle(&cfg, &json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"run_query","arguments":{"sql":"DELETE FROM t"}}})).unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("blocked"));
    }
}
