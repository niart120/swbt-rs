# M9 quality gate: Windows

## 条件

- 日付: 2026-08-01 JST
- branch: `feat/unit-010-m9-portability-release`
- host: Windows x86_64
- package: `swbt-rs 0.1.0`
- Bumble revision: `b8c7cd625bc2ac2f58a4beb4ade1264426969819`

## 成功した command

| command | 検査対象 |
|---|---|
| `cargo fmt --all --check` | Rust source の整形 |
| `cargo +1.87.0 check --all-targets --all-features --locked` | MSRV、全 target/feature の compile |
| `cargo check --all-targets --all-features --locked` | current toolchain の compile |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | 全 target/feature の lint |
| `cargo test --all-targets --all-features --locked` | all-feature unit/integration/example test |
| `cargo test --all-targets --locked` | default feature 境界 |
| `cargo test --lib protocol:: --no-default-features --locked` | Bumble-free protocol 65 tests |
| `cargo test --doc --all-features --locked` | public example 1件 |
| `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked` | public rustdoc |
| `cargo build --all-features --locked` | all-feature package build |
| `cargo build --no-default-features --locked` | core-only package build |
| `cargo tree --no-default-features --edges normal --locked` | default graph に Bumble/rusb がないこと |
| `cargo-deny 0.20.2 --locked check` | advisory、ban、license、source policy |
| `cargo package --locked --list` | package allowlist 124 files |
| `git diff --check` | whitespace error |

all-feature library test は 300 passed / 2 ignored。`swbt-probe` 9 tests、統合 test と example testも
成功した。hardware test 5件は実 adapter と人手操作を必要とするため ignored。default feature library
test は 256 passed / 1 ignored、no-default protocol は 65 passed。

`cargo-deny` は `advisories ok, bans ok, licenses ok, sources ok`。複数版警告は
`bumble-transport` の広い依存 graph に由来し、`deny.toml` で warning としている。

## remote gate

PR #11 の initial head `627ba7eceed1a8cd7460be6450700636886244cd` に対する GitHub Actions run
`30649158739` は9 jobすべて成功した。追加した Windows job は3分10秒、dependency-policy は22秒。
fmt、MSRV、stable check、clippy、test、protocol-pure、doc も同じ head SHA で成功した。

この記録を追加した final head に対する check は merge 前に再実行し、PR 上で確認する。

## 未成功 / 未実行

- `cargo package --locked`: `bumble-controller@0.1.0` が crates.io にないため停止。
- package archive smoke: archive が生成されないため未実行。
- Linux adapter hardware: 利用可能な専用 host/adapter がなく未実行。
- Windows hardware: M3-M8 の承認済み実機証跡を再利用し、この release-docs/CI 変更では再実行しない。
- publish、tag、GitHub Release: 対象外で、当該 turn の明示承認なし。

## 判定

local source/docs/dependency gate と initial PR head の remote CI は pass。crate archive と release は
registry 配布境界で blocked。final head の remote CI は merge 前に PR 上で確認する。
