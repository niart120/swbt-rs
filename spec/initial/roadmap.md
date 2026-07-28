# 実装ロードマップ

- 状態: **決定**
- 基準断面: [source-baseline.md](source-baseline.md)
- API: [api.md](api.md)
- architecture: [architecture.md](architecture.md)
- test gate: [testing.md](testing.md)

この文書は実装順序と milestone ごとの exit criteria を定義する。日付ではなく依存関係と証拠で進捗を判定する。

## 1. 実装原則

```text
M0 repository / dependency gate
  ↓
M1 pure input + protocol parity
  ↓
M2 runtime worker + fake transport
  ↓
M3 Bumble external HCI bring-up
  ↓
M4 virtual Classic SDP/HID integration
  ↓
M5 Pro Controller fresh pairing + Periodic
  ↓
M6 profile compatibility + reconnect + Direct
  ↓
M7 Joy-Con L/R
  ↓
M8 IMU / diagnostics / probe
  ↓
M9 portability / release
```

実機依存の作業に入る前に、pure protocol と fake transport の failure semantics を固定する。Bumble integration の問題を NX protocol へ混ぜない。

各 milestone は `spec/wip/unit_連番/` で作業仕様を作り、完了後に `spec/complete/unit_連番/` へ移す。複数 milestone を一つの巨大 PR にまとめない。

## 2. release target

### 2.1 `0.1.0-alpha.1`

対象:

- Pro Controller
- Periodic reporting
- adapter-default identity
- fresh pairing
- Windows 11 + CSR8510 A10 + WinUSB の限定構成
- protocol / fake / virtual integration tests
- hardware probe 手順

非対象:

- reconnect の保証
- Direct controller
- Joy-Con
- explicit local address
- Linux / macOS 保証
- crates.io 公開

### 2.2 `0.1.0-alpha.2`

追加対象:

- profile schema v2 read / write
- stored link key reconnect
- DirectProController
- profile round-trip compatibility
- structured diagnostics
- `swbt-probe`

### 2.3 `0.1.0-beta.1`

追加対象:

- Joy-Con L/R の Periodic / Direct
- IMU public conversion / wire parity
- limited Linux bring-up
- dependency / license inventory
- API review と semver freeze 候補

### 2.4 `0.1.0`

必要条件:

- required public API の docs と examples
- MSRV / stable CI
- Windows supported matrix
- profile update interruption test
- no unresolved severity-high protocol / cleanup defect
- license 決定
- release checklist と reproducible build metadata
- unsupported platform / feature の明記

## 3. M0: repository と dependency gate

### 3.1 作業

- `src/lib.rs` を追加し library target 名を `swbt` にする
- `src/main.rs` の placeholder binary を削除または後続 CLI 用に移動する
- `rust-version = "1.87"` を設定
- edition 2024 を維持
- `#![forbid(unsafe_code)]`
- `Cargo.lock` を commit
- Bumble crate を exact revision に固定
- `serde`、`thiserror`、`tracing` 等の direct dependency 方針を決める
- formatter / clippy / test / docs の GitHub Actions を追加
- MSRV job を追加
- Dependabot または Renovate が Bumble git rev を勝手に更新しないよう設定
- license 方針を maintainer decision として記録
- initial docs のリンク切れ / terminology check を CI に追加

### 3.2 exit criteria

- `cargo +1.87 check --all-targets`
- current stable の `cargo fmt --check`
- current stable の `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets`
- `cargo doc --no-deps`
- dependency source が単一 Bumble revision
- default branch に placeholder `Hello, world!` が残っていない
- library consumer の compile test がある
- license 未決の場合、release job が明示的に無効

### 3.3 risk gate

`bumble-transport` の build が supported CI image で成立しない場合、NX 実装へ進まず、dependency feature / native toolchain を先に解決する。

## 4. M1: input model と pure protocol parity

### 4.1 対象

- `Button`
- `Stick`
- `ImuFrame`
- `InputState`
- `ControllerColors`
- controller profiles
- `0x30` input report builder
- `0x01` / `0x10` output report parser
- subcommand responder
- virtual SPI
- report session state
- IMU block encoder

### 4.2 作業

