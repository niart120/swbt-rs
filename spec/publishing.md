# 0.1.0 公開手順

- 状態: **公開済み**
- 公開版: `0.1.0`
- crates.io公開日時: 2026-08-02 03:41:11 JST (2026-08-01T18:41:11.828209Z)
- 公開元main: `0dc1f7c9a42a47f04b4c56d34502af9cd4f88168`
- 最終監査日: 2026-08-02 (JST)

この文書は release candidate の再現、停止条件、検査、rollback を一か所にまとめる。2026-08-02の
公開turnで、`swbt-rs` 0.1.0の`cargo publish`、同一commitのproduction tag `v0.1.0`、GitHub Release
作成についてユーザの明示承認を得た。

## 解消済みの依存条件

- standalone public repository
  [`niart120/swbt-bumble-backend`](https://github.com/niart120/swbt-bumble-backend) の
  `main@0a4a2d99bc3ed3807464d4f902c20d9fd16b188a` から、
  [`swbt-bumble-backend@0.1.1`](https://crates.io/crates/swbt-bumble-backend/0.1.1) を公開した。
  crates.io owner は `niart120`、archive checksum は
  `1cc2c8d7d9c8cecfd203cd039fb3c3f8a9c39b072230f977b1e12e526b1bc667` である。
- `swbt-rs` は一時 path patch と Bumble Git dependency を残さず、registry 上の
  `swbt-bumble-backend = "=0.1.1"` を解決する。
- T13後のclean `cargo package --locked` は120 files / 1.5 MiB（圧縮259.1 KiB）のarchiveを生成した。
  展開archiveからのMSRV offline all-feature testはlibrary 273 passed / 1 ignored、hardware 5 ignored、
  他target successで、default testとall/no-default buildも成功した。
- [registry backend dependency / package audit](complete/unit_010/evidence/dependency-package-audit-20260802.md)で
  Git source 0、cargo-denyのadvisory/license/source policy、Windows 33 / Linux 34 componentの
  CycloneDX 1.5 SBOM、license欠落0、local pathと秘密情報の不在を確認した。
- `niart120/swbt-rs`のGitHub Private Vulnerability Reportingを有効化し、`SECURITY.md`から
  `https://github.com/niart120/swbt-rs/security/advisories/new`へ案内した。2026-08-02のGitHub APIは
  `enabled:true`を返した。
- [workload soak / backend rollback evidence](complete/unit_010/evidence/operational-cutover-windows-20260802.md)
  では、連続2回の60秒IMU run、RustからPython 0.6.0への同一profile copyによる切戻し、A入力、
  neutral終了、両backend終了後のadapter再利用、profileのバイト不変を確認した。

## 公開前条件の解消と固定点

公開承認前の停止条件だった`publish = false`は専用の
[release PR #15](https://github.com/niart120/swbt-rs/pull/15)で解除した。PRの9 checksと、merge後main
`0dc1f7c9a42a47f04b4c56d34502af9cd4f88168`の
[CI run 30713005380](https://github.com/niart120/swbt-rs/actions/runs/30713005380)は全job成功である。

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

公開元mainの`Cargo.lock` SHA-256は
`40109791FB91C479AF355F4B1A07F59A3E7F3680F35C8E5CF0E311A3D021629F`。clean mainから生成した
120 files / 1.5 MiB（圧縮259.0 KiB）の最終archiveとcrates.io registry checksumは
`387c32c578d283ee0ea3195b5b2a0c79b397ad0cf95539070e81825498015a13`で一致した。

公開前T13監査時点のclean archive SHA-256は
`57C6496601BFD721C71B7771BD8B2847AE1E584DA6FC172939F9103CFB5383A2`。Windows/Linux SBOM SHA-256は
それぞれ`C66AA641628F00E13B1C4FDDC0A582AA12D7E9F9FC27A77EC60D46150CDA1346`、
`3F085008B1D3F44A2ADF89BCAC6973D120C25C3455E61A499CD5959B0BC81085`である。

## local gate

clean checkout で次を実行する。

```powershell
cargo fmt --all --check
cargo +1.87.0 check --workspace --all-targets --all-features --locked
cargo check --workspace --all-targets --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cargo test -p swbt-rs --all-targets --locked
cargo test -p swbt-rs --lib protocol:: --no-default-features --locked
cargo test --doc -p swbt-rs --all-features --locked
cargo build -p swbt-rs --all-features --locked
cargo build -p swbt-rs --no-default-features --locked
cargo deny --locked check
cargo package -p swbt-rs --locked --list
cargo package -p swbt-rs --locked
git diff --check
```

`cargo package -p swbt-rs --locked --list` では `src/`、`docs/`、`examples/`、`tests/` と公開 root file だけを
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
key material がないことを検査する。`cargo-cyclonedx`のroot local pathは
`tools/normalize-cyclonedx.ps1`でpackage URLへ正規化し、dependency referenceを再検査する。

## hardware と既知の制限

公開可否は [対応環境と USB adapter](../docs/platform-support.md) の matrix に従う。Windows の限定構成
以外を実機確認済みと表記しない。M8 の subscriber interval variation、Linux 実機未検証、macOS
unsupported、明示 local address の実機確認が CSR8510 A10 に限られること、`Drop` の best-effort
cleanup を release note に残す。

## 公開結果

- clean `main@0dc1f7c9a42a47f04b4c56d34502af9cd4f88168`から、ローカルのCargo credentialを使って
  `cargo publish --locked`を1回実行した。publish workflowは使用しておらず、credential値は記録していない。
- [`swbt-rs@0.1.0`](https://crates.io/crates/swbt-rs/0.1.0)はyankされておらず、ownerは`niart120`である。
  crates.io APIのchecksumは最終archiveと一致した。
- local packageを依存に使わない新規Cargo projectで`swbt-rs = "=0.1.0"`と`bumble` featureをregistryから解決し、
  Rust 1.87でbuildした。lockfileは`swbt-rs`と`swbt-bumble-backend`のregistry sourceとchecksumを持つ。
- annotated tag `v0.1.0`のtag objectは`46a508ee77e78831155350ae7da6f91a32ad83ca`、peeled commitは
  公開元mainと同じ`0dc1f7c9a42a47f04b4c56d34502af9cd4f88168`である。
- [GitHub Release v0.1.0](https://github.com/niart120/swbt-rs/releases/tag/v0.1.0)は
  2026-08-02 03:41:55 JSTに公開し、draftでもprereleaseでもない。

## rollback と中断

crates.io の公開版は置換できない。誤公開時は対象版を yank し、理由と影響範囲を advisory/release note
へ記録する。Git tag を別 commit へ付け替えず、修正版は新しい版として公開する。

runtime/backend の rollback は [トラブルシューティング](../docs/troubleshooting.md) の順序に従い、
Rust process の close、adapter release、profile copy を確認してから Python 基準断面へ戻す。profile を
変換せず、秘密値を証跡へ含めない。
