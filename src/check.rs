//! Static SQL checks: read-only enforcement and the mistakes agents make most with data.

use crate::schema::{normalize, Schema};
use serde::Serialize;
use sqlparser::ast::{
    BinaryOperator, Expr, GroupByExpr, JoinConstraint, JoinOperator, ObjectName, Query, Select,
    SelectItem, SetExpr, Statement, TableFactor, Visit, Visitor,
};
use sqlparser::dialect::{BigQueryDialect, Dialect, GenericDialect, PostgreSqlDialect};
use sqlparser::parser::Parser;
use std::collections::{BTreeSet, HashSet};
use std::ops::ControlFlow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The query must not run.
    Block,
    /// Likely wrong or expensive; the agent should fix it or say why not.
    Warn,
    /// Context worth knowing.
    Info,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub rule: String,
    pub severity: Severity,
    pub message: String,
    pub fix: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
    /// Tables the query reads, as written.
    pub tables: Vec<String>,
}

impl Report {
    pub fn blocked(&self) -> bool {
        self.findings.iter().any(|f| f.severity == Severity::Block)
    }
    pub fn warnings(&self) -> usize {
        self.findings.iter().filter(|f| f.severity == Severity::Warn).count()
    }
    fn push(&mut self, severity: Severity, rule: &str, message: String, fix: &str) {
        let dup = self
            .findings
            .iter()
            .any(|f| f.rule == rule && f.message == message);
        if !dup {
            self.findings.push(Finding {
                rule: rule.to_string(),
                severity,
                message,
                fix: fix.to_string(),
            });
        }
    }
}

const AGGREGATES: &[&str] = &[
    "COUNT(", "COUNTIF(", "SUM(", "AVG(", "MIN(", "MAX(", "ANY_VALUE(", "APPROX_", "ARRAY_AGG(",
    "STRING_AGG(", "LOGICAL_AND(", "LOGICAL_OR(", "STDDEV", "VARIANCE(", "PERCENTILE_",
];

fn dialect_for(name: &str) -> Box<dyn Dialect> {
    match name {
        "postgres" => Box::new(PostgreSqlDialect {}),
        "bigquery" => Box::new(BigQueryDialect {}),
        _ => Box::new(GenericDialect {}),
    }
}

/// Checks one SQL text. `schema` enables table-name and partition checks.
pub fn check_sql(sql: &str, dialect: &str, schema: Option<&Schema>, allow_writes: bool) -> Report {
    let mut report = Report::default();
    let d = dialect_for(dialect);
    let statements = match Parser::parse_sql(d.as_ref(), sql) {
        Ok(s) => s,
        Err(e) => {
            report.push(
                Severity::Block,
                "parse-error",
                format!("The SQL could not be parsed as {dialect}: {e}"),
                "Fix the syntax, or set the right warehouse in dataskill.toml.",
            );
            return report;
        }
    };
    if statements.is_empty() {
        report.push(Severity::Block, "empty", "No SQL statement found.".into(), "Pass a query.");
        return report;
    }
    if statements.len() > 1 {
        report.push(
            Severity::Warn,
            "multiple-statements",
            format!("{} statements in one request.", statements.len()),
            "Send one query at a time so each one is checked and costed on its own.",
        );
    }

    let mut c = Collector {
        report: Report::default(),
        relations: BTreeSet::new(),
        ctes: HashSet::new(),
        idents: HashSet::new(),
        allow_writes,
        bigquery: dialect == "bigquery",
    };

    for stmt in &statements {
        match stmt {
            Statement::Query(q) => c.check_top_level(q),
            Statement::Explain { .. } => {}
            other if !allow_writes => {
                let kind = other
                    .to_string()
                    .split_whitespace()
                    .next()
                    .unwrap_or("This")
                    .to_ascii_uppercase();
                c.report.push(
                    Severity::Block,
                    "read-only",
                    format!("{kind} changes data or structure; only SELECT queries may run."),
                    "Give the statement to a person to run. (allow_writes = true only lets check and guard accept it for review; dataskill never executes writes.)",
                );
            }
            _ => {}
        }
        let _ = stmt.visit(&mut c);
    }

    // Schema checks need the full set of relations and identifiers.
    let mut tables = Vec::new();
    for rel in &c.relations {
        let bare = normalize(rel);
        if c.ctes.contains(&bare) {
            continue;
        }
        tables.push(rel.clone());
        if let Some(schema) = schema {
            match schema.find_table(rel) {
                None => c.report.push(
                    Severity::Warn,
                    "unknown-table",
                    format!("Table `{rel}` is not in the schema memory."),
                    "Look it up with `dataskill lookup <part of the name>`; run `dataskill map` if the schema changed.",
                ),
                Some(t) => {
                    if let Some(p) = &t.partitioned_by
                        && !c.idents.contains(&normalize(p)) {
                            c.report.push(
                                Severity::Warn,
                                "partition-filter",
                                format!(
                                    "`{}` is partitioned by `{p}`, but the query never uses `{p}`, so it scans the whole table.",
                                    t.name
                                ),
                                "Filter on the partition column, e.g. WHERE <partition column> >= <start date>.",
                            );
                        }
                }
            }
        }
    }
    if schema.is_none() && !tables.is_empty() {
        c.report.push(
            Severity::Info,
            "no-schema-memory",
            "No schema memory found, so table names were not verified.".into(),
            "Run `dataskill map` once to build it.",
        );
    }

    report.findings.extend(c.report.findings);
    report.tables = tables;
    report
}

