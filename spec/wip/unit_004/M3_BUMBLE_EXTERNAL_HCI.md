# M3: Bumble external HCI bring-up

- 状態: **着手中**
- branch: `feat/unit-004-m3-bumble-hci`
- 初期設計の正本:
  - `spec/initial/roadmap.md` M3
  - `spec/initial/architecture.md` transport port、Bumble 統合、resource ownership
  - `spec/initial/api.md` adapter discovery、lifecycle、error、公開範囲
  - `spec/initial/testing.md` Adapter-only tests、CI、release evidence
  - `spec/initial/migration-strategy.md` Bluetooth 境界と段階的移行
- Bumble 基準断面:
  - repository: `https://github.com/chaitanyarahalkar/bumble-rs`
  - revision: `bbac2a6803b8cab0920ab725a23aa408fc4fed85`
  - 2026-07-29 時点の upstream `main` と同一
- 前提: `spec/complete/unit_003/M2_RUNTIME.md`
- target adapter: CSR8510 A10 `0A12:0001`、WinUSB、Windows 11
- 最終更新: 2026-07-29

## 1. 目的

M2 の model 非依存 `TransportPort` と worker ownership に、Bumble の external USB HCI
backend を接続する。公開 API では USB adapter の no-open discovery、selector、open、
close、構造化 error を提供し、crate 内部では `ExternalHost` と `Device` を worker thread
だけが所有する。

M3 の成功は Bluetooth HID 互換性を意味しない。M3 で確認するのは、USB HCI adapter の
列挙、claim、reset、capability/local address 取得、activity wake、transport termination、
resource cleanup である。Switch、pairing、SDP、HID channel、NX handshake は M4 以降の
検証対象とする。

## 2. Intent Delta

| 境界 | M2 完了時 | M3 完了後 | 保証 |
|---|---|---|---|
| adapter discovery | opaque `AdapterSelector` だけ | `list_adapters()` と `AdapterInfo` | USB handle を open/claim しない |
| selector | 文字列を保存するだけ | USB index、VID/PID、serial、occurrence、port path を open 時に検査 | Bumble selector 型を公開しない |
| transport open | fake port だけ | Bumble USB split、`ExternalHost`、configured `Device`、HCI initialization | model 型を transport が保持しない |
| capability | protocol 用 address を factory が先に注入 | `TransportPort::open` が local address と HCI/Classic capability を返す | HCI 初期化後の値だけを使う |
| wait | command/shutdown/fake event が coalescing notifier を起動 | Bumble reader enqueue 後も同じ notifier を起動 | idle short polling を導入しない |
| terminal | fake source termination | USB unplug/read failure を sticky terminal state へ変換 | terminal を一度だけ通知し、worker を起こす |
| cleanup | fake transport と outer worker を回収 | reader cancellation、host/device drop、USB release、outer worker join | repeated close と reopen で claim/thread を残さない |
| public lifecycle | `open()` は unsupported | `bumble` feature では HCI-open worker を所有して `Open` になる | `pair()` は M4/M5 前に成功させない |

## 3. 対象範囲

- `AdapterSelector` の crate-private 文字列参照と USB selector 検査
- `AdapterInfo` と `list_adapters()`
- USB device/interface class `E0/01/01` による HCI candidate 判定
- `usb:0`、VID/PID、serial、occurrence、port path
- `bumble` / `adapter-tests` feature と固定 git dependency
- model 非依存 `TransportConfig` と `TransportCapabilities`
- `M::SPEC` から local name、Class of Device、extended inquiry response への値投影
- `open_split_transport`
- `ExternalHost`
- `DeviceConfiguration` と `Device::from_config`
- `ExternalHost::initialize_device`
- local address、HCI/LMP version、Classic capability
- discoverability/connectability を off に保つ HCI configuration
- Bumble reader activity と M2 `ActivityNotifier` の統合
- USB unplug、reader failure、sticky terminal state
- explicit close、repeated close、reader/worker join、process 後の reopen
- adapter discovery/open/initialization error の分類と source 保持
- local address/HCI version を含む secret-free trace
- target adapter による 100 回 open/init/close
- Bumble 込み MSRV、build time/size、license report
- M2 の M3 向け `dead_code` 属性の撤去

## 4. 対象外

