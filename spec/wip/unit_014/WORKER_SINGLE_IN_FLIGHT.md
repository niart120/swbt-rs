# Worker command 単一 in-flight 化 仕様書

## 1. 概要

### 1.1 目的

公開 controller API の直列な呼び出し契約に合わせ、worker command 経路を一件の
in-flight responseだけを保持する構造へ縮小する。複数commandのqueue、batch処理、FIFO配送のための
状態と公開`Busy` errorを削除し、pending operation、transport event、deadline、priority shutdownの
既存動作を維持する。

### 1.2 起点 / source

| source | 内容 | path |
|---|---|---|
| GitHub Issue | worker command経路を単一in-flightへ簡素化する | `https://github.com/niart120/swbt-rs/issues/18` |
| Issue comment | 公開APIの直列性、削除候補、維持すべきpending/shutdown契約、測定項目 | `https://github.com/niart120/swbt-rs/issues/18#issuecomment-5153305181` |
| user decision | 後方互換性より削除を優先し、`Busy`をdeprecatedとして残さない。後続変更があるため、このunitでは`0.2.0`へ版を変更しない | 2026-08-02の対話 |
| user follow-up | セットアップ済みの実機で再検証する | 2026-08-02の対話 |
| initial spec | worker所有権、typed command、error、runtime timing | `spec/initial/architecture.md`、`api.md`、`testing.md` |

### 1.3 use case

| actor / boundary | 入力または状態 | 期待する観測結果 | 制約 |
|---|---|---|---|
| controller利用者 | `pair`、`reconnect`、入力操作 | 一操作ずつworker responseを同期的に受け取る | queue満杯を通常の利用者向け分岐にしない |
| worker | 即時完了command | 一反復で一件だけ処理し、responseを一度だけ返す | 続けてtransport pollとdeadline処理へ進む |
| worker | pending中のpair/reconnect/tap | 一件のresponseを完了または中断まで保持する | pending中もtransport eventとshutdownを処理する |
| controller利用者 | response受信前のworker終了 | `WorkerFailed`を受け取る | backend詳細を公開文言へ出さない |
| runtime保守者 | responseに対応するin-flightがない内部状態 | terminal worker failureとして観測する | completionを別commandへ配送しない |

## 2. 対象範囲

- command channel容量を1へ固定する
- `CommandReceiver`が一件のcompletionだけを保持する
- workerのcommand batch設定を削除し、一反復で一件だけ処理する
- step内のcommand progressを単一状態へ縮小する
- requestごとのresponseを一度だけ配送し、caller側dropを成功扱いにする
- `ErrorKind::Busy`、内部`Busy`、response buffer満杯errorをdeprecated期間なしで削除する
- 単一command条件に合わせてruntime measurementを更新し、変更前後を比較する
- 既存プロファイルを使い、Pro ControllerのPeriodic／Direct再接続を実機で回帰確認する
- 公開API破壊を仕様、変更記録、PR本文に明記する

## 3. 対象外

- transport eventのpoll batch制御
- priority shutdown、Drop shutdown、cleanup、joinの順序
- pair/reconnect/tapの非同期進行モデル
- worker threadの廃止またはasync runtimeの導入
- Bumble backend、USB/HCI、Bluetooth air、Switch実機の性能変更または性能測定
- Cargo package versionの`0.2.0`への更新
- Issue #18に含まれない公開API整理

## 4. 関連 docs

- `spec/initial/architecture.md`
- `spec/initial/api.md`
- `spec/initial/testing.md`
- `spec/initial/QUALITY_GATES.md`
- `spec/complete/unit_003/M2_RUNTIME.md`
- `spec/complete/unit_013/LOW_RISK_STRUCTURE_CLEANUP.md`

## 5. 振る舞い仕様

