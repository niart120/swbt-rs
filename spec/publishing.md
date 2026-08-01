# 0.1.0 公開手順

- 状態: **公開停止中**
- candidate: `0.1.0`
- 最終監査日: 2026-08-02 (JST)

この文書は release candidate の再現、停止条件、検査、rollback を一か所にまとめる。production tag、
GitHub Release、`cargo publish`、publish workflow の実行には、その turn でのユーザの明示承認が必要。

## 解消済みの依存条件

- standalone public repository
  [`niart120/swbt-bumble-backend`](https://github.com/niart120/swbt-bumble-backend) の
  `main@0a4a2d99bc3ed3807464d4f902c20d9fd16b188a` から、
  [`swbt-bumble-backend@0.1.1`](https://crates.io/crates/swbt-bumble-backend/0.1.1) を公開した。
  crates.io owner は `niart120`、archive checksum は
  `1cc2c8d7d9c8cecfd203cd039fb3c3f8a9c39b072230f977b1e12e526b1bc667` である。
- `swbt-rs` は一時 path patch と Bumble Git dependency を残さず、registry 上の
  `swbt-bumble-backend = "=0.1.1"` を解決する。
- `cargo package --locked --allow-dirty` は120 files / 1.4 MiB（圧縮258.0 KiB）のarchiveを生成した。
  展開archiveからの `cargo +1.87.0 test --all-targets --all-features --locked --offline --quiet` は
  library 271 passed / 1 ignored、hardware 5 ignored、他target successだった。

## 現在の停止条件

0.1.0 は次の全条件を解消するまで公開しない。

1. `Cargo.toml` の `publish = false` を維持している。
2. unit_012 T08 の実機回帰は完了したが、公開対象となる `swbt-rs` 0.1.0 の exact head、merge SHA、
   remote checkをまだ固定していない。
3. backend切り替え後のlicense判定とWindows/Linux SBOMをrelease candidateで再生成していない。
4. 2026-08-01 の GitHub API で `niart120/swbt-rs` の Private Vulnerability Reporting は
   `enabled:false` であり、
   `SECURITY.md` に恒久的な非公開報告先を記載できていない。

fork 元 `chaitanyarahalkar/bumble-rs` への issue/PR はこの手順に含めない。自己所有 fork の
[Issue #1](https://github.com/niart120/bumble-rs/issues/1) だけでbackend切り出しを追跡する。

## candidate の固定

release candidate では次を同じ記録へ残す。

- `swbt-rs` の merge 後 commit SHA
- `Cargo.lock` の SHA-256
- `swbt-bumble-backend` version、registry checksum、standalone repository commit
- Bumble source lineage の fork revision `cb55e2d98dc7b7b0227c43772c9ae184034dd9a1`
- Cargo package version と Rust MSRV
- Windows/Linux の依存一覧、license 判定、CycloneDX SBOM hash
- 実機確認した OS、adapter、driver、console version と未検証条件

現在の `Cargo.lock` SHA-256 は
`861E05B96E9A3E4725A482221B95FB36D288F20515A02F1589BF118C61EB9D2C`。release commit は merge 前に
確定できないため、実際の release 承認後に main の対象 SHA とともに記録する。

## local gate

clean checkout で次を実行する。

```powershell
cargo fmt --all --check
cargo +1.87.0 check --all-targets --all-features --locked
cargo check --all-targets --all-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
cargo test --all-targets --locked
cargo test --lib protocol:: --no-default-features --locked
cargo test --doc --all-features --locked
cargo build --all-features --locked
cargo build --no-default-features --locked
cargo deny --locked check
cargo package --locked --list
cargo package --locked
git diff --check
```

`cargo package --locked --list` では `src/`、`docs/`、`examples/`、`tests/` と公開 root file だけを
含むことを確認する。`.agents/`、`.codex/`、`.github/`、`spec/`、`tools/`、実機 trace、raw profile、
秘密鍵を含めない。

生成した `.crate` を空の一時 directory へ展開し、default/all-feature build、test、examples を
archive 内から実行する。repository checkout の未収録 file を参照していないことを確認する。

## remote gate

release candidate PR で Linux、Windows、MSRV、dependency-policy の全 job が同じ head SHA に対して
成功していることを確認する。CI build は Linux hardware evidence と扱わない。PR は repository の
通常の merge/cleanup 手順で main へ取り込み、main を同期してから release candidate SHA を固定する。

## dependency、license、SBOM

`deny.toml` は advisory、license、source の正本である。未知 license、許可していない Git source、
advisory ignore を残した状態で公開しない。Windows/Linux の all-feature graph から CycloneDX 1.5 JSON
を再生成し、component 数、license 欠落数、SHA-256 を記録する。SBOM に local path、token、profile、
key material がないことを検査する。

## hardware と既知の制限

公開可否は [対応環境と USB adapter](../docs/platform-support.md) の matrix に従う。Windows の限定構成
以外を実機確認済みと表記しない。M8 の subscriber interval variation、Linux 実機未検証、macOS
unsupported、明示 local address の実機確認が CSR8510 A10 に限られること、`Drop` の best-effort
cleanup を release note に残す。

## swbt-rs 公開承認後

停止条件をすべて解消し、当該 turn で公開操作の明示承認を得た場合だけ、次へ進む。

1. `publish = false` の解除と配布 dependency を専用 release change として review する。
2. GitHub Private Vulnerability Reporting を有効化し、`SECURITY.md` を非公開報告 URL へ更新する。
3. main の candidate SHA、Cargo.lock hash、backend version、SBOM hash、全 check を記録する。
4. crates.io Trusted Publishing または同等の短命 credential を設定した専用 workflow を review する。
   現在は publish workflow を置いていない。
5. 承認済み candidate だけを一度公開し、`cargo info swbt-rs@0.1.0` と新規 project から取得・build する。
6. 公開した同じ commit に production tag と GitHub Release を対応付ける。

## rollback と中断

crates.io の公開版は置換できない。誤公開時は対象版を yank し、理由と影響範囲を advisory/release note
へ記録する。Git tag を別 commit へ付け替えず、修正版は新しい版として公開する。

runtime/backend の rollback は [トラブルシューティング](../docs/troubleshooting.md) の順序に従い、
Rust process の close、adapter release、profile copy を確認してから Python 基準断面へ戻す。profile を
変換せず、秘密値を証跡へ含めない。
