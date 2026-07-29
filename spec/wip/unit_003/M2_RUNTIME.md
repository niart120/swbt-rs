# M2: typed runtime と profile frontend

- 状態: **着手中**
- branch: `feat/unit-003-m2-runtime`
- 初期設計の正本:
  - `spec/initial/roadmap.md` M2
  - `spec/initial/architecture.md` builder、worker、reporting、protocol、transport、lifecycle
  - `spec/initial/api.md` controller、builder、lifecycle、入力操作、status、error
  - `spec/initial/testing.md` builder/profile lifecycle、runtime integration、concurrency、timing
  - `spec/initial/migration-strategy.md` reporting semantics と resource scope
- Python 基準断面: `niart120/swbt-python@84d2723b127f70fc78e12f4496f5c40af0ccfb0a`
- 前提: `spec/complete/unit_002/M1_PROTOCOL.md`
- 最終更新: 2026-07-29

## 1. 目的

M1 の純粋 protocol component を、model と reporting mode が型で固定された同期 runtime
へ接続する。`Controller<M, R>` は bounded command channel と 1 本の worker thread を通して
操作し、worker が connection session、入力状態、report timer、scheduler、transport port を
単独所有する。

M2 では crate-private の fake transport、fake monotonic clock、fake profile store を使い、
実 USB adapter や wall-clock sleep なしで runtime semantics を検証する。Bumble transport は
M3 で同じ port へ接続する。

## 2. Intent Delta

| 境界 | 現在 | M2 完了後 | 保証 |
|---|---|---|---|
| `Controller<M, R>` | 型 identity だけを持つ | validated config、worker client、join、status/snapshot projection を持つ | `Send`、非 `Sync`、公開操作は `&mut self` |
| `ControllerBuilder<M, R>` | field と method がない | adapter、profile path、colors、Periodic period を型付きで検証する | `build()` は transport/worker を開始しない |
| worker | 不在 | `ControllerWorker<M, R>` が 1 thread で command、transport event、deadline を処理する | runtime state を複数 thread から mutate しない |
| report send | pure bytes 候補まで | `ReportSender<M>` が input/reply の唯一の interrupt send 経路になる | acceptance 後だけ timer、IMU、session、Direct state を commit |
| Periodic | marker だけ | local state 先行 commit、absolute deadline、overrun skip、300 ms reply holdoff | missed tick を burst 送信しない |
| Direct | marker だけ | connected 時の caller-driven send transaction | rejection 前は snapshot 不変、accepted 後の disconnect は commit 済み |
| connection session | protocol 値型だけ | monotonically increasing ID、neutral reset、stale event discard、observed subcommand | disconnect/reopen 間で protocol/runtime state を共有しない |
| profile frontend | kind/colors だけ | raw envelope、`PairingProfile<M>`、existing/ephemeral build、create-new orchestration | model mismatch と path error は transport open 前 |
| status | 不在 | lifecycle と counters を read-only snapshot として公開 | `M::KIND` / `R::KIND` から導出し worker I/O を待たない |

## 3. 対象範囲

- generic `Controller<M, R>` / `ControllerBuilder<M, R>` の同期 blocking API
- crate-private `ControllerWorker<M, R>`、typed command、bounded queue、join
- crate-private `TransportPort` / `TransportEvent` / `SendAcceptance`
- crate-private fake transport と event log
- monotonic clock と deterministic fake clock
- `InputStateStore<M>` と status/snapshot projection
- `ReportSender<M>` による `0x30` / `0x21` の直列化
- output parse、rumble observation、subcommand observation、reply send
- acceptance 後の timer、IMU encoding state、protocol session commit
- Periodic scheduler、8 ms default、validated `1 ms..=1 s` period
- Direct acceptance transaction
- common `press` / `release` / `tap` / `neutral` と reporting 固有 `apply` / `send`
- lifecycle の configured/open/connecting/ready/closing/closed/failed
- open/close/reopen、session reset、stale event、worker failure/backpressure
- raw profile envelope と `PairingProfile<M>` の model 検査
- builder の ephemeral/existing build と crate-private backend を使う create-profile orchestration
- 6 model × reporting 組み合わせの共通 fake-runtime harness
- M1 の一時 `dead_code` 属性撤去
- activity wait 方式の測定と選択

## 4. 対象外

- Bumble、USB/HCI、adapter discovery、`ExternalHost` の実装: M3
- SDP、HIDP、L2CAP、virtual Bluetooth link: M4
- 実 adapter と Switch を使う pairing、normal-input readiness: M5
- Python profile の lossless key compatibility、unknown field preservation の完成、atomic replace、
  lock contention、reconnect: M6
- Joy-Con の virtual/hardware runtime evidence: M7
- stable diagnostics event schema、long-run IMU、probe、実機 ACK 観測: M8
- crates.io package blocker、release/rollback: M9
- public custom transport/clock/profile-store injection
- `AnyController`、model 非依存 input、raw HID/HCI API
- controller method の `create_profile()`

M2 の `create_profile()` 成功は crate-private fake backend に対する orchestration 成功であり、
実 adapter や Switch で pairing できることの根拠にはしない。public backend は M3/M5 の
実装前に成功を捏造せず、利用不能な capability を構造化 error で返す。

## 5. 振る舞い仕様

### 5.1 transport acceptance

- `TransportPort` は model 非依存で、NX の `Button<M>` / `InputState<M>` を受け取らない。
- interrupt send の `Ok(SendAcceptance)` は local queue / L2CAP path の受理を表す。
- air delivery、completed packet、Switch UI 反映は成功条件に含めない。
- fake transport は accepted、rejected、accepted-then-disconnect を別の script outcome として表す。
- `close()` は冪等。send rejection 後も cleanup を試みる。
- send / poll / fake event injection は open 前と close 後に `TransportErrorKind::Closed` を返す。
  repeated open は最初の notifier を保持して success とする。
- control / interrupt input は model 非依存 event として別 channel identity を保つが、同じ
  runtime output handler へ渡す。
- event queue は bounded とする。source termination より前に受信した event は FIFO 順で
  返してから `SourceTerminated` を返す。event 欠落が発生した queue overflow は
  `EventQueueOverflow` を即時かつ継続して返す。
- terminal 後の send / disconnect / fake event injection は同じ terminal error を返す。
  `close()` は terminal 後も実行できる。
- port は worker の coalescing wake notifier を登録できる。別 thread は wake だけを送り、
  transport object と event queue の drain は worker が単独所有する。worker は 1 回の wake
  で `poll(Duration::ZERO)` が空になるまで有限 batch を繰り返し、次の activity で再度 wake
  される。
- `poll(Duration::ZERO)` は block せず、current queued event を有限 batch で返す。
- transport error の `Debug` / `Display` に pairing key や profile 全文を含めない。

### 5.2 ReportSender と protocol commit

- `0x30` input と `0x21` reply は同じ `ReportSender<M>` を通り、accepted sequence を共有する。
- bytes、candidate timer、candidate IMU state、candidate session は send 前に計算する。
- transport が受理した場合だけ candidate timer と IMU/session state を同時に commit する。
- reply prefix は pre-transition session が ready なら reply 構築直前の committed
  `InputState<M>`、ready でなければ neutral state を使う。player lights reply 自身で
  ready になる場合も、その reply の prefix は neutral とする。
- `0x40` の reply を受理して IMU mode を commit する前に、新 mode の `0x30` を送らない。
- accepted reply の後は Periodic automatic input を monotonic 300 ms hold off する。
- send rejection では timer、IMU/session state、holdoff を変更しない。
- accepted sequence の代表 checkpoint は input timer `0`、reply timer `1`、次 input timer `2`
  とする。rejection 後も timer、session、committed IMU state は不変とする。retry の
  quaternion bytes は fake time も同じ場合だけ一致し、時刻が進んだ場合は current committed
  state と新しい `now_ns` から再計算する。
- input/reply accepted counter は transport acceptance 後だけ増やす。
- Direct explicit input と trailing neutral は Periodic automatic holdoff の対象外。

### 5.3 output observation

- parse 済み subcommand ID は reply preparation より先に current session へ記録する。
- semantic error、unsupported subcommand、reply send rejection でも記録を戻さない。
- malformed output は subcommand ID を捏造しない。
- `0x10` rumble-only は reply を送らない。
- control / interrupt のどちらから届いても同じ parser と observation 順を使う。
- rumble-only と malformed output は timer、protocol session、accepted counter を変更しない。
- production worker は `OutputReport` の借用中に output observation、subcommand observation、
  `ReportSender::prepare_reply()`、`send_reply()` を同じ event 処理内で完了し、request を worker
  iteration を越えて保存しない。M1 互換テスト用の `OutputPreparation<'a>` と
  `prepare_output_report()` は `cfg(test)` に限定する。

### 5.4 Periodic

- `apply()` と common input helper は validation 後に local `InputState<M>` を先に commit する。
- 未接続でも local state を更新できる。
- wire failure で local stateを rollback しない。
- protocol readiness と reply holdoff の終了後だけ automatic input を送る。
- deadline は monotonic absolute time で進める。
- overrun では過去 tick をまとめて送らず、未来の最初の deadline へ進める。
- tick ごとに committed latest state を 1 件だけ snapshot して送る。
- disconnect 中は停止し、新 session では neutral から再開する。
- connection 前または session 間の `apply()` は local snapshot へ反映するが、新 connection
  の neutral reset がそれを上書きする。これは stale input を新しい Switch connection へ
  持ち越さない Rust 固有の安全契約とする。

### 5.5 Direct

- `send()` と common input helper は Ready connection を要求する。
- candidate state の `0x30` が受理された後だけ snapshot を commit する。
- validation error または send rejection では previous snapshot を維持する。
- accepted 後に disconnect event が到着した場合、その send は commit 済みとする。
- user-input periodic scheduler は持たない。
- `tap()` は press と release を同じ worker-owned transaction として処理する。
- tap duration は `0..=24 h`。境界値 0 と 24 h を受け付け、超過は send 前に
  `InvalidInput` とする。
- release rejection では最後に受理された pressed state を維持する。
- tap duration 中も worker は inbound output と subcommand reply を処理する。Direct input
  operation は release 完了まで直列化するが、sender 全体を deadline まで占有しない。
