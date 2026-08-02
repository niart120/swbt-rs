# runtime lifecycle・接続 API 簡素化仕様書

## 1. 概要

### 1.1 目的

open 済み transport を所有する worker の内部 lifecycle、connection session ID、時刻表現、
connection command error を実際の実行経路に合わせて縮小する。公開接続 API は
`Result<ConnectionPath, Error>` と `ErrorKind` に一本化し、不正な組み合わせを構築できる
`ConnectionResult` と重複する `try_*` API を削除する。

### 1.2 起点 / source

| source | 内容 | path |
|---|---|---|
| GitHub issue | lifecycle・内部 error・接続 API・feature 構成の整理 | https://github.com/niart120/swbt-rs/issues/22 |
| issue comment | prerequisites、現行重複、維持する回帰契約、実施順序 | https://github.com/niart120/swbt-rs/issues/22#issuecomment-5153313291 |
| user decision | Issue #22 を二つの work-unit に分け、本 unit で `try_*` 削除を含む runtime/API 整理を行う | 2026-08-02 の対話 |
| user decision | 後続 unit で portable core crate を抽出し、runtime crate の Bumble backend を必須化する | 2026-08-02 の対話 |

### 1.3 use case

| actor / boundary | 入力または状態 | 期待する観測結果 | 制約 |
|---|---|---|---|
| controller 利用者 | open→pair/reconnect→Ready→disconnect | 公開 lifecycle が `Open→Connecting→Ready→Open` と遷移する | stale session は現在 session を変更しない |
| controller 利用者 | `connect(ConnectOptions)` | 成功時は `ConnectionPath`、回復判断可能な失敗は `ErrorKind` を返す | `NoBond` 以外から pair へ fallback しない |
| controller 利用者 | 表現不能な接続 timeout | transport 副作用前に `InvalidInput` を返す | worker terminal failure にしない |
| worker | session ID が `u64::MAX` の次へ進む | 0 を飛ばして 1 を払い出し、旧 session event を破棄する | process 内の識別子であり永続一意性は要求しない |
| worker | close、Drop、panic | 公開 `Closed` と `Failed` の意味を維持する | cleanup は失敗後も残りの phase を続ける |

## 2. 対象範囲

- live worker の内部 lifecycle を `Open`、`Connecting`、`Ready`、`Closing`、`Failed` へ縮小する
- worker constructor 内の `Configured`、`opening`、論理 open 再演、同一 worker の reopen を削除する
- 公開 `LifecycleState` の `Configured` と `Closed` を status projection と controller owner 境界で維持する
- `ReadySession` wrapper を削除し、current session ID の照合を維持する
- connection session ID を 0 を飛ばす wrapping increment にし、`SessionError` を削除する
- monotonic `Duration` から protocol nanoseconds への変換を集約し、内部 clock overflow error を削除する
- caller supplied timeout の表現不能値を `InvalidInput` として transport 副作用前に拒否する
- Pair/Reconnect の重複 error enum を共通 connection command failure へ統合する
- `try_reconnect()`、`try_connect()`、`ConnectionResult`、`ConnectionStatus` を削除する
- README、rustdoc、初期仕様、移行記述、変更履歴を残る API に合わせる

## 3. 対象外

- `swbt-core` package の追加と source 移動
- `bumble` feature の削除、標準化、必須依存化
- `diagnostics-schema` feature の変更
- Cargo package version の `0.2.0` への更新と crates.io 公開
- `ErrorKind` の公開 variant 全体の再設計
- reconnect が `NoBond` 以外で pair へ fallback する変更
- hardware、USB adapter、Switch UI の再検証

## 4. 関連 docs

- `spec/initial/api.md`
- `spec/initial/architecture.md`
- `spec/initial/testing.md`
- `spec/initial/roadmap.md`
- `spec/initial/QUALITY_GATES.md`
- `spec/complete/unit_014/WORKER_SINGLE_IN_FLIGHT.md`
- `spec/complete/unit_015/TOOLING_CRATE_SEPARATION.md`
- `spec/complete/unit_016/CREATE_PROFILE_RUNTIME_SIMPLIFICATION.md`
- `spec/complete/unit_017/PROFILE_MODEL_AND_STORE_SIMPLIFICATION.md`

## 5. 振る舞い仕様

