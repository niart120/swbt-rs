# テスト方針

- 状態: **決定**
- 基準断面: [source-baseline.md](source-baseline.md)
- 型関係: [type-modeling.md](type-modeling.md)
- architecture: [architecture.md](architecture.md)
- roadmap gate: [roadmap.md](roadmap.md)

この文書は、Python 基準断面とのwire差分、model宣言と動的変換、typed runtime、profile lifecycle、Bumble仮想統合、adapter-only test、Switch実機test、CIの必須条件を定義する。

## 1. テスト対象の原則

Rust compilerが型として保証する性質を、専用のcompiler UI testで再検査しない。

対象外:

- 存在しないassociated constantの呼び出し
- 存在しないmethodの呼び出し
- 異なる`Button<M>`や`InputState<M>`間の代入
- Directに`apply()`がないこと
- Periodicに`send()`がないこと
- Direct builderに`report_period()`がないこと

`trybuild`、compile-pass fixture、compile-fail fixture、compiler stderr snapshotは導入しない。library、examples、rustdoc、通常のtest codeがcompileすることは`cargo check`、`cargo test`、`cargo doc`で確認する。

テスト対象にするのは、compilerだけでは正しさを判断できないものに限る。

- modelごとのsupported button集合
- `ControllerKind`、profile名、`ModelSpec`の対応
- `(ControllerKind, ButtonKind)`からNX wire位置へのmapping
- 動的`ButtonKind`変換
- profile JSONの検査
- report bytes
- runtime stateと送信順序
- transport、pairing、cleanup、failure semantics

## 2. テスト分類

| 分類 | Switch実機 | USB adapter | Bumble | 目的 |
|---|---:|---:|---:|---|
| Model / mapping audit | 不要 | 不要 | 不要 | model宣言、button集合、wire mappingの検査 |
| Pure unit | 不要 | 不要 | 不要 | 値型、parser、encoder、profile |
| Golden differential | 不要 | 不要 | 不要 | Python v0.6.0とのwire/data比較 |
| Runtime integration | 不要 | 不要 | 不要 | worker、scheduler、Direct、cleanup |
| Transport contract | 不要 | 不要 | fakeまたはBumble | transport意味とfailure mapping |
| Bumble virtual | 不要 | 不要 | 必要 | Classic、pairing、SDP、HID統合 |
| Adapter-only | 不要 | 必要 | 必要 | USB open、HCI init、close、unplug |
| Hardware | 必要 | 必要 | 必要 | pairing、reconnect、input反映 |
| Fuzz / model | 不要 | 不要 | 原則不要 | malformed input、state machine、race |
| Packaging | 不要 | 不要 | build時に必要 | MSRV、docs、crate、license、examples |

Default CIはModel / mapping audit、Pure、Golden、Runtime、Transport contract、Bumble virtual、Packagingを必須にする。
PureとGoldenは`swbt-core`で実行し、その依存graphにBumble、`rusb`、`tracing`、profile writerが
含まれないことを検査する。runtimeはdefault/no-defaultの両方でBumble、`rusb`、`tracing`を含む。

## 3. model宣言とmappingのaudit

model宣言を単一正本にしても、宣言したdomain data自体が正しいとは限らない。次を検査する。

- `ControllerModel::KIND`がmodelごとに一意
- `ControllerKind`とprofile文字列が一対一
- supported button集合に重複がない
- supported button集合がPython基準断面と一致する
- stick capabilityがmodel仕様と一致する
- `M::SPEC.kind == M::KIND`
- `M::SPEC.profile_name == M::PROFILE_NAME`
- 全supported buttonにwire mappingが存在する
- unsupported buttonにwire mappingを公開しない
- `TryFrom<ButtonKind> for Button<M>`の成功集合がsupported集合と一致する

`ButtonKind`の数値は論理IDであり、wire位置とは別契約として検査する。

```text
(ControllerKind, ButtonKind)
    -> report byte index
    -> bit mask
```

Joy-Con L/Rの`SL`と`SR`を含め、model差をgolden fixtureで固定する。`ButtonKind as u8`をreport offsetやbit numberへ直接使う実装は採用しない。

## 4. fixture原則

全golden fixtureにprovenanceを記録する。

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

fixture generatorが固定するもの:

- Python dependency lock / version
- source commit
- controller kind
- colors
- logical button / stick / IMU input
- timer byte
- report mode / IMU mode
- monotonic timestamp
- output report / subcommand payload

