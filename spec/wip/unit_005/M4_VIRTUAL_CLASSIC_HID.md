# M4 virtual Classic SDP/HID

- 状態: **着手中**
- milestone: M4
- branch: `feat/unit-005-m4-virtual-classic-hid`
- 正本:
  - `spec/initial/roadmap.md` 7
  - `spec/initial/architecture.md` 14、15、18、19
  - `spec/initial/testing.md` 9、10
- Python 基準断面:
  - repository: `niart120/swbt-python`
  - revision: `84d2723b127f70fc78e12f4496f5c40af0ccfb0a`
- Bumble fork:
  - repository: `https://github.com/niart120/bumble-rs`
  - branch: `fix/external-host-reader-lifecycle`
  - revision: `48f1bc36169b2692d2a61e87eda4223b126dca2b`
  - public fork と branch push だけを許可範囲とし、upstream PR は作成しない

## 1. 目的

物理 adapter を使わず、Bumble の `LocalLink` と software controller 上で incoming Classic
connection、pairing policy、SDP `0x0001`、HID control `0x0011`、HID interrupt `0x0013`、
NX handshake、disconnect/reconnect を一体で検証する。

M4 完了時には、M3 の HCI-open runtime が Classic connection と HID channel を
`TransportEvent` へ変換し、worker が実際の `bumble-host::Device` と SDP/HIDP codec を通して
Ready へ到達できる。Switch 実機での成功は M5 の証拠であり、M4 の virtual 成功から推測しない。

## 2. Intent Delta

| 境界 | M3 完了時 | M4 完了後 | 保証 |
|---|---|---|---|
| model declaration | local name、Class of Device、NX device info、色 | HID report descriptor と SDP policy も同じ model 宣言から投影 | transport が model generic を持たず、構築済み値だけを受け取る |
| pairing command | public `pair()` は unsupported | open runtime の worker command が transport pairing window を開始 | pair 成功は同じ session の NX readiness 後だけ |
| Classic connection | HCI ACL event を worker event へ変換しない | pairing window 中の1 peerだけを受け入れ、connection event を生成 | pairing window 外と2 peer目を暗黙受入しない |
| SDP | 未登録 | PSM `0x0001` で model-specific HID record と continuation を配信 | SDP は worker event にせず transport 内で bounded 処理する |
| HIDP | send は常に reject | PSM `0x0011` / `0x0013` を `bumble-hid::DeviceRuntime` と結ぶ | worker は HIDP header、CID、MTU を扱わない |
| send semantics | fake transport だけが accepted | open interrupt channel の local L2CAP queue 受理後に accepted | air delivery や peer UI 反映を成功条件にしない |
| virtual integration | fake `TransportEvent` 注入 | `Device + LocalLink + software controller` の実 packet path | 6 model×reporting を共通 suite で通す |
| cleanup | HCI reader と outer worker | SDP/HID channel、Classic ACL、session state も順序付き cleanup | disconnect/reconnect で旧 CID と旧 event を再利用しない |

## 3. 現物監査

### 3.1 確認済み

固定 Bumble fork には次が存在する。

- `bumble_host::HostTransport`。`ExternalHost` と
  `bumble_controller::LocalLink` が同じ interface を実装する
- `Device::register_classic_channel_server`、`take_accepted_classic_channels`、
  `classic_channel`、SDU send/take、channel disconnect、ACL disconnect
- `DeviceEvent::ClassicConnectionEstablished` / `Disconnected` と
  `ClassicPairingEvent`
- `bumble_sdp::SdpServer` の service search、attribute、continuation
- `bumble_hid::DeviceRuntime` の control request dispatch と control/interrupt DATA decode
- `ClassicChannel` の PSM、CID、local MTU、peer MTU、state
- `MemoryKeyStore` と BR/EDR SMP/CTKD の stored link-key path

`SdpL2capServer` は `ChannelManager` を直接要求するため、host-owned `Device` からはそのまま
使えない。`SdpServer` と `Device` の SDU API を結ぶ swbt 内部 adapter を置く。
`DeviceRuntime` も同様に、channel SDU と `TransportEvent` の間を薄い adapter で結ぶ。

### 3.2 virtual pairing の証拠範囲

固定 `bumble-controller::LocalLink` は Classic connection と L2CAP を再現するが、外部
controller が発生させる Secure Simple Pairing の全 HCI event sequence は再現しない。
証拠を次の2層に分ける。

- scripted `HostTransport` で IO capability、confirmation、passkey/OOB rejection、
  authentication completion、link-key notification の policy と HCI command を検査する
