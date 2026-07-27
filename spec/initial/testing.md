# テスト方針

- 状態: **決定**
- 基準断面: [source-baseline.md](source-baseline.md)
- architecture: [architecture.md](architecture.md)
- roadmap gate: [roadmap.md](roadmap.md)

この文書は、実機なしで固定する契約、Bumble を含む仮想統合、adapter-only test、Switch 実機 test、CI の必須条件を定義する。

## 1. テスト分類

| 分類 | Switch 実機 | USB adapter | Bumble | 目的 |
|---|---:|---:|---:|---|
| Pure unit | 不要 | 不要 | 不要 | 値型、parser、encoder、profile validation |
| Golden differential | 不要 | 不要 | 不要 | Python v0.6.0 と wire / data behavior を比較 |
| Runtime integration | 不要 | 不要 | 不要 | worker、scheduler、Direct transaction、cleanup |
| Transport contract | 不要 | 不要 | fake または Bumble | transport の意味と failure mapping |
| Bumble virtual integration | 不要 | 不要 | 必要 | Classic、pairing、SDP、HID を software controller で統合 |
| Adapter-only | 不要 | 必要 | 必要 | USB open、HCI init、close、unplug |
| Hardware | 必要 | 必要 | 必要 | pairing、reconnect、subcommand、入力反映 |
| Fuzz / model | 不要 | 不要 | 原則不要 | malformed input、state machine、race |
| Packaging | 不要 | 不要 | build 時に必要 | MSRV、docs、crate、license、examples |

default CI は Pure、Golden、Runtime、Transport contract、Bumble virtual、Packaging を必須にする。Adapter-only と Hardware は専用 runner / developer machine で実行する。

## 2. fixture の原則

### 2.1 provenance

全 golden fixture に次を記録する。

```json
{
  "format": "swbt.fixture",
  "schema_version": 1,
  "source_repository": "niart120/swbt-python",
  "source_commit": "84d2723b127f70fc78e12f4496f5c40af0ccfb0a",
  "generator": "tools/generate_python_fixtures.py",
  "case": "pro.neutral.report_0x30"
}
```

fixture 本体が binary の場合、同名の metadata JSON または TOML を置く。

生成元 SHA なしの「期待値」は追加しない。手書き fixture は `origin = "manual"` と根拠を記す。

### 2.2 immutable input

fixture generator は次を固定する。

- Python dependency lock
- Python version
- `swbt-python` commit
- controller kind
- colors
- input state
- timer byte
- report mode / IMU mode
- session state
- monotonic timestamp input
- output report bytes / subcommand payload

Python を Rust test 実行時の必須 dependency にしない。CI は commit 済み fixture を読む。fixture 更新 job だけが Python environment を使う。

### 2.3 review

fixture 更新 PR は generated diff を binary のまま承認しない。hex dump または structured decode を併記し、変更理由を説明する。

## 3. Pure unit tests

### 3.1 `Button`

- enum と wire mapping を分離
- 各 controller kind の supported set
- Pro で `SL` / `SR` reject
- Joy-Con L で ABXY / right stick reject
- Joy-Con R で D-pad / left stick reject
- multiple button packing
- duplicate button input の正規化
- stable logical iteration order

### 3.2 `Stick`

- raw `0` / `2048` / `4095`
- out-of-range を型境界で生成不能または error
- normalized `-1.0` / `0.0` / `1.0`
- Python と同じ asymmetric center conversion
- half-way rounding
- amount `0.0` / `1.0`
- NaN / ±infinity reject
- diagonal `tilt(1.0, 1.0)` を許可
- direction helper の sign

可能なら `proptest` で `raw → accessor`、normalized range、packing invariant を検査する。

### 3.3 `ImuFrame`

- raw i16 min / max
- neutral
- accel / gyro partial constructor
- with-method が反対側を保持
- `0.070 dps/raw` の rad/s round-trip
- `1/4096 G/raw` の G round-trip
- conversion boundary
- NaN / infinity
- 1 frame replication
- invalid frame count 0 / 2 / 4
- same IMU mode re-request が encoding epoch を reset