- Switch 実機
- incoming Classic connection、pairing、reconnect、stored key
- SDP `0x0001`
- HID control/interrupt PSM、HIDP bridge、accepted send
- NX output、typed input/reply、readiness handshake
- virtual `LocalLink` / software controller
- profile key-store compatibility と atomic profile update
- explicit local address の controller programming
- SCO/audio transport
- public raw USB、HCI、Bumble、CID、custom transport API
- Linux/macOS の adapter 実機保証

`Controller::open()` は HCI resource を取得して lifecycle `Open` まで進める。
`Controller::pair()` と public `create_profile()` は、M4/M5 の Classic/HID path が完成するまで
`UnsupportedCapability` を返し、controller を `Ready` にしない。Open 状態の input 操作は
既存 lifecycle contract に従い成功させない。

## 5. 振る舞い仕様

### 5.1 no-open discovery

- `list_adapters()` は libusb context と device/config/interface descriptor だけを読む。
- USB device handle の open、kernel driver detach、interface claim、configuration/alternate
  setting 変更、HCI command は行わない。
- device descriptor 自体が `E0/01/01`、または device class `0x00` かついずれかの interface
  が `E0/01/01` の device だけを返す。
- candidate 順は Bumble の `usb:N` 選択と同じ libusb 列挙順にする。
- descriptor/config を読めない個別 device は candidate に含めず、secret を含まない trace
  に件数を残す。libusb context または device list 自体を取得できない場合は
  `ErrorKind::AdapterDiscovery` を返して typed source を保持する。
- strict no-open では USB serial string と product stringを読まない。serial descriptor の
  index があるかどうかだけを `AdapterInfo::has_serial_number()` で示す。

`AdapterInfo` は次の不変条件を持つ。

- primary selector: HCI candidate indexに基づく `usb:N`
- `vendor_id()` / `product_id()`: device descriptor の 16-bit 値
- `bus_number()` / `port_numbers()`: libusb topology
- `has_serial_number()`: serial descriptor index の有無であり、serial string の取得成功ではない
- `Debug` に serial string、OS instance ID、profile/key material を含めない

`bumble` feature 無効時も関数と型は公開するが、`list_adapters()` は USB を列挙せず
`UnsupportedCapability` を返す。

### 5.2 selector

M3 が受け付ける selector は Bumble `UsbSpec` と同じ次の形に限定する。

```text
usb:<HCI candidate index>
usb:<VID hex4>:<PID hex4>
usb:<VID hex4>:<PID hex4>/<serial>
usb:<VID hex4>:<PID hex4>#<zero-based occurrence>
usb:<bus decimal>-<port decimal>[.<port decimal>...]
```

- hex は大文字小文字を受け付ける。
- VID/PID だけの場合は Bumble と同じく最初の一致を選ぶ。重複を明示する場合は
  `#occurrence` または serial を使う。
- serial selector は open 処理の一部として device handle を開いて照合する。discovery が
  serial string を先読みしたとは扱わない。
- empty serial、invalid hex/decimal、存在しない candidate、USB 以外の scheme は
  `ErrorKind::TransportOpen`。
- class 判定を迂回する `!` と M3 対象外の `+sco=` は拒否する。
- `AdapterSelector` は利用者の入力を保持し、Bumble `UsbSelector` を公開しない。

### 5.3 TransportConfig

`TransportConfig` は owned、model 非依存の crate-private 値型とする。

- local name
- 24-bit Class of Device
- 240-byte extended inquiry response
- Classic enabled
- accept-any false
- connectable false
- discoverable false

`M::SPEC.protocol` から local name と Class of Device を投影し、complete local name AD structure
を extended inquiry response に格納する。Pro、Joy-Con L、Joy-Con R の値は reporting mode
で変えない。Bumble port は generic `M` を型引数に持たない。

### 5.4 open と初期化順序

open は次の順序を守る。

1. selector と `TransportConfig` を検査
2. `open_split_transport`
3. reader activity/cancellation boundary を組み込んだ `ExternalHost` を生成
4. `DeviceConfiguration` に Classic enabled、accept-any/connectable/discoverable false、
   model 由来 name/Class of Device を設定
