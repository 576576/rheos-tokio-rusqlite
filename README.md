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
> VCS 路径 `crates/rhei-tokio-rusqlite`。
>
> **相对上游的改动（仅此三项）：**
>
> 1. **改名**：package 名 `rhei-tokio-rusqlite` → `rhei-tokio-rusqlite-continue`（避免与 crates.io 上已存在的上游版本撞名）。
>    lib 名仍为 `rhei_tokio_rusqlite`，因此是 **drop-in 替换**，下游代码里的 `use rhei_tokio_rusqlite::…` 不用动。
> 2. **脱离原工作区**：补回 `[workspace]` / `[workspace.package]` / `[workspace.dependencies]` 表
>    （上游位于 Rhei 工作区内，这些字段原本由根 `Cargo.toml` 提供），并把 `repository` / `homepage` 指向本仓库。
> 3. **依赖升级**：`rusqlite` 0.39 → **0.40**，并更新 `Cargo.lock`（tokio 1.50 → 1.53，crossbeam-channel 0.5.15 → 0.5.17）。
>    `src/lib.rs`、`src/error.rs` **未作任何改动**，5 个单元测试 + 4 个文档测试全部通过。
>
> **归属与许可：** 原始代码的著作权归原作者 **Valerio Liani** 及 Rhei 贡献者所有，以 **Apache-2.0** 许可发布
> （见 [LICENSE](LICENSE)）。本仓库不主张对原作品的所有权，仅作存档与继续维护之用；
> 上游恢复访问后，欢迎合并、重定向或归档本仓库。

---

<p align="center">
  <em>Async rusqlite wrapper using a dedicated OS thread + crossbeam channel</em>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License"></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/edition-2021-orange" alt="Edition 2021"></a>
  <a href="https://docs.rs/rusqlite"><img src="https://img.shields.io/badge/rusqlite-0.40-blue" alt="rusqlite 0.40"></a>
</p>

> **关于本文**：上游 crate 携带的 README 其实是 **Rhei 整个工作区**的说明（HTAP 引擎、DataFusion/DuckDB、
> Python 绑定、CLI、Arrow Flight SQL……），与本 crate 无关，且其中引用的 `assets/full-white.png`、`README_CN.md`、
> `E2E_TEST_REPORT.md` 等资源并不随 crate 发布（链接全部失效）。本 README 已重写为**只描述本 crate**，
> 需要了解上游整体项目请看 <https://docs.rs/crate/rhei-tokio-rusqlite/2.0.0/source/> 中的原始文件。

## 这是什么

`rusqlite` 是同步 API —— 每次调用都会阻塞当前线程。直接在 Tokio 任务里跑会卡住 async executor；
常规解法 `spawn_blocking` 则是每次调用都从线程池取一个线程，有额外的调度开销。

本 crate 换了个思路：**为整条 connection 独占一个 OS 线程**。调用方通过 `Sender<Message>`
（crossbeam 无界通道）投递闭包，结果经 `oneshot::Receiver<T>` 回传。因为线程是长期存活的，
既没有每次调用的建线程成本，`rusqlite::Connection` 也永远不会跨线程移动。

## 安装

```toml
[dependencies]
rhei-tokio-rusqlite-continue = "2.0"
```

**从上游迁移（drop-in）：**

```toml
# 之前
rhei-tokio-rusqlite = "2.0"
# 之后
rhei-tokio-rusqlite-continue = "2.0"
```

代码无需改动 —— 本 fork 保留了上游的 lib 名 `rhei_tokio_rusqlite`，所以 `use rhei_tokio_rusqlite::Connection;`
照旧可用。

## 快速开始

```rust
use rhei_tokio_rusqlite::Connection;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::open_in_memory().await?;

    // DDL + DML 放在同一个闭包里：connection 不可重入，
    // 事务横跨多个 call() 会死锁。
    conn.call(|c| {
        c.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)", [])?;
        c.execute("INSERT INTO users (name) VALUES (?1)", ["Alice"])?;
        c.execute("INSERT INTO users (name) VALUES (?1)", ["Bob"])?;
        Ok(())
    }).await?;

    let count: i64 = conn.call(|c| {
        c.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .map_err(Into::into)
    }).await?;

    assert_eq!(count, 2);

    conn.close().await?;
    Ok(())
}
```