- `LocalLink` では両 peer に同じ stored Classic link key を与え、暗号化後の BR/EDR
  SMP/CTKD completion、SDP、HID、NX handshake、reconnect を検査する

これは Switch との fresh SSP 成功を示さない。実 HCI adapter と Switch の pairing は M5 で
検証する。

### 3.3 upstream 境界

M4 は現在の fork API だけで実装を開始する。実装中に `Device` から accepted channel、
SDU、pairing event、disconnect、drain を安全に駆動できないことが判明した場合は、
swbt 側の contract test を先に作り、同じ public fork branch へ最小修正を push する。
ユーザ指示により upstream issue/PR は作成しない。

## 4. 対象範囲

- Python v0.6.0 の HID descriptor と SDP policy の source-audit fixture
- model 宣言から transport 用 `HidServiceConfig` への projection
- Bumble SDP service record builder
- SDP channel lifecycle と continuation
- HIDP control/interrupt bridge
- incoming Classic connection と pairing window
- stored link-key、NoInputNoOutput pairing policy、key redaction
- `TransportPort::start_pairing`
- production `BumbleSession` の Classic/HID event、send、drain、disconnect
- public `Controller::pair()` から既存 worker Pair command への接続
- `LocalLink` 上の Switch test peer
- 6 model×reporting の共通 virtual suite
- reverse HID channel order、malformed PDU、MTU、disconnect/reconnect

## 5. 対象外

- Switch 実機、USB adapter を使う pairing
- fresh pairing 成功率、20 run、Switch UI input reflection
- profile schema v2 の link-key lossless read/write、Python/Rust profile compatibility
- filesystem への key 保存、atomic replace、lock contention
- stored peer からの production reconnect と power-cycle
- public `connect()` / reconnect policy
- Pro/Joy-Con 固有の実機 SDP 差
- explicit local Bluetooth address
- IMU mode の追加仕様、long-run diagnostics、probe CLI
- upstream PR

M4 の virtual test で用いる in-memory key は固定の非秘密 test vector とし、production
profile persistence の完成を意味しない。

## 6. 振る舞い仕様

### 6.1 model-specific HID service config

`M::SPEC.protocol` から owned、model 非 generic の `HidServiceConfig` を作る。最低限、
local name、203-byte HID report descriptor、SDP scalar policy を持つ。Bumble の型を
`ModelSpec` と public API に保存しない。

Python revision `84d2723...` の3 model は同じ 203-byte descriptor を使う。SDP policy は
次の値を正本とする。

| field | Pro | Joy-Con L/R |
|---|---:|---:|
| service name | model local name | `Wireless Gamepad` |
| service description | absent | `Gamepad` |
| provider | absent | `Nintendo` |
| device release | absent | `0x0100` |
| Bluetooth profile version | `0x0101` | `0x0100` |
| parser version | `0x0111` | `0x0111` |
| device subclass | `0x08` | `0x08` |
| country code | `0x21` | `0x00` |
| virtual cable | true | true |
| reconnect initiate | true | true |
| remote wake | true | absent |
| HID profile version | `0x0101` | `0x0100` |
| supervision timeout | `0x0C80` | `0x0C80` |
| normally connectable | true | false |
| boot device | false | true |
| SSR max/min | `0xFFFF` / `0xFFFF` | `0x0640` / `0x0320` |

fixture には Python revision、source path、descriptor length と SHA-256、全 scalar 値を記録する。

### 6.2 SDP record

service handle は `0x00010001`。record は最低限次を含む。

- HID service class
- L2CAP + HIDP control PSM `0x0011`
- public browse root
- en / UTF-8 / language base `0x0100`
- service name と model policy に存在する description/provider
- HID profile descriptor
- L2CAP + HIDP interrupt PSM `0x0013`
- HID descriptor list type `0x22`
- HID language ID `0x0409` / base `0x0100`
- 6.1 の HID attributes

SDP PSM `0x0001` を `Device` に1回だけ登録する。accepted channel ごとに独立した
`SdpServer` を作り、peer MTU を response 分割へ渡す。同じ transaction の continuation state
だけを再利用し、disconnect 後の新 channel へ持ち越さない。

1 worker poll 相当で処理する SDP request は最大16 SDU とする。残件がある場合は transport
activity を再通知する。

decode 可能な request は `SdpServer` へ渡す。header から transaction ID を取得できる
malformed request には `INVALID_REQUEST_SYNTAX (0x0003)` を返す。transaction ID も読めない
truncated request はその SDU を破棄して診断 count を進め、worker や process を panic させない。

### 6.3 HIDP bridge

