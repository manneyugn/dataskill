# Five rules for agents touching data

Copy these into your `CLAUDE.md`, `AGENTS.md` or `.cursor/rules`. They work with or without the rest of dataskill.

1. **Look names up; never guess them.** Every table and column you use must come from the schema (`dataskill lookup`), not from memory or from what a name "probably" is.
2. **Read-only by default.** Never run INSERT, UPDATE, DELETE, MERGE, CREATE, DROP or TRUNCATE. Write the statement out and hand it to a person.
3. **Price it before you run it.** Dry-run every query (`dataskill guard`). If it scans more than the limit, make it cheaper; never raise the limit yourself.
4. **Know the grain.** Before writing SQL, state what one row of the result is. Aggregate the many side before a join, and never use DISTINCT to hide duplicate rows.
5. **Check the answer, not just the query.** A query that runs can still be wrong. Before reporting a number, compare it with something you trust: a row count, a second query path, a known total. Say what you checked.