/// True when every statement is a plain query. `run` executes only these, whatever `allow_writes` says.
pub fn is_read_only(sql: &str, dialect: &str) -> bool {
    let d = dialect_for(dialect);
    match Parser::parse_sql(d.as_ref(), sql) {
        Ok(stmts) => {
            !stmts.is_empty()
                && stmts.iter().all(|s| match s {
                    Statement::Query(q) => !matches!(q.body.as_ref(), SetExpr::Insert(_) | SetExpr::Update(_)),
                    _ => false,
                })
        }
        Err(_) => false,
    }
}

struct Collector {
    report: Report,
    relations: BTreeSet<String>,
    ctes: HashSet<String>,
    idents: HashSet<String>,
    allow_writes: bool,
    bigquery: bool,
}

impl Collector {
    fn check_top_level(&mut self, q: &Query) {
        if q.limit.is_some() || q.fetch.is_some() {
            return;
        }
        if let SetExpr::Select(sel) = q.body.as_ref()
            && !is_aggregated(sel) {
                let fix = if self.bigquery {
                    "Add LIMIT while exploring. LIMIT does not reduce bytes BigQuery scans; filter on the partition column for that."
                } else {
                    "Add LIMIT while exploring."
                };
                self.report.push(
                    Severity::Warn,
                    "no-limit",
                    "The query returns raw rows with no LIMIT; the result could be huge.".into(),
                    fix,
                );
            }
    }

    fn inspect_setexpr(&mut self, s: &SetExpr) {
        match s {
            SetExpr::Select(sel) => self.inspect_select(sel),
            SetExpr::SetOperation { left, right, .. } => {
                self.inspect_setexpr(left);
                self.inspect_setexpr(right);
            }
            SetExpr::Insert(_) | SetExpr::Update(_) if !self.allow_writes => self.report.push(
                Severity::Block,
                "read-only",
                "A data-modifying statement is nested inside the query.".into(),
                "Only SELECT queries may run.",
            ),
            _ => {}
        }
    }

    fn inspect_select(&mut self, sel: &Select) {
        if sel.projection.iter().any(|i| {
            matches!(i, SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_, _))
        }) {
            self.report.push(
                Severity::Warn,
                "select-star",
                if self.bigquery {
                    "SELECT * reads every column, and BigQuery bills by columns scanned.".into()
                } else {
                    "SELECT * returns every column, including ones you do not need.".into()
                },
                "List only the columns you need; `dataskill describe <table>` shows them.",
            );
        }