PythonをRust test実行時の必須dependencyにしない。CIはcommit済みfixtureを読む。

fixture更新時はbinary差分だけで承認せず、hex dumpまたはstructured decodeを併記する。

## 5. Pure unit tests

### 5.1 `ButtonKind`とwire mapping

- explicit discriminant `0x00..=0x13`
- discriminant重複なし
- logical order
- arbitrary numeric code parse error
- modelごとのbutton集合
- `(ControllerKind, ButtonKind)` mapping
- byte index / mask boundary
- Joy-Con L/Rの`SL` / `SR`
- reserved bitを立てない

### 5.2 `Button<M>`の動的境界

- `kind()` projection
- supported `ButtonKind`だけ`TryFrom`成功
- unsupported kindは`UnsupportedInput`
- duplicate button入力の正規化
- stable logical iteration

型が異なるmodel間で代入できないこと自体はtestしない。

### 5.3 `Stick`

- raw `0` / `2048` / `4095`
- out-of-range
- normalized `-1.0` / `0.0` / `1.0`
- Pythonと同じasymmetric conversion
- rounding
- amount boundary
- NaN / infinity
- diagonal tilt

値型testをmodelごとに複製しない。modelが持つstick能力はmodel宣言auditで扱う。

### 5.4 `ImuFrame` / `ImuSamples`

- i16 boundary
- neutral
- partial constructor / with-method
- `0.070 dps/raw` round-trip
- `1/4096 G/raw` round-trip
- non-finite / overflow
- `Repeat`が3frameへ展開
- `Frames`が順序保持

`ImuFrame`は全model共通なので、model別に同じ変換testを複製しない。

### 5.5 `InputState<M>`

各modelについて次を検査する。

- neutral
- supported buttonの保持
- complete replacement
- semantic candidate生成
- clone / equality
- new session neutral reset
- stick値の保持
- IMU frameの保持

Proはdual sticks、Joy-Con Lはleft only、Joy-Con Rはright onlyというdomain dataはmodel宣言auditで検査する。

### 5.6 colors / SPI

- default
- RGB24 boundary
- `0x6050` bytes
- custom colors
- model profile range
- input state非干渉

### 5.7 input report `0x30`

- 49 bytes
- report ID / timer
- battery / connection nibble
- model-specific button mapping
- model-specific stick packing
- vibrator byte
- 36-byte IMU block
- reserved bytes
- IMU mode
- deterministic time
- next encoding state
- timer wrap

### 5.8 parser / responder

- `0x01` / `0x10`
- malformed packet
- arbitrary bytesでpanicしない
- raw rumble preservation
- subcommand `0x02` / `0x03` / `0x04` / `0x08` / `0x10` / `0x21` / `0x30` / `0x40` / `0x48`
- model-specific device info / SPI / elapsed button
- readiness effects
- unsupported commandでsession不変
- reply prefixをtyped stateから取得

### 5.9 profile document / typed profile

- exact format / schema v2
- identity variants
- kind strings
- namespace shape
- malformed UTF-8 / JSON
- address validation
- multiple peer rejection
- strict Classic key parseとunknown/legacy/LE field rejection
- secret-safe `Debug`
- deterministic JSON
- `PairingProfile<M>`のkind mismatch

## 6. builderとprofile lifecycle tests

### 6.1 `build()`

- `profile_path=None`でephemeral controller
- existing matching profileでconfigured controller
- existing mismatched profileをadapter open前に拒否
- nonexistent pathは`ProfileNotFound`
- `build()`でadapter / worker / USBを作らない

### 6.2 `create_profile()`

fake filesystemとfake transportを使い、順序をevent logで検査する。

```text
profile_plan
profile_create_empty
profile_configure_typed
transport_open
pairing_start
protocol_ready
return_controller
```

検証:

- profile path未指定は`ProfilePathRequired`
- targetを事前検査せず、create-new競合は`ProfileAlreadyExists`、上書きなし
- runtimeのdefault/no-default buildはいずれもBumble backendへ到達する
- invalid identityはtransport open前に失敗
- empty envelope persistenceがtransport openより先
- 同じcreate呼出しでは保存後のprofile readを行わず、保存bytesとruntime configが同じ型付きprofileから作られる
- envelopeのcontroller kindが`M::KIND`
- pairing failureでもvalid empty envelopeが残る
- failure時にworker / transportをcleanup
- success時のreturn controllerは`Ready`
- existing empty profileのretryはbuild→open→pairで行う