accepted PSM `0x0011` を control、`0x0013` を interrupt とする。channel open event は
同じ CID につき1回だけ生成し、control→interrupt と interrupt→control の両方を受け入れる。

incoming SDU は `bumble_hid::DeviceRuntime` で decode する。

- control/interrupt の OUTPUT DATA `0xA2 + NX report` は HIDP header を除いた
  `NX report` を `TransportEvent::HidOutput` にする
- control request が `SendControl` を返した場合は同じ control CID へ送る
- unsupported control request は Bumble codec の handshake response を返す
- malformed control は invalid-parameter handshake を返せる場合だけ返し、panic しない
- malformed interrupt と非 DATA interrupt は破棄し、worker event にしない

worker からの NX input/reply は `bumble_hid::device_data` で `0xA1` を付け、interrupt CID へ
送る。serialized length が peer MTU を超える場合は queue に入れず `SendRejected`。
control/interrupt の CID、MTU、HIDP header は transport 外へ公開しない。

### 6.4 pairing window

`TransportPort::start_pairing` を追加する。open 前、closed 後、terminal 後は typed error。
open 中の最初の呼び出しは次を行う。

1. pairing session を開始する
2. Classic connectable を true
3. Classic discoverable を true
4. incoming connection request を待つ

同じ session の repeated call は冪等とする。最初に受け入れた peer address を session に
latch し、pairing window 中でも2 peer目は受け入れない。window 外の incoming request は
`accept_classic` しない。

NoInputNoOutput policy は次の HCI command を生成する。

- IO capability request: capability `0x03`、OOB absent `0x00`、
  dedicated bonding / MITM not required `0x02`
- user confirmation: latched peer だけ positive reply
- PIN、passkey、OOB request: negative reply
- authentication/simple-pairing failure: pairing command を失敗させ、HID channel を待たない
- link key request/notification: Bumble key-store path を使い、key bytes を log/error/debug に出さない

Classic ACL が確立したら discoverable を false にする。connectable は HID channel が開くまで
維持し、Ready または pairing failure/timeout/close で false にする。

### 6.5 transport event と session

Classic connection ごとに current connection handle、peer、SDP/control/interrupt CID、
HID runtime を所有する。

- ACL established: `Connected` を1回
- HID channel open: channel ごとに `HidChannelOpened` を1回
- HID DATA: 6.3 の `HidOutput`
- ACL disconnected: reason を保持した `Disconnected` を1回

SDP traffic と pairing event は transport 内部で消費し、worker の NX event queue へ入れない。
worker 向け event queue は64件を上限とする。満杯時は packet を黙って捨てず、sticky
`EventQueueOverflow` にする。

disconnect 後は current handle、peer、全 CID、SDP continuation、HID runtime、pairing window
を破棄する。旧 CID の遅延 SDU と旧 handle の event は新 session に渡さない。

### 6.6 send、drain、disconnect、close

`send_interrupt` は current interrupt channel が Open であり、peer MTU 内の packet を
`Device::send_classic_channel_sdu` が local ACL queue へ受理した場合だけ `ACCEPTED`。

`drain_interrupt(timeout)` は current ACL の host-to-controller packet が完了するまで
transport activity を使って進める。timeout は成功に変換しない。

`disconnect()` は interrupt、control、SDP channel、Classic ACL の順に best-effort cleanup
を続ける。channel が既に消えている場合は成功扱い。primary と cleanup error は既存の
error aggregation 境界へ渡す。`close()` は M3 の reader shutdown/join まで続け、冪等。

### 6.7 public pair

`Controller::pair(timeout)` は open runtime がなければ `TransportClosed`。open runtime では
既存 bounded Pair command を enqueue し、worker が `TransportPort::start_pairing` に成功して
から readiness を待つ。

Pair command の完了条件は同じ connection session で次を満たすこと。

- Classic ACL established
- control/interrupt 両 HID channel open
- bootstrap neutral `0x30` accepted
- supported report-mode `0x03` reply accepted
- non-zero player-lights `0x30` reply accepted
- handshake state 回収

timeout、disconnect、pairing/transport failure は成功にしない。M4 では public
`create_profile()` の production success を有効化しない。

### 6.8 virtual Switch peer

test peer は Bumble `Device + LocalLink` を使い、swbt 側と同じ fixed link key の
in-memory key store を持つ。test-only orchestration は次を packet path 上で実行する。

1. Classic connection
2. encryption と BR/EDR SMP/CTKD completion
3. SDP channel open と service search/attribute continuation
4. HID control/interrupt channel open
5. swbt bootstrap neutral input の受信
6. output `0x01` subcommand `0x03` と `0x30` の送信
7. swbt `0x21` reply の受信
8. Ready 後の typed input `0x30` の受信
9. disconnect、同じ key で virtual reconnect

