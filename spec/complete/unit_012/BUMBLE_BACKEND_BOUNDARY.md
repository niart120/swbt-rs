# Bumble backend 配布境界 仕様書

## 1. 目的

`swbt-rs` 0.1.0 の `cargo package --locked` を止めている Bumble Git dependency を、
Bluetooth Classic HID に必要な実装だけを持つ単一の `swbt-bumble-backend` crate へ置き換える。
fork workspace 24 package の crates.io 公開案は採用しない。必要な実装を standalone repository
`niart120/swbt-bumble-backend` へ抽出する。初回版0.1.0の実機回帰で抽出差分2件を修正し、
最終的に公開した0.1.1を`swbt-rs`のexact registry dependencyとする。

## 2. 起点

- `spec/initial/roadmap.md` M9 の `cargo package`、clean install、release dependency 固定
- `spec/complete/unit_010/M9_PORTABILITY_RELEASE.md` T08 の registry dependency blocker
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
- `swbt-rs` の `cargo publish`、production tag、GitHub Release。
- 明示承認のなかった `swbt-bumble-backend` の production tag と GitHub Release。
- 固定fork基準断面との同等性回復を超えるBluetooth/HIDの振る舞い変更。

## 5. 振る舞い仕様

| 入力または状態 | 期待結果 |
|---|---|
| T02 時点の暫定 `bumble` feature build | 8 direct dependency が元の `bumble*` package 名と単一 revision `cb55e2d` を解決する |
| 暫定 fork `main` | 実動作修正3 commitを含み、package 改名2 commitを含まない |
| current `cargo package --locked` | `swbt-bumble-backend@0.1.1` を crates.io から解決し、archive の build が成功する |
| backend public API | open、pair、reconnect、poll、interrupt send、disconnect、close を提供し、Bumble固有型を露出しない |
| backend dependency graph | fork 由来の別 package、Git dependency、local path dependencyを正規化 manifest に残さない |
| backend archive | `LICENSE`、`NOTICE`、README、改変表示を含み、clean directory で build/testできる |
| swbt-rs backend adoption | `bumble` feature が registry version の `swbt-bumble-backend` だけをfork由来依存として持つ |
| backend publish authorizationあり | 承認済みの`swbt-bumble-backend@0.1.0`と修正版0.1.1だけを crates.io に公開し、production tag と GitHub Release は作らない |

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
| green | T06f4: 終了処理と最終archiveを完成する | backend lifecycle / package | disconnect、reader cancellation/join、残留入力なし、clean archive build/testが成功する |
| green | T07: swbt-rs を registry backendへ切り替える | integration/package | default/all-feature gate、`cargo package --locked`、archive smokeが成功する |
| green | T08: 実機回帰を確認する | hardware | pairing、再接続、入力、IMU、明示local address、power-cycle、reader cleanupの既存契約を再確認する |
| green | T09: 旧改名branchを後片付けする | repository cleanup | 実験revisionの参照を不採用証跡だけに残し、remote/local branchを削除してIssue #1へ記録する |

T08 は実機回帰を伴うため、scripted test と archive smoke だけで green にしない。T08 完了前に
unit_012 を `spec/complete` へ移動せず、`swbt-rs` 0.1.0 を公開しない。

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

### 7.2 移行境界