浮動小数比較 tolerance は式と値を test に明記し、漠然とした epsilon を共有しない。

### 3.4 `InputState`

- neutral buttons / sticks / IMU
- consuming builder
- complete replacement
- semantic candidate generation
- clone / equality
- controller profile validation は state 自体に埋め込まない
- new session neutral reset
- serialization が public contract でないこと

### 3.5 `ControllerColors` / SPI

- default values
- 24-bit boundary
- `0x6050` bytes
- custom colors
- Pro / Joy-Con profile 固有 range
- out-of-range `Rgb24` reject
- color mutation が input state に影響しない

### 3.6 input report `0x30`

- total 49 bytes
- report id
- timer
- battery / connection nibble
- button bytes
- left / right 12-bit stick packing
- vibrator byte
- 36-byte IMU block
- all reserved bytes
- Pro / Joy-Con kind differences
- standard / quaternion / disabled IMU mode
- deterministic explicit time
- next encoding state
- timer wrap `0xFF → 0x00`

### 3.7 output report parser

- `0x01` packet id / rumble / subcommand
- `0x10` rumble-only
- minimum / exact / extra length policy
- unsupported report id
- malformed short packet
- arbitrary bytes never panic
- parser error offset / category
- raw rumble bytes preservation

### 3.8 subcommand responder

対象:

- `0x02` device info
- `0x03` set report mode
- `0x04` trigger buttons elapsed time
- `0x08` shipment / pairing related
- `0x10` SPI read
- `0x21` NFC/IR MCU config ACK
- `0x30` player lights
- `0x40` IMU enable / mode
- `0x48` vibration enable

検証:

- ACK / subcommand id / payload
- supported / unsupported branch
- session transition
- report mode + non-zero lights readiness
- elapsed button mapping by controller kind
- SPI boundary / length
- unsupported command が session を壊さない
- reply state prefix を pure builder が直接決めず sender から受け取る
- `0x40` mode state と ACK ordering 用 effect を分離

### 3.9 profile document

- exact `format`
- schema v2 only
- identity variants
- controller kind `pro` / `joycon_l` / `joycon_r`
- namespaces object shape
- unknown required value reject
- malformed UTF-8 / JSON
- root non-object
- missing field
- address format / U/L bit / I/G bit / reserved LAP
- controller mismatch
- multiple current peers
- key field hex parse
- redacted Debug
- deterministic sorted pretty JSON と trailing newline

## 4. Golden differential tests

### 4.1 wire cases

最低限の fixture set:

```text
fixtures/python-v0.6.0/
  protocol/
    pro/
      neutral-0x30.*
      button-a.*
      buttons-l-r.*
      sticks-boundaries.*
      imu-disabled.*
      imu-standard.*
      imu-quaternion.*
    joycon-l/
    joycon-r/
    output-report-parse.*
    subcommand-replies.*
    spi-ranges.*
```

各 Rust test は bytes が一致しない場合、offset、expected、actual、field decode を表示する。

### 4.2 semantic cases

wire bytes 以外にも次を fixture 化する。

- Stick normalized result
- IMU physical conversion
- supported input matrix
- readiness effect
- profile normalized JSON
- PairingKeys JSON field names
- error category mapping where stable

Python exception message の完全一致は要求しない。

### 4.3 differential runner

開発用 command candidate:

```text
cargo xtask fixtures verify
cargo xtask fixtures regenerate --python ../swbt-python
cargo xtask protocol diff <fixture>
```

`regenerate` は clean worktree と exact source SHA を要求する。current Python main を暗黙使用しない。

## 5. Runtime integration tests

fake transport と fake clock を使い、wall clock wait を避ける。

### 5.1 lifecycle

- build で adapter を開かない
- open
- repeated open
- close
- repeated close
- close without neutral
- reopen
- open failure
- worker panic
- command receiver termination
- Drop best-effort と explicit close の差
- close 中 input reject
- Failed から cleanup

### 5.2 handshake