| 振る舞い | 入力・状態 | 期待結果 | 備考 |
|---|---|---|---|
| command enqueue | workerが受信可能 | 容量1のchannelへrequestを置き、workerをwakeする | 公開操作はresponse完了まで`&mut self`を保持する |
| 内部二重enqueue | 容量1のslotが使用中 | `Busy`ではなく内部不変条件違反として分類する | 公開APIの通常分岐ではない |
| 即時完了 | press/release/neutral/apply/sendまたは即時error | 対応する一件のresponseへ結果を一度だけ返す | FIFO複数配送は持たない |
| pending | pair/reconnect/tap開始 | responseを送らずin-flight completionを保持する | 後続commandは処理しない |
| 同一step完了 | duration zeroのtap | pending開始とrelease完了を一つの最終completionへ畳む | responseは一度だけ返す |
| caller drop | response receiverが先にdrop | workerを失敗させずcompletionを破棄する | 所有権上、二重送信はできない |
| response欠落 | completion対象のin-flightがない | terminal command-delivery failureとしてworkerを終了する | `WorkerFailed`へ投影する |
| pending中のevent | HID output、disconnect、readiness進行 | eventを処理し、replyまたはtyped completionを返す | poll batchは変更しない |
| shutdown | idle、即時command後、pending operation中 | 通常commandより優先し、pending responseをtyped shutdownで完了する | cleanup契約は変更しない |
| 公開error | 任意の公開controller操作 | `ErrorKind::Busy`を返さず、variant自体も公開面から削除する | version更新は後続release作業 |

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| done | 一件のcommand responseがpending、同一step完了、caller drop、欠落時failureの各状態で正しい相手へ一度だけ配送される | regression / edge | unit | 複数FIFO配送testを単一in-flight状態遷移へ置換した |
| done | workerが一反復で一件だけcommandを処理し、transport eventとPeriodic deadlineへ進む | regression | unit | command batch値と複数command結果を削除した |
| done | pair/reconnect/tapのpending中もHID output、disconnect、priority shutdownを処理し、待機callerがtyped結果または`WorkerFailed`を受け取る | regression | worker integration | 単一in-flightと一件のqueued waiterで終了経路を検証した |
| done | 公開controller操作が同期完了し、公開・CLI・exampleのerror分類に`Busy`が残らない | behavior change | integration / package | deprecated variantを残していない |
| done | 単一command条件のruntime measurementでcommand latency、8 ms interval、pending接続中のreply、shutdownに明確な悪化がない | performance | release measurement / worker integration | release測定、pending接続test、実機reconnectを組み合わせて確認した |
| machine done / UI pending | 既存プロファイルのPro ControllerがPeriodic／Directの両modeで再接続し、HID応答、入力、正常終了を観測できる | regression | hardware | machine logは成功。Switch画面上の観測はユーザ確認待ち |

## 7. 設計メモ

- 構造変更と公開error削除を含むmixed changeである。privateな単一in-flight構造を先に確立し、
  `Busy`削除を別のTDD itemとして検証する。
- enqueueはblocking sendへ変えない。公開APIでは満杯へ到達せず、内部不整合時にcallerを停止させない。
- channelの`Full`はcrate-privateな不変条件違反として`ErrorKind::Internal`へ投影する。
- `CommandCompletion`は送信時に所有権を消費する。responseは一件専用の非同期channelで返し、
  receiver切断だけを無視できるため、`ResponseBufferFull`は不要である。
- step内でpendingとcompleteの両方が発生し得るが、一件のcommandに対する状態遷移である。
  複数件の結果を保持する`Vec`ではなく単一状態へ畳む。
- `ErrorKind::Busy`削除は0.1.0利用者に対するソース互換性を壊す。ユーザ判断により削除を優先するが、
  Cargo versionは後続変更をまとめるrelease作業まで`0.1.0`を維持する。
- 性能比較では変更前と変更後のrelease measurementを保持する。旧16-command飽和条件と
  新単一command条件は負荷が異なるため、fairness値を直接比較せず、直列command latency、periodic
  interval、pending接続中reply、shutdownを判定対象にする。

## 8. 対象ファイル

| path | change | 内容 |
|---|---|---|
| `src/runtime/command.rs` | modify | 容量1、単一completion、内部enqueue/delivery error |
| `src/runtime/worker.rs` | modify | 一反復一command、単一step progress、poll batch維持 |
| `src/runtime/worker_thread.rs` | modify | 単一pending responseの終了・panic・shutdown検証 |
| `src/controller/runtime.rs` | modify | production tuningからcommand capacity/batchを削除 |
| `src/controller/runtime_measurement.rs` | modify | 単一command measurementと比較可能なmetadata |
| `src/runtime/error_map.rs`、`src/error.rs` | modify | `Busy`削除と内部不変条件error投影 |
| `src/controller/mod.rs` | modify | 公開rustdocからqueue error説明を削除 |
| `src/bin/swbt-probe.rs`、`examples/*.rs` | modify | 公開`Busy`分類削除 |
| `spec/wip/unit_014/evidence/` | new | 変更前後のraw、summary、manifest |
| `spec/wip/unit_014/WORKER_SINGLE_IN_FLIGHT.md` | new / modify | 作業仕様、TDD、検証、self-review |