backend 完成までは自己所有 fork `main@cb55e2d` を Git dependency として使った。T07 では
一時的な `[patch.crates-io]` を削除し、`swbt-bumble-backend = "=0.1.1"` を crates.io から
解決する境界へ移行した。`swbt-rs` の `publish = false` は維持する。

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
- `spec/complete/unit_010/M9_PORTABILITY_RELEASE.md`
- `spec/complete/unit_012/`
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
| T02 `cargo metadata --all-features --locked --format-version 1 --no-deps` | success: 8 direct Git dependency、元の `bumble*` package名、単一 `cb55e2d` |
| T07 `cargo metadata --all-features --locked --format-version 1` | success: `swbt-bumble-backend@0.1.0` の source は crates.io registry、manifest は Cargo registry cache 配下。lock checksum は公開archiveのSHA-256と一致 |
| T08 final `cargo metadata --all-features --locked --format-version 1` | success: `swbt-bumble-backend@0.1.1` を crates.io registryから解決。lock checksum `1cc2c8d7d9c8cecfd203cd039fb3c3f8a9c39b072230f977b1e12e526b1bc667` は公開archiveと一致 |
| `cargo +1.87.0 check --all-targets --all-features --locked` | success |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | success |
| `cargo test --all-targets --all-features --locked` | success: library 271 passed / 1 ignored、hardware 5 ignored、他target success |
| `cargo test --all-targets --locked --quiet` | success: library 256 passed / 1 ignored、他target success |
| default/all-feature build | success |
| `cargo package --locked --list` | success |
| `cargo package --locked --allow-dirty` | success: 120 files / 1.4 MiB（圧縮258.0 KiB）。公開版 `swbt-bumble-backend@0.1.1` を解決してarchive verification buildが成功 |
| 展開archive `cargo +1.87.0 test --all-targets --all-features --locked --offline --quiet` | success: library 271 passed / 1 ignored、hardware 5 ignored、他target success |
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
| backend lifecycle / final archive | success: fork `feat/swbt-bumble-backend@701f0dd` でcredit待ちdrain、切断後の送信拒否、stale handle完了の回収、reader close/join後のHCI I/O解放、pending event破棄を確認。package全72 test、build、fmt、clippy `-D warnings`、rustdoc `-D warnings`が成功。clean `cargo package --locked` は29 files / 437.0 KiB（圧縮87.4 KiB）のarchiveを生成し、展開archiveの全72 testも成功。SHA-256 `bde8fb6a5948324f2094db1f359a72feb6212353ee2cc00727f12b0270f785ce` |
| backend release candidate | success: fork `feat/swbt-bumble-backend@dfc5c86` でローカルadapter住所によるbond-store名前空間選択、失敗時の`InvalidBondStore`分類、Rust 1.87互換を確認。`cargo +1.87.0 check/test/clippy`、rustdoc、`cargo publish --dry-run --locked`が成功。clean archiveは29 files / 439.5 KiB（圧縮87.8 KiB）、展開archiveの全73 testが成功。SHA-256 `a4b2e781ff3039be1791d95561ee198083d5d1f975d857ada94368bbbebc110c` |
| backend standalone repository | success: `git subtree split` でbackendに関係する15 commitの履歴を保持し、public repo [`niart120/swbt-bumble-backend`](https://github.com/niart120/swbt-bumble-backend) の `main@306c7ed` を正本にした。元`chaitanyarahalkar/bumble-rs@bbac2a6`、中間fork `niart120/bumble-rs@cb55e2d`、standalone抽出後の変更をREADME/NOTICE/PROVENANCEと19 fileの変更表示で分離し、元NOTICE本文の一致と全commit linkを確認した。GitHub Actions run `30702412817` はUbuntu/Windows test、Rust 1.87、fmt/Clippy/rustdoc、packageの全jobが成功し、Private Vulnerability Reportingも有効。公開archiveは32 files / 460.3 KiB（圧縮91.1 KiB）、SHA-256 `b4df874d56ef7dbeb62ba6f06eeac71b8ef699f8151722812a313b1099121e55` |
| backend T08 regression fixes | success: `122a685`でHCI version 6以下のlegacy LE event maskを復元し、`fa40553`で明示interrupt reportをin-flight ACL credit後ろのhost queueへ受理した。各scripted testは修正前red、修正後green。backend全75 test、fmt、Clippy `-D warnings`、rustdoc `-D warnings`、build、package、展開archive testが成功 |
| backend 0.1.1 release | success: standalone `main@0a4a2d99bc3ed3807464d4f902c20d9fd16b188a`、GitHub Actions run `30706567219` のRust 1.87、Ubuntu/Windows test、quality、packageが成功。32-file archiveのcrates.io checksumは`1cc2c8d7d9c8cecfd203cd039fb3c3f8a9c39b072230f977b1e12e526b1bc667`、ownerは`niart120` |
| backend 0.1.1 cleanup | success: `fix/legacy-le-event-mask`がstandalone `main`と`origin/main`の`0a4a2d9`に完全包含されることを確認し、remote/local branchと修正worktreeを削除した。`swbt-rs`の一時`[patch.crates-io]` worktreeも削除した |
| backend fork branch cleanup | success: standalone split親 `9713b9d` とprovenance反映時の `main@b1d4bab` を確認後、forkのremote/local `feat/swbt-bumble-backend` とlocal `split/swbt-bumble-backend` を削除した。fork checkoutはcleanな `main@cb55e2d` に戻り、旧`swbt-bumble-backend/` directoryとremote feature refが残っていない。standaloneのprovenance作業branchとrelease branchもmain反映・CI成功後にremote/local refと一時worktreeを削除した |
| swbt-rs registry backend adoption | success: 一時`[patch.crates-io]`を削除し、`swbt-bumble-backend = "=0.1.1"`をregistry sourceとchecksum付きで固定した。旧Bumble Git dependency 8件と重複HCI/Classic/L2CAP/SDP/HIDP実装を除去し、adapter error/event/config/identity/session、schema v2 bond-store bridge、all-feature/default gateが成功 |
| swbt-rs registry package | success: `cargo package --locked --allow-dirty` と展開archiveのoffline/all-feature testが成功し、sibling path、fork package、Git dependencyを必要としない |
| backend crates.io publish | success: 初回0.1.0に続き、実機回帰修正版[`swbt-bumble-backend@0.1.1`](https://crates.io/crates/swbt-bumble-backend/0.1.1)を明示承認後に公開し、owner `niart120` とregistry取得を確認した |
| registry 0.1.1 adapter sentinel | success: 別controller application実行中の初回は`transport_open`、終了後に同じregistry-backed binaryで`adapter_opened`。descriptor列挙は両者の間も成功し、WinUSBの排他owner契約と整合 |
| production tag / GitHub Release | not run: このturnの承認対象外 |
| T08 Windows実機回帰 | success: Windows 11 25H2 x86_64、CSR8510 A10 `0A12:0001` / WinUSB、Switch 2 system version 22.5.0、Pro Controllerでfresh pair、保存鍵Periodic/Direct reconnect、A、L+R、両stick、60秒IMU 2回、neutral close、profile不変、power-cycle後adapter reopenを確認。UI観測は入力反映、横移動、カクつきなし、終了後の移動・入力残りなし。詳細は`evidence/registry-backend-hardware-windows-20260802/SUMMARY.md` |

## 10. 先送り事項

- Linux、他adapter、他console versionの実機回帰は対象外とし、`docs/platform-support.md`の未検証条件を維持する。

## 11. 完了チェックリスト

- [x] 目的、対象範囲、対象外を更新した
- [x] 24 package公開案を不採用として記録した
- [x] 暫定依存をfork `main@cb55e2d`へ戻した
- [x] package blockerをclean worktreeで再現した
- [x] 単一backendのIssueと完了条件を記録した
- [x] backend source/API inventoryを完了した
- [x] backend実装を完了した
- [x] registry archive smokeを完了した
- [x] 実機回帰を完了した
- [x] 旧改名branchを削除した
- [x] unit_012をcompleteへ移動した