5. `Device::from_config(0, config)`
6. `ExternalHost::initialize_device` で HCI Reset、capability、packet pool を初期化
7. `ReadBdAddr` で controller public address を取得
8. local name、Class of Device、Secure Simple Pairing mode、extended inquiry response を設定
9. `WriteScanEnable { scan_enable: 0 }` で inquiry/page scan を停止
10. `TransportCapabilities` を返し、worker loop へ所有権を移す

`Device::power_on` は呼ばない。`initialize_device` と `power_on` の二重 Reset/capability
initialization を避け、M3 は external HCI binary と同じ同期 command 経路を使う。

どの step で失敗しても、取得済み resource に対して reader cancellation、sink/source drop、
USB release、worker 回収を試みる。cleanup failure は primary error を上書きせず related error
として保持する。

### 5.5 capability と diagnostics

`TransportPort::open` は `TransportCapabilities` を返す。

- local public address `[u8; 6]`
- optional HCI version/subversion
- optional LMP version/subversion
- company identifier
- Classic capability
- USB VID/PID/bus/device address

Classic capable は次をすべて満たす場合だけ true とする。

- LMP feature page 0 が存在する
- page 0 byte 4 の `BR/EDR Not Supported` mask `0x20` が clear
- Classic ACL packet length が 0 より大きい
- Classic ACL packet count が 0 より大きい

local address と HCI/LMP version は構造化 trace と adapter-only evidence に残す。pairing key、
profile JSON、USB serial、OS instance ID は trace に出さない。NX device-info に渡す address
byte order は M1 protocol fixture と照合してから接続し、未検証の並びを採用しない。

### 5.6 error 分類

- discovery context/device-list failure: `ErrorKind::AdapterDiscovery`
- selector invalid/not found、permission、driver/backend、USB open/claim/split failure:
  `ErrorKind::TransportOpen`
- HCI Reset、capability、identity command、ReadBdAddr failure:
  `ErrorKind::TransportOpen`
- open 後の source end/read failure: crate-private `TransportErrorKind::SourceTerminated`
- close/worker completion/join failure: 既存 cleanup phase と `WorkerFailed`

open/write の `bumble_transport::Error` は source chain に保持する。reader failure も upstream
境界で文字列だけにせず typed source を保持する。公開 error の `Display` / `Debug` は backend
message や secret を展開せず、`source()` で原因を追えるようにする。

`TransportEnded` は M3 の内部 terminal observation 名であり、新しい public `ErrorKind` は
追加しない。unplug 後の terminal は一度だけ worker を起こし、その後の poll/send/disconnect
では同じ sticky terminal result を返す。`close()` 自体は terminal 後も cleanup を続ける。

### 5.7 activity、termination、close

- reader は packet/ended/failed を `ExternalHost` queue に enqueue した後で M2 の
  `ActivityNotifier` を呼ぶ。
- enqueue 前の通知は、worker の空 poll と enqueue が競合して wake を失うため禁止する。
- worker は command、priority shutdown、report deadline、Bumble reader activity を同じ
  coalescing channel で待つ。
- idle 時の short polling は導入しない。poll interval を代替案にする場合は M2 と同じ
  retained measurement を取り直す。
- USB reader は bounded read の間で cancellation を検査し、explicit close で停止できる。
- `ExternalHost` は reader `JoinHandle` を所有し、shutdown 後の completion と join を返す。
- outer controller worker は transport close の完了後に既存 `WorkerOwner` で join する。
- repeated `close()` は成功し、open/init/close 後の同じ adapter を別 controller/process から
  reopen できる。
- unplug は `TransportEnded` を生成し、reader と outer worker を回収する。

### 5.8 upstream gate

固定 revision の現物監査では次が不足している。

- `ExternalHost` reader queue enqueue 後の activity callback
- reader shutdown API と `JoinHandle`
- USB `PacketSource::read_packet` の cancellation
- reader error の typed source 保持

固定 revision は 2026-07-29 時点の upstream `main` であり、別の未取得 upstream commit は
存在しない。現状の `ExternalHost::new` は reader thread を detach し、USB source は
10 ms endpoint read を内部で無限に繰り返す。このままでは idle reader を停止できず、
100 回 open/init/close、process 後 reopen、worker join を完了扱いにしない。