- public `Controller` は非 `Sync` かつ `tap(&mut self)` が blocking なので、同じ public
  handle から concurrent `close()` を発行できるとは扱わない。pending tap は worker
  deadline として保持し、transport disconnect、worker shutdown、crate-private priority
  close で中断する。M2 で public cancellation handle は追加しない。

### 5.6 handshake と Ready

- new connection は link、HID control channel、HID interrupt channel が同じ session ID で
  そろうまで handshake を開始しない。
- 両 channel 後に bootstrap neutral `0x30` を直ちに送る。最初の parse 済み subcommand
  まで 1 秒 absolute deadline で再試行し、同じ deadline の missed retry を burst しない。
- Ready には supported `0x03` report mode reply と non-zero `0x30` player lights reply の
  transport acceptance が必要。reply preparation だけでは Ready にしない。
- stateful reply の acceptance、handshake state 回収、session ID 再確認の後だけ Ready を公開する。
- Periodic は最後の accepted reply から 300 ms holdoff が終わり scheduler を開始した時点、
  Direct は protocol ready 直後に Ready とする。Direct は確認用 periodic input を送らない。
- reply rejection、Ready 前 disconnect、operation timeout、stale session は Ready にしない。

### 5.7 lifecycle と session

- `build()` 後は `Configured`。profile read/validation 以外の I/O、worker start、transport open は行わない。
- `open()`、`close()`、`close_without_neutral()` は冪等。
- `close()` は connected なら trailing neutral の受理を試み、失敗しても残りの cleanup と join を続ける。
- `close_without_neutral()` は trailing neutral を送らない。
- close は既に処理中の reply/input を完了し、trailing neutral、bounded drain、disconnect、
  transport close、worker completion、join の順に進める。reply/neutral rejection でも
  disconnect、close、join を続ける。
- explicit close は worker completion を受け取った後に join する。
- `Drop` は priority shutdown と wake を送り、pairing、neutral、drain を行わない。worker
  completion channel を bounded time だけ待ち、completion 後だけ join する。期限内に
  completion しない faulty port の handle は detach し、M3 port contract の failure として
  扱う。正常 fake/backend では completion と join を検証し thread を残さない。
- 各 new connection に increasing session ID を割り当て、protocol session、timer、holdoff、
  observed subcommand、committed input を neutral baseline へ戻す。
- session ID が current と一致しない transport event は破棄する。M2 の public status に
  stale counter は追加せず、test support の observation と M8 diagnostics で検査する。
- close 開始後の new input command は `Shutdown` として拒否する。

### 5.8 command、failure、snapshot

- command channel は bounded。full の場合は unbounded wait せず `Busy` を返す。
- pair は reporting 固有 input の外側にある typed command として同じ bounded channel と
  one-shot response を使う。worker が pair を受理してから Ready、timeout、disconnect、
  shutdown のいずれかを確定するまで、その response を先頭に保持して後続 input を dequeue
  しない。Ready status を公開した後だけ pair response を成功させる。
- report tick は command queue に積まず worker scheduler が所有する。
- public blocking call は worker response、worker termination、operation timeout のいずれかを観測する。
- worker panic / unexpected termination は `WorkerFailed` へ変換し、join を回収する。
- status は短い read lock だけで返し、transport poll や command response を待たない。
- snapshot は model-typed `InputState<M>` を返す。
- controller kind と reporting kind は `M::KIND` / `R::KIND` から毎回投影し、重複 field を持たない。
- `GamepadStatus` の更新元:
  - `lifecycle`: worker state machine
  - `connected`: current session の link/channel state
  - `controller_kind` / `reporting_kind`: `M::KIND` / `R::KIND`
  - `report_mode`: accepted current protocol session
  - `input_reports_accepted`: accepted `0x30`
  - `replies_accepted`: accepted `0x21`
  - `last_subcommand`: parse 済み ID。semantic/send failureでも保持
  - `last_disconnect_reason`: current session の最後の disconnect
  - `worker_failure`: terminal error または panic。profile/key material を含めない
- error mapping は queue full=`Busy`、closing command=`Shutdown`、Direct の非 Ready send=
  `TransportClosed`、worker channel termination/panic=`WorkerFailed`、backend 未実装=
  `UnsupportedCapability` とする。
- M2 で追加する recoverable category は `ProfilePathRequired`、`ProfileNotFound`、
  `ProfileAlreadyExists`、`InvalidProfile`、`ProfileControllerMismatch`、`TransportClosed`、
  `ConnectionTimeout`、`ConnectionFailed`、`Protocol`、`UnsupportedCapability`、`Busy`、
  `WorkerFailed`、`Shutdown`、`Internal`。Bumble 型を public variant にせず、profile I/O、
  transport、worker source は `Error::source()` chain に保持する。

### 5.9 profile frontend

- M2 の raw envelope は `format = "swbt.profile"`、`schema_version = 2`、
  `controller_kind`、identity、key-store object を持つ。
- raw document を `PairingProfile<M>` に変換するとき `controller_kind == M::KIND` を検査する。
- `build()` の profile path なしは ephemeral controller。
- existing path は read/parse/model validation を行うが、worker と transport は作らない。
- nonexistent path は `ProfileNotFound`、model mismatch は `ProfileControllerMismatch`。
- `create_profile()` は path 必須、existing target を上書きしない。
- create-profile preflight は builder、path、identity、target existence の順で検査する。
  backend capability は target validation 後かつ create-new 前に検査する。
- target existence の事前検査は競合時の no-replace 保証ではない。create-new 自体も既存競合を
  `ProfileAlreadyExists` として拒否する。
- empty envelope create-new と typed reopen は transport open より先。
- runtime backend は resource 未取得の attempt を返し、orchestrator が open、pair、失敗時
  cleanup、成功時の Ready 所有権移譲まで同じ attempt を保持する。
- fake open/pair failure でも valid な empty adapter-default envelope は残し、
  `cleanup_without_neutral` を1回実行して worker/transport resource を解放する。T32 の fake が
  検査するのは without-neutral 経路の選択と一回性である。T20 の explicit
  close-without-neutral と同じく neutral だけを省略して bounded drain を残す実 worker 接続は、
  T33 の完了条件とする。
- attempt の早期 Drop は fallback cleanup を1回実行する。明示 cleanup と Ready 所有権移譲は
  fallback を解除する。実 worker の bounded Drop timeout/detach は T35 で検証する。
- concrete attempt の cleanup 自身が失敗した場合は、primary と cleanup の source を利用者が
  辿れる error 境界を T33 で決める。primary は従来どおり `Error::source()` に保持し、
  同じ操作で生じた cleanup は `Error::related_error()` から別の `Error` として辿れるように
  する。cleanup と join が両方失敗した場合も同じ related chain の順序を固定する。非公開
  field に保持するだけ、または cleanup を primary の原因として `source()` に直列化する
  だけでは完了扱いにしない。
- fake success は `Ready` controller を返す。
- `ProfileIdentity::LocalAddress` は M5 gate 前なので transport open 前に `UnsupportedCapability`。
- `LocalAddress` は `XX:XX:XX:XX:XX:XX` の6 octetをdisplay順で保持し、individualかつ
  locally administeredな値だけを受理する。下位3 octetをbig-endian LAPとして
  `0x9E8B00..=0x9E8B3F` はreserved inquiry LAPなので拒否する。public constructorの違反は
  `InvalidInput`、profile JSON内の違反は`InvalidProfile`とし、`Debug`は全octetを伏せる。
- T30 は `pair_timeout` に未規定の範囲制限を追加せず、T31のpairing境界へそのまま渡す。

M2 は empty envelope と orchestration に必要な shape を固定する。Python key field の完全な parse、
unknown field の lossless round-trip、atomic replace、lock は M6 で完成させる。

### 5.10 M2 で追加する public surface

- `Controller<M, R>::builder(...)`
- builder の `profile_path()`、`controller_colors()`、`build()`、`create_profile()`
- Periodic builder の `report_period()`
- opaque string newtype の `AdapterSelector`
- `LifecycleState`、`GamepadStatus`、`CreateProfileOptions`、`ProfileIdentity`、`LocalAddress`
- `open()`、`pair()`、`status()`、`close()`、`close_without_neutral()`
- `press()`、`release()`、`tap()`、`neutral()`、`snapshot()`
- Periodic の `apply()` / `report_period()`、Direct の `send()`

M2 の通常 build に concrete transport backend はまだない。public `open()` / `pair()` は
`UnsupportedCapability` を返し、worker thread と file を残さない。public
`create_profile()` は backend capability を target create-new より先に検査し、M5 までは
file を作らず `UnsupportedCapability` を返す。crate-private backend injection を使う同じ
orchestrator の test だけが empty envelope、fake open/pair、Ready return を検証する。

`reconnect()`、`connect()`、`try_reconnect()`、`try_connect()` と bond semantics は M6
まで public surface に追加しない。adapter discovery は M3 へ残す。T34 の3入口は
backend availability を返す M2 の契約であり、後続 milestone の未実装 method を常に失敗する
stub として先に公開する方針ではない。

### 5.11 Python 基準断面との差分

次は未検証の偶発差ではなく、Rust 初期設計を優先する意図的差分として専用 test にする。

- Python は quaternion IMU encoding state を transport send 前に更新するため、rejected input
  でも state が進み得る。Rust は M1/M2 の candidate 境界に従い acceptance 後だけ commit する。
- Python は connection 前に Periodic で commit した state を最初の normal input へ使う。
  Rust は new connection ごとに neutral reset し、stale input を別の Switch session へ
  自動持ち越ししない。
- Python の bootstrap retry は各 send 完了後から 1 秒を待つ。Rust は scheduler と同じ
  absolute deadline を使い、send latency による drift と missed retry burst を避ける。
- Python の asyncio task/lock ordering は互換対象にせず、observable report order、
  acceptance、committed state、readiness を互換対象にする。

Periodic tap の release send rejection では local released state を維持する。Direct だけが
最後に accepted された pressed state を維持する。この差は Python 基準断面と Rust reporting
semantics の両方に一致する。

### 5.12 runtime causal fixture

M1 の pure protocol fixture は変更せず、M2 は
`tests/fixtures/python-v0.6.0/runtime/runtime-semantics.json` を別に固定する。fixture は
async task の scheduling order や wall-clock 値ではなく、scripted acceptance、fake-clock
time、accepted report ID/timer、committed input、session、holdoff の causal checkpoint を持つ。