- link だけでは connect 完了しない
- control / interrupt の両 channel
- bootstrap neutral
- 1 秒 retry
- first valid subcommand 後 retry 停止
- supported `0x03 30`
- unsupported report mode
- non-zero player lights
- zero lights
- reply acceptance failure
- ready 前 disconnect
- timeout
- old session event reject
- handshake state 回収後 scheduler start
- Periodic holdoff
- Direct no-holdoff / no-confirmation-report

### 5.3 common sender ordering

- reply と input の単一 sequence
- timer byte acceptance 後更新
- failed send で timer を進めない
- IMU state acceptance 後更新
- reply state prefix の時点
- `0x40` ACK より新 mode input が後
- close neutral と pending reply
- disconnect と queued send
- multiple inbound subcommands fairness

各 case は accepted report list と semantic event list の両方を検証する。

### 5.4 Periodic scheduler

- exact deadline progression
- send duration を差し引かない absolute schedule
- overrun skip
- no burst catch-up
- latest state
- holdoff
- disconnect pause
- new session epoch
- period validation
- timer wrap
- close cancellation
- 8 ms default
- fake clock で 1 時間相当を即時実行
- report count overflow は u64 saturating / wrapping policy を固定

### 5.5 Direct transaction

- no periodic user input
- send success commit
- build failure no commit
- validation failure no commit
- transport rejection no commit
- transport accepted then later disconnect は commit 済み
- semantic helper report 1 件
- tap press / delay / release
- close during tap
- release failure retains pressed state
- other pre-existing buttons preserved
- profile validation before send
- queue full
- operation response dropped

### 5.6 status

- lifecycle snapshot
- counters
- last subcommand
- disconnect reason
- worker failure
- status call が worker I/O を待たない
- sensitive value 非含有
- cumulative vs session-scoped field reset

## 6. Transport contract tests

fake と Bumble adapter が共有する contract suite を作る。

```rust
fn transport_contract<T: TransportHarness>(factory: impl Fn() -> T) {
    // open / poll / send acceptance / disconnect / close / failure mapping
}
```

検証:

- open 前 send reject
- repeated open policy
- poll timeout
- event order
- control / interrupt routing
- send success の acceptance 意味
- send failure
- disconnect event exactly once
- close idempotence
- terminal source
- no event after final close
- no key in Debug
- bounded queue
- peer / session correlation

fake 固有の都合を contract に入れない。

## 7. Bumble virtual integration

### 7.1 topology

```text
swbt Device + External/Local host adapter
            ↕ LocalLink / software controller
Switch test peer Device
```

physical HCI transportを使わず、Bumble controller / host / L2CAP を実際に通す。

### 7.2 Classic connection

- inquiry visibility
- connection request
- accept / reject policy
- role
- authentication event
- encryption state
- disconnect reason
- reconnect with stored link key
- unknown peer outside pairing window reject

### 7.3 SDP

- PSM `0x0001`
- service search
- service attribute
- service search attribute
- continuation state
- MTU fragmentation policy
- malformed PDU
- multiple sequential clients
- channel close cleanup
- controller kind record bytes

### 7.4 HIDP

- PSM `0x0011` / `0x0013`
- reverse channel open order
- duplicate channel
- wrong PSM
- protocol request / response
- idle request
- suspend / exit suspend
- virtual cable unplug
- data input / output report type
- malformed message
- channel MTU
- disconnect

### 7.5 NX handshake

Switch peer driver が Python fixture の output reports を送信し、Rust response を検査する。

- bootstrap
- device info
- report mode
- SPI reads
- player lights
- IMU / vibration
- ready
- periodic / direct behavior
- reconnect

### 7.6 exit criteria

Bumble virtual suite は default CI で実行し、physical adapter test より前の blocker とする。

## 8. Adapter-only tests

feature または ignored test:

```text
cargo test --features adapter-tests --test adapter_open -- --ignored
```

環境変数例:

```text
SWBT_ADAPTER=usb:0
SWBT_ADAPTER_RUNS=100
```

