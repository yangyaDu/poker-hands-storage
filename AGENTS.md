# AGENTS.md

## 项目指令
- 默认 target: `x86_64-pc-windows-msvc`，禁止 GNU target（会误用 32 位 dlltool）
- SQLite 通过 `libloading` 动态加载；Windows 可通过 `PHS_SQLITE3_LIB` 指定 `sqlite3.dll`

## 首次配置

```bash
git config core.hooksPath .githooks
```

## validate 命令

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --target x86_64-pc-windows-msvc
```