- Python 基準断面から golden fixture generator を作る
- fixture に source SHA、generator version、controller kind、input parameters を記録
- neutral / button / stick / IMU report bytes を固定
- output parser の valid / malformed corpus を作る
- subcommand `0x02` / `0x03` / `0x04` / `0x08` / `0x10` / `0x21` / `0x30` / `0x40` / `0x48` を実装
- SPI known range と out-of-range policy を固定
- Joy-Con profile の unsupported input matrix を data として定義
- protocol module が filesystem、thread、Bumble に依存しないことを test
- Python fixture と Rust result の差分表示 tool を用意

### 4.3 exit criteria

- neutral `0x30` が 49 bytes
- Python fixture と対象 report / reply が byte-for-byte 一致
- Stick の全境界と normalized rounding が一致
- IMU raw / physical unit conversion が tolerance 内で一致
- malformed report で panic しない
- property test で encode length / reserved field invariant が成立
- protocol tests が Bumble dependency を link せず実行可能
- Miri 対象 unit test が通る
- source audit fixture が commit 済み

### 4.4 非対象

- worker thread
- USB
- pairing
- profile filesystem write
- real-time periodic scheduling

## 5. M2: runtime worker と fake transport

### 5.1 対象

- concrete controller skeleton
- sealed traits
- worker command channel
- lifecycle state machine
- state store
- report sender
- periodic scheduler
- handshake
- fake `TransportPort`
- deterministic clock

### 5.2 作業

- `ProController` / `DirectProController` を fake transport 上で構築
- `open` / `close` / reopen
- connection event injection
- `0x21` reply と `0x30` の共通送信順
- periodic deadline skip
- direct acceptance transaction
- tap cancellation / release failure
- disconnect / stale session event
- worker panic propagation
- bounded command queue / backpressure
- close 中の command reject
- status snapshot
- `Drop` と explicit close の差を test
- activity wait 方式の microbenchmark を作る

### 5.3 exit criteria

- wall-clock sleep を使わず周期 test が決定的に通る
- Periodic は state commit と send failure semantics が仕様通り
- Direct は acceptance 前 failure で state を変更しない
- reply が relevant input report に追い越されない
- close neutral / no-neutral が両方 test 済み
- fake disconnect 後に state と session が neutral reset
- reopen で timer / IMU / readiness を引き継がない
- queue overflow が bounded error
- worker leak / thread leak test が通る
- Loom を採用する場合、command/close race の model test が通る

### 5.4 decision gate

activity wait 実装を次の計測で決める。

- idle CPU
- 8 ms period の p50 / p95 / p99 jitter
- command response latency
- HCI event response latency
- shutdown latency

## 6. M3: Bumble external HCI bring-up

### 6.1 対象

- Bumble git dependency
- adapter selector
- no-open adapter discovery
- USB transport open / split
- `ExternalHost`
- `Device` initialization
- adapter-only diagnostics
- close

Switch 実機は不要。USB Bluetooth adapter は必要。

### 6.2 作業

- `usb:0` selector
- VID/PID / serial alias
- no-open USB HCI classification
- `open_split_transport`
- controller reset / capability query
- local address read
- Classic capability check
- permission / driver error mapping
- reader termination と worker shutdown
- repeated open / close
- transport failure injection where possible
- transitive dependency size / build time measurement
- `bumble-rs` API gap list を issue 単位で記録

### 6.3 exit criteria

- Windows target adapter で 100 回 open / initialize / close を実行し resource leak がない
- no-open discovery が device handle claim を残さない
- invalid selector と permission error が区別される
- local controller address と HCI version を trace できる
- transport unplug を `TransportEnded` へ変換できる
- worker が unplug 後に join する
- MSRV build が Bumble を含めて通る
- dependency license report が生成できる

### 6.4 upstream gate

次のいずれかが必要なら、M4 前に上流 issue / PR を作る。

- activity receiver への access
- external `Device` の accepted Classic channel API 不足
- Classic discoverable / connectable policy 不足
- key store trait との互換問題
- USB close / cancellation 問題

## 7. M4: virtual Classic SDP/HID integration

### 7.1 対象

- `bumble-controller::LocalLink`
- two software controllers
- Classic incoming connection
- pairing
- SDP PSM `0x0001`
- HID control `0x0011`
- HID interrupt `0x0013`
- `bumble_hid::DeviceRuntime`
- `SwbtHidChannelBridge`

### 7.2 作業

