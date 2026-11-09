---
name: dbt-model
description: Add or change a dbt model with tests and documentation, and prove it builds. Use when asked to create a model, staging layer, mart, or to change an existing dbt model.
---

# dbt model

## Steps

1. Read `dbt_project.yml` and two or three existing models in the same folder. Follow their naming (`stg_`, `int_`, `fct_`, `dim_`), materialization and style exactly.
2. Find sources and upstream models with `dataskill lookup` and `ref()`/`source()`; never hard-code a fully qualified table name in a model.
3. Write the model. State the grain in a comment on the first line (`-- one row per order`).
4. Add to the model's `.yml`:
   - a description of the model and of every column you added;
   - `unique` and `not_null` tests on the grain key;
   - `relationships` tests on foreign keys, and `accepted_values` on status columns.
5. Compile and check: `dbt compile -s <model>`, then `dataskill check target/compiled/**/<model>.sql`.
6. Build: `dbt build -s <model>` (runs the model and its tests). Use a dev target; never build against production unless the user says so.
7. If the model replaces an old one, compare them with the `reconcile` skill.

## Done when

`dbt build -s <model>` passes with all tests green, and you report the row count and the grain.