## 线程模型

```text
async caller
  │  conn.call(|c| { … })
  │
  ▼
Sender<Message>  ──crossbeam channel──►  background OS thread
                                              │ rusqlite::Connection
                                              │ executes closure
                                              ▼
oneshot::Receiver<R>  ◄── tokio oneshot ──  result
```

- `Connection` 是廉价可 `Clone` 的句柄：所有克隆共享同一个 channel、同一个后台线程。
- 因此并发调用是**安全的，但不是并行的** —— 它们会在 channel 上排队。
  考虑到 SQLite 自身的串行化语义，这通常正合适；多克隆并发时建议开启 WAL。
- 丢弃最后一个克隆即断开 channel，后台线程退出事件循环，`rusqlite::Connection` 被干净地 drop。
  需要拿到显式的关闭结果（以及关闭错误）时，改用 `Connection::close()`。

## 与 `tokio-rusqlite` 的差异

[`tokio-rusqlite`](https://docs.rs/tokio-rusqlite) 使用 `spawn_blocking`（每次调用从线程池取一个线程）。
本 crate 把一个 `std::thread` 永久绑定到一个 `rusqlite::Connection`，对于大量短小连续调用的场景更划算
（没有线程池churn），并且保证 SQLite 的 `PRAGMA` / `BEGIN` / `COMMIT` 会话状态不会在别的线程上被观察到。

## API

| 方法 | 说明 |
|------|------|
| `Connection::open(path)` | 打开文件型数据库，立即派生后台线程；仅在成功打开（或返回错误）后返回 |
| `Connection::open_in_memory()` | 打开私有内存库，适合测试与临时数据 |
| `Connection::call(f)` | 在后台线程上执行闭包 `FnOnce(&mut rusqlite::Connection) -> Result<R, Error>` 并等待结果 |
| `Connection::close()` | 显式关闭并等待后台线程确认，返回关闭期间的错误 |

闭包必须是 `Send + 'static`（按值捕获，用 `move |c| { … }`），且应尽量短小 —— 它持有整条 connection。

## 错误处理

所有可能失败的操作都返回 `Error`：

| 变体 | 含义 |
|------|------|
| `Error::Rusqlite(rusqlite::Error)` | rusqlite 层错误（SQL 语法、约束冲突、I/O 等），最常见 |
| `Error::ConnectionClosed` | 后台线程已停止（调用过 `close()`，或所有发送端在派发前已 drop） |
| `Error::Close(rusqlite::Error)` | `rusqlite::Connection::close` 在显式关闭时报错 |
| `Error::Other(String)` | 闭包内自定义的错误信息 |

`Error` 实现了 `From<rusqlite::Error>`，所以传给 `call` 的闭包里可以直接对 rusqlite 方法用 `?`。

## 依赖

| Crate | 版本 | 说明 |
|-------|------|------|
| `rusqlite` | `0.40`（feature `bundled`） | 编译并静态链接 SQLite，`libsqlite3-sys` 0.38.2 |
| `tokio` | `1`（feature `full`） | 仅用到 `sync::oneshot`；`full` 是上游设定，实际可按需收窄 |
| `crossbeam-channel` | `0.5` | 无界通道，用于向后台线程投递闭包 |

## 测试

```bash
cargo test          # 5 个单元测试 + 4 个文档测试
```

`rusqlite` 的 `bundled` feature 会在首次构建时用 `cc` 编译 SQLite，需要本机有 C 编译器。

## License

Apache-2.0。原始代码著作权归 **Valerio Liani**（<https://github.com/ValerioL29/Rhei>）及 Rhei 贡献者所有，
详见 [LICENSE](LICENSE)。