        let mut has_join = false;
        for twj in &sel.from {
            for j in &twj.joins {
                has_join = true;
                let plain_table = matches!(j.relation, TableFactor::Table { .. } | TableFactor::Derived { .. })
                    && !is_unnest_like(&j.relation);
                let cartesian = match &j.join_operator {
                    JoinOperator::CrossJoin => true,
                    JoinOperator::Join(c)
                    | JoinOperator::Inner(c)
                    | JoinOperator::Left(c)
                    | JoinOperator::LeftOuter(c)
                    | JoinOperator::Right(c)
                    | JoinOperator::RightOuter(c)
                    | JoinOperator::FullOuter(c) => matches!(c, JoinConstraint::None),
                    _ => false,
                };
                if cartesian && plain_table {
                    self.report.push(
                        Severity::Warn,
                        "cartesian-join",
                        format!("Join to `{}` has no join condition, so every row pairs with every row.", j.relation),
                        "Add ON <key> = <key>, or confirm a cross join is intended.",
                    );
                }
            }
        }
        if sel.from.len() > 1 {
            has_join = true;
            let extra_tables = sel.from[1..].iter().any(|t| {
                matches!(t.relation, TableFactor::Table { .. } | TableFactor::Derived { .. })
                    && !is_unnest_like(&t.relation)
            });
            if extra_tables && sel.selection.is_none() {
                self.report.push(
                    Severity::Warn,
                    "cartesian-join",
                    "Comma-separated tables with no WHERE clause form a cross join.".into(),
                    "Use explicit JOIN ... ON with the join keys.",
                );
            }
        }
        if sel.distinct.is_some() && has_join {
            self.report.push(
                Severity::Warn,
                "distinct-after-join",
                "SELECT DISTINCT over a join often hides a fan-out (duplicate rows from a one-to-many join).".into(),
                "Check the join keys and grain; aggregate the many side first instead of de-duplicating at the end.",
            );
        }
    }
}

impl Visitor for Collector {
    type Break = ();

    fn pre_visit_query(&mut self, q: &Query) -> ControlFlow<()> {
        if let Some(with) = &q.with {
            for cte in &with.cte_tables {
                self.ctes.insert(normalize(&cte.alias.name.value));
            }
        }
        self.inspect_setexpr(&q.body);
        ControlFlow::Continue(())
    }