test peer は report bytes を直接 `TransportEvent` に注入しない。L2CAP SDU と HIDP codec を
必ず通す。

## 7. TDD Test List

- [x] **T01 — model HID/SDP source audit**
  - Python v0.6.0 fixture が descriptor 203 bytes と SHA-256、Pro/Joy-Con policy を固定する。
  - 3 model×2 reporting の projection が reporting mode に依存しない。
- [x] **T02 — SDP record and continuation**
  - 3 model の service record attributes が fixture と一致する。
  - small peer MTU で search/attribute continuation を完走し、channel ごとに state を分離する。
  - malformed/truncated request が panic しない。
- [x] **T03 — HIDP bridge**
  - `0xA2` control/interrupt output を header なし NX payload へ変換する。
  - control response、unsupported、malformed、input `0xA1` encode、peer MTU reject を固定する。
- [x] **T04 — Classic device session**
  - PSM 3件の一度だけの登録、reverse HID channel order、one-shot open event を固定する。
  - Connected/HID output/Disconnected と旧 CID 破棄、64 event overflow を固定する。
- [ ] **T05 — pairing contract and public pair**
  - worker Pair command が `start_pairing` を先に呼び、begin failure を typed error にする。
  - NoInputNoOutput command、peer latch、window 外/2 peer目 reject、key redaction を固定する。
  - public `pair()` を open runtime へ接続し、closed/timeout/disconnect/repeated call を検査する。
- [ ] **T06 — production Bumble port**
  - `BumbleSession` poll が Device、pairing、SDP、HID を駆動して event を返す。
  - interrupt send acceptance、drain timeout、disconnect、close/join の error/cleanup を固定する。
- [ ] **T07 — Pro Periodic virtual end-to-end**
  - actual `LocalLink` packet path で stored-key pairing→SDP continuation→HID→NX Ready を通す。
  - typed A input の `0x30` と neutral cleanup を peer で確認する。
- [ ] **T08 — virtual matrix and resilience**
  - Pro/Joy-Con L/Joy-Con R×Periodic/Direct の6組を共通 suite で通す。
  - reverse channel order、malformed SDP/HIDP、disconnect/reconnect、旧 session event を検査する。
- [ ] **T09 — completion gate**
  - Rust 1.87、all/default/no-default、clippy、test、build、rustdoc、fmt、diff check を通す。
  - fork revision、未実行 hardware/SSP/profile persistence、residual risk を self-review に残す。

### 7.1 TDD cycle evidence

| phase | item | evidence |
|---|---|---|
| refactor-done | T01 | red: `TransportConfig` に `hid_service` がなく compile error。green: Python `84d2723b127f70fc78e12f4496f5c40af0ccfb0a` の clean tree から、Bumble を import せず descriptor 203 bytes、SHA-256 `25f0b3b7e59bdfec05e8cced16e43a8878509865a0cb223f05025c556f3bedba`、3 model の SDP policy を生成し、fixture audit と 3 model projection test が成功。refactor: descriptor/policy の正本を `src/model/hid.rs`、owned runtime projection を `TransportConfig` に分離。生成器再実行前後の fixture SHA-256 は `2F2376E21498163662EF83506EA1EA37EBDCC5CE34D9F4723396393131B9EC4F` で一致。`cargo test --all-targets --all-features --locked`、default test、all/default clippy、Rust 1.87 all-feature check、fmt、diff check が成功 |
| refactor-done | T02 | red: `HidSdpChannel`、service record handle、`bumble` / `bumble-sdp` 依存、`TransportConfig.hid_service` の transport 内可視性がなく compile error。green: fork `48f1bc36169b2692d2a61e87eda4223b126dca2b` の `bumble-sdp::SdpServer` を channel ごとに所有し、3 model の全 service attributes、small MTU continuation、2 channel の continuation 分離、truncated/unknown/length mismatch の `INVALID_REQUEST_SYNTAX` を検査する3 test が成功。refactor: record builder、完全 PDU 長検査、server ownership を private `runtime::transport::sdp` に分離し、Bumble 型を public API へ公開しない。all-feature test 240 passed / 2 ignored、default test 228 passed / 1 ignored、all/default clippy、Rust 1.87 all-feature check、fmt、diff check が成功 |
| refactor-done | T03 | red: `HidpBridge`、typed event/error がなく compile error。green: fork `48f1bc36169b2692d2a61e87eda4223b126dca2b` の `bumble-hid::DeviceRuntime` で control/interrupt `0xA2` を decode し、NX payload から header を除去、unsupported control response `0x03`、unknown event、malformed/trailing bytes、input `0xA1` encode、control/interrupt peer MTU reject を検査する5 test が成功。refactor: codec/dispatch は fork に委譲し、swbt private bridge は header 境界、typed event、MTU だけを所有。all-feature test 245 passed / 2 ignored、default test 228 passed / 1 ignored、all/default clippy、Rust 1.87 all-feature check、fmt、diff check が成功 |
| refactor-done | T04 | red: `ClassicDeviceSession`、3 PSM 定数、`bumble-controller` / `bumble-l2cap` 依存がなく compile error。green: actual `Device + LocalLink` path で PSM `0x0001` / `0x0011` / `0x0013` の一度だけの登録、interrupt→control open、CID one-shot、control/interrupt `0xA2`、malformed control `0x04` response、interrupt noise discard、`0xA1` send と peer MTU、1 poll 16 SDP SDU と再通知、disconnect/旧 handle 破棄、64件 queue の65件目 overflow を検査する6 test が成功。refactor: connection handle/peer、SDP channel ごとの continuation、HID CID/MTU、worker event queue を private session に集約。all-feature test 251 passed / 2 ignored、default test 228 passed / 1 ignored、all/default clippy、Rust 1.87 all-feature check、fmt、diff check が成功 |
| pending | T05-T09 | 各 item の red、green、refactor、command/result を実装 commit ごとに追記する |