- virtual peer を Nintendo Switch role の test driver として実装
- inquiry / page / connection request
- SSP event sequence
- stored link key reconnect の基本 path
- SDP service search / attribute request
- HID control / interrupt channel open order variation
- HIDP handshake / protocol / idle request
- NX output report injection
- swbt input / reply receive
- channel close / ACL disconnect
- malformed HIDP / SDP request
- MTU edge cases
- simultaneous SDP and HID traffic fairness

### 7.3 exit criteria

- physical adapter なしで pair → SDP → HID channels → NX handshake を通せる
- control / interrupt の open order が逆でも両方揃うまで ready にしない
- SDP continuation request が通る
- HIDP control request に `DeviceRuntime` が正しく応答
- invalid PSM / CID / message で panic しない
- virtual disconnect で channel と session を cleanup
- link key を再利用した virtual reconnect が通る
- transport contract tests が fake と Bumble virtual の両実装で通る

### 7.4 stop condition

virtual integration を通せない状態で実機 packet を手作業 patch しない。Bumble adapter boundary または upstream gap を先に解決する。

## 8. M5: Pro Controller fresh pairing と Periodic

### 8.1 対象環境

初期 supported candidate:

- Windows 11
- CSR8510 A10
- WinUSB
- Nintendo Switch 2
- firmware 22.1.0
- `adapter-default` identity

この構成以外は観測を記録できるが、M5 exit criteria には数えない。

### 8.2 作業

- discoverable / connectable window
- fresh pairing
- Pro Controller SDP identity
- HID channel accept
- bootstrap neutral
- observed subcommand sequence
- `0x21` replies
- protocol readiness
- 8 ms periodic input
- A tap、L+R hold、sticks、neutral
- close neutral と ACL drain
- pairing failure trace
- packet / event trace の redaction review
- 20 回以上の clean pairing run

### 8.3 exit criteria

- fresh pairing 成功率と失敗理由を記録
- successful run で control / interrupt / subcommand / ready が trace 可能
- A tap が Switch UI に反映
- L+R が 500 ms 以上保持
- left/right stick direction が反映
- neutral 後に入力が残らない
- close 後に adapter を再 open できる
- 20 successful run 中、hang / leaked handle / stale input が 0
- hardware evidence を date、OS build、dongle ID、driver、Switch firmware と共に保存
- `0.1.0-alpha.1` の release note draft

### 8.4 非対象

- profile reconnect
- Direct
- Joy-Con
- explicit local address

## 9. M6: profile compatibility、reconnect、Direct

### 9.1 profile compatibility

- Python schema v2 parser
- Python fixture read
- Rust write → Python read
- Python write → Rust read
- key field preservation
- atomic create / replace
- lock contention
- controller kind mismatch
- adapter-default namespace resolution
- multiple current peer rejection

### 9.2 reconnect

- stored Classic link key
- active reconnect / incoming bonded reconnect
- no-bond result
- timeout
- stale / rejected bond
- explicit re-pair path。bond を暗黙削除しない
- reconnect failure 後の clean close

### 9.3 Direct

- `DirectProController`
- `send`
- semantic helper transaction
- tap press/release transaction
- no periodic user input
- close neutral exception
- Periodic profile reuse

### 9.4 exit criteria

- Python v0.6.0 profile fixture を Rust が lossless に read
- Rust 作成 profile を Python v0.6.0 が read
- same profile を Periodic / Direct が再利用
- real hardware reconnect が複数 power cycle で成功
- invalid bond を自動削除しない
- Direct idle 中に periodic `0x30` がない
- Direct send failure で snapshot が前 state
- profile update interruption test で元 file または新 file のどちらかが valid
- key material が log / panic output に出ない
- `0.1.0-alpha.2` criteria を満たす

## 10. M7: Joy-Con L/R

### 10.1 対象

- `JoyConL` / `JoyConR`
- `DirectJoyConL` / `DirectJoyConR`
- controller kind profile
- device info / SPI / colors
- SL / SR elapsed time
- side-specific input validation
- Periodic / Direct
- fresh pairing / reconnect

### 10.2 作業順

1. pure protocol fixtures
2. fake runtime validation
3. virtual Bluetooth
4. Joy-Con L hardware
5. Joy-Con R hardware
6. profile reuse
7. Direct

### 10.3 exit criteria

- unsupported button / stick を commit 前に拒否
- Joy-Con L の D-pad / left stick / SL+SR
- Joy-Con R の ABXY / right stick / SL+SR
- controller kind mismatch を adapter open 前に拒否
- colors の SPI bytes が Python fixture と一致
- Periodic / Direct の readiness contract が Pro と同じ
- left / right の evidence を別 run として記録
- `JoyConPair` を暗黙追加していない

