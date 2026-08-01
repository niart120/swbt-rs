# Bumble backend 配布境界 仕様書

## 1. 目的

`swbt-rs` 0.1.0 の `cargo package --locked` を止めている Bumble Git dependency を、
Bluetooth Classic HID に必要な実装だけを持つ単一の `swbt-bumble-backend` crate へ置き換える。
fork workspace 24 package の crates.io 公開案は採用せず、暫定期間は自己所有 fork
`niart120/bumble-rs` の `main` に固定する。

## 2. 起点

- `spec/initial/roadmap.md` M9 の `cargo package`、clean install、release dependency 固定
- `spec/wip/unit_010/M9_PORTABILITY_RELEASE.md` T08 の registry dependency blocker
- `spec/publishing.md` の 0.1.0 公開停止条件
- 自己所有 fork の [Issue #1](https://github.com/niart120/bumble-rs/issues/1)
- 24 package 改名案の公開範囲、保守責任、Apache-2.0 配布物検査

2026-08-01 時点で自己所有 fork の `main` は
`cb55e2d98dc7b7b0227c43772c9ae184034dd9a1` である。この revision は reader shutdown、
ACL flush 観測、Vendor Event 応答の実動作修正3 commitだけを含む。

## 3. 対象範囲

- `swbt-rs` の暫定 Git dependency を元の `bumble*` package 名と fork `main@cb55e2d` に戻す。
- `swbt-bumble-backend` が所有する最小 API と実装範囲を固定する。
- USB HCI、必要な HCI command/event/ACL、Classic pairing/reconnect/link key、Classic L2CAP、
  SDP、HIDP、session close を backend 候補に含める。
- backend の公開 API から Bumble workspace 固有型を隠し、`swbt-rs` との循環依存を作らない。
- Apache-2.0 の `LICENSE`、`NOTICE`、attribution、改変表示を backend archive に収録する。
- `swbt-rs` を registry 上の単一 backend crate へ切り替え、clean archive smoke まで検証する。
- 旧 package 改名案と dry-run を、採用しなかった実験として証跡に残す。

## 4. 対象外

- fork workspace 24 package を `swbt-bumble*` 名で crates.io に公開すること。
- fork 元 `chaitanyarahalkar/bumble-rs` または `google/bumble` への issue / PR。
- A2DP、AVRCP、audio codec、GATT、ATT、LE advertising、RFCOMM、Android emulator、
  汎用 TCP / WebSocket transport を backend の要件に含めること。
- この作業だけを根拠にした `cargo publish`、production tag、GitHub Release。
- Bluetooth/HID の観測可能な振る舞い変更。

## 5. 振る舞い仕様

| 入力または状態 | 期待結果 |
|---|---|
| 暫定 `bumble` feature build | 8 direct dependency が元の `bumble*` package 名と単一 revision `cb55e2d` を解決する |
| 暫定 fork `main` | 実動作修正3 commitを含み、package 改名2 commitを含まない |
| current `cargo package --locked` | crates.io に `bumble-controller@0.1.0` がないため停止し、公開可能と報告しない |
| backend public API | open、pair、reconnect、poll、interrupt send、disconnect、close を提供し、Bumble固有型を露出しない |
| backend dependency graph | fork 由来の別 package、Git dependency、local path dependencyを正規化 manifest に残さない |
| backend archive | `LICENSE`、`NOTICE`、README、改変表示を含み、clean directory で build/testできる |
| swbt-rs backend adoption | `bumble` feature が registry version の `swbt-bumble-backend` だけをfork由来依存として持つ |
| publish authorizationなし | `publish = false` を維持し、crate upload、tag、Releaseを実行しない |

## 6. TDD Test List

| status | item | layer | 完了条件 |
|---|---|---|---|
| green | T01: 実動作修正だけを fork `main` に固定する | fork history | `main@cb55e2d` が改名前の3 commitを含み、workspace check/testが成功する |
| green | T02: swbt-rs の暫定依存を元 package 名へ戻す | Cargo metadata | `Cargo.toml` と `Cargo.lock` が `bumble*`、`cb55e2d` だけを解決し、default/all-feature gateが成功する |
| green | T03: 現在の registry blocker を再現する | package | clean `cargo package --locked --list` は成功し、`cargo package --locked` は `bumble-controller` 不在で停止する |
| green | T04: 単一 backend の作業境界を自己所有 fork に記録する | cross-repo design | Issue #1 に対象範囲、対象外、ライセンス、clean archive、実機回帰条件がある |
| green | T05: 必要な Bumble source と API を inventory する | backend design | USB HCI、HCI、Classic host、L2CAP、SDP、HIDP、key storeの採否をsource/test単位で記録する |
| green | T06a: backend crate の配布骨格を作る | package / license | 自己所有forkの独立packageがfork/path/Git通常依存なしでpackageでき、LICENSE、NOTICE、README、改変表示を含む |
| green | T06b: core value と HCI codec を内部化する | backend codec | address、UUID、Classic key、必要なHCI command/event/ACLだけを内部moduleとtestへ移す |
| green | T06c: Classic L2CAP、SDP、HIDPを内部化する | backend protocol | LE credit channelを含めず、Classic signaling、SDP continuation、HIDP codecが移植testを通る |
| green | T06d: Classic host と bond state を抽出する | backend host | pairing/reconnect、ACL credit、L2CAP channel、link-key永続化がtest-only peerで動く |
| green | T06e: USB HCI と external reader を抽出する | backend transport | command/event/ACL、reader cancellation/join、adapter metadataがscripted/USB testを通る |
| green | T06f1: backend の公開境界を定義する | backend public API | adapter、設定、bond store、event、error、session APIがBumble内部型を露出せず、公開API testとrustdocを通る |
| green | T06f2: HCI session 初期化とevent変換を実装する | backend session | controller初期化、identity設定、pair/reconnectに必要なcommand/event変換がscripted testを通る |
| green | T06f3: SDP/HID session を統合する | backend integration | pair→SDP continuation→HID channel→HID outputとinterrupt sendが公開event境界で動く |
| pending | T06f4: 終了処理と最終archiveを完成する | backend lifecycle / package | disconnect、reader cancellation/join、残留入力なし、clean archive build/testが成功する |
| pending | T07: swbt-rs を registry backendへ切り替える | integration/package | default/all-feature gate、`cargo package --locked`、archive smokeが成功する |
| pending | T08: 実機回帰を確認する | hardware | pairing、再接続、入力、IMU、明示local address、power-cycle、reader cleanupの既存契約を再確認する |
| green | T09: 旧改名branchを後片付けする | repository cleanup | 実験revisionの参照を不採用証跡だけに残し、remote/local branchを削除してIssue #1へ記録する |

T06a-T08 は backend 実装を伴うため、この依存・文書整理だけで green にしない。全項目完了前に
unit_012 を `spec/complete` へ移動せず、0.1.0 を公開しない。

## 7. 設計判断

### 7.1 採用しなかった案

24 package の一括改名は Cargo 上の名前衝突を回避できるが、利用しない protocol を含む全 package の
版管理、脆弱性対応、yank、問い合わせ対応を引き受ける。dry-run archive には root の
`LICENSE` と `NOTICE` が含まれておらず、そのまま公開できる状態でもなかった。この案は
「技術的に未完了」ではなく「配布境界として不採用」とする。

詳細は
[`evidence/abandoned-registry-package-experiment-20260801.md`](evidence/abandoned-registry-package-experiment-20260801.md)
に残す。

必要なsource/API、除外するprotocol、public API、test移行単位は
[`evidence/bumble-backend-source-inventory-20260801.md`](evidence/bumble-backend-source-inventory-20260801.md)
に固定した。

### 7.2 暫定境界

`swbt-rs` は backend 完成まで自己所有 fork `main@cb55e2d` を Git dependency として使う。
これは repository source からの build を支えるが、crates.io 公開条件を満たさない。
`publish = false` は維持する。

### 7.3 恒久境界

`swbt-bumble-backend` は薄い facade にしない。必要な Apache-2.0 派生コードを単一 package 内へ
整理し、fork workspace packageへの依存を残さない。小さいのは公開 API と配布単位であり、
Classic host と USB transport の切り出し作業量を小さいと仮定しない。

## 8. 対象ファイル

- `Cargo.toml`
- `Cargo.lock`
- `README.md`
- `docs/platform-support.md`
- `spec/initial/source-baseline.md`
- `spec/publishing.md`
- `spec/wip/unit_010/M9_PORTABILITY_RELEASE.md`
- `spec/wip/unit_012/`
- 自己所有 fork `niart120/bumble-rs` の Issue #1 と branch

## 9. 検証

```powershell
cargo metadata --all-features --locked --format-version 1
cargo fmt --all --check
cargo +1.87.0 check --all-targets --all-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
cargo test --all-targets --locked
cargo build --all-features --locked
cargo build --no-default-features --locked
cargo package --locked --list
cargo package --locked
git diff --check
```

### 9.1 現在の検証結果

| command / check | result |
|---|---|
| fork `main` fast-forward | success: `bbac2a6..cb55e2d`、実動作修正3 commit |
| fork `cargo +1.87.0 check --workspace --all-targets --all-features --locked` | success |
| fork `cargo test --workspace --all-features --locked` | success |
| `cargo metadata --all-features --locked --format-version 1 --no-deps` | success: 8 direct Git dependency、元の `bumble*` package名、単一 `cb55e2d` |
| `cargo +1.87.0 check --all-targets --all-features --locked` | success |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | success |
| `cargo test --all-targets --all-features --locked --quiet` | success: library 321 passed / 4 ignored、hardware 5 ignored、他target success |
| `cargo test --all-targets --locked --quiet` | success: library 268 passed / 1 ignored、他target success |
| default/all-feature build | success |
| `cargo package --locked --list` | success |
| `cargo package --locked` | blocked: crates.io に `bumble-controller@0.1.0` がない |
| fork branch cleanup | success: `feat/swbt-registry-package-names` のremote/local refを削除し、`git ls-remote --heads origin main feat/swbt-registry-package-names` は `main@cb55e2d` だけを返した |
| backend source/API inventory | success: USB/HCI/Classic host/L2CAP/SDP/HIDP/bondの採否、public API、test移行単位をfixed source pathへ対応付けた |
| backend crate scaffold | success: fork `feat/swbt-bumble-backend@53fe453` の依存0件のarchive 8 files / 15.8 KiBがLICENSE、NOTICE、READMEを含み、package verifyと1 testが成功 |
| backend core/HCI codec | success: fork `feat/swbt-bumble-backend@f396aeb` のbackend所有値型、command/event/ACL framing、L2CAP fragmentation/reassembly、SCO/ISO拒否を10 testで確認。archive verify成功 |
| backend Classic protocols | success: fork `feat/swbt-bumble-backend@35e62e0` のClassic L2CAP/ERTM、SDP continuation、HIDPを27 testで確認。LE credit sourceと既存L2CAP adapterを含まない21-file archive verify成功 |
| backend Classic host | success: fork `feat/swbt-bumble-backend@50df9e3` のbond load/store、pairing/encryption、ACL credit、test-only peer SDU、disconnect cleanupを36 testで確認。archive verify成功 |
| backend USB/external reader | success: fork `feat/swbt-bumble-backend@eb3b67a` のselector/endpoint、分割packet framing、command/event/ACL I/O、cancel/join、terminalを43 testで確認。直接依存は`rusb`だけ、source unsafe 0件、archive verify成功 |
| backend public API | success: fork `feat/swbt-bumble-backend@c7e5f3b` のadapter/config/bond/event/error/opaque session境界を外部API test 4件で確認。package全47 test、clippy `-D warnings`、rustdoc `-D warnings`が成功し、Classic link keyのDebug表示をredactした |
| backend HCI session | success: fork `feat/swbt-bumble-backend@c6a89c4` の14-command初期化、Classic capability、pair/reconnect、link-key、reader通知、CSR volatile rewrite/re-enumeration/readbackをscripted testで確認。package全65 test、clippy `-D warnings`、rustdoc `-D warnings`が成功 |
| backend SDP/HID session | success: fork `feat/swbt-bumble-backend@c39d711` でpair→SDP continuation→control/interrupt channel→HID output→interrupt inputを同一scripted sessionで確認し、能動再接続のcontrol→interrupt順序とevent queue overflowのterminal化も確認。package全68 test、fmt、clippy `-D warnings`、rustdoc `-D warnings`、diff checkが成功 |
| crates.io publish / production tag / GitHub Release | not run: 対象外かつ明示承認なし |
| 実機回帰 | not run: USB/HCI実装は追加したが、このTDD項目はscripted I/Oだけを対象とし、実機確認はT08で行うため |

## 10. 先送り事項

- backend session 実装: T06f4で終了処理と最終archiveを完成する。
- registry archive と clean-install smoke: `swbt-bumble-backend` 公開後にT07で実行する。
- 実機回帰: T06f4とT07の完了後にT08で実行する。
- GitHub Private Vulnerability Reporting: M9 の独立した公開停止条件として残す。

## 11. 完了チェックリスト

- [x] 目的、対象範囲、対象外を更新した
- [x] 24 package公開案を不採用として記録した
- [x] 暫定依存をfork `main@cb55e2d`へ戻した
- [x] package blockerをclean worktreeで再現した
- [x] 単一backendのIssueと完了条件を記録した
- [x] backend source/API inventoryを完了した
- [ ] backend実装を完了した
- [ ] registry archive smokeを完了した
- [ ] 実機回帰を完了した
- [x] 旧改名branchを削除した
- [ ] unit_012をcompleteへ移動した
