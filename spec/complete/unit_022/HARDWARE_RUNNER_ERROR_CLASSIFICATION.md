# Hardware runner error classification 仕様書

## 1. 概要

### 1.1 目的

明示local adapter identityの書換え開始後に最終状態を検証できなかった
`ErrorKind::AdapterIdentityRecoveryRequired`を、hardware runnerの3 scenarioすべてで
`adapter_identity_recovery_required`として証跡へ記録する。通常のadapter open失敗や未知errorと
区別し、物理power-cycleと元identityの確認が必要な状態であることを機械判定できるようにする。

### 1.2 起点 / source

| source | 内容 | path |
|---|---|---|
| GitHub Issue #32 | hardware runnerの共通error mappingからrecovery-required分類が欠落している | `https://github.com/niart120/swbt-rs/issues/32` |
| user decision | package内の最小修正と回帰testで進め、公開共通APIは追加しない | 2026-08-03の対話 |
| public error contract | write開始後にidentityを検証できない場合のtyped recovery boundary | `crates/swbt-core/src/error.rs` |
| existing probe contract | 同じerrorを`adapter_identity_recovery_required`へ分類する実装とunit test | `tools/swbt-probe/src/main.rs`, `tools/swbt-probe/src/tests.rs` |

### 1.3 use case

| actor / boundary | 入力または状態 | 期待する観測結果 | 制約 |
|---|---|---|---|
| hardware runner利用者 | controller操作が`AdapterIdentityRecoveryRequired`で失敗 | `operation_failure.error_kind`が`adapter_identity_recovery_required` | 3 scenarioの共通分類を使う |
| 証跡利用者 | recovery-required failureを含むrun | `runner_complete.success`が`false` | 自動retryや成功扱いにしない |
| 証跡保管者 | errorがmessage、source、related errorを持つ | 分類とrelated有無しか記録しない | selector、address、USB serial、profile pathも出さない |

## 2. 対象範囲

- hardware runnerの共通`ErrorKind`分類へ`AdapterIdentityRecoveryRequired`を追加する
- 現在公開されている既知の`ErrorKind`とrunner分類文字列をtable testで固定する
- `emit_controller_failure()`が生成するeventの分類、related有無、秘密情報非出力をunit testする
- `pro-periodic`固有分類がrecovery-required分類を上書きしないことを確認する
- 3 scenarioの失敗終了判定が`success: false`を維持することを確認する

## 3. 対象外

- backendのadapter identity書込み、read-back、復旧処理
- physical power-cycleの自動化とerror後の自動retry
- evidence schema versionとevent fieldの変更
- `ErrorKind`の追加、削除、公開methodの追加
- hardware runnerとprobeの共通crate、共有公開定数、共通公開mapping API
- hardware runnerの既存scenario固有分類の一般化
- 実機、USB adapter、Switch UIによる再現

## 4. 関連 docs

- `spec/initial/api.md`
- `spec/initial/architecture.md`
- `spec/initial/testing.md`
- `spec/initial/roadmap.md`
- `spec/complete/unit_011/EXPLICIT_LOCAL_ADDRESS.md`
- `spec/complete/unit_015/TOOLING_CRATE_SEPARATION.md`
- `docs/troubleshooting.md`

## 5. 振る舞い仕様

| 振る舞い | 入力・状態 | 期待結果 | 備考 |
|---|---|---|---|
| common classification | `AdapterIdentityRecoveryRequired` | `adapter_identity_recovery_required` | `unknown`へ落とさない |
| common event | 上記kindの`Error` | `operation_failure`に分類名と`related_failure_present`を出す | message、Debug、sourceは出さない |
| related failure | related errorなし / あり | `related_failure_present`が`false` / `true` | related errorの内容は出さない |
| terminal result | 上記errorを受けたrun | `runner_complete.success`が`false` | cleanup eventの有無を変更しない |
| scenario projection | pro-periodic / pro-profile / joycon-profile | すべて共通分類を使用する | pro-periodicの既存`InvalidKeyStore` / `NoBond`上書きは維持 |
| current known kinds | 現在公開中の各`ErrorKind` | 共通mappingで意図したstable stringになる | `#[non_exhaustive]`向けfallbackは残す |

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| refactor-done | T01 recovery-required errorは3 scenario共通の秘密情報を含まない失敗証跡でstable分類され、runを成功扱いにしない | regression / edge | unit / package | 全known kind、related semantics、redaction、probe既存契約との一致を固定した |

status は `todo`、`red`、`green`、`refactor-done`、`refactor-skipped`、`deferred` を使う。

### 6.1 TDD cycle evidence

| phase | item | evidence |
|---|---|---|
| red | T01 | `cargo test -p swbt-hardware-runner current_error_kinds_have_explicit_evidence_names --locked`は、`AdapterIdentityRecoveryRequired`の実値が`unknown`、期待値が`adapter_identity_recovery_required`で失敗した |
| green | T01 | 共通mappingへ当該variantの1 armだけを追加し、同じcommandが成功した |
| refactor-done | T01 | stdout捕捉や実機fault injectionを追加せず、productionのfailure/completion event生成を副作用のない関数へ分離した。分類、relatedなし/あり、message・Debug・source・selector・address・serial・path非出力、`success: false`をunit testし、hardware runner 17 testsとprobe 19 testsが成功した |

