//! Project configuration, read from `dataskill.toml`.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = "dataskill.toml";
const DEFAULT_MAX_BYTES: u64 = 10 * 1024 * 1024 * 1024; // 10 GiB
const DEFAULT_MAX_ROWS: usize = 200;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// "bigquery" or "postgres". Anything else is checked with a generic SQL dialect.
    #[serde(default = "default_warehouse")]
    pub warehouse: String,
    /// Largest scan allowed before a query is refused, e.g. "10GB" or "500MB".
    #[serde(default)]
    pub max_bytes: Option<String>,
    /// Most rows `run` returns to the agent.
    #[serde(default)]
    pub max_rows: Option<usize>,
    /// Lets `check` and `guard` accept writes (e.g. reviewing migrations). `run` never executes writes.
    #[serde(default)]
    pub allow_writes: bool,
    #[serde(default)]
    pub bigquery: BigQuery,
    #[serde(default)]
    pub postgres: Postgres,
    #[serde(skip)]
    pub root: PathBuf,
    #[serde(skip)]
    pub found: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct BigQuery {
    /// Project that runs (and pays for) the queries.
    pub project: Option<String>,
    /// Datasets to map, as "dataset" or "project.dataset".
    #[serde(default)]
    pub datasets: Vec<String>,
    pub location: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Postgres {
    /// Name of the environment variable that holds the connection URL. Default DATABASE_URL.
    pub url_env: Option<String>,
    /// Schemas to map. Default ["public"].
    #[serde(default)]
    pub schemas: Vec<String>,
}

fn default_warehouse() -> String {
    "bigquery".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Config {
            warehouse: default_warehouse(),
            max_bytes: None,
            max_rows: None,
            allow_writes: false,
            bigquery: BigQuery::default(),
            postgres: Postgres::default(),
            root: PathBuf::from("."),
            found: false,
        }
    }
}

impl Config {
    /// Finds `dataskill.toml` in the current directory or a parent, or uses defaults.
    pub fn load() -> Result<Config> {
        if let Ok(p) = std::env::var("DATASKILL_CONFIG") {
            return Self::from_file(Path::new(&p));
        }
        let cwd = std::env::current_dir()?;
        let mut dir: Option<&Path> = Some(cwd.as_path());
        while let Some(d) = dir {
            let candidate = d.join(CONFIG_FILE);
            if candidate.is_file() {
                return Self::from_file(&candidate);
            }
            dir = d.parent();
        }
        Ok(Config { root: cwd, ..Config::default() })
    }

    pub fn from_file(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let mut cfg: Config =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        cfg.root = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        cfg.found = true;
        Ok(cfg)
    }

    pub fn dialect(&self) -> &'static str {
        match self.warehouse.to_ascii_lowercase().as_str() {
            "bigquery" | "bq" => "bigquery",
            "postgres" | "postgresql" | "pg" => "postgres",
            _ => "generic",
        }
    }

    pub fn max_bytes(&self) -> Result<u64> {
        match &self.max_bytes {
            None => Ok(DEFAULT_MAX_BYTES),
            Some(s) => parse_size(s),
        }
    }

    pub fn max_rows(&self) -> usize {
        self.max_rows.unwrap_or(DEFAULT_MAX_ROWS)
    }

    pub fn memory_dir(&self) -> PathBuf {
        self.root.join(".dataskill")
    }

    pub fn schema_path(&self) -> PathBuf {
        self.memory_dir().join("schema.json")
    }
}

/// Parses sizes like "10GB", "500 MB", "1TiB" or a plain number of bytes.
pub fn parse_size(s: &str) -> Result<u64> {
    let t = s.trim().to_ascii_uppercase().replace(' ', "");
    let split = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let n: f64 = num
        .parse()
        .with_context(|| format!("'{s}' is not a size such as 10GB"))?;
    let mult: f64 = match unit {
        "" | "B" => 1.0,
        "KB" | "KIB" | "K" => 1024.0,
        "MB" | "MIB" | "M" => 1024f64.powi(2),
        "GB" | "GIB" | "G" => 1024f64.powi(3),
        "TB" | "TIB" | "T" => 1024f64.powi(4),
        _ => bail!("unknown size unit in '{s}'; use B, KB, MB, GB or TB"),
    };
    Ok((n * mult) as u64)
}

pub fn human_bytes(b: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", units[i])
    }
}

pub const TEMPLATE: &str = r#"# dataskill configuration. Docs: https://github.com/<you>/dataskill

# "bigquery" or "postgres"
warehouse = "bigquery"

# Queries that would scan more than this are refused.
max_bytes = "10GB"

# Most rows `dataskill run` hands back to the agent.
max_rows = 200

# true lets `check` and `guard` accept writes, e.g. to review migration files.
# `run` and the run_query tool never execute writes, whatever this says.
allow_writes = false

[bigquery]
# Project that runs and pays for queries.
project = "my-project"
# Datasets to map into schema memory: "dataset" or "project.dataset".
datasets = ["analytics"]
# location = "US"

[postgres]
# Environment variable holding the connection URL.
url_env = "DATABASE_URL"
schemas = ["public"]
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("10GB").unwrap(), 10 * 1024 * 1024 * 1024);
        assert_eq!(parse_size("500 mb").unwrap(), 500 * 1024 * 1024);
        assert_eq!(parse_size("2048").unwrap(), 2048);
        assert!(parse_size("ten").is_err());
        assert_eq!(human_bytes(1536), "1.5 KB");
    }
}
