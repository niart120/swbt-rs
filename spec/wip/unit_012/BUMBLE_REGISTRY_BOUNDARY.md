# Bumble registry 配布境界 仕様書

## 1. 目的

`swbt-rs` 0.1.0 の `cargo package --locked` を止めている Bumble fork の crates.io 名衝突と
未公開依存を解消する。公開 fork 上で衝突しない package 名、版、依存閉包を準備し、公開前に
再現可能な検査を完了させる。

## 2. 起点

- `spec/initial/roadmap.md` M9 の `cargo package`、clean install、release commit/Bumble revision
- `spec/wip/unit_010/M9_PORTABILITY_RELEASE.md` T08 の registry dependency blocker
- `spec/publishing.md` の 0.1.0 公開停止条件
- `Cargo.toml` と `Cargo.lock` が固定する Bumble fork revision

2026-08-01 の main `b61476f1320906e3b01af1f5e49f832d9740741f` では、
`cargo package --locked` が crates.io に `bumble-controller@0.1.0` がないため停止する。
`bumble@0.1.0` は同 fork と無関係な既存 crate である。

## 3. 対象範囲

- swbt-rs の通常・任意依存から到達する Bumble 22 package と、その package 検証に必要な
  dev 依存 `bumble-hfp`、`bumble-at` の計 24 package。
- package 名を `swbt-bumble` / `swbt-bumble-*` とし、Rust 側の依存 key は既存の
  `bumble` / `bumble-*` のまま保つ `package` alias。
- fork 内 path dependency に exact version requirement と registry package 名を付ける。
- public fork の作業 branch、package file list、workspace build/test、依存順序、公開前検査。
- 公開済み dependency を参照する swbt-rs manifest と archive smoke の準備。

## 4. 対象外

- Bumble upstream への issue または PR。
- `bumble-drivers` と `bumble-pandora`。swbt-rs の package 検証閉包から到達しない。
- crate 実装の挙動変更、公開 API 変更、USB/HCI/入力処理の変更。
- ユーザの当該 turn の明示承認前に行う crates.io publish、production tag、GitHub Release。
- `cargo package --no-verify` だけを package 成功の根拠にすること。

## 5. 振る舞い仕様

| 入力または状態 | 期待結果 |
|---|---|
| current swbt-rs all-feature graph | fork 由来の通常閉包 22 package と package 検証閉包 24 package を機械的に再現できる |
| fork package metadata | 対象 24 package が衝突しない `swbt-bumble*` 名、`0.1.0`、Apache-2.0、source repository を持つ |
| fork 内 package dependency | Rust import 名を変えず、`package = "swbt-bumble-*"` と exact `=0.1.0` を持つ |
| renamed fork branch | workspace check/test と対象 package の file list/manifest 検査が成功し、挙動差分を含まない |
| swbt-rs Bumble dependency | 同一 fork revisionの renamed package を alias 経由で解決し、default/all-feature gate が成功する |
| registry 未公開 | `publish = false` を維持し、公開と clean registry archive smoke を成功扱いにしない |
| 24 package 公開後 | `cargo package --locked` と生成 archive の default/all-feature build、test、examples が clean directory で成功する |

## 6. TDD Test List

| status | item | layer | 完了条件 |
|---|---|---|---|
| green | T01: registry 配布閉包を固定する | dependency graph | 通常閉包 22、dev 依存込み 24、対象外 2 package を current metadata から再現する |
| green | T02: 対象 package 名と内部依存を一意に変換する | fork manifest | rename 24 package、alias/exact version 付き内部依存 87 辺、対象外 2 package を確認。fork commit `2f5c853` |
| green | T03: manifest だけの変更で fork workspace の挙動を保つ | fork gate | 旧 import 名を `[lib] name` で維持。MSRV check、workspace test、24 package file list、基底 package verify が成功。fork commit `5fb0f6d` |
| green | T04: swbt-rs が renamed fork revision を同一 import surface で利用する | integration | default/all-feature check、clippy、test、build、doc が成功。fork 由来 22 package を単一 revision から解決 |
| green | T05: 公開順序と公開前検査を再現可能にする | release | 9 layer の依存順序、未登録/owner なし、prefix 検索 0 件、24 archive checksum、公開停止点を evidence に記録 |
| blocked | T06: registry archive から clean install を検証する | package | 明示承認後に 24 package を公開し、`cargo package --locked` と archive smoke を成功させる |
| green | T07: M9 と公開手順を current evidence に更新する | docs/spec | unit_011 完了、fork revision、lock hash、CI、PVR 無効、24 package 未公開の停止条件を反映 |