最低 case set:

- shared timer の input→reply→input
- pre-ready neutral reply prefix と ready 後 current-state prefix
- Direct acceptance / rejection
- in-flight old-mode input→`0x40` ACK→new-mode input
- rejected `0x40` 後の timer/session/holdoff/observed/new-mode input
- Periodic pre-connection update と disconnect callback の neutralize
- Periodic / Direct release rejection
- rejected quaternion input 後の Python session observation
- bootstrap send latency を含む Python retry deadline

parity case は Rust expected と直接比較する。new-session reset、rejected IMU commit、
absolute bootstrap retry は `baseline_observation` として保持し、5.11 の
`rust_spec_delta` test が意図的な差と根拠を検査する。worker panic、queue full、
stale event、close race は Python fixture に入れず、Rust の deterministic test に置く。

## 6. TDD Test List

| state | 振る舞い | test level | red / green / refactor evidence |
|---|---|---|---|
| refactor-skipped | T01: Python 基準断面から runtime causal fixture を固定し、source provenance と case set を audit する | characterization | red: `cargo test --test runtime_fixture_audit --locked` は fixture 未作成の `include_str!` error。green: 同 command で 3 tests passed。固定 Python 3.13 / commit / tree を検査する生成器が production fake runtime を実行し、13 causal cases を生成。audit は case 分類、再生に必要な step、主要因果値を固定。接続後の bootstrap 受理を因果 gate にして task scheduling を期待値から除外。連続生成 SHA-256 `A50E11EF251B29A17F89C06E9C59640C9268D7F98B014A6D4ED21E3DFF72F118` 一致。生成器と audit の責務が分離済みのため refactor-skipped |
| refactor-done | T02: model 非依存 transport contract が open/send/poll/routing/wake/overflow/terminal error/repeated close を検査する | transport contract | red: `cargo test --lib runtime::transport::tests --locked` は未定義 transport API の compile error。green: 同 command で 6 tests passed。refactor: lifecycle と notifier を同じ lock で管理して close と injection/send/termination を直列化。source termination 前の FIFO event、即時 sticky overflow、terminal 後の操作拒否と close、wake 後の有限 batch drain と再通知、sanitized source chain を明示 |
| refactor-skipped | T03: connection-local observed subcommand set が重複を除き、protocol candidate commit と独立して reset できる | runtime unit | red: `cargo test --lib runtime::connection::tests --locked` は `ObservedSubcommands` 未定義の compile error。green: 同 command で 2 tests passed。全 256 ID を検査する bit set が重複を除き、明示 reset 後に同じ ID を再観測できる。型は `ProtocolSession` や candidate outcome を保持せず observation の rollback 経路を持たない。実装が単一責務のため refactor-skipped |
| refactor-done | T04: protocol facade が state/session/time から `0x30` bytes、next timer、next IMU state を返す | protocol unit | red: `cargo test --lib protocol::tests::facade::input_preparation --locked` は `prepare_input_report` 未定義の compile error。green: 同 command で 2 tests passed、既存を含む facade 10 tests passed。disabled/quaternion の state、committed session、timer、`now_ns` を合成し、同一入力は同じ candidate、異なる時刻は異なる IMU candidate を返して current session を変更しない。refactor: `OutputPreparation` と対になる `InputPreparation`、`next_imu_encoding_state()` に命名を統一 |
| refactor-done | T05: `ReportSender<M>` が input/reply のtimer sequenceとacceptance後candidate commitを共有し、rejection後はcommitted stateを保ってretry時刻から再準備する | runtime unit | red: `cargo test --lib runtime::sender::tests --locked` は `ReportSender` 未定義の compile error。green: 同 command で 2 tests passed。accepted `0x30 → 0x21 → 0x30` の timer `0 → 1 → 2`、stateful reply rejection 前後の commit 不変、accepted retry 後の session commit、quaternion input rejection 後の timer/session 不変と新しい `now_ns` の再準備、accept 後の disconnect event で commit を戻さないことを検査。refactor: timer/session candidate を `SenderCommit` に束ね、transport acceptance 後の一括代入を共通 helper へ集約 |
| refactor-done | T06: reply prefix が pre-transition readiness に基づき neutral/current を選ぶ | runtime unit | red: `cargo test --lib runtime::sender::tests::reply_prefix --locked` は `ReportSender::prepare_reply` 未定義の compile error。green: 同 command で 1 test passed、sender 全 3 tests passed。report-mode だけを受理した未 Ready session、Ready にする player-lights reply の rejection と retry では neutral prefix を使い、retry 受理後に新しく準備した reply は current input prefix を使う。rejection 前後で timer/session は不変。refactor: sender は raw report を解析せず `SubcommandRequest` を受け取り、T07 の parse → observe → prepare 順を保ったまま pre-transition の committed session だけで prefix を選ぶ |
| refactor-done | T07: output handler が control/interruptを同じ順で処理し、subcommandをprepare前に記録し、malformed/rumble-onlyで状態を捏造せず、T03 observation の module-wide `dead_code` 許可を未接続 method 単位へ狭める | runtime unit | red: `cargo test --lib runtime::output::tests --locked` は `OutputHandling`、`OutputHandlingError`、`handle_output` 未定義の compile error。green: 同 command で 2 tests passed。control / interrupt の同一ケース列で raw rumble の channel、report ID、packet ID、8 bytes を semantic error、unsupported subcommand、send rejection より前に観測し、subcommand ID 観測と accepted retry も検査。truncated `0x01` は output/subcommand observation を作らず、`0x10` は raw rumble だけを観測し、どちらも timer、session、interrupt send を変更しない。refactor: parser → output observation → subcommand observation → preparation → send を1関数に固定し、1イベントだけ借用する `OutputHandlingContext` と protocol / transport source を保持する typed error に分離。connection module 全体の `dead_code` 許可を削除し、未接続の `reset()` は T17、`is_empty()` は T18、output handler は T21 までの項目単位の許可へ狭めた |
| refactor-done | T08: in-flight old-mode inputを許しつつ、accepted `0x40` ACKが最初のnew-mode inputより先になる | runtime integration | red: `cargo test --lib runtime::output::tests::in_flight_old_mode --locked` は test-only fake の `AcceptedThenEvent` 未定義の compile error。green: 同 command で 1 test passed、output handler 全 3 tests passed。old-mode input の transport 受理中に interrupt channel の `0x40 / 0x02` event を queue し、input 完了時は timer 1 / IMU disabled、event 処理後は accepted ACK で timer 2 / IMU enabled、その後の最初の input で timer 3 / IMU timestamp 0 を検査。accepted wire 順は `(0x30, 0, zero IMU) → (0x21, 1, subcommand 0x40) → (0x30, 2, non-zero IMU)`。refactor: fake の accepted / accepted-then-disconnect / accepted-then-event を acceptance 記録後の optional event queue へ集約。production API、thread、外部化した input candidate は追加していない |
| refactor-skipped | T09: Periodic state store が未接続でも先行commitし、wire failureでrollbackせず、new sessionでneutralへresetする | runtime unit | red: `cargo test --lib runtime::state::tests --locked` は `InputStateStore` 未定義の compile error。green: 同 command で 3 tests passed。transport を作る前の非 neutral commit、scripted send rejection 後も同じ snapshot を維持して sender timer が 0 のままであること、connection 前と session 間に設定した state を neutral reset が上書きすることを検査。refactor: store は worker が単独所有する `InputState<M>` 1 field と clone / move / neutral 代入だけで、同期、接続判定、rollback、reporting policy を持たないため構造変更を省略。T11 で型と `snapshot()`、T12 で `commit()` の一時 `dead_code` を削除し、未接続の `new()` は T21、`reset_to_neutral()` は T17 の理由を method 単位で残した |
| refactor-skipped | T10: Periodic scheduler が8 ms absolute deadline、overrun skip、no burstをfake clockで計算する | scheduler unit | red: `cargo test --lib runtime::scheduler::tests --locked` は `ReportScheduler`、`SchedulerError`、`TickDecision` 未定義の compile error。green: 同 command で 3 tests passed。100 ms 起点の初回 deadline 108 ms、110 ms の late tick 後も `now + period` の 118 ms ではなく absolute phase の 116 ms、132 ms overrun では `Due { skipped: 3 }` を1件だけ返して次を140 msとし、同じ時刻の再評価が `NotDue` になることを fake monotonic clock で検査。zero period と初期/更新 deadline overflow は typed errorを返し、wrapせず更新失敗時の deadline を維持する。refactor: scheduler は clock、sender、state、transportを所有せず、2つの `Duration` と O(1) の checked deadline 計算だけで単一責務のため構造変更を省略。T11 で Periodic policy へ接続して一時 `dead_code` を削除。T21/T22 のworker/wait、T27 の既定値/範囲検証は未着手 |
| refactor-done | T11: Periodic runtime がlatest stateだけを送り、accepted replyの300 ms holdoffを守り、rejected replyではholdoffしない | runtime integration | red: `cargo test --lib runtime::periodic::tests --locked` は `PeriodicPolicy`、`AutomaticInput`、`PeriodicResult` 未定義の compile error。green: 同 command で 3 tests passed。100 ms周期で accepted `0x40` reply後は299 msまで `HeldOff`、holdoff中にlocal stateをAからBへ更新し、300 ms境界でBの`0x30`を1件だけ送る。accepted wire順は `(0x21, timer 0, subcommand 0x40) → (0x30, timer 1, button B)`、同じ300 msの再評価は`NotDue`、次deadlineは400 ms。rejected `0x40` replyではholdoffを設定せずdeadline 100 msとtimer 0を保ち、その境界でbutton Aの`0x30`を送る。accepted replyを0/100 msで受理した場合は最後のcompletionから400 msまで延長し、reply timer 0/1の後にinput timer 2を送る。refactor: model非依存のpolicyがscheduler/holdoffだけを所有し、state/protocol/sender/transportをdue時に借用する境界へ分離。T09/T10の接続済み一時`dead_code`を削除し、scheduler errorをsource保持型にした。Ready判定はT19、session resetはT17、worker/wait接続はT21/T22に残した |
| refactor-skipped | T12: Direct sendがacceptance後だけcommitし、rejectionでは不変、accepted-then-disconnectはcommit済みになる | runtime unit | red: `cargo test --lib runtime::direct::tests --locked` は `send_candidate` 未定義の compile error。green: 同 command で 2 tests passed。accepted button Aは`0x30 / timer 0`受理後にlocal snapshotへcommitし、続くbutton X rejectionではsnapshot Aとsender timer 1を維持、同じXのretry acceptance後にsnapshot Xとtimer 2へ進む。accepted wireはA/timer 0とX/timer 1の2件だけ。`AcceptedThenDisconnect` はsend成功直後にsnapshotとtimerをcommitし、後続`poll(Duration::ZERO)`でdisconnect eventを観測した後も戻さない。refactor: Direct固有stateを作らず、owned candidateをsend中だけ借用してacceptance後にstoreへmoveするcrate-private関数1つで完結しているため構造変更を省略。validation/helperはT13、session resetはT17、Ready判定はT19、worker/event接続はT21に残した |
| refactor-skipped | T13: press/release/neutralのtyped state変換がempty inputを拒否し、tap duration 0/24 h/超過を検証してreporting別commit入口へ渡る | controller unit | red: `cargo test --lib controller::input::tests --locked` は candidate、tap plan、Periodic commit入口未定義の compile error。green: 同 command で5 tests passed、`cargo test --lib runtime::direct::tests --locked` で2 tests passed。pressは重複を除いた和集合、releaseは差集合としてstick/IMUを保持し、既押下pressと未押下releaseをno-opとして受理する。press/release/tapのempty iteratorは`InvalidInput`、tapは0と24 hを受理し24 h + 1 nsをsend前に拒否する。owned candidateはPeriodicの即時local commit入口とDirectの既存acceptance後commit入口へ渡り、`ReportingKind`のruntime分岐を追加していない。refactor: pure candidate生成、private `TapPlan`、reporting別入口が既に分離され、Ready、send順、delay/cancellationはT14/T15に残したため構造変更を省略 |
| refactor-done | T14: Periodic tapが非Readyではpress前に拒否され、Readyではpress/releaseをlocal commitし、両send失敗でもrollbackせず最初のerrorを返す | runtime unit | red: `cargo test --lib runtime::periodic::tests::periodic_tap --locked` は `begin_tap` と `PeriodicError::NotReady` 未定義の compile error。green: 同 command で2 tests passed、Periodic全5 tests passed。非Readyではstore snapshot、sender timer/session、transport試行とfailure scriptを変更しない。Readyでは初期`[A, ZL]`とtap対象`[A, B]`からpress`[A, B, ZL]`、release`[ZL]`を各send前にlocal commitし、`SendRejected`と`Closed`の両失敗でも2 payloadを試行してrelease stateを維持し、先頭の`SendRejected`を返す。validated durationは`TapPlan`が保持し、readinessはT19が接続する明示引数、absolute deadline/waitはT21/T22へ残した。refactor: `commit_and_send_candidate`へlocal commit→snapshot→send順を集約した。T21 接続時はpress前にabsolute release deadlineとnanosecond上限を検査し、先頭errorをcommand completion、後続のterminal errorをworker停止理由として別に保持する |
| refactor-skipped | T15: Direct tapがpress rejection後はreleaseせず、accepted press後はinbound replyを許し、release rejectionでpressedを維持し、disconnect/shutdownでdelayを中断する | runtime integration | red: `cargo test --lib runtime::direct::tests::direct_tap --locked` はDirect tap state machine、context、stimulus、error未定義のcompile error。green: 同 commandで3 tests passed、Direct全5 tests passed。protocol-ready後のpress rejectionはattemptを1件だけ記録し、pendingとreleaseを作らずprevious snapshot/timerを維持する。accepted press後は80 ms deadlineの1 ns前をPendingとし、queued `0x08` outputを既存handlerへ渡してreplyを受理した後、deadlineのrelease rejectionでpressed snapshotとtimerを維持する。attempt順は`0x21/timer 0` report-mode reply、`0x21/1` lights reply、`0x30/2` press、`0x21/3` inbound reply、`0x30/4` rejected release。disconnect reason `0x13`とshutdown stimulusはtyped interruptionとしてpendingをconsumeし、press以外のinputを試行しない。absolute deadlineとns上限はpress前に検査する。refactor: pendingがrelease candidate/deadlineだけを所有し、1 stimulusごとに資源を一時借用する消費型stepでoutput errorを非terminal、disconnect/shutdownをterminalに分離済みのため構造変更を省略。Ready算出、lifecycle、poll/shutdown優先、disconnect neutral resetはT16/T17/T19/T21/T22へ残した |
| refactor-done | T16: lifecycle state machineがopen/close/reopenと冪等遷移を決定的に処理し、closing commandを`Shutdown`にする | runtime unit | red: `cargo test --lib runtime::lifecycle::tests --locked` はlifecycle state/action/command error/state machine未定義のcompile error。green: 同commandで3 tests passed。Configured/Closedのopen requestはtransport開始を1回だけ要求し、成功通知まで元stateを維持、open失敗後だけretry可能にする。Open/Connecting/Readyのrepeated open、Closing/Closedのrepeated closeとclose completionはno-op。Configuredのcloseはcleanupを作らずClosed、Open/Connecting/Ready/Failedのcloseは1回だけClosingへ入り、completion後にClosed、そこからreopenできる。close開始後とreopen成功前のinput、およびClosing中のopenはtyped `Shutdown`、Failedはtyped `Failed` とする。refactor: open開始中をstate machine内のboolに閉じ込めてrequestなしの成功確定と二重開始を防止し、Closedのrepeated closeが完了を再通知する退行を既存testで検出して明示no-opへ修正。cleanup順はT20、worker接続とpublic error mappingはT21/T26へ残した |
| refactor-done | T17: increasing session IDがtimer/session/holdoff/observed/inputをresetし、stale eventを破棄して observation `reset()` の一時 `dead_code` 許可を削除する | runtime unit | red: `cargo test --lib runtime::session::tests --locked` は`ConnectionSessions`未定義のcompile error。green: 同commandで2 tests passed。Periodic session 1でaccepted `0x03/0x30`、`0x30/0x01`、`0x48/0x01`、`0x40/0x02` replyと`0x30` inputを送りtimer 5、report mode、lights、vibration、IMU mode、encoding時刻100 ms、reply holdoff 350 ms、4 subcommand observation、button Aを確定後、session 2開始の1操作でtimer 0、`ProtocolSession::default()`、holdoffなし、empty observation、neutral inputへresetする。Direct入口はperiod stateを受け取らず、currentを終了してもlast-issuedを保持してIDを1→2へ進める。session 1でevent生成時にtagしたdisconnectはsession 2で破棄し、session 2でtagしたConnectedだけを通す。refactor: fallibleなchecked ID準備をinfallible reset/commitより先に分け、generic reporting resetをDirect/Periodicの名前付き入口へ置換。observationとinput resetの一時`dead_code`許可を削除した。scheduler開始/rebaseはT19、worker ingressでのtag順はT21、実transport sourceへのID付与はM4へ残した |
| refactor-done | T18: handshakeが両HID channel後にbootstrap neutralを送り、最初のsubcommandまでabsolute retryし、observation `is_empty()` の一時 `dead_code` 許可を削除する | runtime unit | red: `cargo test --lib runtime::handshake::tests --locked` は`Handshake`と`HandshakeProgress`未定義のcompile error。green: 同commandで5 tests passed。current sessionのlinkとControl/Interruptがそろった時刻100 msにneutral `0x30` timer 0を送り、channel 2順序、stale sessionのsignal/step破棄、重複signal、実transport内250 ms latency後も次期限1100 ms、3400 ms overrunの`skipped=1`とno burst、最初のparsed subcommandで期限回収を検査。topology完成前にsubcommandを観測した場合もbootstrap acceptanceまでは完了せず、初回と期限retryの連続rejectionはtimer 0を維持しつつ期限を1→2→3秒へ進める。refactor: `HandshakeCompletion`へsession IDを保持してT19の再照合境界を作り、transport結果を非終端の`BootstrapAttempted`へ分離してT21がretry継続とlifecycle failureを判断できる形にした。`ObservedSubcommands::is_empty()`はproduction handshakeから参照し一時許可を削除 |
| refactor-done | T19: Readyがsame-sessionのaccepted report-mode/nonzero-lights replyとhandshake回収を要求し、rejection/zero-lights/disconnect/timeoutを回収して終了する | runtime integration | red: `cargo test --lib runtime::readiness::tests --locked` はreadiness gate、dormant Periodic scheduler、lifecycle接続遷移が未定義のcompile error。green: 同commandで4 tests passed。100 msのbootstrap `0x30/timer 0`後、110 msのreport-mode reply rejectionはtimer 1とsessionを維持し、120 msのaccepted retry、130 msのaccepted zero-lights、140 msのaccepted nonzero-lightsを`0x21/timer 1..3`で処理する。最後のreply holdoff境界440 msまではConnecting、境界でsendせずschedulerを開始してReadyとなり、最初のPeriodic inputは448 msの`0x30/timer 4`。Directはcurrent handshake completionとaccepted mode/lightsの直後にReadyとなり、確認用inputを追加しない。current session不一致とhandshake proof不一致を別errorで拒否し、bootstrap直後のdisconnectはactive handshakeを回収、holdoff 420 msより早いoperation deadlineは399 msでPending、400 msで`TimedOut`となり、いずれもschedulerを開始せずOpenへ戻す。refactor: accepted stateを`ReportSender::session().protocol_ready()`へ一本化し、`ReadySession` proofをconsumeするlifecycle遷移と`Option<ReportScheduler>`によるdormant/armed境界へ分離。transport cleanupはT20、poll/error分類と呼出し統合はT21、最短deadline待機はT22に残した |
| refactor-done | T20: close/close-without-neutralがpending send、neutral、drain、disconnect、transport close、completion、joinの順とfailure継続を守る | runtime integration | red: `cargo test --lib runtime::cleanup::tests --locked` はcleanup sequence/context/phase/close errorとinterrupt drain contract未定義のcompile error。green: 同commandで5 tests passed。処理中reply `0x21/timer 0`の完了後、connectedな通常closeはneutral `0x30/timer 1`、1秒bounded drain、disconnect、transport close、completion token、joinの順に進み、repeated closeはconnectedでも全操作とjoinを再実行しない。close-without-neutralと未接続closeはneutralだけを省略し、残りのcleanupを同順序で行う。reply、neutral、drain、disconnect、transport closeの各失敗後も後続phaseとjoinを実行し、reply失敗は元commandだけへ返してclose outcomeへ重複集約せず、cleanupは最初のphase付きtransport errorを保持する。cleanupとjoinが同時に失敗した場合は両方をtyped variantへ保持する。Fake transportのdrainはopen中だけ成功するcontractも検査。refactor: cleanupを一回限りのlifecycle遷移、順序付きphase、消費型completion tokenへ分離し、send acceptance tokenの可視性をruntime内に限定した。実workerへのpending/completion接続はT21、実`JoinHandle`のtermination/panic検出と`WorkerFailed`変換はT24へ残した |
| refactor-done | T21: worker coreがshutdownを優先し、bounded command batch、HCI/reply、due deadlineを飢餓なく処理し、T02 transport、T05 sender、T07 output handler、残る M1 protocol の一時 `dead_code` 許可を削除する | worker unit | red: `cargo test --lib runtime::worker::tests --locked` は `WorkerCore`、worker command、clock、shutdown、budget interface が未定義の compile error。green: 同 command で16 tests passed、transport contract 6 tests passed。1 step で command 2件の後に1 poll batch 3 eventとrejected reply、due Periodic reportを処理し、command 1件とHCI 1件を次 stepへ残す。shutdownはstep開始時、処理済みcommand、reply、terminal error、due completionより優先し、cleanup前に確定したcommand/operation resultを`StepProgress`へ保持する。topology完成直後のbootstrapを同一poll batch内の最初のoutputより先に送り、`SendRejected`だけをabsolute deadlineでretryし、terminal transport errorはsourceを公開文言へ出さずFailedにする。accepted replyの300 ms holdoff、operation timeoutと同時刻retryの順、pending tap中のinbound replyと後続command直列化、Periodic tapの先頭`SendRejected` completionと後続`Closed` worker failureの分離をworker境界で固定した。Periodic tapはpress前にabsolute deadlineとu64 nanosecond上限を検査する。Connecting中のreason `0x13` disconnectはtyped readiness errorを返してOpenへ戻り、retry deadlineとhandshakeを回収する。refactor: Direct/Periodic差をsealed worker policyへ分離し、event batchをcurrent sessionでingress時にtagする。T02/T05/T07とM1 protocolのmodule-wide `dead_code`許可を削除し、compatibility fixture/test getterは`cfg(test)`、T22以降またはM3で使うsymbolはconsumerを明記したitem単位の許可へ狭めた。gate: `cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --all-targets --all-features --locked` 167 tests、default/no-default各168 tests、`cargo test --release --lib runtime::worker::tests --locked` 16 tests、default/all-features build、Bumble-free tree、`git diff --check` はすべて成功 |
| refactor-skipped | T22: coalescing wakeがidle時にblockし、command/transport/shutdown通知または指定deadlineで一度だけ起床する | worker unit | red: `cargo test --lib runtime::worker::tests:: --locked` は `ChannelWorkerWaiter`、`WorkerWaitRequest`、`WorkerWaiter`、`WorkerWaitError`、`wait_for_next_iteration` 未定義の compile error。green: 同 command で21 tests passed。transport初回`open()`より前に容量1のactivity channelを作り、command enqueue、transport event、shutdown latchを公開してから同じnotifierのcloneで通知する。保留中の3通知は1 tokenへ集約し、idleは`recv()`、未来deadlineは絶対時刻のままscripted waiterへ渡してfake clockを310 msから318 msへ進める。`immediate`と到来済みdeadlineは待機せず、待機直前にもdeadlineを再確認する。deadlineと通知が重なった場合はtokenを捨てず、lost wakeを避けるため次の空stepを最大1回許す。receiver切断は通常wakeと区別した`WorkerWaitError::Disconnected`を返す。実時間sleepは不使用。実thread schedulingとproduction `recv_timeout()`の実時間timeoutはT24のloop接続まで未検証。refactor: `StepProgress::wait_request()`を純粋な待機判断、`WorkerWaiter`を待機port、`ChannelWorkerWaiter`を`recv`/`recv_timeout` adapterに分離済みで、T23のqueueとT24のthread loopを先取りしないため構造変更を省略。gate: `cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --all-targets --all-features --locked` 172 tests、default/no-default各173 tests、`cargo test --release --lib runtime::worker::tests:: --locked` 21 tests、default/all-features build、`cargo fmt --check`、`git diff --check` はすべて成功 |
| refactor-done | T23: full command queueがwall-clock waitなしで即時`Busy`を返す | worker integration | red: `cargo test --lib runtime::command::tests --locked` は `CommandDeliveryError`、`CommandEnqueueError`、`command_channel`、`StepProgress::take_command_results` 未定義の compile error。green: 同 command で4 tests、`cargo test --lib runtime::worker::tests::bounded_queue_feeds_one_worker_command_and_its_typed_response --locked` で1 test passed。容量1 queueの先頭commandは受理してwakeし、2件目は`try_send`から即時`Busy`を返してwakeしない。worker側で先頭を取得した後は次を受理でき、receiver切断はfull履歴があっても`Disconnected`として`Busy`と区別する。受理の線形化点はqueueへの格納成功で、その後だけactivityを通知する。one-shot responseのcapacityは1で、cloneしないsenderをrequest envelopeからworkerのFIFOへmoveする。`Pending`はsenderを保持し、`Complete`だけが先頭をconsumeする。duration 0のDirect tapを実`WorkerCore`へ渡し、同一`StepProgress`の`[Pending, Complete(Ok(()))]`、due action 1件、response 1件をfake clockで検査した。typed `Shutdown`を元commandへ返し、callerがresponseを破棄してもworker deliveryを失敗させない。refactor: command queue、response ownership、worker進捗の配送を`CommandClient`、`CommandReceiver`、消費型`CommandCompletion`へ分離し、`StepProgress`からcommand resultをmoveで回収する。worker termination/panic時のqueued/in-flight response回収とthread/joinはT24、public `Busy` / `WorkerFailed` / `Shutdown` mappingはT26へ残した。gate: `cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --all-targets --all-features --locked` 177 tests、default/no-default各178 tests、release command 4 testsとworker 22 tests、default/all-features buildは成功 |
| refactor-done | T24: worker termination/panicがwaiting responseを`WorkerFailed`にし、completion後にjoinを回収する | worker integration | red: `cargo test --lib runtime::worker_thread::tests --locked` は `spawn_worker_thread`、`WorkerFailureCause`、`WorkerJoinError`、`WorkerThreadOutcome`、`CommandResponseError`、blocking `CommandResponse::recv` 未定義の compile error。green: 同 command で4 tests passed。command batch 1で先頭commandの具体的`Direct(NotReady)` completionを配送した後のterminal transport failureでは、後続queued responseだけを`WorkerFailed`にする。Ready Directの1秒tapを`Pending`にして後続commandをqueueへ残したままtransport pollをpanicさせ、typed panic completionを公開してから唯一の`CommandReceiver`をdropすることで両responseを`WorkerFailed`にし、その後`resume_unwind`したpanicをjoinで回収する。panic payloadはworker outcomeへ保持しない。priority closeをtap開始直後にlatchedした経路では`StepProgress::Pending`と`Closed.interrupted`を順に配送し、元commandへ具体的`Direct(Interrupted(Shutdown))`を返した後、cleanup completionを受信してjoinする。activity channel切断は通常wakeと分けた`Wait(Disconnected)`としてcompletion後にjoinする。経過時間とsleepは成功条件に使っていない。refactor: thread loop、typed terminal cause、completion receiver、消費型`JoinHandle`を`worker_thread` moduleへ分離し、`CommandReceiver`を`catch_unwind`の外で所有する。core failureとresult delivery failure、cleanup failureとjoin failureを同時に保持し、clock/wait/shutdown/reporting state/commandへ`Send + 'static`境界を追加した。queueを`try_recv`で個別drainせずreceiver dropを唯一の未解決response切断点にする。`Drop`のpriority shutdown、bounded completion timeout、detachはT25、public `WorkerFailed` / `Shutdown` mappingとstatus更新はT26へ残した。gate: `cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --all-targets --all-features --locked` 181 tests、default/no-default各182 tests、release worker-thread 4 tests、default/all-features build、`cargo fmt --all --check`、`git diff --check`はすべて成功 |
| refactor-done | T25: `Drop`がpriority shutdownで正常fake workerをbounded completion/joinし、neutral/drainを行わない | worker integration | red: `cargo test --lib runtime::worker_thread::tests::drop_skips_neutral_and_drain_then_joins_the_completed_worker --locked` と `cargo test --lib runtime::worker_thread::tests::drop_request_replaces_only_an_untaken_explicit_shutdown --locked` は `WorkerOwner`、priority shutdown channel、completion waiter 未定義の compile error。green: `cargo test --lib runtime::worker_thread::tests --locked` は6 tests、うちT25の2 testsが成功。ownerは唯一のcommand clientを先にdropし、未取得のexplicit shutdownを`Dropped`で上書きしてstate公開後にactivityを通知する。worker取得後は上書きしない。Drop専用cleanupはneutralとinterrupt drainを省略し、disconnectとtransport closeを行う。scripted waiterが100 msのbounded wait要求を記録し、worker/transportのteardown中はcompletionが未公開でownerも待機中、barrier解放後にcompletion受信とjoinが完了する順序を検査した。これによりcompletionをjoin-ready tokenとし、経過時間は成功条件に使っていない。production waiterは`recv_timeout`を使い、timeoutまたはcompletion channel切断ではtokenなしにjoinせずhandleをdetachする。このscripted timeout/detach分岐の検査はT35、公開error/status mappingはT26に残した。refactor: explicit closeとDropをtyped `ShutdownRequest`で分け、`CleanupSequence::for_drop()`と共通のcompletion解釈へ分離した。terminal cause公開後にcommand receiverをdropするT24のresponse順を維持し、explicit `WithoutNeutral`がdrainを維持する既存cleanup 5 tests、worker priority 22 testsも成功。gate: `cargo clippy --all-targets --all-features --locked -- -D warnings`、all-features/default/no-defaultの全target各183 tests、release worker-thread 6 tests、default/all-features build、`cargo fmt --all --check`、`git diff --check`はすべて成功 |
| refactor-done | T26: status field/counter/error mappingを構造で検査し、blocked transport中もstatus/snapshotが独立projectionから返る | runtime integration | red: `cargo test --lib runtime::status_tests --locked` は public `LifecycleState` / `GamepadStatus`、status projection、`Controller::status()` / `snapshot()`、M2 error category/mapping 未定義の compile error。green: 同 command は4 tests、`cargo test --lib runtime::worker_thread::tests --locked` は7 tests passed。`Arc<RwLock<ProjectionState<M>>>` を controller の read handle と worker/component の write handleだけで共有し、kindは保存せず毎回`M::KIND` / `R::KIND`から投影する。Periodic snapshotはlocal commit直後、Direct snapshotと0x30/0x21累積counter・report modeはtransport acceptance後、last subcommandはparse直後かつsemantic/reply send前、lifecycle/connected/disconnect reasonは各typed state mutation直後に短いwriteで公開し、lockをpoll/send/cleanup/response wait/joinへ持ち越さない。容量1 barrier transportでDirect send受理後のpollを停止し、その間に別threadのpublic status/snapshotがReady・counter 2・button Aを返すことを経過時間閾値なしで検査した。new sessionはmode/subcommand/reason/snapshotだけをresetしcounterを保持、rejected Direct/replyはsnapshot/counterを進めずparsed IDを保持する。public ErrorKindへM2の14 categoryを追加し、queue full=Busy、closing=Shutdown、Direct non-Ready=TransportClosed、worker channel/state/panic/join=WorkerFailed、backend absence=UnsupportedCapabilityをtyped mapperで固定した。transport/backend source chainは`Error::source()`へ保持し、public Display/Debugとworker_failureは固定文言だけを公開する。run-loop、worker-owned teardownのpanicをcompletion前に捕捉してstatusをFailedへ更新し、cleanup failureとjoin panicは`CleanupAndJoin`として両方保持する。refactor: public diagnostics、shared projection、runtime error mapperを分離し、accepted counterの正本をReportSender、last IDの正本をObservedSubcommandsに置いた。WorkerCore再生成時のcounter引継ぎはT31条件としてdev journalへ記録。gate: `cargo clippy --all-targets --all-features --locked -- -D warnings`、all-features/default/no-defaultの全target各188 tests、release status 4 testsとworker-thread 7 tests、default/all-features build、`cargo fmt --all -- --check`、`git diff --check`はすべて成功 |
| refactor-done | T27: builder configがadapter必須、Periodic 8 ms default、`1 ms..=1 s`、colorsを検証し、Directにperiod stateを持たせない | controller unit | red: `cargo test --lib controller::config_tests --locked` は root `AdapterSelector`、内部 `BuilderConfig`、`Controller::builder()` 未定義の compile error。green: 同 command で4 tests passed。adapterを`builder(adapter)`の必須引数かつ非`Option` fieldとして文字列どおり保持し、selector解釈はM3へ残した。profile pathはI/Oせず保持だけを行い、colorsは未指定なら`M::SPEC`のモデル別既定値、指定時は型付き値を保持する。Periodicはraw `Duration`を既定8 msで保持し、construction operation前の副作用なしvalidationでprivate newtypeへ変換する。1 msと1 sを受理し、0、1 ms未満の直前値、1 s超過の直後値を`InvalidInput`とした。Directのbuilder/config associated stateはともに`()`で、`Option<Duration>`やperiod用fieldを持たない。adapter必須とDirectに`report_period()`がないことは型で保証し、方針どおりcompile-fail testは追加していない。refactor: reporting別のraw/validated configを外部から到達不能なsealed associated typeへ分離し、T29でprofile検証後に作る`ControllerConfig`と混同しないよう、この段階の非I/O設定を`BuilderConfig`とした。公開builderはprivate runtime型を露出しない。gate: `cargo clippy --all-targets --all-features --locked -- -D warnings`、all-features/default/no-defaultの全target各192 tests、release focused 4 tests、doctest 1 test、`cargo doc --no-deps --all-features --locked`、default/all-features build、`cargo fmt --all -- --check`、`git diff --check`はすべて成功 |
| refactor-done | T28: raw profileからtyped profileへのmodel検査が構造化errorを返し、known secret sentinelを`Debug`へ出さない | profile unit | red: `cargo test --lib profile::document_tests --locked` は `ProfileDocument` / `PairingProfile<M>` 未定義の compile error。green: 同 command で4 tests passed。`serde_json::from_slice`でUTF-8/JSONを解析し、`format = "swbt.profile"`、`schema_version = 2`、`ControllerKind::ALL`と`profile_name()`による3種のkind、identity variant外形、key-storeと`namespaces`のobject外形を検査する。raw `Value`全体を保持してunknown fieldやkey materialを捨てず、完全なkey/peer検査とlossless writeはM6へ残した。malformed UTF-8/JSONはsource chainを保持した`InvalidProfile`、root/format/schema/kind/shape不正は固定文言の`InvalidProfile`とする。3 modelのmatching documentは`PairingProfile<M>`へ変換し、recognized kindと`M::KIND`の不一致だけを`ProfileControllerMismatch`とした。raw/typed profileは手書き`Debug`でidentity/key-store/documentを伏せ、実際のnested `link_key`とunknown fieldに置いたknown sentinelがraw/typed/errorの`Debug`とerrorの`Display`へ出ないことを検査した。refactor: JSON/serde型をprivate document moduleへ閉じ、M2 public surfaceに含めないraw/typed型だけをcrate-private profile境界へ公開した。typed kindは重複値ではなく`M::KIND`から投影する。`adapter-default`と`exp-local-address`の外形だけをここで固定し、address値の意味検査はT30、filesystem read/buildはT29へ残した。gate: `cargo clippy --all-targets --all-features --locked -- -D warnings`、all-features/default/no-defaultの全target各196 tests、release focused 4 tests、doctest 1 test、default/all-features build、`cargo fmt --all -- --check`、`git diff --check`はすべて成功 |
| refactor-done | T29: buildがephemeral/existing/missing/mismatchを処理し、profile read以外のworker/transport I/Oを開始しない | integration | red: `cargo test --lib controller::build_tests --locked` は `ProfileConfig`、`ProfileReadPort`、`build_with_profile_reader()` 未定義の compile error。green: 同 command で5 tests passed。builder validationを最初に行い、invalid periodではprofileを読まない。pathなしは`ProfileConfig::Ephemeral`を持つConfigured controllerをprofile I/O 0件で構築し、existing pathは同じpathを1回だけreadして`ProfileDocument`のschema検査と`PairingProfile<M>`のmodel検査後にpathとtyped profileを`ProfileConfig::Persistent`へmoveする。`NotFound`だけをsource付き`ProfileNotFound`、その他のfilesystem read失敗をsource付き`Internal`、model不一致を`ProfileControllerMismatch`とし、公開Display/Debugへpathを出さない。成功時はadapter、colors、reporting固有config、Configured/connected=false/neutralの同一status projectionを`Controller`が所有する。Configured時のpublisherも保持し、T31で同じprojectionをworkerへ渡せるようにした。build helperが受け取るI/O capabilityはcrate-private `ProfileReadPort`だけで、`Controller`に`WorkerOwner`や`TransportPort`はなく、adapter open、worker/thread開始、USB I/Oを呼び出せない構造とした。refactor: production `std::fs::read` adapterとfake readerを分離し、fake event logでephemeralの0 readとexisting/missing/mismatchのexactly-one readを検査した。旧test専用`from_status`は正規のephemeral buildとtest-only publisher cloneへ置換し、config必須不変条件を維持した。create/write、target存在検査、LocalAddressの意味検査はT30、transport open・worker開始・pairingはT31以降へ残した。READMEとcrate docsをM2のConfigured build公開面へ更新。gate: `cargo clippy --all-targets --all-features --locked -- -D warnings`、all-features/default/no-defaultの全target各201 tests、`cargo test --release --lib controller::build_tests --locked` 5 tests、doctest 1 test、`cargo doc --no-deps --all-features --locked`、default/all-features build、`cargo fmt --all -- --check`、`git diff --check`はすべて成功 |
| refactor-done | T30: create-profile target validationがpath required、LocalAddress unsupported、existing no-overwriteをcreate/open前に処理する | integration | red: `cargo test --lib controller::create_profile_tests --locked` はprofile identity/store module、root `CreateProfileOptions` / `LocalAddress` / `ProfileIdentity`、target validation method未定義のcompile error。green: 同commandで4 tests、`cargo test --lib profile::identity_tests --locked`で3 tests、`cargo test --test profile_identity_contract --locked`で1 test passed。builder validation、path required、identity gate、target inspectの順に固定し、invalid period、pathなし、explicit LocalAddressはtargetをinspectせず、それぞれ`InvalidInput`、`ProfilePathRequired`、`UnsupportedCapability`とした。AdapterDefaultのexisting targetはread-only inspect 1件後に`ProfileAlreadyExists`、absent targetだけがprivate `CreateProfilePlan<M,R>`となる。target inspectのI/O失敗はsource付き`Internal`とし、公開Display/Debugへpathを出さない。target portは`inspect()`しか持たずcreate/write/open能力を渡さないため、T30のno-overwrite/no-openは構造で保証する。`Absent`は予約ではないことをport contractへ明記し、preflight後の競合はT31のatomic create-newでも`ProfileAlreadyExists`へ再分類する。planはvalidated `BuilderConfig`、owned path、未加工の`pair_timeout`を所有し、identity gate通過後なのでAdapterDefaultだけを表す。zero timeoutも独自に拒否しない。public `LocalAddress`はPython固定基準どおり6-octet colon表記、individual/local bit、big-endian LAPのreserved inquiry範囲`0x9E8B00..=0x9E8B3F`を検査し、octetを`Debug`で伏せる。public constructor違反は`InvalidInput`、profile JSON内の違反はsource付き`InvalidProfile`。refactor: public optionsをconnection、identity値をprofile、read-only target portをprofile store、preflight planをcontroller create moduleへ分離した。public `create_profile()`、production target adapter、backend capability、create-new/reopen/open/pair/Readyは追加せずT31/T34へ残した。gate: `cargo clippy --all-targets --all-features --locked -- -D warnings`、all-features/default/no-defaultの全target各210 tests、releaseのcreate 4 tests・identity 3 tests・public contract 1 test、doctest 1 test、`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked`、default/all-features build、`cargo fmt --all -- --check`、`git diff --check`はすべて成功 |
| refactor-done | T31: create-profile successがcreate-new→typed reopen→open→pair→Readyのevent順を守る | integration | red: `cargo test --lib controller::create_profile_tests::create_profile_persists_and_reopens_before_open_then_returns_ready_controller --locked` は `ProfileCreatePort`、`ProfileReadPort`、`CreateProfileRuntimeBackend`、`ReadyRuntime` 未定義の compile error。green: `cargo test --lib controller::create_profile_tests --locked` は5 tests passed。Pro/Periodic の代表例で共有event logを使い、target inspect、backend capability、create-new、同じstoreからのread-back、typed `PairingProfile<Pro>`を含むconfigでのopen、未加工60秒timeoutによるpair開始、protocol Readyを完全一致で検査した。persistしたschema v2 empty envelopeは`format`、`controller_kind = pro`、adapter-default identity、空namespacesを持ち、JSONのkey順、indent、末尾改行は固定していない。`ProfileCreatePort`はread-only preflight portを広げずにread/create-new能力を追加し、create-newの既存競合を`AlreadyExists`契約からsource付き`ProfileAlreadyExists`、その他をsource付き`Internal`へ写す。書込後はin-memory documentを流用せず既存`read_typed_profile`でparse/model検査し、完了後だけstatusとruntime所有権を持つReady controllerを返す。fake status projectionもaccepted bootstrap 1件、reply 2件、report modeとlast subcommandの`0x30`を持つReady不変条件に合わせたが、実transport acceptanceの根拠にはしていない。fake runtime tokenがcontroller生存中だけ保持され、controller dropで破棄されることを検査した。`Controller`ではruntime fieldをconfig/statusより前に宣言し、drop順もruntimeを先にした。refactor: profile read portをprofile store境界へ移し、preflight plan、typed reopen後plan、opened backend state、`ReadyRuntime<M,R>`のphaseを分離した。現行worker loopにReady結果の同期応答がないため実threadを捏造せず、pair command/one-shot completionとsealed command型はT33、create-new競合とopen/pair failure cleanup・empty envelope残存はT32、public backend不在はT34に維持した。gate: `cargo clippy --all-targets --all-features --locked -- -D warnings`、all-features/default/no-defaultの全target各211 tests、`cargo test --release --lib controller::create_profile_tests --locked` 5 tests、`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked`、default/all-features build、`cargo fmt --all -- --check`、`git diff --check`はすべて成功 |
| refactor-done | T32: create-new競合を上書きせず拒否し、open/pair失敗時にprofileを残してruntime attemptをcleanupする | integration | red: `cargo test --lib controller::create_profile_tests::pair_failure_keeps_valid_empty_profile_and_cleans_opened_runtime --locked` は `CreateProfileRuntimeAttempt`、backendの`Attempt`/`begin_attempt`が未定義で、旧`Opened`/`open`/`pair_to_ready`を要求するcompile error。green: `cargo test --lib controller::create_profile_tests --locked` は9 tests passed。T32追加4 testsで、preflight後のcreate-new競合をsource付き`ProfileAlreadyExists`へ写して競合側bytesを変更せずruntime open前に停止すること、open途中とpair開始後の失敗ではvalid Pro adapter-default empty envelopeを残し、明示`cleanup_without_neutral`を1回実行してtransport/worker probeを非activeにし、resourceを1回だけ破棄することを検査した。resource取得済みattemptの早期Dropは明示cleanupなしでfallbackを1回実行する。orchestratorはresource未取得の`begin_attempt()`からopen、pair、失敗cleanup、成功時`ReadyRuntime`への所有権移譲まで同じattemptを保持する。refactor: explicit cleanupとDrop fallbackを別event/countに分け、empty envelope検査をsuccess/failureで共用した。cleanup自身のerror mappingと実worker接続はT33、production Dropのbounded timeout/detachはT35、実filesystemのatomic persistenceはM6へ残した。gate: `cargo clippy --all-targets --all-features --locked -- -D warnings`、all-features/default/no-defaultの全target各215 tests、`cargo test --release --lib controller::create_profile_tests --locked` 9 tests、`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked`、default/all-features build、`cargo fmt --all -- --check`、`git diff --check`はすべて成功。 |
| refactor-done | T33: pair one-shotがReadyまで後続inputを止め、6 model×reportingの共通smokeがbuild→実worker＋fake transport Ready→最小input→明示close/joinを通り、concrete attemptのprimary＋cleanup error mappingを固定する | integration | red: `cargo test --lib runtime::worker::tests::pair_response_stays_pending_until_ready_and_blocks_following_input --locked` は外側の`RuntimeCommand`、`step_runtime()`、parallel failureを保持する`Error::with_related()`が未定義のcompile error。green: pair 2 tests、controller fake-runtime 15 tests、error mapping 7 testsが成功。pairはreporting固有inputの外側にあるtyped commandとして同じbounded FIFOとone-shotを使い、Ready、timeout、disconnect、shutdown、terminal worker failureまで先頭responseを保持する。sealed reporting GATでPeriodic/Direct command型を固定し、`WorkerCore → command_channel → spawn_worker_thread → WorkerOwner → ReadyRuntimePort → ReadyRuntime → Controller`をproduction型のまま接続した。Pro/Joy-Con L/Joy-Con R×Periodic/Directの6 named casesはConfigured neutral、Pair、protocol Ready、モデル固有buttonを含む`apply()`/`send()`、snapshot、明示close、completion、joinを通り、common `press()`、`release()`、duration 0の`tap()`、`neutral()`と`close_without_neutral()`も同じbridgeで検査した。Periodic Readyはwaiterが要求した絶対300 ms deadlineへmanual clockを進め、2秒の実時間はdeadlock watchdogにだけ使った。primaryは従来どおり`Error::source()`、cleanup・delivery・joinは`Error::related_error()`へ発生順に保持し、Closedはcleanup→delivery→join、Failedはcause→delivery→cleanup→joinとした。Pair中のterminal poll failureとopen直後からPair enqueue前にworkerが終了する競合では、joined terminal outcomeをprimaryへ昇格してtyped `TransportError`を保持し、worker thread内でstatusをFailedにしてwithout-neutralの1秒drain、disconnect、transport closeを一度だけ実行する。transportは`open()`呼出し前にattemptへguardし、部分openのerror/panicをDropのdrainなしdisconnect→closeで回収する。thread spawnはOS thread作成成功後にworker resourceをhandoffし、spawn/start失敗時もcaller側でwithout-neutral cleanupしてcleanup failureをrelatedへ保持する。refactor: `Any`や任意tokenを使わずtyped `ReadyRuntimePort`へ責務を分け、terminal outcomeをfailureの正本にし、Rust 1.87で未対応のlet-chain 8件を同じ評価順のnested `if`へ置換した。public backend不在はT34、Dropの100 ms completion timeout/detachはT35に残した。gate: `cargo +1.87 check --all-targets --all-features --locked`、stable check、clippy `-D warnings`、all-features/default/no-defaultの全target各lib 211 tests＋integration/example全件、releaseのcontroller runtime 15 tests・pair 2 tests・error mapping 7 tests、rustdoc `-D warnings`、default/all-features build、no-default dependency treeのBumble不在、`cargo fmt --all -- --check`、`git diff --check`はすべて成功。 |
| refactor-done | T34: concrete backend不在のpublic open/pair/create-profileが`UnsupportedCapability`を返し、worker/thread/fileを残さない | public integration | red: `cargo test --test backend_unavailable_contract --locked` は public `open()` / `pair()` / builder `create_profile()` 未定義の27件の`E0599` compile error。green: 同commandで4 tests passed。Pro/Joy-Con L/Joy-Con R×Periodic/Directの6 public aliasはConfigured controllerに対する`open()` / `pair()`を各2回実行し、毎回`UnsupportedCapability`、status全体とneutral snapshot不変、後続inputは`TransportClosed`とした。controller unitでも各失敗後に`_runtime.is_none()`を検査し、activity channel、transport factory、worker spawnへ到達する所有権を持たない。public `create_profile()`はbuilder→path→identity→target inspect→backend capabilityの順を維持し、pathなしは`ProfilePathRequired`、regular file・directory・dangling symlinkは`ProfileAlreadyExists`で内容またはentryを維持、absent targetはfileを作らず`UnsupportedCapability`とした。production target adapterは`symlink_metadata()`の`NotFound`だけをAbsentとし、その他のI/O errorをsource付き`Internal`へ写すread-only portで、public経路に`ProfileCreatePort`やruntime attemptを渡さない。crate-private orchestratorは事前検査済みplanをbackend検査済みprivate型へconsumeしてからだけpersistでき、capability failureのeventは`InspectTarget → CheckBackendCapability`で終了し、bytes、runtime resource、cleanup countはいずれも0。refactor: preflight planとbackend検査済みplanを型で分け、fake successとpublic failureが同じcapability gateを通るようにした。OS thread数、sleep、経過時間は成功条件に使っていない。README、crate docs、public rustdocを現在のfail-only境界へ更新。gate: `cargo +1.87 check --all-targets --all-features --locked`、stable check、clippy `-D warnings`、all-features/default/no-defaultの全target各lib 213 tests＋integration/example全件、release public contract 4 tests・create-profile 10 tests・runtime ownership 1 test、rustdoc `-D warnings`、default/all-features build、`cargo fmt --all -- --check`、`git diff --check`はすべて成功 |
| planned | T35: scripted no-completion waiterがDropのbounded timeout→detachをwall-clock待ちなしで検査する | worker unit | |