controller objectに`create_profile()`が存在しないこと自体はtestしない。

explicit local addressは入力検証、書換え前後のreadback、失敗時の
`AdapterIdentityRecoveryRequired`、秘密値を出さないdiagnosticsを検査する。

### 6.3 persistence failure

- parent create failure
- temp create / write / flush / sync failure
- create-new race
- atomic replace failure
- orphan temp cleanup
- Windows rename semantics

自動backup testは作らない。更新中断では旧または新fileのどちらかがvalidであることを検査する。
同一pathの複数writerは非対応のため、lock contentionとstale-writer検出をtest対象にしない。

## 7. Golden differential tests

```text
tests/fixtures/python-v0.6.0/
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
      dpad.*
      left-stick.*
      sl-sr.*
    joycon-r/
      button-a.*
      right-stick.*
      sl-sr.*
    output-report-parse.*
    subcommand-replies.*
    spi-ranges.*
  profile/
    pro.*
    joycon-l.*
    joycon-r.*
```

semantic fixture:

- Stick conversion
- IMU conversion
- model supported button matrix
- button wire mapping
- stick capability matrix
- readiness effect
- profile JSON
- PairingKeys field names

差分時はoffset、expected、actual、field decode、modelを表示する。

## 8. Runtime integration tests

fake transport / fake clockで、次の6組を共通harnessに通す。

```text
Pro × Periodic
Pro × Direct
JoyConL × Periodic
JoyConL × Direct
JoyConR × Periodic
JoyConR × Direct
```

### 8.1 lifecycle

- open / repeated open
- close / repeated close
- close without neutral
- reopen
- open failure
- worker panic / command termination
- Drop vs explicit close
- close中input reject

### 8.2 handshake

- linkだけではconnect完了しない
- control / interrupt両channel
- bootstrap neutral / retry
- valid subcommand後retry停止
- supported report mode
- non-zero lights
- acceptance failure
- ready前disconnect
- timeout
- stale session
- Periodic holdoff
- Direct no-holdoff

### 8.3 sender ordering

- reply / input単一sequence
- timer acceptance後更新
- failed sendでtimer不変
- IMU state acceptance後更新
- reply prefix typed state
- `0x40` ACK ordering
- close neutral / pending reply

### 8.4 Periodic

- local state commit
- deadline progression
- overrun skip / no burst
- latest state
- holdoff
- disconnect pause
- new session
- 8ms default

### 8.5 Direct

- no periodic user input
- success commit
- validation / rejection no commit
- accepted then disconnectはcommit済み
- semantic helper report 1件
- tap transaction
- release failure retains pressed

### 8.6 runtime projection

- status kindを`M::KIND` / `R::KIND`から導出
- model-specific `ModelSpec`をprotocolへ渡す
- transport eventはmodel非依存
- runtime coreにuntyped button vectorを永続保持しない

## 9. Transport contract tests

model-independent suite:

- open前send reject
- repeated open
- poll timeout
- control / interrupt routing
- acceptance意味
- disconnect exactly once
- close idempotence
- terminal source
- bounded queue
- key非出力

transport contractに`Button<M>`やcontroller-specific stateを入れない。

## 10. Bumble virtual integration

```text
ControllerWorker<M, R>
      ↕ TransportPort
Bumble Device + LocalLink
      ↕
Switch test peer
```

- inquiry / connection / role
- authentication / encryption
- stored link key reconnect
- SDP `0x0001`
- HID `0x0011` / `0x0013`
- reverse channel order
- malformed SDP / HIDP
- model-specific service record
- typed report
- disconnect cleanup

6組を通し、pairing / SDPがreporting modeに依存しないことを確認する。

## 11. Adapter-only tests

```text
cargo test --features adapter-tests --test adapter_open -- --ignored
```

- no-open discovery
- selector / aliases
- USB claim
- HCI reset / capabilities
- local address
- Classic capability
- repeated 100 runs
- unplug
- permission / driver error
- process exit後再open

## 12. Hardware tests

初期matrix:

| ID | OS | dongle | driver | console | firmware | model | reporting | identity | status |
|---|---|---|---|---|---|---|---|---|---|
| W11-CSR-S2-P-P | Windows 11 | CSR8510 A10 | WinUSB | Switch 2 | 22.5.0（ユーザ報告） | Pro | Periodic | adapter-default | M5/M6/M8で検証済み |
| W11-CSR-S2-P-D | Windows 11 | CSR8510 A10 | WinUSB | Switch 2 | 22.5.0（ユーザ報告） | Pro | Direct | adapter-default | M6で検証済み |
| W11-CSR-S2-L-P | Windows 11 | CSR8510 A10 | WinUSB | Switch 2 | 22.5.0（ユーザ報告） | Joy-Con L | Periodic | adapter-default | M7で検証済み |
| W11-CSR-S2-L-D | Windows 11 | CSR8510 A10 | WinUSB | Switch 2 | 22.5.0（ユーザ報告） | Joy-Con L | Direct | adapter-default | M7で検証済み |
| W11-CSR-S2-R-P | Windows 11 | CSR8510 A10 | WinUSB | Switch 2 | 22.5.0（ユーザ報告） | Joy-Con R | Periodic | adapter-default | M7で検証済み |
| W11-CSR-S2-R-D | Windows 11 | CSR8510 A10 | WinUSB | Switch 2 | 22.5.0（ユーザ報告） | Joy-Con R | Direct | adapter-default | M7で検証済み |

stages:

1. adapter open
2. profile create-newまたはexisting load
3. discoverability
4. pairing
5. HID channels
6. subcommand readiness
7. model-supported button
8. model-supported stick
9. IMU
10. neutral
11. close
12. reconnect
13. Direct

「report accepted」と「Switch UI反映」を区別する。

M8のPro Periodicは、stored-key reconnectから60秒のnon-neutral IMU、neutral report、close、profile
完全一致、adapter reopenまでをmachine traceで確認した。別の15秒pure yaw runでは、ユーザがSwitch画面の
横移動、目視カクつきなし、終了後の移動・入力残りなしを確認した。machine traceの
`trace_elapsed_ns`はstatus投影後のsubscriber観測時刻で、無線送信完了や画面反映時刻ではない。両runの
subscriber観測intervalは8 ms目標に対してp95 errorが1周期を超えたため、UI成功とは別にM9のS2
release制限として扱う。詳細は
[`unit_009 evidence`](../complete/unit_009/evidence/pro-imu-diagnostics-windows-20260801/SUMMARY.md)
を参照する。

## 13. Profile compatibility tests

| writer | reader | expectation |
|---|---|---|
| Python v0.6.0 | Rust `PairingProfile<M>` | matching modelのみ成功 |
| Rust | Python v0.6.0 | same kind / identity / keys |
| Rust | Rust | stable normalized JSON |
| future schema | current Rust | explicit unsupported error |

actual hardware keyをfixtureにcommitしない。

## 14. Fuzz / concurrency

fuzz:

- output parser
- HIDP adapter
- SDP boundary
- profile JSON
- PairingKeys hex
- SPI range
- ButtonKind parser
- button wire mapping index

invariant:

- no panic
- no unbounded allocation
- deterministic error category
- no secret echo
- unsupported dynamic buttonから`Button<M>`を生成しない
- wire mappingがreserved領域を越えない

concurrency model:

- close vs send
- disconnect vs tap release
- worker failure vs response wait
- queue full vs shutdown
- status update vs worker exit
- Drop vs explicit close

## 15. timing

専用jobで測定:

- 8ms period
- accepted timestamp
- p50 / p95 / p99 / max jitter
- overrun
- command / reply latency
- idle CPU
- shutdown latency

Default CIはfake clock invariantを必須にする。

## 16. CI

required:

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
model-mapping-audit
```

```bash
cargo fmt --all --check
cargo +1.87 check --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo doc --no-deps
cargo deny check
```

`trybuild`、`cargo test --test ui`、compiler stderr snapshot jobは追加しない。

optional:

```text
adapter-windows
hardware-windows
timing-dedicated
fuzz-nightly
linux-experimental
```

## 17. release evidence

- source commit
- Bumble revision
- Cargo.lock hash
- CI run
- model / mapping audit
- fixture provenance
- profile creation ordering test
- hardware matrix
- adapter-only run
- known failures
- license report
- crate checksum
- profile round-trip

型制約そのものの証拠として人工的な不正コードのcompile failureを保存しない。release evidenceはdomain mapping、runtime behavior、wire互換性、実機結果へ集中させる。