T06 の `blocked` は設計不明ではなく外部状態と権限の境界を示す。T02-T05 と T07 は公開せずに進める。
全 Test List 完了前に unit_012 を complete へ移動せず、swbt-rs の PR を merge しない。

## 7. 対象ファイル

- public fork branch の対象 24 package manifest、対象外 2 package から対象 package へ向く
  dependency alias、workspace metadata
- `Cargo.toml`、`Cargo.lock`
- `spec/wip/unit_010/M9_PORTABILITY_RELEASE.md`
- `spec/publishing.md`
- `spec/initial/source-baseline.md`
- `spec/wip/unit_012/`

## 8. 検証

```powershell
cargo metadata --all-features --locked --format-version 1
cargo fmt --all --check
cargo +1.87.0 check --all-targets --all-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
cargo test --all-targets --locked
cargo build --all-features --locked
cargo build --no-default-features --locked
$env:RUSTDOCFLAGS = "-D warnings"; cargo doc --no-deps --all-features --locked
cargo package --locked --list
cargo package --locked
git diff --check
```

fork 側は対象 package ごとの `cargo package --list` と workspace gate を実行する。registry 公開前に
完全な package verify が registry 解決で停止した場合は、その停止点を記録し成功に読み替えない。

### 8.1 現在の検証結果

| command / check | result |
|---|---|
| `cargo metadata` による fork 配布閉包検査 | success: runtime 22 package、dev 依存込み 24 package |
| fork manifest alias 検査 | success: renamed 24 package、internal edge 87、対象外 `bumble-drivers` / `bumble-pandora` |
| fork `cargo +1.87.0 check --workspace --all-targets --all-features --locked` | success |
| fork `cargo test --workspace --all-features --locked` | success |
| fork 24 package の `cargo package --locked --list` | success: 各 6–64 files |
| fork `cargo package --locked -p swbt-bumble` | success: 18 files、圧縮 63.7 KiB、verify build success |
| swbt-rs `cargo +1.87.0 check --all-targets --all-features --locked` | success |
| swbt-rs `cargo clippy --all-targets --all-features --locked -- -D warnings` | success |
| swbt-rs `cargo test --all-targets --locked --quiet` | success: library 268 passed / 1 ignored、他 target success |
| swbt-rs `cargo test --all-targets --all-features --locked --quiet` | success: library 321 passed / 4 ignored、他 target success、hardware 5 ignored |
| swbt-rs default/all-feature build | success |
| `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --all-features --locked` | success |
| `cargo fmt --all --check` / `git diff --check` | success |
| current `cargo package --locked --allow-dirty --list` | success: 126 files |
| current `cargo package --locked --allow-dirty` | blocked: crates.io に `swbt-bumble@0.1.0` がない |
| GitHub Private Vulnerability Reporting API | `enabled:false` |
| changed docs relative-link / placeholder audit | success |

public fork branch は `niart120/bumble-rs` の `feat/swbt-registry-package-names`、head は
`5fb0f6ddb811d1ad43dffa6e72a5d8cc6096fb07` である。upstream PR / issue は作成していない。
公開順序、archive checksum、正規化 manifest、name availability は
[`evidence/bumble-package-preflight-20260801.md`](evidence/bumble-package-preflight-20260801.md) に記録した。

## 9. 先送り事項

- crates.io 名は予約できない。公開直前に 24 名を再確認する。
- crates.io publish と swbt-rs 0.1.0 公開は、各操作を行う turn の明示承認を得る。
- GitHub Private Vulnerability Reporting は M9 の別停止条件として残る。

## 10. 完了チェックリスト

- [x] 目的、対象範囲、対象外を確認した
- [x] TDD Test List を作成した
- [x] fork の manifest 変換と検査を完了した
- [x] public fork branch を push し、revision を固定した
- [ ] swbt-rs の全 gate を完了した
- [ ] registry package と archive smoke を完了した
- [x] M9 の current evidence を更新した
- [ ] PR merge cleanup を完了した
