# M9 package audit: Windows

## 条件

- 日付: 2026-08-01 JST
- branch: `feat/unit-010-m9-portability-release`
- package: `swbt-rs 0.1.0`
- Bumble revision: `b8c7cd625bc2ac2f58a4beb4ade1264426969819`
- worktree: 各 `cargo package` command の実行時は clean

## baseline

`Cargo.toml` に `include` と Git dependency の version requirement がない状態で確認した。

| command | result |
|---|---|
| `cargo package --locked --list` | success。ただし `.agents/`、`.codex/`、`.github/`、`spec/`、実機 trace、開発 tool を候補に含んだ |
| `cargo package --locked` | failed。`bumble` dependency に version requirement がなく、manifest 検査で停止した |

## package file selection

`Cargo.toml` の `include` を allowlist とし、次を対象にした。

- `src/**`
- `examples/**`
- `tests/**`
- `Cargo.toml` と `Cargo.lock`
- `README.md`、`LICENSE`、`CHANGELOG.md`、`SECURITY.md`

release 文書まで commit した clean worktree で `cargo package --locked --list` を実行し、124 files を得た。
Cargo が生成する `.cargo_vcs_info.json` と正規化前 manifest の `Cargo.toml.orig` を含む。
`.agents/`、`.codex/`、`.github/`、`spec/`、`tools/` と実機 trace は含まない。

## dependency normalization

固定 fork workspace の対象 8 crate は version `0.1.0` である。Git URL と revision を維持したまま
exact requirement `=0.1.0` を追加した。これは Cargo が package manifest から Git source を除去し、
registry dependency へ正規化するために必要な metadata である。

clean worktree の `cargo package --locked` は packaging を開始した後、次の registry 解決で停止した。

```text
no matching package named `bumble-controller` found
location searched: crates.io index
required by package `swbt-rs v0.1.0`
```

2026-08-01 に `cargo info <name>@0.1.0 --registry crates-io` で確認した範囲では、
`bumble-controller`、`bumble-transport`、`bumble-hci` は crates.io に存在しない。
`bumble@0.1.0` は存在するが repository が `google/bumble` の別 crate であり、固定 fork workspace の
同名 crate を置き換えない。したがって不足 subcrate の公開だけでは解決せず、crate 名または backend
の配布境界を再設計する必要がある。

## 判定

- package file selection: pass
- Git dependency version metadata: pass
- verified package archive: blocked by registry name collision and unpublished dependencies
- `publish = false`: 維持
- `cargo package --no-verify`: not run。archive smoke と registry dependency 解決を検証しないため、
  release gate の代替にしない
