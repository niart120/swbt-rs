# create-profile runtime 直線化仕様書

## 1. 概要

### 1.1 目的

`ControllerBuilder::create_profile()`の公開動作を維持したまま、保存済みprofileの即時再読込、工程ごとのprivate typestate、productionでは何もしないpairing hook、複数の`Option`で表したruntime作成状態を削減する。

失敗時のcleanupと成功時の所有権移譲は、transportまたはworkerを現在所有する小さいguardから追跡できる形にする。

### 1.2 起点 / source

| source | 内容 | path |
|---|---|---|
| GitHub Issue | create-profileのtypestateとruntime seamを直線化する | `https://github.com/niart120/swbt-rs/issues/20` |
| user request | Issueの本文・返信、現行実装・テストを確認後、`tdd-workflow`で解消する | 2026-08-02のユーザ指示 |
| initial spec | create-profile順序、profile lifecycle、cleanup契約 | `spec/initial/architecture.md`、`testing.md`、`type-modeling.md`、`migration-strategy.md` |
| completed work | M2のcreate-profile orchestration | `spec/complete/unit_003/M2_RUNTIME.md` |
| completed work | explicit local addressとpersisted identity | `spec/complete/unit_011/EXPLICIT_LOCAL_ADDRESS.md` |

### 1.3 use case

| actor / boundary | 入力または状態 | 期待する観測結果 | 制約 |
|---|---|---|---|
| profile作成利用者 | absentなpathとadapter-defaultまたはexplicit local identity | valid empty profileを保存してからtransportを開き、Ready controllerを返す | 既存targetを置換しない |
| feature無効利用者 | preflightが有効なcreate-profile要求 | profileを作らず`UnsupportedCapability`を返す | builder、path、identity、targetの既存検証順序を維持する |
| runtime失敗利用者 | transport open、worker開始、pairingのいずれかが失敗 | 作成済みprofileを残し、所有中resourceを一度cleanupしてprimary errorを返す | cleanup失敗はrelated errorとして保持する |
| controller所有者 | pairingがReadyまで成功 | worker ownerがcontrollerへ一度だけ移り、明示closeまたはDropが二重cleanupしない | public APIとstatus契約を変えない |

## 2. 対象範囲

- create-profile用の単一orchestratorと検証済み入力
- 生成した`PairingProfile<M>`から保存bytesとruntime configを作る経路
- feature無効時の直接`UnsupportedCapability`返却
- transport open途中とworker所有中のcleanup guard
- `PairDriver`とproductionまで伝播する型引数の削除
- `ControllerRuntimePort`を具体的なworker ownerへ置き換えるcrate-private所有権境界
- create-profile、runtime、profile storeの関連テスト整理
- 現在仕様を表す`spec/initial`のread-back記述更新

## 3. 対象外

- profile JSON schemaと公開`PairingProfile` APIの変更
- profile create-new/updateのatomicity、lock、temporary file方式の変更
- worker loop、command channel、transport実装の大幅変更
- pairing/reconnect/readinessのprotocol意味変更
- public API、`ErrorKind`、error source/related chainの分類変更
- virtual Classic test、probe、hardware runnerの操作手順変更
- 実機pairing、USB access、Switch UI確認

## 4. 関連 docs

- `spec/initial/architecture.md`
- `spec/initial/testing.md`
- `spec/initial/type-modeling.md`
- `spec/initial/migration-strategy.md`
- `spec/complete/unit_003/M2_RUNTIME.md`
- `spec/complete/unit_011/EXPLICIT_LOCAL_ADDRESS.md`

## 5. 振る舞い仕様

| 振る舞い | 入力・状態 | 期待結果 | 備考 |
|---|---|---|---|
| preflight順序 | create-profile要求 | builder、path、identity、target existence、backend capabilityの順で失敗する | target inspectionは予約ではなく、create-newもno-replace |
| empty profile保存 | preflight成功 | `M::KIND`と要求identityを持つ型付きempty profileをtransport open前にcreate-newする | pairing失敗でもprofileは残す |
| 同一型付きprofileの利用 | create-new成功 | 保存bytesを生成した同じ`PairingProfile<M>`をruntime configへ移す | 同じcreate呼出しでは保存直後のread-backを行わない |
| 後続読込 | 後の`build()`、`open()`、`reconnect()` | 保存済みbytesを読込・parse・model検査し、persisted identityを正本にする | create呼出し後の外部変更は後続操作で観測する |
| feature無効 | preflight成功、`bumble`なし | profile/transport side effectなしで`UnsupportedCapability` | 到達不能な`Internal`分岐を作らない |
| partial transport cleanup | transport object作成後、open前・途中の失敗 | bounded drain、disconnect、closeを試行する | panic/早期returnのDrop fallbackはdisconnect、closeを試行する |
| worker cleanup | worker開始後のpair失敗 | worker経由のwithout-neutral cleanupとjoinを行う | terminal workerが既にcleanup済みなら繰り返さない |
| error合成 | primary処理とcleanupの両方が失敗 | primaryを`Error::source()`、cleanupを`Error::related_error()`から辿れる | 秘密値をDisplay/Debugへ出さない |
| Ready移譲 | pairがprotocol Readyまで成功 | worker ownerをReady controllerへ一度だけ移す | 明示closeとDropの既存契約を維持する |

### 5.1 Intent Delta

初期仕様と`unit_003`、`unit_011`は、create-new直後にprofileを再読込し、そのpersisted identityを同じcreate呼出しのruntime configの正本にするとしていた。本unitでは次へ変更する。