## 9. 検証

| command | result | notes |
|---|---|---|
| `cargo test --lib runtime::command::tests --all-features --locked` | success | 4 passed |
| `cargo test --lib runtime::worker::tests --all-features --locked` | success | 28 passed |
| `cargo test --lib runtime::worker_thread::tests --all-features --locked` | success | 9 passed |
| `cargo test --all-targets --all-features --locked` | success | lib 269 passed / 1 ignored、bin・integration・example target成功、hardware manual 5 ignored、profile compat manual 1 ignored |
| `cargo test --locked` | success | lib 253 passed / 1 ignored、default integration・doctest成功、profile compat manual 1 ignored |
| `cargo fmt --all --check` | success | 差分なし |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | success | warning 0 |
| `cargo +1.87 check --all-targets --all-features --locked` | success | MSRV 1.87 |
| `cargo build --all-features --locked` / `cargo build --locked` | success | all-feature / default production build |
| `cargo test --doc --all-features --locked` | success | 1 passed |
| `RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps --locked` | success | public rustdoc warning 0 |
| `cargo package --locked` | success | 120 files、1.4 MiB、251.7 KiB compressed。`swbt-rs 0.1.0`を検証し、publishは未実行 |
| `git diff --check` | success | whitespace errorなし |
| `pwsh -NoProfile -File tools/measure_m2_runtime.ps1 -OutputDirectory target\\measurements\\m2-activity-wait\\unit014-before-20260802` | success | release profile、42,002 records、clean commit `afb20b0`。response p99 1.4 µs、Periodic lateness p99 1.9693 ms、skip 0、idle shutdown p99 56.1 µs、16-command飽和shutdown p99 54.4 µs |
| `pwsh -NoProfile -File tools/measure_m2_runtime.ps1 -OutputDirectory target\\measurements\\m2-activity-wait\\unit014-after-20260802` | success | release profile、42,002 records、clean commit `c001475`。response p99 1.4 µs、Periodic lateness p99 1.8136 ms、skip 0、idle shutdown p99 52.8 µs、単一queued shutdown p99 53.1 µs、fairness skip / burst 0 |
| `cargo run --release --locked --all-features --example pro_periodic_hardware -- ... --run 15` | success | fresh Pairは5.292秒でReady。reply 16件、入力、neutral、close、profile検証、adapter reopen成功 |
| `cargo run --release --locked --all-features --example pro_profile_hardware -- ... --mode periodic ... --run 16` | success | reconnectは1.987秒でReady。reply 16件、入力、neutral、close、adapter reopen、profile byte不変を確認 |
| `cargo run --release --locked --all-features --example pro_profile_hardware -- ... --mode direct ... --run 17` | success | reconnectは2.625秒でReady。Ready後idle入力0件、reply 16件、入力、neutral、close、adapter reopen、profile byte不変を確認 |

### 9.1 実機観測の境界

- 既存の旧プロファイルを使ったPeriodic run 15は、adapterとschema v2 profileの検出後、`NoBond`で
  接続前に終了した。ファイルは変更されていない。
- 旧プロファイルを上書きせず別プロファイルへfresh Pairし、そのprofileでPeriodic run 16とDirect
  run 17を成功させた。machine logにはpath、raw profile、peer address、key materialを出していない。
- machine logはtransport受理とworker状態を示す。Switch画面上のA、L+R、左右stick、neutralの反映は
  ユーザ観測待ちであり、確認前にmachine結果から推定しない。

## 10. 先送り事項

- `ErrorKind::Busy`削除を含む次回公開版の`0.2.0` version更新とreleaseは、本unit完了後の後続変更を
  まとめるrelease作業で扱う。
- Bumble、USB/HCI、Bluetooth air、Switch実機を含むlatencyは本unitのruntime measurementでは
  測定しない。worker/channelのfake transport境界で悪化がないことを確認し、実機は機能回帰として扱う。

## 11. チェックリスト

- [x] 対象範囲と対象外を確認した
- [x] TDD Test List を更新した
- [ ] 検証結果または未実行理由を記録した
- [x] package / release / public APIに触れる場合のgateを記録した