各 TDD item は 1 回の red / green / 必要な refactor として進める。複数 item を同じ commit に
まとめない。thread integration の real timeout は deadlock watchdog にだけ使い、成功条件は
barrier、channel、fake transport の観測順で判定する。経過時間で「非 blocking」を判定しない。
compile-fail 専用依存は追加せず、reporting 固有 method の存在は通常の crate compile と
利用例で検査する。

### 6.1 decision gate measurement

次は TDD item ではなく、T22 の deterministic invariant が green になった後の実時間測定である。
通常 test に時間閾値を入れない。

| metric | condition | record |
|---|---|---|
| idle CPU | Direct/Open、activity/deadlineなし、10 s | process CPU delta、wake/loop count |
| 8 ms jitter | Periodic、即時accept、10,000 tick | deadline差のp50/p95/p99/max、8 ms比、skip数 |
| command latency | idle workerへ1件ずつ10,000回 | enqueue→処理/responseのp50/p95/p99/max |
| transport latency | fake event notifyを10,000回 | inject→drainのp50/p95/p99/max |
| shutdown latency | idle時とqueue飽和時を各1,000回 | priority close→completion/joinのp50/p95/p99/max |
| fairness | command/HCI継続投入とPeriodic 10,000 tick | reply/deadline遅延、overflow、burst件数 |