| 振る舞い | 入力・状態 | 期待結果 | 備考 |
|---|---|---|---|
| live worker 初期状態 | open 済み transport から worker を構築 | 内部・公開 status とも `Open` から始まる | `Configured` は controller 構築後・open 前の公開 projection にだけ残す |
| close | Open/Connecting/Ready/Failed の worker | cleanup を一度開始し、通常 close は公開 `Closed`、failure cleanup は公開 `Failed` を維持する | 同一 worker の reopen は提供しない |
| readiness | current session の handshake 完了 | 同じ session ID のときだけ `Ready` | wrapper token は契約にしない |
| stale event | 前 session ID の event | current session の state/status/input を変更しない | ID wrap 後も同じ比較規則を使う |
| session wrap | last issued が `u64::MAX` | 次 ID は 1 | ID 0 は作らない |
| clock projection | monotonic duration が `u64` nanoseconds を超える | protocol timestamp は `u64::MAX` に飽和し、専用 clock error を返さない | 通常の process lifetime では到達不能 |
| timeout validation | protocol timestamp domain を超える timeout | `InvalidInput`、pairing/reconnect 開始なし、worker は `Open` | timeout message は semver 契約にしない |
| reconnect failure | no-bond/timeout/pre-ready disconnect | `NoBond`/`ConnectionTimeout`/`ConnectionFailed` | source chain を維持する |
| connect fallback | reconnect が `NoBond` かつ pairing 許可 | pair を一度開始し、成功時 `ConnectionPath::Paired` | 他の error では pair を開始しない |
| public connection surface | crates.io 利用者 | `pair`、`reconnect`、`connect` と `ErrorKind` で成功・失敗を判断する | `try_*` と status/result DTO は 0.2.0 向け非互換削除 |

### 5.1 Intent Delta

- `spec/initial/api.md` の初期公開面から `try_reconnect`、`try_connect`、
  `ConnectionResult`、`ConnectionStatus` を削除し、`ErrorKind` による回復判断を正本にする。
- `spec/initial/architecture.md` の worker lifecycle は live worker の状態と公開 status projection を
  分離し、controller close 後の reopen は新しい worker owner を構築する経路として記述する。
- `spec/initial/architecture.md` の session ID は永続単調増加ではなく、0 を使わない wrapping ID とする。
- feature/package 境界の Intent Delta は後続 `unit_019` で反映する。

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| refactor-done | T01 open 済み transport から始まる worker が公開 `Open→Connecting→Ready→Open→Closing→Closed` を維持し、failure cleanup 後の公開 status は `Failed` のままになる | regression / characterization | runtime unit / integration | internal `Configured`、`opening`、同一 worker reopen を除去した |
| refactor-done | T02 session ID が `u64::MAX` の次に 1 へ wrap し、wrap 前 session の event を current session event として受理しない | edge | runtime unit | `SessionError` と上位 error variant を除去した |
| todo | T03 表現不能な caller timeout は transport 副作用前に `InvalidInput` となり、長時間稼働を模した clock projection は専用 overflow errorなしで monotonic timestamp を飽和する | edge / regression | controller/runtime unit | clock/deadline errorを共通 helper と入力境界へ集約する |
| todo | T04 pair/reconnect が共通 connection failure を通っても no-bond、timeout、pre-ready disconnect、protocol/worker failure の公開 `ErrorKind` と source を維持する | regression | runtime/error unit / integration | PairingError/ReconnectError の重複を除去する |
| todo | T05 公開接続面が `pair`、`reconnect`、`connect -> Result<ConnectionPath, Error>` に一本化され、README、rustdoc、初期仕様、移行記述、package archive が同じ API を示す | behavior / regression | public API / docs / package | compile-fail fixture は追加せず、残す API の integration test、rustdoc、package gateで確認する |

status は `todo`、`red`、`green`、`refactor-done`、`refactor-skipped`、`deferred` を使う。

### 6.1 TDD cycle evidence

| phase | item | evidence |
|---|---|---|
| red | T01 | live worker が論理 open を再演せず `Open` から始まる test を追加した。focused test は actual `Configured` / expected `Open` で失敗した |
| green | T01 | private `RuntimeState` を live worker の5状態へ縮小し、constructor は `Open` から開始した。lifecycle 5件、runtime 120件、公開 controller runtime 30件が成功した |
| refactor-done | T01 | internal lifecycle と公開 `LifecycleState` projection を分離した。通常 cleanup は status に `Closing→Closed` を記録し、failure cleanup は既存 `Failed` projection を上書きしない。all-feature library Clippy、fmt、diff check が成功した |
| red | T02 | `last_issued = u64::MAX` の session から次 session を開始する test を追加した。focused test は `SessionError::IdExhausted` で失敗した |
| green | T02 | session ID を 0 を飛ばす wrapping increment に変更した。最大 ID の event を保持したまま次 session が 1 になり、旧 event を拒否する focused test が成功した |
| refactor-done | T02 | 到達不能になった `SessionError` と `WorkerCoreError::Session`、ID のみを包んでいた `ReadySession` を削除した。runtime 121件と all-feature library Clippy が成功した |

## 7. 設計メモ

- `Controller::_runtime: Option<ControllerRuntime<...>>` は worker owner の有無であり、`Some` でも
  worker が `Failed` になり得る。公開 lifecycle 全体を `Option` だけから導出しない。