## 11. M8: IMU、diagnostics、probe

### 11.1 IMU

- public physical unit conversion
- standard / quaternion mode
- same-mode re-request epoch reset
- ACK ordering
- raw three-frame input
- long-run state stability
- trace redaction

### 11.2 diagnostics

- stable event names
- `GamepadStatus`
- environment snapshot
- report accepted counters
- disconnect reason
- unsupported subcommand
- packet trace opt-in policy
- correlation / session id

### 11.3 `swbt-probe`

subcommand candidate:

```text
swbt-probe adapters
swbt-probe open --adapter usb:0
swbt-probe pair --controller pro --profile path --trace trace.jsonl
swbt-probe reconnect --controller pro --profile path --trace trace.jsonl
swbt-probe profile inspect path
swbt-probe profile verify path
```

probe は public library の consumer とし、private transport API を直接使わない。adapter-only debug だけ例外にする場合は dedicated internal feature とする。

### 11.4 exit criteria

- IMU fixture parity
- IMU mode transition ordering test
- hardware trace で accepted mode を確認
- trace に key / sensitive raw data がない
- probe exit code と error category が文書化
- JSONL schema sample が docs にある
- profile inspect が key value を既定で redaction
- `0.1.0-beta.1` criteria を満たす

## 12. M9: portability と release

### 12.1 Windows

- supported driver setup
- device claim / release
- unplug behavior
- package installation
- release artifact
- troubleshooting

### 12.2 Linux

- libusb permission / udev
- kernel driver detach / reattach
- adapter open / close
- virtual tests
- hardware test
- supported / experimental label の決定

### 12.3 macOS

初期対象外。Bumble USB transport の実用性、permission、driver ownership を調査してから roadmap へ追加する。

### 12.4 release engineering

- public API review
- rustdoc
- examples compile
- `cargo package`
- license files
- SBOM / dependency licenses
- changelog
- semver policy
- security contact
- hardware matrix
- known limitations
- reproducible source baseline

### 12.5 exit criteria

- `0.1.0` checklist 完了
- supported matrix の各構成で fresh pairing と reconnect
- clean install 手順を別 machine で再現
- crate tarball に秘密 fixture / hardware trace がない
- docs の全 command が current code で実行可能
- release commit と Bumble revision が記録
- application backend rollback procedure がある

## 13. explicit local address milestone

M6 以降の独立 milestone とする。release 番号には直結させない。

必要な成果:

- adapter identity backend interface
- CSR8510 A10 command / storage semantics の確認
- expected-address guard
- partial failure recovery
- power cycle test
- profile namespace migration
- duplicate address prevention guidance
- Python profile interoperability
- irreversible operation の明示

この milestone が完了するまで、API 型は存在しても production support は `UnsupportedCapability` とする。

## 14. upstream contribution policy

Bumble gap が見つかった場合、次の順で対応する。

1. swbt transport contract test で必要挙動を固定
2. minimal reproduction を Bumble 側 test として作る
3. upstream issue / PR
4. merge 前に必要なら temporary fork revision
5. upstream merge 後に official revision へ戻す
6. fork-only code と patch note を削除

fork を無期限に保守する前提で architecture を組まない。

## 15. issue 優先度

| severity | 条件 | 対応 |
|---|---|---|
| S0 | key 漏えい、adapter 永続破損、process 外への危険な副作用 | 開発停止、公開 artifact 無効化 |
| S1 | stale input、close hang、pairing key loss、profile corruption | 対象 milestone blocker |
| S2 | reconnect failure、unsupported subcommand、high jitter | release blocker または明示制限 |
| S3 | diagnostics 欠落、ergonomics、build size | roadmap で管理 |
| S4 | cosmetic docs / naming | 通常修正 |

## 16. progress evidence

milestone 完了 PR には次を含める。

- 対象仕様へのリンク
- 実装範囲 / 非範囲
- test command と結果
- fixture provenance
- hardware を使った場合の matrix row
- 未検証事項
- rollback / cleanup
- Bumble revision と upstream issue
- `dev-journal/YYYY-MM-DD.md` の判断記録

「コードが compile する」だけを Bluetooth / protocol milestone の完了条件にしない。