測定 command、host、build profile、queue容量、command batch上限、生データ保存先、採用値を
検証表と dev journal に記録する。Windows process CPU は外部 harness から取得し、
crate に platform-specific `unsafe` を追加しない。

### 6.2 package gates

- production caller 接続後、M1 の module-wide `dead_code` 許可を削除する。
- default / no-default / all-features の通常 build で production 参照を確認する。
- package gate は TDD item の振る舞い完了と混同しない。

## 7. 設計メモ

- async runtime は追加せず、`std::thread` と同期 port を使う。
- runtime core 全体を `Arc<Mutex<_>>` に入れない。worker が mutable state を単独所有する。
- controller と worker が共有するのは短時間 lock の status/snapshot projection と termination signal。
- lock を保持したまま transport poll/send、response wait、join を行わない。
- `ReportSender<M>` は timer と `ProtocolSession` を所有する。transport は worker が所有し、
  send call ごとに `&mut dyn TransportPort` を sender へ渡す。
- observed subcommand set は reply candidate に含まれる `ProtocolSession` と分け、worker の
  connection-session wrapper が所有する。これにより reply rejection でも observation を戻さない。
- Periodic と Direct の差は sealed crate-private runtime policy に置き、runtime core で
  `ReportingKind` を毎操作 match しない。