検証:

- no-open discovery
- primary selector / aliases
- USB claim
- HCI reset
- supported command query
- local address
- Classic capability
- close
- repeated 100 runs
- unplug
- permission denied
- wrong driver
- two similar VID/PID adapters
- serial selector
- descriptor read failure
- process exit 後の再 open

adapter-only test は Switch、pairing、HID PSM を要求しない。

実行結果 metadata:

- UTC / local date
- OS edition / build
- architecture
- Rust version
- Bumble revision
- VID/PID
- serial redacted option
- driver
- libusb version
- run count
- failures

## 9. Hardware tests

### 9.1 marker

Rust test harness で無期限 wait をしない。専用 binary / ignored test を使い、timeout を必須にする。

candidate:

```text
cargo test --features hardware-tests --test switch_pairing -- --ignored --nocapture
```

または:

```text
cargo run --features hardware-tests --bin swbt-hardware-test -- pair-pro ...
```

interactive UI verification が必要な項目は automated pass と区別して記録する。

### 9.2 test stages

1. adapter open
2. discoverability
3. pairing
4. HID control / interrupt
5. subcommand readiness
6. A tap
7. button hold
8. sticks
9. neutral
10. close
11. reconnect
12. Direct
13. IMU
14. Joy-Con

失敗時は stage と最後の event を記録する。

### 9.3 matrix

初期 matrix:

| ID | OS | dongle | driver | console | firmware | controller | identity | status |
|---|---|---|---|---|---|---|---|---|
| W11-CSR-S2-01 | Windows 11 | CSR8510 A10 | WinUSB | Switch 2 | 22.1.0 | Pro | adapter-default | 要検証 |
| W11-CSR-S2-JL | Windows 11 | CSR8510 A10 | WinUSB | Switch 2 | 22.1.0 | Joy-Con L | adapter-default | 要検証 |
| W11-CSR-S2-JR | Windows 11 | CSR8510 A10 | WinUSB | Switch 2 | 22.1.0 | Joy-Con R | adapter-default | 要検証 |
| LNX-USB-S2-01 | Linux | 未確定 | libusb | Switch 2 | 未確定 | Pro | adapter-default | 先送り |

実行ごとに row を上書きせず evidence log を追加する。matrix の status は最終成功 run と known regression を参照する。

### 9.4 pairing

- Switch UI に expected name / icon
- pairing complete
- link key write
- profile validity
- key material redaction
- timeout cleanup
- reject cleanup
- re-pair requires explicit action

### 9.5 channels / subcommands

- PSM / CID open event
- observed order
- output report ids
- observed subcommand ids
- reply acceptance
- ready conditions
- unknown command
- disconnect

### 9.6 input reflection

自動観測が難しいため、証拠を分ける。

- wire accepted: library trace
- console behavior: operator checklist / video / capture
- timing: host monotonic trace
- cleanup: reconnect / UI neutral state

「report accepted」を「Switch UI に反映」と同一視しない。

### 9.7 soak

supported release 前:

- periodic 30 min
- repeated tap 1000
- repeated connect / close
- console sleep / wake
- adapter unplug
- process Ctrl-C
- profile write during pairing
- Direct burst
- IMU long run

stale input、hang、profile corruption は S1 とする。

## 10. Profile compatibility tests

fixture:

```text
tests/fixtures/profiles/
  python-v0.6.0/
    pro-adapter-default-empty.json
    pro-adapter-default-bonded.json
    pro-local-address-bonded.json
    joycon-l-empty.json
    joycon-r-empty.json
    malformed/
```

matrix:

| writer | reader | expectation |
|---|---|---|
| Python v0.6.0 | Rust | lossless domain parse |
| Rust | Python v0.6.0 | load success、same controller kind / identity / keys |
| Python | Python | fixture control |
| Rust | Rust | stable normalized JSON |
| future schema | current Rust | explicit unsupported schema error |

key material fixture は test-only synthetic value とし、実機 key を commit しない。

filesystem failure tests:

