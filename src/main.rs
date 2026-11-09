//! dataskill: a guard between coding agents and your data warehouse.

mod check;
mod config;
mod guard;
mod mcp;
mod schema;
mod warehouse;

use anyhow::{bail, Context, Result};
use check::{check_sql, Report, Severity};
use clap::{Parser, Subcommand};
use config::{human_bytes, Config, CONFIG_FILE, TEMPLATE};
use schema::Schema;
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "dataskill", version, about = "A guard between coding agents and your data warehouse")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Write a starter dataskill.toml in this folder.
    Init,
    /// Static checks on SQL text, files or stdin. No warehouse access needed.
    Check {
        /// SQL text or .sql files; reads stdin when empty or "-".
        inputs: Vec<String>,
        /// Print JSON.
        #[arg(long)]
        json: bool,
        /// Exit non-zero on warnings too (for CI).
        #[arg(long)]
        strict: bool,
        /// Override the dialect: bigquery, postgres or generic.
        #[arg(long)]
        dialect: Option<String>,
    },
    /// Checks plus a dry run: bytes scanned against the cost limit.
    Guard {
        input: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Guard, then run read-only and print at most max_rows rows as CSV.
    Run {
        input: Option<String>,
        #[arg(long)]
        max_rows: Option<usize>,
    },
    /// Build the schema memory (.dataskill/schema.json and SCHEMA.md) from the warehouse.
    Map {
        /// Import a schema.json instead of reading the warehouse.
        #[arg(long)]
        from_json: Option<String>,
    },
    /// Search table and column names in the schema memory.
    Lookup { term: String },
    /// Show one table's columns, partition column and size.
    Describe { table: String },
    /// Run as an MCP server over stdio.
    Mcp,
}

fn read_input(input: Option<&str>) -> Result<(String, String)> {
    match input {
        None | Some("-") => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            Ok(("stdin".into(), s))
        }
        Some(x) if Path::new(x).is_file() => {
            Ok((x.to_string(), std::fs::read_to_string(x).with_context(|| format!("reading {x}"))?))
        }
        Some(x) => Ok(("query".into(), x.to_string())),
    }
}

fn load_schema(cfg: &Config) -> Option<Schema> {
    match Schema::load(&cfg.schema_path()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("warning: ignoring schema memory: {e:#}");
            None
        }
    }
}

fn print_report(source: &str, r: &Report) {
    if r.findings.is_empty() {
        println!("ok    {source}: no problems found");
        return;
    }
    for f in &r.findings {
        let tag = match f.severity {
            Severity::Block => "BLOCK",
            Severity::Warn => "warn ",
            Severity::Info => "info ",
        };
        println!("{tag} {source}: [{}] {}", f.rule, f.message);
        println!("      fix: {}", f.fix);
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let cfg = Config::load()?;
    match cli.cmd {
        Cmd::Init => {
            if Path::new(CONFIG_FILE).exists() {
                bail!("{CONFIG_FILE} already exists here");
            }
            std::fs::write(CONFIG_FILE, TEMPLATE)?;
            println!("Wrote {CONFIG_FILE}. Set your warehouse and datasets, then run `dataskill map`.");
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Check { inputs, json, strict, dialect } => {
            let schema = load_schema(&cfg);
            let dialect = dialect.unwrap_or_else(|| cfg.dialect().to_string());
            let inputs = if inputs.is_empty() { vec!["-".to_string()] } else { inputs };
            let mut blocked = false;
            let mut warned = false;
            let mut all = Vec::new();
            for input in &inputs {
                let (source, sql) = read_input(Some(input))?;
                let r = check_sql(&sql, &dialect, schema.as_ref(), cfg.allow_writes);
                blocked |= r.blocked();
                warned |= r.warnings() > 0;
                if json {
                    all.push(serde_json::json!({"source": source, "report": r}));
                } else {
                    print_report(&source, &r);
                }
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&all)?);
            }
            Ok(if blocked {
                ExitCode::from(2)
            } else if strict && warned {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
        Cmd::Guard { input, json } => {
            let (source, sql) = read_input(input.as_deref())?;
            let schema = load_schema(&cfg);
            let g = guard::guard(&cfg, schema.as_ref(), &sql);
            if json {
                println!("{}", serde_json::to_string_pretty(&g)?);
            } else {
                print_report(&source, &g.check);
                println!("{}", g.summary);
            }
            Ok(if g.may_run() { ExitCode::SUCCESS } else { ExitCode::from(2) })
        }
        Cmd::Run { input, max_rows } => {
            let (_, sql) = read_input(input.as_deref())?;
            let schema = load_schema(&cfg);
            let g = guard::guard_run(&cfg, schema.as_ref(), &sql);
            if !g.may_run() {
                print_report("query", &g.check);
                eprintln!("{}", g.summary);
                return Ok(ExitCode::from(2));
            }
            let n = max_rows.map(|n| n.min(cfg.max_rows())).unwrap_or(cfg.max_rows());
            eprintln!("{}", g.summary);
            print!("{}", warehouse::run(&cfg, &sql, n)?);
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Map { from_json } => {
            let schema = match from_json {
                Some(p) => Schema::load(Path::new(&p))?.with_context(|| format!("{p} not found"))?,
                None => warehouse::map(&cfg)?,
            };
            let dir = cfg.memory_dir();
            schema.save(&dir)?;
            let cols: usize = schema.tables.iter().map(|t| t.columns.len()).sum();
            let bytes: u64 = schema.tables.iter().filter_map(|t| t.bytes).sum();
            println!(
                "Mapped {} tables, {cols} columns{} into {}",
                schema.tables.len(),
                if bytes > 0 { format!(" ({} of data)", human_bytes(bytes)) } else { String::new() },
                dir.display()
            );
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Lookup { term } => {
            let schema = load_schema(&cfg).context("no schema memory yet; run `dataskill map` first")?;
            let hits = schema.lookup(&term);
            if hits.is_empty() {
                println!("Nothing matches '{term}'.");
                return Ok(ExitCode::from(1));
            }
            for h in hits {
                println!("{h}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Describe { table } => {
            let schema = load_schema(&cfg).context("no schema memory yet; run `dataskill map` first")?;
            match schema.describe(&table) {
                Some(d) => {
                    print!("{d}");
                    Ok(ExitCode::SUCCESS)
                }
                None => {
                    println!("No table matches '{table}'. Try `dataskill lookup {table}`.");
                    Ok(ExitCode::from(1))
                }
            }
        }
        Cmd::Mcp => {
            mcp::serve(&cfg)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(3)
        }
    }
}