- public `ReportingMode` に外部実装可能な associated runtime 型を追加しない。
- command queue の `try_send` 成功を受理の線形化点とし、queueへ格納した後だけactivityを通知する。
  full は待機せず`Busy`、receiver切断は`Disconnected`として区別し、失敗したenqueueは通知しない。
- command response は operation ごとの容量1 one-shot channel とし、error message だけを契約にしない。
  response sender はcloneせず、caller側のrequest envelopeからworker側のFIFOへ所有権を移す。
  `Pending`では保持し、`Complete`だけが先頭をconsumeする。callerがresponseを破棄した後のcompletionは
  worker failureにしない。worker termination/panic時のqueued/in-flight response回収はT24で接続する。
- worker threadは各`StepProgress`と`Closed.interrupted`の具体的command resultを先に配送する。
  terminal causeを容量1 completion channelへ公開した後に唯一の`CommandReceiver`をdropし、
  unresolvedなqueued/in-flight one-shotだけを切断する。queueを`try_recv`で個別drainすると
  最後のempty確認とconcurrent enqueueの間を閉じられないため採用しない。
- panic捕捉closureはworker coreとcommand receiverを借用し、panic completionの公開、
  command receiver drop、`resume_unwind`の順に処理する。controller側のworker ownerはcompletionを
  受信してから消費型`JoinHandle`を1回だけjoinする。panic payloadはtyped outcomeへ含めない。
