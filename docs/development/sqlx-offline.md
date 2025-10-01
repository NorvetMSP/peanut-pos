# SQLx Offline Metadata Workflow

This repository uses SQLx compile-time ("offline") verification with per-query metadata files generated under each service's `.sqlx/` directory.

## Why per-query files?

SQLx 0.7+ stores one JSON file per macro query (e.g. `query!`, `query_as!`). This avoids gigantic merge conflicts and makes diffs localized to the query that changed.

## What gets generated

For every Rust macro invocation of a SQLx query in a service:

```text
services/<service>/.sqlx/query-<hash>.json
```

Each file contains:

- `query`: final SQL with `$1`, `$2`, ... placeholders
- `describe.columns`: result set column metadata (empty for commands without returning rows)
- `describe.parameters`: inferred parameter types (order corresponds to placeholders)
- `describe.nullable`: nullability for each column in the result (if any)

## Self-maintaining regeneration script

Run:

```powershell
# From repo root
powershell -NoLogo -NoProfile -File .\regenerate-sqlx-data.ps1 -Prune -Features integration
```

Key behaviors:

- Ensures database exists; can reset with `-ResetDatabase`.
- Applies migrations unless `-SkipMigrations` passed.
- Parses `Cargo.toml` per service and only applies requested features that are actually declared (avoids spurious builds).
- Generates fresh `.sqlx` metadata via `cargo sqlx prepare`.
- `-Prune` deletes prior `.sqlx` folder first, dropping stale query metadata so only current queries remain.
- Prints a per-service query count and warns on zero-query services.
- `-FailOnZero` (optional) converts zero-query warnings into a hard error to enforce macro coverage.

Merged legacy `sqlx-data.json` output was intentionally removed; the per-query files are the canonical source of truth.

## When to run the script

Run after ANY of these changes:

- You add, modify, or remove a `query!` / `query_as!` / `query_scalar!` macro.
- Database schema changes (new migrations) that affect existing queries.
- Upgrading `sqlx` or enabling new SQLx features that alter type mappings.

## Making queries compile-time validated

Runtime API example (NOT captured):

```rust
sqlx::query("UPDATE users SET name = $1 WHERE id = $2")
  .bind(name)
  .bind(id)
  .execute(&pool)
  .await?;
```

Convert to macro form:

```rust
sqlx::query!(
    "UPDATE users SET name = $1 WHERE id = $2",
    name,
    id
).execute(&pool).await?;
```

Benefits:

- Compile-time validation of SQL syntax, column names, and types.
- Offline rebuilds succeed with `SQLX_OFFLINE=1` (CI friendly).

## Zero-query services

If a service shows zero captured queries it usually means:

- All queries are using the dynamic runtime interface (`sqlx::query(...)`).
- Features that gate macro queries weren’t enabled.

Refactor stable-shape queries to macros where possible. For highly dynamic filtering logic, consider:

- Building multiple static macro variations selected by branching, or
- Using optional parameter patterns (`WHERE ($1 IS NULL OR col = $1)`) to keep a single static form.

## CI enforcement (recommended)

Add a pipeline step:

```powershell
# Windows PowerShell example
env:SQLX_OFFLINE = 1
cargo build --workspace
```

If a teammate adds a new macro query but skips regeneration, build fails with a message about missing metadata.

## Common flags

| Flag | Purpose |
|------|---------|
| `-Prune` | Drop existing `.sqlx` to eliminate stale files before regenerating |
| `-ResetDatabase` | Force drop & recreate database (dev only) |
| `-AutoResetOnChecksum` | Auto-reset if applied migration checksum mismatch detected |
| `-SkipMigrations` | Skip running migrations (faster if schema unchanged) |
| `-Features <list>` | Only enable declared feature flags per service |
| `-FailOnZero` | Treat zero captured queries as an error |

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `sqlx offline artifact not produced (missing .sqlx)` | Build failed or no macro queries compiled | Check build errors; ensure macros present and features enabled |
| Zero query count but you added macros | Conditional compilation removed them | Verify `#[cfg(...)]` conditions & feature flag inclusion |
| Types mismatch after migration | Stale metadata | Rerun script after applying migration |

## Developer workflow summary

1. Write / modify queries using `query!` family macros.
2. Run regeneration with pruning: `./regenerate-sqlx-data.ps1 -Prune` (add `-Features integration` if needed).
3. Commit updated `.sqlx/query-*.json` files.
4. Push; CI builds with `SQLX_OFFLINE=1` pass.

## FAQ

**Why remove merged `sqlx-data.json`?**  
Redundant; per-query files are authoritative and granular. Removing merged output reduces churn and avoids confusion about which file to edit.

**Can we auto-prune always?**  
You can, but optional `-Prune` is safer when diagnosing differences. Consider making it default later.

**Do we commit `.sqlx` directories?**  
Yes. They are required for offline builds.

---

Maintainer tip: If query hash files pile up, run with `-Prune` and commit the cleaned set.