T06 の green 前に、exact revision を持つ upstream PR または temporary fork で上記を解消する。
temporary fork を使う場合も exact commit を `Cargo.toml` / `Cargo.lock` に固定し、最小再現、
upstream issue/PR、差分、撤去条件をこの spec に記録する。外部 repository への write、
fork 作成、PR 作成はユーザ承認後だけ実行する。

no-open discovery は swbt の公開契約なので repo 内の `rusb` adapter で実装し、上流 public
probe API の有無を M3 completion blocker にしない。

### 5.9 public open/close

- `Controller::open()` は `bumble` feature では production factory を構築し、transport open
  と worker start を完了してから成功する。
- 同じ controller の repeated `open()` は追加 claim/thread を作らず成功する。
- open 成功後の status lifecycle は `Open`、`connected` は false。
- M3 の `pair()` は `UnsupportedCapability` を返し、open resource と lifecycle を維持する。
- `close()` / `close_without_neutral()` は Open 状態でも transport cleanup と join を行う。
- close 後の repeated close は成功する。close 後の同じ object を reopen できる。
- default build では open 前に `UnsupportedCapability` を返し、USB/worker side effect を
  起こさない。

## 6. TDD Test List

各 item は `tdd-one-cycle` で red、green、必要な refactor まで進め、1 item の論理変更を
1 commit にする。

- [ ] **T01 — no-open adapter discovery**
  - fake USB descriptor graph から HCI class candidate だけを `AdapterInfo` へ変換する。
  - fake backend の open/claim counter は 0 のまま。
  - public `list_adapters()` の feature-disabled error も固定する。
- [ ] **T02 — USB selector grammar**
  - index、VID/PID、serial、occurrence、port path、case variationを受け付ける。
  - invalid/unsupported/forced/SCO selector を typed open error にする。
- [ ] **T03 — model-independent config projection**
  - Pro/Joy-Con L/Joy-Con R の name、Class of Device、extended inquiry response を検査する。
  - Periodic/Direct で結果が同じであることを検査する。
- [ ] **T04 — open returns initialized capabilities**
  - `TransportPort::open` を capability-returning contract にし、fake open ordering、
    local address、HCI/LMP、Classic判定、source-preserving error mapping を検査する。
- [ ] **T05 — Bumble dependency and synchronous initialization**
  - fixed revision の transport/host/HCI dependency を feature gate する。
  - injected Bumble boundary で split→host→configured device→initialize→address/identity/
    scan-off の順序と partial failure cleanup を検査する。
- [ ] **T06 — reader wake、cancellation、terminal、join**
  - upstream gate を解消した exact revision を固定する。
  - enqueue 後 wake、wake coalescing、unplug terminal 一回、sticky error、reader shutdown/
    completion/join を検査する。
- [ ] **T07 — production Controller open/close**
  - HCI-open runtime ownershipを Controller に接続する。
  - idempotent open/close/reopen、Open status、M3 pair unsupported、outer worker join を検査する。
  - M2 の M3 向け `dead_code` 属性を撤去する。
- [ ] **T08 — adapter error integration**
  - invalid selector、not found、permission、driver、claim、Reset、ReadBdAddr failure を
    `AdapterDiscovery` / `TransportOpen` に分類し、公開 message を sanitize する。
- [ ] **T09 — adapter-only hardware lifecycle**
  - ignored test で no-open discovery、selector aliases、HCI reset/capability/address、
    Classic capability、unplug、process reopen を検査する。
  - CSR8510 A10 で 100 回 open/init/close を実行し、全 iteration の close/join を記録する。
- [ ] **T10 — package evidence**
  - Rust 1.87 all-features check/test/build、default/no-default build、rustdoc を通す。
  - default 対 all-features の clean build time と binary/rlib size を記録する。
  - dependency license report と Cargo.lock hash を保存する。

## 7. 設計メモ

### 7.1 dependency

現在の optional `bumble` dependency は core-only で I/O API を含まない。M3 は同じ exact
revision の `bumble-transport`、`bumble-host`、`bumble-hci` を direct optional dependency
として追加する。no-open discovery には `rusb` を direct optional dependency として追加する。
`bumble-controller` は M4 の virtual integration まで追加しない。