- `InputState<M>` は worker command と snapshot の両方で model type を維持する。
- fake clock test は core を明示 step し、wall-clock sleep を使わない。
- activity wait の第一候補は、bounded command `sync_channel`、容量 1 の coalescing wake
  `sync_channel<()>`、command queue 外の priority shutdown latch の組み合わせとする。
  activity channel は transport の初回 `open()` より前に1回だけ作り、command enqueue、
  transport activity、shutdown latch の公開後に同じ notifier の clone で通知する。
- wake token は仕事の件数ではなく再評価が必要なことを表す。worker は `step()` 後に
  token を掃除しない。容量1は保留tokenを最大1個に集約するが、受信直後の新通知は次の
  tokenになり得る。deadlineと通知の同時到来ではlost wakeを避けるため、最大1回の空stepを許す。
- `StepProgress` が immediate、または最短deadlineが到来済みなら待機せず次の`step()`へ進む。
  それ以外はdeadlineなしで`recv()`、未来の絶対deadlineまで`recv_timeout()`する。
  待機直前にもclockを読み直し、channel切断は通常wakeと区別して終了側へ返す。wakeまたは
  timeout後は理由別に処理を分けず、shutdown、bounded command batch、non-blocking transport
  drain、due deadlineの順で完全な`step()`を実行する。