- create-newするbytesとruntime configは、同じ生成済み`PairingProfile<M>`から作る。
- `FileProfileStore`のwrite、flush、sync、no-replace公開とread-back可能性はstore単体テストで検査する。
- create-new後からruntime openまでに別writerがtargetを変更した場合、その変更を同じcreate呼出しへ取り込む契約は持たない。
- 後続の`build()`、`open()`、`reconnect()`は従来どおり保存済みprofileを読込み、persisted identityを使う。

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| green | T01 create-profileが保存後のread capabilityを要求せず、保存bytesと同じ型付きmodel/identityをruntimeへ渡し、Ready ownerを一度だけ移譲する | new/regression | profile/controller/runtime integration | adapter-defaultとlocal identity、保存→open→pair順序、Drop一回性 |

### 6.1 TDD cycle evidence

| phase | item | evidence |
|---|---|---|
| red | T01 | `FakeProfileStore`から`ProfileReadPort`を外し、期待イベントから`ReadBack`を削除した。`cargo test -p swbt-rs --lib controller::create_profile_tests --all-features --locked`は、`ProfileCreatePort`が`ProfileReadPort`を要求する`E0277`で失敗した |
| green | T01 | `ProfileCreatePort`をtarget inspectionとcreate-newだけへ狭め、crate-privateなtyped empty profileを保存bytesとruntime configへ移譲した。同じcommandで13 tests passed。adapter-default/local identity、保存→open→pair→Ready、失敗cleanup、controller Drop一回性を確認した |

Tidy decision:

- classification: mixed
- action: split
- reason: read-back削除は観測可能なbehavior changeとして先にcommitし、`PairDriver`、runtime backend/attempt、`ControllerRuntimePort`の削除は同じgreen baselineを使うstructure changeとして後続commitへ分ける

T01をgreenにした後、既存のcreate-profile/runtime契約テストをgreen baselineとして次の構造整理を`refactor-after-green`で分割する。

- `PairDriver`と`RuntimeComponents`のtest専用型引数を、`TransportPort::start_pairing()` / `poll()`で自律するtest transportへ移す。
- `CreateProfileRuntimeBackend`、`CreateProfileRuntimeAttempt`、工程別typestateを単一orchestratorとresource guardへ置き換える。
- `ControllerRuntimePort`とtest tokenを削除し、`ControllerRuntime`が具体的な`WorkerOwner<RuntimeCommand<M, R>>`を所有する。

## 7. 設計メモ

- `PairingProfile<M>`へcrate-privateなempty constructorを置き、型付き値からJSONをserializeする。
- `ProfileCreatePort`はtarget inspectionとcreate-newだけを要求し、read portを継承しない。
- transport objectはworkerへ移すまで専用guardが所有する。期待された失敗では明示cleanupの結果を返し、panic時の`Drop`は失敗を返さないfallbackとする。
- worker開始後のguardはowner transferを表す最小限の`Option`だけを持つ。terminal outcome回収またはReady移譲でdisarmする。
- test pairing scriptはworkerが呼ぶ`TransportPort::start_pairing()`を起点にし、pair command enqueue後のproduction hookを持たない。
- 完了済みwork unitは当時の判断履歴として変更しない。

## 8. 対象ファイル

| path | change | 内容 |
|---|---|---|
| `src/controller/create.rs` | modify | orchestrator、runtime owner、cleanup/error合成 |
| `src/controller/runtime.rs` | modify | runtime open guard、worker owner生成、pairing seam削除 |
| `src/controller/mod.rs` | modify | feature dispatchとcreate-profile入口 |
| `src/controller/config.rs` | modify | 生成済みprofileによるconfig確定 |
| `src/profile/document.rs` | modify | crate-private typed empty profile生成 |
| `src/profile/store.rs` | modify | create/read port責務分離 |
| `src/controller/create_profile_tests.rs` | modify | T01とorchestrator契約test |
| `src/controller/runtime_tests.rs` | modify | transport-driven pairingとcleanup/ownership regression |
| `tests/backend_unavailable_contract.rs` | modify | feature無効時の公開順序契約 |
| `spec/initial/*.md` | modify | create-profile read-backのIntent Delta反映 |
| `spec/wip/unit_016/CREATE_PROFILE_RUNTIME_SIMPLIFICATION.md` | new | 作業仕様、TDD記録、gate |

## 9. 検証

| command | result | notes |
|---|---|---|
| `cargo test --workspace --all-targets --all-features --locked` | success | 着手前baseline。root lib 269 passed / 1 ignored。実機testはignored |
| `cargo test -p swbt-rs --all-targets --no-default-features --locked` | success | 着手前baseline。root lib 253 passed / 1 ignored |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | success | 着手前baseline |
| `rustup run 1.87.0 cargo check --workspace --all-targets --all-features --locked` | success | 着手前MSRV baseline |
| `cargo test -p swbt-rs --lib controller::create_profile_tests --all-features --locked` | success | T01 green。13 passed |
| `cargo test -p swbt-rs --lib controller::create_profile_tests --no-default-features --locked` | success | T01 featureなし。10 passed |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | success | T01 commit前gate |
| `cargo fmt --all --check` | success | T01 commit前gate |
| `git diff --check` | success | T01 commit前gate |

## 10. 先送り事項

- none

## 11. チェックリスト

- [x] 対象範囲と対象外を確認した
- [x] TDD Test Listを作成した
- [ ] T01のRED/GREEN/refactorを記録した
- [ ] create-profileとruntimeの既存契約testを維持した
- [ ] 全検証結果または未実行理由を記録した
- [x] Cargo metadata、release、public APIは変更対象外と確認した
- [ ] `spec/initial`のIntent Deltaを反映した
- [ ] self-reviewを完了した