## 8. 対象ファイル

- `Cargo.toml`
- `Cargo.lock`
- `src/model/`
- `src/runtime/transport/`
- `src/runtime/worker.rs`
- `src/controller/mod.rs`
- `src/controller/runtime.rs`
- `tests/fixtures/`
- `tests/bumble_virtual.rs`
- `spec/wip/unit_005/`

必要性を TDD cycle で確認したファイルだけを追加する。公開 `transport` module、public custom
transport trait、Bumble 型を含む public API は追加しない。

## 9. 検証

targeted test は各 TDD item の red/green で実行する。completion gate:

```powershell
cargo +1.87.0 check --all-targets --all-features --locked
cargo +1.87.0 test --all-targets --all-features --locked
cargo test --all-targets --locked
cargo test --all-targets --no-default-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo clippy --all-targets --locked -- -D warnings
cargo +1.87.0 build --all-features --locked
cargo +1.87.0 build --locked
cargo +1.87.0 build --no-default-features --locked
$env:RUSTDOCFLAGS="-D warnings"
cargo +1.87.0 doc --all-features --no-deps --locked
cargo fmt --all -- --check
git diff --check
```

virtual test は wall-clock sleep、USB、network、Switch 実機を成功条件に使わない。timeout は
deadlock watchdog としてだけ使い、packet/event/barrier の観測順で成功を判定する。

## 10. 先送り事項

- Switch 2 fresh pairing、20 run、semantic input reflection: M5
- filesystem key store、Python profile compatibility、production reconnect、Direct profile reuse: M6
- Joy-Con 実機と左右別 evidence: M7
- stable diagnostics schema、long-run、probe: M8
- Linux、license/SBOM、release packaging: M9
- explicit local address: 独立 milestone

M4 で必要な accepted Classic channel、SDP/HIDP bridge、pairing window、virtual end-to-end を
M5 の実機調整へ先送りしない。

## 11. 完了チェックリスト

- [ ] T01-T09 がすべて完了している
- [x] Python revision と HID/SDP fixture provenance を記録した
- [x] public API に Bumble/L2CAP/HIDP/SDP 型を公開していない
- [x] PSM `0x0001` / `0x0011` / `0x0013` を実 packet path で検査した
- [x] SDP continuation と malformed request を検査した
- [x] HIDP control/interrupt、reverse order、MTU を検査した
- [ ] pairing window と NoInputNoOutput policy を検査した
- [ ] stored key の virtual pairing/reconnect を検査し、key bytes を出力していない
- [ ] Pro Periodic が virtual packet path で NX Ready へ到達した
- [ ] 6 model×reporting の共通 suite が成功した
- [x] disconnect cleanup と旧 session event 破棄を検査した
- [ ] production Bumble send/drain/disconnect/close を検査した
- [ ] Rust 1.87 と通常 quality gate が成功した
- [ ] upstream PR を作成していない
- [ ] self-review で未実行 hardware/SSP/profile persistence と residual risk を明記した
- [ ] placeholder、未根拠の完了表現、secret を含む evidence が残っていない
- [ ] `spec/complete/unit_005/` へ移動した