    fn pre_visit_relation(&mut self, r: &ObjectName) -> ControlFlow<()> {
        self.relations.insert(r.to_string());
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, e: &Expr) -> ControlFlow<()> {
        match e {
            Expr::Identifier(i) => {
                self.idents.insert(normalize(&i.value));
            }
            Expr::CompoundIdentifier(parts) => {
                if let Some(last) = parts.last() {
                    self.idents.insert(normalize(&last.value));
                }
            }
            Expr::BinaryOp { left, op, right }
                if matches!(op, BinaryOperator::Eq | BinaryOperator::NotEq)
                    && (is_null(left) || is_null(right)) =>
            {
                self.report.push(
                    Severity::Warn,
                    "null-comparison",
                    format!("`{e}` is never true: comparing with NULL gives NULL."),
                    "Use IS NULL or IS NOT NULL.",
                );
            }
            Expr::InSubquery { negated: true, .. } => {
                self.report.push(
                    Severity::Warn,
                    "not-in-subquery",
                    "NOT IN (subquery) returns no rows at all if the subquery yields a NULL.".into(),
                    "Use NOT EXISTS, or filter NULLs out of the subquery.",
                );
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}

fn is_null(e: &Expr) -> bool {
    e.to_string().eq_ignore_ascii_case("NULL")
}

fn is_unnest_like(t: &TableFactor) -> bool {
    matches!(t, TableFactor::UNNEST { .. } | TableFactor::Function { .. } | TableFactor::TableFunction { .. })
        || t.to_string().to_ascii_uppercase().starts_with("UNNEST")
}

fn is_aggregated(sel: &Select) -> bool {
    let grouped = match &sel.group_by {
        GroupByExpr::All(_) => true,
        GroupByExpr::Expressions(v, _) => !v.is_empty(),
    };
    grouped
        || sel.projection.iter().any(|item| {
            let s = item.to_string().to_ascii_uppercase();
            AGGREGATES.iter().any(|a| s.contains(a))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Column, Table};

    fn rules(r: &Report) -> Vec<&str> {
        r.findings.iter().map(|f| f.rule.as_str()).collect()
    }

    fn schema() -> Schema {
        Schema {
            warehouse: "bigquery".into(),
            generated_at: "x".into(),
            tables: vec![
                Table {
                    name: "proj.shop.orders".into(),
                    columns: vec![Column { name: "order_date".into(), data_type: "DATE".into() }],
                    partitioned_by: Some("order_date".into()),
                    ..Default::default()
                },
                Table { name: "proj.shop.users".into(), ..Default::default() },
            ],
        }
    }

    #[test]
    fn blocks_writes() {
        for sql in [
            "DELETE FROM shop.orders WHERE 1=1",
            "DROP TABLE shop.orders",
            "UPDATE shop.orders SET x = 1",
            "INSERT INTO shop.orders VALUES (1)",
            "CREATE TABLE t AS SELECT 1",
        ] {
            let r = check_sql(sql, "bigquery", None, false);
            assert!(r.blocked(), "{sql} should be blocked: {:?}", rules(&r));
        }
    }

    #[test]
    fn allow_writes_lifts_block() {
        let r = check_sql("DELETE FROM t WHERE id = 1", "postgres", None, true);
        assert!(!r.blocked());
    }

    #[test]
    fn clean_aggregate_passes() {
        let sql = "SELECT user_id, COUNT(*) AS n FROM shop.orders WHERE order_date >= '2026-01-01' GROUP BY user_id";
        let r = check_sql(sql, "bigquery", Some(&schema()), false);
        assert!(r.findings.is_empty(), "{:?}", r.findings);
        assert_eq!(r.tables, vec!["shop.orders".to_string()]);
    }

    #[test]
    fn flags_common_mistakes() {
        let r = check_sql("SELECT * FROM shop.orders", "bigquery", Some(&schema()), false);
        let got = rules(&r);
        assert!(got.contains(&"select-star"));
        assert!(got.contains(&"no-limit"));
        assert!(got.contains(&"partition-filter"));
        assert!(!r.blocked());
    }

    #[test]
    fn flags_unknown_table_but_not_ctes() {
        let sql = "WITH recent AS (SELECT order_date FROM shop.orders WHERE order_date > '2026-01-01') SELECT COUNT(*) FROM recent JOIN shop.payments p ON TRUE";
        let r = check_sql(sql, "bigquery", Some(&schema()), false);
        let unknown: Vec<_> = r.findings.iter().filter(|f| f.rule == "unknown-table").collect();
        assert_eq!(unknown.len(), 1, "{:?}", r.findings);
        assert!(unknown[0].message.contains("payments"));
    }

    #[test]
    fn flags_null_traps_and_cartesian() {
        let sql = "SELECT DISTINCT o.id FROM a o JOIN b u ON o.uid = u.id CROSS JOIN c WHERE o.x = NULL AND o.id NOT IN (SELECT id FROM d) LIMIT 5";
        let got: Vec<String> = check_sql(sql, "postgres", None, false)
            .findings
            .into_iter()
            .map(|f| f.rule)
            .collect();
        for rule in ["null-comparison", "not-in-subquery", "cartesian-join", "distinct-after-join"] {
            assert!(got.iter().any(|g| g == rule), "missing {rule}: {got:?}");
        }
    }

    #[test]
    fn unnest_cross_join_is_fine() {
        let sql = "SELECT t.id, tag FROM shop.orders t CROSS JOIN UNNEST(t.tags) AS tag WHERE t.order_date > '2026-01-01' LIMIT 10";
        let r = check_sql(sql, "bigquery", Some(&schema()), false);
        assert!(!rules(&r).contains(&"cartesian-join"), "{:?}", r.findings);
    }

    #[test]
    fn read_only_detection() {
        assert!(is_read_only("WITH a AS (SELECT 1) SELECT * FROM a", "postgres"));
        assert!(!is_read_only("UPDATE t SET x = 1", "postgres"));
        assert!(!is_read_only("SELECT 1; DELETE FROM t", "postgres"));
    }

    #[test]
    fn parse_error_blocks() {
        let r = check_sql("SELEC id FROM", "bigquery", None, false);
        assert!(r.blocked());
        assert_eq!(rules(&r), vec!["parse-error"]);
    }
}