- target exists on create
- parent absent
- temp create failure
- write short / error
- sync failure
- atomic replace failure
- lock contention
- process kill point simulation
- read-only file
- invalid permission
- Windows rename semantics
- orphan temp cleanup

## 11. Fuzzing

fuzz target:

- output report parser
- HIDP message parser adapter
- SDP request adapter boundary
- profile JSON parser
- PairingKeys hex conversion
- SPI read range calculation
- controller command decoder if CLI added

invariant:

- no panic
- no unbounded allocation
- maximum input size
- deterministic error category
- no secret echo
- parser never reads out of bounds

seed corpus に Python hardware trace の raw key-free packet と generated boundary cases を使う。

## 12. Concurrency / model testing

候補 tool:

- Loom: command / close / response channel
- Shuttle: scheduler ordering
- Miri: pure unsafe-free code と ownership issue
- ThreadSanitizer: nightly / platform availabilityに応じて

最低 model:

- close vs send
- disconnect vs tap release
- worker failure vs response wait
- queue full vs shutdown
- status update vs worker exit
- Drop vs explicit close

単一 worker architecture でも、public thread、Bumble reader、worker の3者があるため race test を省略しない。

## 13. timing test

実時間 timing test は専用 job とし、unit test の correctness gate と分ける。

測定:

- target period 8 ms
- accepted send timestamp
- p50 / p95 / p99 / max jitter
- overrun count
- command latency
- reply latency
- idle CPU
- shutdown latency

CI VM の hard threshold は flaky になるため、default CI は fake clock invariant を必須にし、dedicated runner だけ performance threshold を持つ。

threshold 初期候補は M2 measurement で決め、根拠なく仕様へ固定しない。

## 14. Diagnostics tests

- event name
- required fields
- session id
- monotonic / wall timestamp
- status / error category
- key redaction
- raw packet default off
- malformed string escaping
- JSONL subscriber sample
- tracing disabled でも behavior 不変
- slow subscriber が worker を block しない構成

library は subscriber callback を同期 hot path で直接呼ばない。`tracing` の callsite overhead を benchmark する。

## 15. CI

### 15.1 required jobs

```text
fmt
check-msrv
check-stable
clippy
unit
golden
runtime-integration
transport-contract
bumble-virtual
doc
examples
miri-selected
cargo-deny
fixture-audit
```

代表 command:

```bash
cargo fmt --all --check
cargo +1.87 check --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo doc --no-deps
cargo deny check
```

Bumble build に C toolchain / protoc が必要なら CI image に明示し、developer machine の偶然の install に依存しない。

### 15.2 optional jobs

```text
adapter-windows
hardware-windows
timing-dedicated
fuzz-nightly
linux-experimental
```

optional job failure を隠さず、status と log artifact を残す。release gate に昇格した構成は required job へ移す。

### 15.3 cache

- Cargo registry / git db / target cache
- Bumble git revision を cache key に含める
- generated fixture を cache にだけ置かず repository に commit
- native build artifact の OS / toolchain 混在を避ける

## 16. code coverage

coverage percentage を単独 gate にしない。次の critical branch coverage を優先する。

- every error kind
- profile failure point
- scheduler overrun
- direct acceptance failure
- handshake timeout / disconnect
- close partial failure
- stale session
- malformed report
- key redaction

coverage report は補助指標として保存する。

## 17. test naming

形式:

```text
<component>_<condition>_<expected>
```

例:

```text
direct_send_transport_reject_keeps_last_accepted_state
handshake_report_mode_without_player_lights_is_not_ready
profile_create_existing_path_does_not_overwrite
periodic_overrun_skips_missed_deadlines
```

`works`、`basic`、`test1` のように契約が読めない名前を使わない。

## 18. release evidence

release candidate ごとに次を保存する。

- source commit
- Bumble revision
- Cargo.lock hash
- CI run
- fixture provenance audit
- hardware matrix
- adapter-only run
- known failures
- license report
- binary / crate checksum
- migration profile round-trip result

実機未検証機能を docs だけで supported と表記しない。