`bumble-transport` は transport ごとの Cargo feature 分割がなく、Tokio、tonic、WebSocket、
audio 関連も依存 graph に入る。M3 では build time/size/license を測定し、依存削減を
未測定の印象で判断しない。

### 7.2 ownership

`BumbleTransportPort` は `ExternalHost`、`Device`、selector、`TransportConfig`、
`TransportCapabilities`、terminal state を所有し、generic `M` を持たない。worker thread
以外から `ExternalHost` / `Device` を共有しない。`Sync` は要求しない。

M2 の factory は local address を open 前に要求しているため、T04 で
`TransportPort::open -> TransportCapabilities` へ変更し、protocol の device-info address は
HCI `ReadBdAddr` 後に構築する。未検証の placeholder address は使わない。

### 7.3 target adapter

2026-07-29 の read-only PnP 調査では次を確認した。

- CSR8510 A10 `0A12:0001`: `E0/01/01`、WinUSB、状態 OK/Started
- MediaTek `0489:E13A`: `E0/01/01`、BTHUSB、状態 OK/Started

M3 で claim/reset するのは CSR8510 A10 だけとする。MediaTek は Windows Bluetooth stack が
使用中のため開かない。実機 test は candidate index ではなく
`usb:0A12:0001` を使い、列挙順の変化で MediaTek を開かないようにする。PnP instance ID の
末尾は USB serial と確認できていないため、serial selector evidence へ流用しない。最初の
実 USB open/reset 前に対象を再確認し、ユーザの実機操作承認を得る。

## 8. 対象ファイル

- `Cargo.toml`
- `Cargo.lock`
- `deny.toml`
- `src/lib.rs`
- `src/adapter.rs` または `src/adapter/`
- `src/error.rs`
- `src/model/`
- `src/runtime/transport/`
- `src/runtime/worker.rs`
- `src/controller/runtime.rs`
- `src/controller/mod.rs`
- `tests/adapter_discovery.rs`
- `tests/adapter_open.rs`
- `.github/workflows/ci.yml`
- `spec/wip/unit_004/`

対象ファイルは TDD cycle で必要性を確認してから追加する。初期設計の directory tree を
先回りして空 module にしない。

## 9. 検証

通常 gate:

```powershell
cargo fmt --all --check
cargo +1.87 check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo test
cargo build --all-features
cargo build
cargo doc --no-deps --all-features
git diff --check
```

Adapter-only:

```powershell
cargo test --features adapter-tests --test adapter_open -- --ignored
```

package evidence:

```powershell
cargo deny check
```

実行した command、target adapter、driver、Bumble revision、Cargo.lock hash、iteration 数、
unplug 手順、build time/size、license report を completion evidence に記録する。通常 CI や
fake test の成功を USB claim/release、HCI 応答、unplug、100-run の根拠にしない。

## 10. 先送り事項

- accepted Classic channel、pairing、SDP、HIDP: M4
- Switch 実機 pairing と normal-input readiness: M5
- profile key-store trait と Python key compatibility: M6
- Joy-Con hardware matrix: M7
- stable diagnostics event schema と long-run probe: M8
- release package evidence の集約: M9

upstream gate の activity callback、reader cancellation/join、typed reader error は M4 に
先送りしない。M3 completion checklist の前提として解消する。

## 11. 完了チェックリスト

- [ ] T01-T10 がすべて完了している
- [ ] public API に Bumble/rusb 型を公開していない
- [ ] no-open discovery が handle open/claim を行わない
- [ ] CSR8510 A10 の local address、HCI/LMP version、Classic capability を記録した
- [ ] unplug が `TransportEnded` となり reader/outer worker を回収した
- [ ] 100 回の open/init/close が全件成功し、同 adapter を reopen できた
- [ ] `Controller::open` / close / reopen が idempotent
- [ ] M3 の `pair()` が成功を捏造しない
- [ ] Rust 1.87 と通常 quality gate が成功した
- [ ] build time/size と license report を記録した
- [ ] upstream/fork revision、差分、撤去条件を記録した
- [ ] M2 の M3 向け temporary `dead_code` 属性を撤去した
- [ ] placeholder、未根拠の完了表現、secret を含む evidence が残っていない
- [ ] self-review で未実行検証と residual risk を明記した
- [ ] `spec/complete/unit_004/` へ移動した
