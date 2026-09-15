# rhei-tokio-rusqlite-continue

> ## ⚠️ Fork 声明 / Fork Notice
>
> 本仓库是非官方的**续维护 fork**，上游项目为 [`rhei-tokio-rusqlite`](https://crates.io/crates/rhei-tokio-rusqlite)
> （Rhei 工作区中的子 crate `crates/rhei-tokio-rusqlite`），对应版本 **v2.0.0**。
>
> **为什么不是常规 fork：** 上游仓库 <https://github.com/ValerioL29/Rhei> 当前**无法访问**（HTTP 404，
> 验证时间 `2026-09-15T15:13:38.873Z` / 北京时间 2026-09-15 23:13:38 UTC+8），GitHub 的 fork 入口已不可用，
> 因此本仓库以复制源码的方式重建，而非通过 GitHub fork。
>
> **代码来源：** docs.rs 构建该版本文档所使用的发布包 ——
> <https://docs.rs/crate/rhei-tokio-rusqlite/2.0.0/source/>，
> 对应上游 commit [`d473689c50e718ce60097777773df045cfd1cc04`](https://github.com/ValerioL29/Rhei/tree/d473689c50e718ce60097777773df045cfd1cc04)，
> VCS 路径 `crates/rhei-tokio-rusqlite`。`src/lib.rs`、`src/error.rs` 未作任何功能性改动。
>
> **归属与许可：** 原始代码的著作权归原作者 **Valerio Liani** 及 Rhei 贡献者所有，以 **Apache-2.0** 许可发布
> （见 [LICENSE](LICENSE)）。本仓库不主张对原作品的所有权，仅作存档与继续维护之用；
> 上游恢复访问后，欢迎合并、重定向或归档本仓库。
>
> **本仓库相对上游的唯一改动：** 为让 crate 脱离原工作区独立构建，在 `Cargo.toml` 中补回了
> `[workspace]` / `[workspace.package]` / `[workspace.dependencies]` 表（上游位于工作区内，这些字段由根 `Cargo.toml` 提供），
> 并把 `repository` / `homepage` 指向本仓库。库代码本身零改动。
>
> 以下为原 README 原文（内容面向 Rhei 整个工作区），未作删改。

---

<p align="center">
  <img src="assets/full-white.png" alt="Rhei" width="320">
</p>

<p align="center">
  <em>Lightweight, serverless Hybrid Transactional/Analytical Processing in Rust</em>
</p>

<p align="center">
  <a href="https://github.com/ValerioL29/Rhei/actions"><img src="https://img.shields.io/github/actions/workflow/status/ValerioL29/Rhei/ci.yml?branch=main&label=CI" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License"></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-1.91%2B-orange" alt="Rust"></a>
  <a href="https://arrow.apache.org"><img src="https://img.shields.io/badge/Arrow-58-green" alt="Arrow"></a>
</p>

<p align="center">
  <a href="README_CN.md">中文</a>
</p>

---

Rhei pairs **Rusqlite** (OLTP) with a pluggable OLAP layer (**DataFusion** or **DuckDB**), connected by trigger-based CDC replication and automatic SQL query routing. It can also operate as a **sidecar DBMS**, following an external database (SQLite, PostgreSQL) via timestamp-based CDC and maintaining temporal (SCD Type 2) history for point-in-time queries.

**Highlights:**
- Zero-config HTAP: writes go to OLTP, analytical queries auto-route to OLAP
- CDC-powered sync with background replication and pruning
- Sidecar mode: attach to any SQLite/PostgreSQL as a read-only analytical mirror
- Arrow Flight SQL server for network-accessible OLAP queries (ADBC/JDBC/DBeaver)
- Python bindings via PyO3 (fully async, asyncio + anyio)
- CLI tool (`rh`) with interactive REPL, live TUI dashboard, and headless service mode
- Docker-ready: multi-stage Dockerfile, 66MB optimized binary

## Performance

<table>
<tr><td>

**Rust API** (release mode, LTO)

| Operation | DataFusion | DuckDB |
|-----------|-----------|--------|
| Insert 700 rows | 10,267/s | 8,797/s |
| CDC sync | 141,326 evt/s | 43,579 evt/s |
| CRUD 225 ops | 12,072/s | 13,998/s |
| Concurrent 8x50 | 10,654/s | 14,779/s |

</td><td>

**Python API** (asyncio via PyO3)

| Operation | Throughput |
|-----------|-----------|
| Insert 500 rows | 2,491/s |
| CDC sync | 32,142 evt/s |
| Batch 400 rows | 30,733/s |

</td></tr>
<tr><td>

**Sidecar** (DataFusion temporal, InMemory baseline)

| Metric | Value |
|--------|-------|
| INSERT sync p50 (10 rows/cycle) | 228us |
| INSERT sync p99 (10 rows/cycle) | 943us |
| Initial sync @ 10K rows | 488K evt/s |
| Initial sync @ 100K rows | 466K evt/s |

</td><td>

**FlightSQL** (100K rows, streaming + zstd)

| Query type | Local | Network | Overhead |
|------------|-------|---------|----------|
| Full scan | 1,713 q/s | 67 q/s | 26x |
| Filtered | 466 q/s | 181 q/s | 2.6x |
| GROUP BY | 261 q/s | 23 q/s | 11x |

</td></tr>
</table>

**Vortex storage cross-mode** — the durability dial (v2.0, 50-cycle steady-state sync, 10 rows/cycle):

| Storage mode | sync p50 | sync p99 | Initial @ 100K |
|---|---:|---:|---:|
| `InMemory` (volatile) | 308 µs | 699 µs | 387K rows/s |
| `Vortex` local (durable) | 5.5 ms | 7.2 ms | 473K rows/s |
| `Vortex` S3-compatible (durable + distributed) | 209 ms | 437 ms | 186K rows/s |

See [`E2E_TEST_REPORT.md`](E2E_TEST_REPORT.md) for the full v2.0 methodology and the Postgres-source numbers.

## Architecture

```
                              Standard HTAP Mode
                              ==================

Client ──> HtapEngine (facade)
              |
              |-- SqlParserRouter ──> AST-based routing (OLTP or OLAP)
              |
              |-- RusqliteEngine (OLTP)
              |     |-- Write connection (dedicated thread + crossbeam channel)
              |     |-- Read pool (round-robin, WAL mode)
              |     +-- CDC triggers ──> _rhei_cdc_log (json_array)
              |
              |-- OlapBackend
              |     |-- DataFusion (pure Rust, async, InMemory / Vortex { url })
              |     +-- DuckDB (C++ via FFI, MVCC concurrent reads)
              |
              +-- CdcSyncEngine
                    |-- Background sync loop (configurable interval)
                    |-- Batch INSERT grouping + CDC pruning
                    +-- Destructive (mirror) or Temporal (SCD Type 2)


                              Sidecar Mode
                              ============

External DB (SQLite / PostgreSQL)
  |  polls by updated_at > watermark
  v
TimestampCdcConsumer ──> CdcSyncEngine ──> OlapBackend
  |                                           |
  +-- INSERT/UPDATE/DELETE heuristics         +-- Point-in-time queries
  +-- Soft-delete detection                       via _rhei_valid_from/_rhei_valid_to
```

## Quick Start

### Rust

```rust
use std::sync::Arc;
use arrow::datatypes::{DataType, Field, Schema};
use rhei::{HtapConfig, HtapEngine, TableSchema};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let engine = HtapEngine::new(HtapConfig {
        oltp_path: "my.db".to_string(),
        ..Default::default()
    }).await?;

    engine.execute("CREATE TABLE orders (id INTEGER PRIMARY KEY, total INTEGER)", &[]).await?;
    engine.register_table(TableSchema::new(
        "orders",
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("total", DataType::Int64, true),
        ])),
        vec!["id".to_string()],
    )).await?;

    engine.execute("INSERT INTO orders VALUES (1, 99)", &[]).await?;
    engine.sync_now().await?;

    // Analytical query auto-routes to OLAP
    let batches = engine.query("SELECT SUM(total) FROM orders").await?;
    Ok(())
}
```

### Python

```python
import anyio
import rhei

async def main():
    async with await rhei.open(oltp_path="my.db") as engine:
        await engine.execute("CREATE TABLE orders (id INTEGER PRIMARY KEY, total INTEGER)")
        await engine.register_table(
            rhei.TableSchema("orders", [("id", "int64"), ("total", "int64")], ["id"])
        )
        await engine.execute("INSERT INTO orders VALUES (1, 99)")
        await engine.sync_now()
        batches = await engine.query("SELECT SUM(total) FROM orders")

anyio.run(main)
```

### CLI (`rh`)

```bash
cargo build -p rhei-tui --release   # binary: rh

rh exec --db my.db "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)"
rh exec --db my.db "INSERT INTO users VALUES (1, 'Alice')"
rh query --db my.db "SELECT COUNT(*) FROM users"    # OLAP read-only
rh repl --db my.db                                   # interactive REPL
rh dashboard                                          # live TUI
```

### Docker

```bash
docker build -t rhei .
docker run --rm rhei info
docker run --rm -v ./config/htap.toml:/etc/rhei/config.toml rhei
```

### Service Mode

```bash
rh serve --config config/htap.toml             # TUI dashboard + background sync
rh serve --config config/htap.toml --headless  # stdout logging (Docker/systemd)
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `datafusion-backend` | Yes | DataFusion OLAP engine (pure Rust, async) |
| `duckdb-backend` | No | DuckDB OLAP engine (C++, bundled build) |
| `full` | No | Both OLAP backends |
| `sidecar` | No | Timestamp-based CDC from external DBs (SQLite + PostgreSQL), incl. RocksDB-persistent watermarks |
| `flight-sql` | No | Arrow Flight SQL gRPC server for OLAP queries (bearer-token auth optional) |
| `rocksdb-cdc` | No | RocksDB-backed CDC log / bridge (5-7× faster writes, crash-durable trigger → sync pipeline) |
| `metrics` | No | Counters/gauges via the `metrics` crate facade |
| `metrics-exporter` | No | Prometheus HTTP metrics endpoint on `rhei-tui` |
| `cloud-storage` | No | S3-compatible object-store backend for DataFusion Vortex storage (`s3://` URL scheme). Local Vortex paths work without this feature. GCS is not supported in v2.0 — use an S3-compatibility shim (e.g. MinIO over `gcsfuse`) and set `AWS_ENDPOINT_URL`. |

```bash
cargo build                                        # DataFusion only (default, smallest)
cargo build --features duckdb-backend              # + DuckDB
cargo build --features sidecar                     # + sidecar (SQLite + PostgreSQL)
cargo build --features flight-sql                  # + Arrow Flight SQL server
cargo build --features "full,sidecar,flight-sql"   # everything
```

## API Reference

| Method | Description |
|--------|-------------|
| `HtapEngine::new(config)` | Create engine (auto-starts sync if `sync_interval` set) |
| `execute(sql, params)` | Write to OLTP (errors in pure sidecar mode) |
| `execute_batch(stmts)` | Multi-statement transaction (much faster) |
| `query(sql)` | Auto-routed: OLTP for point reads, OLAP for analytics |
| `query_with_hint(sql, hint)` | Force OLTP or OLAP routing |
| `register_table(schema)` | Register for replication (CDC triggers + OLAP mirror) |
| `sync_now()` | Single CDC sync cycle |
| `start_sync(interval)` / `stop_sync()` | Background sync loop |
| `initial_sync(table)` / `initial_sync_all()` | Bulk-load OLTP rows to OLAP |
| `add_column(table, col, type)` | Schema evolution: registry + OLAP + triggers |
| `drop_column(table, col)` | Schema evolution: teardown triggers + ALTER |
| `sync_status()` | CDC lag, last synced seq, running state |
| `oltp()` / `olap()` | Direct engine access |

## Sidecar Mode

Attach to an external database as a read-only analytical mirror with full temporal history:

```rust
let config = HtapConfig {
    sync_mode: SyncMode::Temporal,
    sidecar: Some(SidecarConfig {
        source_path: "external.db".to_string(),
        enable_local_oltp: false,
        timestamp_config: TimestampCdcConfig { /* ... */ },
    }),
    ..Default::default()
};
let engine = HtapEngine::new(config).await?;

// Time-travel query: "What was order 42 at time T?"
let batches = engine.query(
    "SELECT * FROM orders
     WHERE id = 42
       AND _rhei_valid_from <= 1700000000
       AND (_rhei_valid_to IS NULL OR _rhei_valid_to > 1700000000)"
).await?;
```

## Arrow Flight SQL

With `--features flight-sql`, Rhei exposes OLAP queries over gRPC using the [Arrow Flight SQL](https://arrow.apache.org/docs/format/FlightSql.html) protocol. Queries stream directly from the OLAP engine (via `query_stream()`) with zstd compression — no buffering of full results in memory.

```toml
# config/htap.toml
[engine]
flight_port = 50051
```

```python
# Python client (ADBC)
import adbc_driver_flightsql.dbapi as flight_sql

conn = flight_sql.connect("grpc://localhost:50051")
cursor = conn.cursor()
cursor.execute("SELECT category, SUM(amount) FROM orders GROUP BY category")
table = cursor.fetch_arrow_table()
```

Compatible with: Python (`adbc_driver_flightsql`), Java (Arrow Flight JDBC), DBeaver, Go (`adbc/driver/flightsql`).

Features: streaming execution, zstd/lz4 compression, deferred query execution (no double-execute), read-only (OLAP only).

## Testing

```bash
cargo test --workspace                                  # DataFusion + unit tests
cargo test --workspace --all-features                   # All 251 Rust tests (v2.0)
cargo test -p rhei-flight --all-features                # 22 FlightSQL tests (inc. bearer-token auth)
uv run pytest -v                                        # 54 Python tests (v2.0)
RHEI_TEST_FLIGHT=1 uv run pytest python/tests/test_flight_sql.py -v  # FlightSQL Python tests (gated)
```

See [E2E_TESTING_SOP.md](E2E_TESTING_SOP.md) for the full testing procedure and [E2E_TEST_REPORT.md](E2E_TEST_REPORT.md) for the latest v2.0 numbers.

## Workspace Crates

| Crate | Description |
|-------|-------------|
| `rhei` | Facade: HtapEngine, HtapConfig, integration tests |
| `rhei-core` | Traits (OlapEngine, OltpEngine, CdcConsumer), types, SchemaRegistry |
| `rhei-tokio-rusqlite` | Async rusqlite wrapper (dedicated thread + crossbeam) |
| `rhei-oltp-rusqlite` | Rusqlite OLTP: write conn + read pool + CDC producer |
| `rhei-olap` | Backend-agnostic OLAP dispatcher (OlapBackend enum) |
| `rhei-datafusion` | DataFusion engine (InMemory / Vortex { url } — local + S3-compatible) |
| `rhei-duckdb` | DuckDB engine (write conn + read pool, MVCC) |
| `rhei-sync` | CdcSyncEngine, SqlParserRouter, CDC-to-DML converter |
| `rhei-cdc-rocksdb` | RocksDB-backed CDC log (durable, 5-7x faster) |
| `rhei-sidecar` | TimestampCdcConsumer (SQLite + PostgreSQL) |
| `rhei-flight` | Arrow Flight SQL gRPC server (streaming + zstd) |
| `rhei-tui` | CLI tool (`rh`) + TUI dashboard |

## Citation

If you use Rhei in your research, please cite it as:

```bibtex
@software{rhei2025,
  author       = {Valerio Liani},
  title        = {{Rhei}: Lightweight Serverless Hybrid Transactional/Analytical Processing in Rust},
  year         = {2025},
  url          = {https://github.com/ValerioL29/Rhei},
  note         = {Apache-2.0 License},
}
```

## License

Apache-2.0