## 7. 設計メモ

Tidy decision:

- classification: mixed
- action: after-green
- reason: error分類の追加はbehavior changeである。event生成関数の分離は秘密情報非出力testのためのstructure changeなので、最小のmapping fixをgreenにした後で分けて行う。
- verification: 同じfocused unit testをgreen前後で実行する。

`ErrorKind`はpublicかつ`#[non_exhaustive]`だが、証跡文字列はworkspace-only toolの契約である。
この修正ではpublic `ErrorKind::as_str()`を追加せず、hardware runnerとprobeの既存package-local mappingを
各packageのunit testで同じ期待文字列へ固定する。将来variant向けfallbackはrunnerの`unknown`とprobeの
`operation_failed`で異なるため、当該分類だけを理由に統合しない。

## 8. 対象ファイル

| path | change | 内容 |
|---|---|---|
| `tools/swbt-hardware-runner/src/support.rs` | modify | recovery-required mapping、副作用のないevent生成関数、mapping/event unit test |
| `tools/swbt-hardware-runner/src/scenarios/pro_periodic.rs` | modify | scenario固有mappingがrecovery-requiredを上書きしない回帰test |
| `spec/wip/unit_022/HARDWARE_RUNNER_ERROR_CLASSIFICATION.md` | new / modify | scope、TDD状態、検証結果 |

## 9. 検証

| command | result | notes |
|---|---|---|
| `cargo test -p swbt-hardware-runner --all-targets --locked` | success | 変更前baseline、15 passed |
| `cargo test -p swbt-probe --all-targets --locked` | success | 変更前baseline、19 passed |
| `cargo test -p swbt-hardware-runner current_error_kinds_have_explicit_evidence_names --locked` | expected failure | `AdapterIdentityRecoveryRequired`だけが`unknown`へ落ちるredを確認 |
| focused T01 unit tests | success | known kind tableとsecret-free unsuccessful eventを各1 passed |
| `cargo test -p swbt-hardware-runner --all-targets --locked` | success | 変更後17 passed。3 scenario unitとCLI smokeを含む |
| `cargo test -p swbt-probe --all-targets --locked` | success | 変更後19 passed。既存recovery-required分類testを含む |
| `cargo test --workspace --all-targets --all-features --locked` | success | workspace全target回帰。既存manual / hardware testのignoreを除き失敗なし |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | success | workspace lint gate、warningなし |
| `cargo fmt --all --check` | success | Rust formatting |
| `git diff --check` / `git diff --check main...HEAD` | success | working tree / branch差分のwhitespace検査 |
| `cargo build --workspace --all-features --locked` | not applicable | Cargo metadata、feature、公開APIを変更しておらず、workspace testが全targetをbuildした |
| hardware / USB / Switch UI | not applicable | package内の分類とJSON event生成だけの変更。synthetic `Error`を使うunit testで検証した |

## 10. 先送り事項

- none

## 11. Self Review

### 11.1 Findings

| severity | finding | evidence | disposition |
|---|---|---|---|
| none | Issue #32の分類、終了結果、秘密情報非出力を満たす | known kind table、failure/completion event unit test、3 scenario source review | 対応完了 |
| none | 公開API、Cargo metadata、schema、公開文書への変更はない | `git diff main...HEAD` | 既存境界を維持 |
| none | scope外のbackend、retry、power-cycle処理へ変更がない | branch差分3 files | 対象外を維持 |

### 11.2 Review Gates

| gate | result | evidence |
|---|---|---|
| Requirements / Scope | pass | Issue #32、対象範囲、対象外とbranch差分が一致 |
| TDD / Tests | pass | T01のred / green / refactor記録、focused / package / workspace test成功 |
| Static | pass | fmt、clippy、diff check成功 |
| Package | not applicable | manifestと公開package behaviorの変更なし |
| Rust API | pass | public item、error variant、所有権、async、feature、unsafeの変更なし |
| Docs Quality | pass | `unit_022`に事実、未実行範囲、完了条件を記録。参照先存在と仮テキスト残りを確認 |
| Integration Review | pass | 3 scenarioが共通emitterを使い、pro-periodic固有mappingも当該variantを共通分類へ渡す |

対象範囲内に残るtest gapはない。実機でrecovery-required errorを再発生させる操作、USB power-cycle、
Switch UI観測は製品backendを変更しない分類修正の対象外であり、実行していない。

## 12. チェックリスト

- [x] 対象範囲と対象外を確認した
- [x] TDD Test Listを更新した
- [x] T01をred / green / refactor / commitした
- [x] 3 scenarioの共通分類を確認した
- [x] 秘密情報非出力とrelated semanticsを確認した
- [x] 検証結果または未実行理由を記録した
- [x] package / release / public APIに触れないことを確認した
- [x] docs-quality-reviewとagentic-self-reviewを完了した
- [x] `spec/complete/unit_022`へ移動した
