//! The guard: static checks, then a dry run against the cost limit. A query runs only if both pass.

use crate::check::{check_sql, is_read_only, Report};
use crate::config::{human_bytes, Config};
use crate::schema::Schema;
use crate::warehouse::{self, Estimate};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Blocked,
    OverLimit,
    DryRunFailed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Guarded {
    pub verdict: Verdict,
    pub summary: String,
    pub check: Report,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate: Option<Estimate>,
    pub limit_bytes: u64,
}

impl Guarded {
    pub fn may_run(&self) -> bool {
        self.verdict == Verdict::Pass
    }
}

pub fn guard(cfg: &Config, schema: Option<&Schema>, sql: &str) -> Guarded {
    let limit = cfg.max_bytes().unwrap_or(u64::MAX);
    let check = check_sql(sql, cfg.dialect(), schema, cfg.allow_writes);
    if check.blocked() {
        return Guarded {
            verdict: Verdict::Blocked,
            summary: "Blocked by static checks; the query was not sent to the warehouse.".into(),
            check,
            estimate: None,
            limit_bytes: limit,
        };
    }
    match warehouse::dry_run(cfg, sql) {
        Err(e) => Guarded {
            verdict: Verdict::DryRunFailed,
            // The warehouse's own message usually names the unknown column or table.
            summary: format!("Dry run failed: {e}"),
            check,
            estimate: None,
            limit_bytes: limit,
        },
        Ok(est) => {
            let bytes = est.bytes.unwrap_or(0);
            let (verdict, summary) = if bytes > limit {
                (
                    Verdict::OverLimit,
                    format!(
                        "Would scan {}, over the {} limit. Narrow it: {}",
                        human_bytes(bytes),
                        human_bytes(limit),
                        if cfg.dialect() == "bigquery" {
                            "filter on the partition column, select fewer columns, or query a smaller pre-aggregated table."
                        } else {
                            "filter on an indexed column, or query a smaller pre-aggregated table."
                        }
                    ),
                )
            } else {
                (
                    Verdict::Pass,
                    format!("OK to run: about {} ({}), under the {} limit.", human_bytes(bytes), est.note, human_bytes(limit)),
                )
            };
            Guarded { verdict, summary, check, estimate: Some(est), limit_bytes: limit }
        }
    }
}

/// Guard for execution: like `guard`, but anything other than a read-only query is refused outright.
pub fn guard_run(cfg: &Config, schema: Option<&Schema>, sql: &str) -> Guarded {
    if !is_read_only(sql, cfg.dialect()) {
        let check = check_sql(sql, cfg.dialect(), schema, false);
        return Guarded {
            verdict: Verdict::Blocked,
            summary: "dataskill only runs read-only queries. Writes are never executed, even with allow_writes; give them to a person to run.".into(),
            check,
            estimate: None,
            limit_bytes: cfg.max_bytes().unwrap_or(u64::MAX),
        };
    }
    guard(cfg, schema, sql)
}
