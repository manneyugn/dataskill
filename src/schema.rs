//! Schema memory: a local map of tables and columns that agents look up instead of guessing.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Schema {
    pub warehouse: String,
    #[serde(default)]
    pub generated_at: String,
    pub tables: Vec<Table>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Table {
    /// Fully qualified name, e.g. "project.dataset.orders" or "public.orders".
    pub name: String,
    #[serde(default)]
    pub columns: Vec<Column>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partitioned_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Column {
    pub name: String,
    #[serde(rename = "type", default)]
    pub data_type: String,
}

/// Lower-cases a SQL name and strips backticks and double quotes.
pub fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| *c != '`' && *c != '"')
        .collect::<String>()
        .to_ascii_lowercase()
}

impl Schema {
    pub fn load(path: &Path) -> Result<Option<Schema>> {
        if !path.is_file() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let s: Schema = serde_json::from_str(&text)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(Some(s))
    }

    /// Writes `schema.json` (for tools) and `SCHEMA.md` (for agents and people).
    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join("schema.json"), serde_json::to_string_pretty(self)?)?;
        std::fs::write(dir.join("SCHEMA.md"), self.to_markdown())?;
        Ok(())
    }

    /// Finds a table by a full or trailing name: "orders", "shop.orders" or "proj.shop.orders".
    pub fn find_table(&self, name: &str) -> Option<&Table> {
        let n = normalize(name);
        self.tables
            .iter()
            .find(|t| normalize(&t.name) == n)
            .or_else(|| {
                let suffix = format!(".{n}");
                self.tables
                    .iter()
                    .find(|t| normalize(&t.name).ends_with(&suffix))
            })
    }

    /// Case-insensitive search over table and column names.
    pub fn lookup(&self, term: &str) -> Vec<String> {
        let t = normalize(term);
        let mut out = Vec::new();
        for table in &self.tables {
            let tn = normalize(&table.name);
            if tn.contains(&t) {
                let extra = table
                    .partitioned_by
                    .as_ref()
                    .map(|p| format!(", partitioned by {p}"))
                    .unwrap_or_default();
                out.push(format!(
                    "table  {}  ({} columns{extra})",
                    table.name,
                    table.columns.len()
                ));
            }
            for c in &table.columns {
                if normalize(&c.name).contains(&t) {
                    out.push(format!("column {}.{}  {}", table.name, c.name, c.data_type));
                }
            }
        }
        out
    }

    pub fn describe(&self, name: &str) -> Option<String> {
        let t = self.find_table(name)?;
        let mut s = format!("{}\n", t.name);
        if let Some(p) = &t.partitioned_by {
            s.push_str(&format!("partitioned by: {p}\n"));
        }
        if let Some(r) = t.rows {
            s.push_str(&format!("rows: {r}\n"));
        }
        if let Some(b) = t.bytes {
            s.push_str(&format!("size: {}\n", crate::config::human_bytes(b)));
        }
        for c in &t.columns {
            s.push_str(&format!("  {}  {}\n", c.name, c.data_type));
        }
        Some(s)
    }

    pub fn to_markdown(&self) -> String {
        let mut s = format!(
            "# Schema memory\n\nWarehouse: {}. Generated {} by `dataskill map`. Do not edit by hand; run `dataskill map` again instead.\n\n",
            self.warehouse, self.generated_at
        );
        for t in &self.tables {
            s.push_str(&format!("## {}\n\n", t.name));
            let mut meta = Vec::new();
            if let Some(p) = &t.partitioned_by {
                meta.push(format!("partitioned by `{p}`"));
            }
            if let Some(r) = t.rows {
                meta.push(format!("{r} rows"));
            }
            if let Some(b) = t.bytes {
                meta.push(crate::config::human_bytes(b));
            }
            if !meta.is_empty() {
                s.push_str(&format!("{}\n\n", meta.join(" · ")));
            }
            s.push_str("| Column | Type |\n| --- | --- |\n");
            for c in &t.columns {
                s.push_str(&format!("| {} | {} |\n", c.name, c.data_type));
            }
            s.push('\n');
        }
        s
    }
}

/// Current UTC time as an ISO 8601 string, without a date library.
pub fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Schema {
        Schema {
            warehouse: "bigquery".into(),
            generated_at: "x".into(),
            tables: vec![Table {
                name: "proj.shop.orders".into(),
                columns: vec![
                    Column { name: "order_id".into(), data_type: "INT64".into() },
                    Column { name: "created_at".into(), data_type: "TIMESTAMP".into() },
                ],
                partitioned_by: Some("created_at".into()),
                rows: Some(10),
                bytes: None,
            }],
        }
    }

    #[test]
    fn finds_by_suffix() {
        let s = sample();
        assert!(s.find_table("orders").is_some());
        assert!(s.find_table("`shop.orders`").is_some());
        assert!(s.find_table("PROJ.SHOP.ORDERS").is_some());
        assert!(s.find_table("hop.orders").is_none());
    }

    #[test]
    fn lookup_matches_columns() {
        let hits = sample().lookup("created");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].contains("created_at"));
    }

    #[test]
    fn iso_time_shape() {
        let t = now_iso();
        assert_eq!(t.len(), 20);
        assert!(t.ends_with('Z'));
    }
}
