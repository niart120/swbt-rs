# M9 fixture / package audit: Windows

## 条件

- 日付: 2026-08-01 JST
- branch: `feat/unit-010-m9-portability-release`
- package: `swbt-rs 0.1.0`
- worktree: `cargo package --locked --list` 実行時は clean

## package file selection

`cargo package --locked --list` は 124 files を返した。root は次に限定された。

- Cargo が生成する `.cargo_vcs_info.json`、`Cargo.toml.orig`
- `Cargo.toml`、`Cargo.lock`
- `README.md`、`LICENSE`、`CHANGELOG.md`、`SECURITY.md`
- `src/`、`docs/`、`examples/`、`tests/`

`.agents/`、`.codex/`、`.github/`、`spec/`、`tools/`、`evidence/`、NDJSON、pcap/pcapng は
候補に含まれなかった。`src/bin/swbt-probe/trace.rs` は製品 source であり、実機 trace ではない。

## fixture provenance

`tests/fixtures/` には JSON 4 files がある。protocol、runtime、HID service fixture は固定
`swbt-python` 0.6.0 commit と generator path を metadata に持つ。profile fixture は次を確認した。

- format: `swbt.profile-fixtures`
- source: `niart120/swbt-python` commit `84d2723b127f70fc78e12f4496f5c40af0ccfb0a`
- generation: model-specific synthetic key material と明記
- cases: Pro、Joy-Con L、Joy-Con R の3件
- link key record: 3件、各16 bytes、各 record は単一 byte の反復値
- namespace address: fixture 全件で固定の合成 address

値自体は evidence に転記しない。実機 profile や hardware run から採取した値ではない。

## credential pattern scan

package 候補に対し、private-key header、AWS access key、GitHub token、Slack token、`sk-` credential の
代表 pattern を検査し、該当 file は0件だった。`gitleaks` は local 環境に未導入のため実行していない。
この pattern scan は全種類の秘密を証明しないため、fixture provenance と package allowlist を主な
判定根拠とする。

## 判定

- package allowlist: pass
- committed fixture provenance: pass
- actual hardware profile / trace in package: none found
- representative credential pattern scan: pass
- clean package archive: registry dependency blocker により未生成