- internal lifecycle と public `LifecycleState` は別型にする。`Configured` と `Closed` は公開 projection、
  live worker は open 済み resource の状態だけを扱う。
- readiness は `ReadinessProgress::Ready(ConnectionSessionId)` として ID を保持し、worker が
  active connection ID と比較した後に lifecycle を `Ready` へ進める。
- session ID wrap は queue に `2^64` session 前の event が残ることを考慮しない。直前 session の
  stale event が wrap 後の ID と一致しないことは検査する。
- protocol timestamp は IMU の elapsed 計算に使うため、wrap ではなく飽和させる。
- public API の削除は observable behavior change である。compiler absence の専用 test は
  `spec/initial/testing.md` の方針に反するため追加しない。

## 8. 対象ファイル

| path | change | 内容 |
|---|---|---|
| `src/runtime/lifecycle.rs` | modify | internal lifecycle 縮小 |
| `src/runtime/worker.rs` | modify | constructor、session、readiness、connection error、時刻処理整理 |
| `src/runtime/readiness.rs` | modify | `ReadySession` 削除と ID 返却 |
| `src/runtime/session.rs` | modify | wrapping session ID |
| `src/runtime/clock.rs` または同等 helper | new / modify | monotonic timestamp 集約 |
| `src/runtime/{direct,periodic,handshake,scheduler,error_map}.rs` | modify | 重複 overflow/error mapping 削減 |
| `src/controller/{mod,create,runtime_tests}.rs` | modify | timeout validation、公開 `try_*` 削除、回帰試験 |
| `src/connection.rs` / `src/lib.rs` | modify | result/status DTO と export 削除 |
| `README.md` / `CHANGELOG.md` | modify | 0.2.0 移行説明と利用例更新 |
| `spec/initial/{api,architecture,testing,migration-strategy}.md` | modify | Intent Delta 反映 |
| `spec/wip/unit_018/RUNTIME_LIFECYCLE_AND_CONNECTION_API_SIMPLIFICATION.md` | new / modify | 作業記録 |

## 9. 検証

| command | result | notes |
|---|---|---|
| `cargo test -p swbt-rs --lib runtime::lifecycle --all-features --locked` | success | T01: 5 passed |
| `cargo test -p swbt-rs --lib runtime:: --all-features --locked` | success | T01: 120 passed |
| `cargo test -p swbt-rs --lib runtime::session::tests::session_id_wraps_to_one_and_rejects_the_previous_max_session --all-features --locked` | success | T02: 1 passed。RED では `SessionError::IdExhausted` で失敗、GREEN で成功 |
| `cargo test -p swbt-rs --lib runtime:: --all-features --locked` | success | T02: 121 passed |
| `cargo test -p swbt-rs --lib controller::runtime_tests --all-features --locked` | success | T01 baseline: 30 passed。T03-T05 後に再実行する |
| `cargo test -p swbt-rs --lib runtime::error_map --all-features --locked` | not run | T04 |
| `cargo fmt --all --check` | not run | final gate |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | not run | final gate |
| `cargo clippy -p swbt-rs --all-targets --no-default-features --locked -- -D warnings` | not run | final gate |
| `cargo test --workspace --all-targets --all-features --locked` | not run | final gate |
| `cargo test -p swbt-rs --all-targets --no-default-features --locked` | not run | final gate |
| `cargo +1.87.0 test -p swbt-rs --all-targets --all-features --locked` | not run | MSRV gate |
| `cargo test --doc -p swbt-rs --all-features --locked` | not run | public rustdoc gate |
| `cargo build --workspace --all-features --locked` | not run | build gate |
| `cargo package -p swbt-rs --locked` | not run | public API/package gate |
| `git diff --check` | not run | final gate |
| hardware / USB / Switch UI | not run | 本 unit は内部構造と software API の整理であり対象外 |

## 10. 先送り事項

- `swbt-core` package 抽出、`swbt` の Bumble backend 必須化、feature 無効分岐と
  `dead_code` 抑制の削除は、合意済み後続 `unit_019` で扱う。
- package version の `0.2.0` 更新と crates.io 公開は、Issue #22 の二 work-unit 完了後の
  release work-unit で扱う。

## 11. チェックリスト

- [x] 対象範囲と対象外を確認した
- [x] TDD Test List を作成した
- [ ] 各 TDD item を個別に検証・commitした
- [ ] 検証結果または未実行理由を記録した
- [ ] package / release / public API に触れる場合の gate を記録した
- [ ] `rust-api-boundary-review`、`rustdoc-style`、`docs-quality-review` を完了した
- [ ] `agentic-self-review` を完了した
- [ ] 完了条件を満たして `spec/complete/unit_018` へ移動した
