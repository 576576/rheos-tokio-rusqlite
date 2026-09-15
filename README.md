# rheos-tokio-rusqlite

> **Fork**：[`rhei-tokio-rusqlite`](https://crates.io/crates/rhei-tokio-rusqlite) 的非官方续维护 fork ——
> 上游 <https://github.com/ValerioL29/Rhei> 已无法访问（404），源码取自 [docs.rs 发布包](https://docs.rs/crate/rhei-tokio-rusqlite/2.0.0/source/)。
> Apache-2.0，原作者 Valerio Liani，见 [LICENSE](LICENSE)。

<p align="center">
  <a href="https://github.com/576576/rheos-tokio-rusqlite/actions"><img src="https://img.shields.io/github/actions/workflow/status/576576/rheos-tokio-rusqlite/ci.yml?branch=main&label=CI" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License"></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/edition-2021-orange" alt="Edition 2021"></a>
</p>

给 `rusqlite` 套一层 async 外壳：**整条 connection 独占一个 OS 线程**，调用方通过 crossbeam 无界通道投递闭包，
结果由 `oneshot` 回传。相比 `tokio-rusqlite` 的 `spawn_blocking`（每次调用从线程池取线程），
这里把一个 `std::thread` 永久绑定到一个 `rusqlite::Connection`，省掉线程池 churn，
也保证 SQLite 的 `PRAGMA` / `BEGIN` / `COMMIT` 会话状态不会跑到别的线程上。

## 安装

```toml
[dependencies]
rheos-tokio-rusqlite = "2.0"
```

依赖 `rusqlite` 0.40（`bundled`，首次构建会用 `cc` 编译 SQLite，需要本机有 C 编译器）、`tokio` 1、`crossbeam-channel` 0.5。

## 快速开始

```rust
use rheos_tokio_rusqlite::Connection;

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
  ▼
Sender<Message>  ──crossbeam channel──►  background OS thread
                                              │ rusqlite::Connection
                                              ▼
oneshot::Receiver<R>  ◄── tokio oneshot ──  result
```

- `Connection` 可廉价 `Clone`：所有克隆共享同一 channel、同一后台线程。
- 并发调用**安全但不并行** —— 在 channel 上排队；多克隆并发时建议开启 WAL。
- 丢弃最后一个克隆即断开 channel，后台线程退出；要拿到显式关闭结果就用 `Connection::close()`。

## API

| 方法 | 说明 |
|------|------|
| `Connection::open(path)` | 打开文件库，立即派生后台线程，成功打开后才返回 |
| `Connection::open_in_memory()` | 打开私有内存库 |
| `Connection::call(f)` | 在后台线程执行 `FnOnce(&mut rusqlite::Connection) -> Result<R, Error>` 并等待结果 |
| `Connection::close()` | 显式关闭并等待后台线程确认，返回关闭期间的错误 |

闭包需 `Send + 'static`（按值捕获），且应尽量短小 —— 它持有整条 connection。

## 错误处理

| 变体 | 含义 |
|------|------|
| `Error::Rusqlite(rusqlite::Error)` | rusqlite 层错误（SQL 语法、约束冲突、I/O） |
| `Error::ConnectionClosed` | 后台线程已停止（已 `close()`，或所有发送端在派发前 drop） |
| `Error::Close(rusqlite::Error)` | `rusqlite::Connection::close` 报错 |
| `Error::Other(String)` | 闭包内自定义错误 |

`Error` 实现了 `From<rusqlite::Error>`，闭包里可直接对 rusqlite 方法用 `?`。

## 测试

```bash
cargo test    # 5 个单元测试 + 4 个文档测试
```

## License

Apache-2.0。原始代码著作权归 **Valerio Liani** 及 Rhei 贡献者所有，详见 [LICENSE](LICENSE)。