- short-timeout transport polling は idle wake と HCI/shutdown latencyを同じ量子へ
  縛るため第一候補にしない。T22 で deterministic wait invariant を固定し、最終採否、
  queue 容量、command batch 上限は 6.1 の測定値で決める。
- fake clock test は `WorkerCore::step` と wait adapter を分け、requested deadline を
  記録する scripted waiter で wall-clock sleep を使わず駆動する。
- Drop completion wait も crate-private adapter とし、normal completion と timeout/detach を
  wall-clock 経過ではなく scripted outcome で unit test する。production adapter だけが
  bounded real timeout を使う。
- activity notifier は M2 の crate-private port contract に加える。M3 concrete port が
  Bumble reader activity を通知できるかを M3 entry gate で検証し、取得不能なら upstream
  変更または wait design の再判断を明示する。
- public builder へ transport/profile-store injection を追加しない。test seam は crate-private。
- profile read は filesystem I/O だが、初期設計の「build は I/O を開始しない」は
  transport/worker/device を開始しない意味として扱う。write は `create_profile()` だけが行う。

## 8. 対象ファイル

| path | change | 内容 |
|---|---|---|
| `Cargo.toml` / `Cargo.lock` | modify | profile DTO に必要な最小 dependency。Bumble feature 境界は維持 |
| `src/controller/` | modify / new | public builder/controller、config、client、reporting 固有 method |
| `src/runtime/` | new | worker、command、state、sender、scheduler、clock、transport、status |
| `src/profile/` | modify / new | raw envelope、typed profile、identity、store port |
| `src/protocol/` | modify | input preparation と runtime caller visibility。observed set は runtime が所有 |
| `src/error.rs` | modify | lifecycle/profile/transport/worker の構造化 `ErrorKind` と source |
| `src/reporting/mod.rs` | modify | crate-private runtime policy |
| `src/lib.rs` | modify | M2 public API と docs |
| `tests/` | modify / new | builder/profile/runtime public contract と 6-combination harness |
| `tests/fixtures/python-v0.6.0/runtime/` | new | Python runtime causal fixture と provenance |
| `tools/generate_python_runtime_fixtures.py` | new | fixed baseline fixture generator |
| `examples/` または `tools/` | new if retained | activity wait measurement entrypoint |
| `README.md` | modify | fake-verified M2 surfaceと未実装 transport の明記 |
| `spec/dev-journal.md` | modify | wait decision、M3/M6へ残す境界、未検証事項 |
| `spec/wip/unit_003/M2_RUNTIME.md` | new / modify | 本作業仕様と検証記録 |

## 9. 検証

item ごとに最小 targeted test を red / green で実行し、結果を本表と TDD Test List に追記する。
最終 gate 候補:

| command | result | notes |
|---|---|---|
| `cargo fmt --all --check` | not run | final gate |
| `cargo +1.87 check --all-targets --all-features --locked` | not run | MSRV |
| `cargo check --all-targets --all-features --locked` | not run | stable |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | not run | warning 0 |
| `cargo test --all-targets --all-features --locked` | not run | full |
| `cargo test --locked` | not run | default feature |
| `cargo test --no-default-features --locked` | not run | Bumble-free runtime |
| `cargo tree --no-default-features --edges normal --locked` | not run | Bumble 不在 |
| `cargo +nightly miri test --lib --no-default-features --locked protocol::` | not run | M1 pure boundary regression |
| `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked` | not run | public docs |
| activity wait measurement command | not run | invariant test と実時間測定を分けて記録 |
| `cargo package --allow-dirty --locked` | expected fail | git Bumble の version requirement。M9 blocker |
| `git diff --check` | not run | whitespace |
| GitHub required checks | not run | PR 作成後 |

実 adapter、Switch、network、cross compile、fuzz は M2 の対象外なので実行しない。

## 10. 先送り事項

- Bumble concrete port、adapter selector の解釈、USB/HCI error source: M3
- HIDP/SDP event と session ID の実 transport mapping: M4
- public `create_profile()` の実 pairing 成功、explicit local address: M5
- full schema v2 compatibility、key preservation、atomic persistence、reconnect: M6
- Joy-Con runtime/hardware evidence: M7
- diagnostics event schema、long-run timing/hardware proof: M8
- checkout action の Node.js 20 deprecation warning と CI action 更新は専用 maintenance 単位
- git Bumble の package blocker: M9

## 11. 完了チェックリスト

- [ ] 対象範囲と対象外を確認した
- [ ] TDD Test List をすべて `refactor-done` または `refactor-skipped` にした
- [ ] 6 model × reporting の fake-runtime harness を通した
- [ ] acceptance 前後の commit と ACK ordering を検証した
- [ ] handshake / Ready を fake event の因果順で検証した
- [ ] lifecycle、stale session、backpressure、panic、join を検証した
- [ ] Drop の bounded shutdown と正常 fake の thread 回収を検証した
- [x] builder/profile orchestration と失敗 cleanup を検証した
- [ ] Python runtime fixture と `rust_spec_delta` を区別して記録した
- [ ] activity wait の測定値と選択理由を記録した
- [x] M1 の一時 `dead_code` 許可を撤去した
- [ ] public API / error / rustdoc / README を review した
- [ ] 検証結果または未実行理由を記録した
- [ ] `spec/complete/unit_003/` へ移動した
