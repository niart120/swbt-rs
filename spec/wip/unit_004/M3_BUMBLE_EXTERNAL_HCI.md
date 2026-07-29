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
- T06 temporary fork:
  - repository: `https://github.com/niart120/bumble-rs`
  - branch: `fix/external-host-reader-lifecycle`
  - revision: `48f1bc36169b2692d2a61e87eda4223b126dca2b`
  - base: `bbac2a6803b8cab0920ab725a23aa408fc4fed85`
  - public fork と branch push だけを実施し、upstream PR は作成していない
- 前提: `spec/complete/unit_003/M2_RUNTIME.md`
- target adapter: CSR8510 A10 `0A12:0001`、WinUSB、Windows 11
- 最終更新: 2026-07-30

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
| capability | protocol 用 address を factory が先に注入 | `TransportPort::open` が local address と HCI/Classic capability を返す | HCI 初期化後の値だけを使い、全ゼロ address と非 Classic controller は worker 起動前に拒否する |
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
- device-level class が `E0/01/01` の場合は config descriptor を要求しない。device class
  `0x00` の場合だけ config/interface descriptor を読み、読めない個別 device は candidate
  に含めず、secret を含まない trace に件数を残す。この規則を Bumble の `usb:N` とそろえ、
  candidate index のずれを作らない。libusb context または device list 自体を取得できない
  場合は `ErrorKind::AdapterDiscovery` を返して typed source を保持する。
- strict no-open では USB serial string と product stringを読まない。serial descriptor の
  index があるかどうかだけを `AdapterInfo::has_serial_number()` で示す。

`AdapterInfo` は次の不変条件を持つ。

- primary selector: HCI candidate indexに基づく `usb:N`
- `vendor_id()` / `product_id()`: device descriptor の 16-bit 値
- `bus_number()` / `port_numbers()`: libusb topology。platform が port path を返さない場合は
  `port_numbers()` を `None` にし、実際の空 path と区別する
- `has_serial_number()`: serial descriptor index の有無であり、serial string の取得成功ではない
- `Debug` に serial string、OS instance ID、profile/key material を含めない

`bumble` feature 無効時も関数と型は公開するが、`list_adapters()` は USB を列挙せず
`UnsupportedCapability` を返す。

### 5.2 selector

M3 が受け付ける selector は Bumble `UsbSpec` のうち次の形に限定する。

```text
usb:<HCI candidate index>
usb:<VID hex1-4>:<PID hex1-4>
usb:<VID hex1-4>:<PID hex1-4>/<serial>
usb:<VID hex1-4>:<PID hex1-4>#<zero-based occurrence>
usb:<bus decimal>-<port decimal>[.<port decimal>...]
```

- VID/PID は 1〜4 桁の hex とし、大文字小文字を受け付ける。
- VID/PID だけの場合は Bumble と同じく最初の一致を選ぶ。重複を明示する場合は
  `#occurrence` または serial を使う。
- serial selector は open 処理の一部として device handle を開いて照合する。discovery が
  serial string を先読みしたとは扱わない。
- empty serial、invalid hex/decimal、存在しない candidate、USB 以外の scheme は
  `ErrorKind::TransportOpen`。
- class 判定を迂回する `!`、M3 対象外の `+sco=`、Bumble transport dispatch が
  metadata と解釈する `[` / `]` は selector 全体で拒否する。これらは serial にも
  許可せず、照合対象 serial が暗黙に切り詰められないようにする。
- `AdapterSelector` は利用者の入力を保持し、Bumble `UsbSelector` を公開しない。

### 5.3 TransportConfig

`TransportConfig` は owned、model 非依存の crate-private 値型とする。

- local name
- 24-bit Class of Device
- complete local name advertising data
- 240-byte extended inquiry response
- Classic enabled
- Classic accept-any false
- connectable false
- discoverable false
- Classic Secure Connections false
- Secure Simple Pairing enabled
- LE / simultaneous LE false

`M::SPEC.protocol` から local name と Class of Device を投影し、complete local name AD structure
を一度生成する。raw AD は `DeviceConfiguration::advertising_data` に設定し、同じ bytes を
zero padding した extended inquiry response に格納する。Bumble の default name
`"Bumble"` を含む advertising data を残さない。Pro、Joy-Con L、Joy-Con R の値は
reporting mode で変えない。Bumble port は generic `M` を型引数に持たない。

### 5.4 open と初期化順序

open は次の順序を守る。

1. selector と `TransportConfig` を検査
2. `open_split_transport`
3. reader activity/cancellation boundary を組み込んだ `ExternalHost` を生成
4. `DeviceConfiguration` に Classic enabled、accept-any/connectable/discoverable false、
   Classic Secure Connections false、Secure Simple Pairing enabled、LE/simultaneous
   false、model 由来 name/Class of Device/advertising data を設定
5. `Device::from_config(0, config)`
6. `ExternalHost::initialize_device` で HCI Reset、capability、packet pool を初期化
7. `ReadBdAddr` で controller public address を取得
8. local name、Class of Device、Secure Simple Pairing mode、extended inquiry response を設定
9. `WriteScanEnable { scan_enable: 0 }` で inquiry/page scan を停止
10. `TransportCapabilities` を返し、worker loop へ所有権を移す

`Device::power_on` は呼ばない。`initialize_device` と `power_on` の二重 Reset/capability
initialization を避け、M3 は external HCI binary と同じ同期 command 経路を使う。
この経路では `DeviceConfiguration` の identity 値や照会結果が controller と `Device` 内部へ
自動同期されない。M3 では `TransportCapabilities` だけを local address/version/features の
正本とし、`Device` 内の未設定 field を診断や protocol address に使わない。

どの step で失敗しても、取得済み resource に対して reader cancellation、sink/source drop、
USB release、worker 回収を試みる。cleanup failure は primary error を上書きせず related error
として保持する。

### 5.5 capability と diagnostics

`TransportPort::open` は `TransportCapabilities` を返す。

- local public address `[u8; 6]`。表示順かつ M1 の NX device-info wire order
- optional version snapshot。存在する場合は HCI version/subversion、LMP version/subversion、
  company identifier の 5 field を一体で保持する
- Classic capability
- USB VID/PID/bus/device address

Classic capable は次をすべて満たす場合だけ true とする。

- LMP feature page 0 が存在する
- page 0 byte 4 の `BR/EDR Not Supported` mask `0x20` が clear
- Classic ACL buffer information が存在する
- Classic ACL packet length が 0 より大きい
- Classic ACL packet count が 0 より大きい

全ゼロ local address は controller identity として受け付けず、crate-private
`InvalidControllerIdentity` を source に持つ `ErrorKind::TransportOpen` とする。Bumble の
`Address::address_bytes()` は HCI little-endian order なので、T05 の Bumble 境界で一度だけ
反転して表示/NX wire order にする。`TransportCapabilities` から `SwitchHidProtocol` へは
そのまま渡し、二度目の反転を行わない。

local address と HCI/LMP version は構造化 trace と adapter-only evidence に残す。pairing key、
profile JSON、USB serial、OS instance ID は trace に出さない。NX device-info に渡す address
byte order は M1 protocol fixture と照合する。汎用 `Debug` は local address を表示せず、
診断時は明示した構造化 field として記録する。

### 5.6 error 分類

- discovery context/device-list failure: `ErrorKind::AdapterDiscovery`
- selector invalid/not found、permission、driver/backend、USB open/claim/split failure:
  `ErrorKind::TransportOpen`
- HCI Reset、capability、identity command、ReadBdAddr failure:
  `ErrorKind::TransportOpen`
- open 後の source end/read failure: crate-private `TransportErrorKind::SourceTerminated`
- close/worker completion/join failure: 既存 cleanup phase と `WorkerFailed`

open/read/write/flush の `bumble_transport::Error` は source chain に保持する。reader/writer
failure も upstream 境界で文字列だけにせず typed source を保持する。serial selector の
VID/PID 一致候補で device open または serial read が失敗した場合も、単なる not-found に
畳み込まず permission/driver source を保持する。公開 error の `Display` / `Debug` は backend
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
- sink write/flush error の typed source 保持
- serial selector の device open/read error 保持

固定 revision は 2026-07-29 時点の upstream `main` であり、別の未取得 upstream commit は
存在しない。現状の `ExternalHost::new` は reader thread を detach し、USB source は
10 ms endpoint read を内部で無限に繰り返す。このままでは idle reader を停止できず、
100 回 open/init/close、process 後 reopen、worker join を完了扱いにしない。

T05 の partial failure cleanup evidence は、有限 scripted source が `None` を返した後に
source/sink の `Drop` を観測した範囲に限る。production USB reader の cancellation、
completion、join、USB claim release、terminal state、reopen は確認していない。固定 revision
の `ExternalHost::new` を使う production initializer は Controller / `TransportPort` から
到達不能のまま保つ。T06 で cancellation-aware read、shutdown API、owned `JoinHandle`、
enqueue 後 wake、typed reader/writer error を持つ exact revision を固定するまで production
path へ接続しない。scripted `Drop` 成功を production cleanup の根拠にしない。

T06 では public temporary fork
`https://github.com/niart120/bumble-rs/tree/fix/external-host-reader-lifecycle` を作成し、
`48f1bc36169b2692d2a61e87eda4223b126dca2b` を `Cargo.toml` / `Cargo.lock` に固定した。
base は upstream `bbac2a6803b8cab0920ab725a23aa408fc4fed85` であり、差分は
`bbac2a6803b8cab0920ab725a23aa408fc4fed85..48f1bc36169b2692d2a61e87eda4223b126dca2b`
の 5 files、+482/-63 lines である。変更対象は `bumble-transport` の
`common.rs`、`host.rs`、`lib.rs`、`usb.rs`、`tests/usb.rs` に限る。

fork では queue send 後の activity callback、`PacketSourceShutdown`、reader completion と
owned `JoinHandle`、USB bounded-read cancellation、reader/write/flush error の typed source、
serial selector の open/read error 伝播を追加した。fork の
`cargo test --workspace --all-targets`、`cargo test -p bumble-transport --all-targets`、
`cargo +1.87.0 check -p bumble-transport --all-targets` は成功した。変更対象に対する
`cargo clippy -p bumble-transport --lib -- -D warnings` と
`cargo clippy -p bumble-transport --test usb -- -D warnings` も成功した。Windows での
fork 全 target clippy と厳格 rustdoc は、base に既存の `tests/specs.rs` の未使用
`PacketSink` import と、`dispatch.rs` の Unix 限定 `UnixServer` link で失敗する。この
2件は T06 差分に混ぜていない。

2026-07-30 のユーザ指示では public fork と branch push だけが許可され、upstream PR の作成は
明示的に禁止された。upstream PR は 0 件である。upstream issue も write 許可の対象外だった
ため作成していない。当初の「upstream issue/PR を記録する」という条件は、この run では
許可境界に従って「未作成理由と branch revision を記録する」に置き換える。この fork branch
は公式 upstream へ取り込まれた実績ではない。

temporary fork の撤去条件は、公式 upstream revision が同等の enqueue 後 callback、
reader shutdown/completion/join、USB cancellation、typed reader/write/flush error、serial
selector error を備えること、その revision へ pin を戻した状態で T06 の 3 lifecycle test、
Rust 1.87 check、all-features/default test、clippy、build、rustdoc が成功することとする。
条件を満たした後にだけ `Cargo.toml` / `Cargo.lock` を公式 revision へ更新し、temporary
branch への依存を削除する。

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

- [x] **T01 — no-open adapter discovery**
  - fake USB descriptor graph から HCI class candidate だけを `AdapterInfo` へ変換する。
  - descriptor probe の capability に open/claim 操作を含めず、production path に
    `Device::open`、detach、claim がないことを確認する。
  - public `list_adapters()` の feature-disabled error も固定する。
- [x] **T02 — USB selector grammar**
  - index、VID/PID、serial、occurrence、port path、case variationを受け付ける。
  - invalid/unsupported/forced/SCO selector を typed open error にする。
- [x] **T03 — model-independent config projection**
  - Pro/Joy-Con L/Joy-Con R の name、Class of Device、extended inquiry response を検査する。
  - Classic/SSP/SC/LE/scan policy と、Periodic/Direct で結果が同じことを検査する。
- [x] **T04 — open returns initialized capabilities**
  - `TransportPort::open` を capability-returning contract にし、fake open ordering、
    local address、HCI/LMP、Classic判定、source-preserving error mapping を検査する。
  - 非 Classic controller は worker 起動前に拒否し、開いた transport を cleanup する。
  - capability の表示順 address が反転せず NX device-info 応答へ届くことを検査する。
- [x] **T05 — Bumble dependency and synchronous initialization**
  - fixed revision の transport/host/HCI dependency を feature gate する。
  - injected Bumble boundary で split→host→configured device→initialize→address/identity/
    scan-off の順序と有限 scripted transport の partial failure cleanup を検査する。
    production reader の cancellation/join と USB release は T06 で検査する。
- [x] **T06 — reader wake、cancellation、terminal、join**
  - upstream gate を解消した exact revision を固定する。
  - enqueue 後 wake、wake coalescing、unplug terminal 一回、sticky error、reader shutdown/
    completion/join を検査する。
- [x] **T07 — production Controller open/close**
  - HCI-open runtime ownershipを Controller に接続する。
  - idempotent open/close/reopen、Open status、M3 pair unsupported、outer worker join を検査する。
  - M2 の M3 向け `dead_code` 属性を撤去する。
- [x] **T08 — adapter error integration**
  - invalid selector、not found、permission、driver、claim、Reset、ReadBdAddr failure を
    `AdapterDiscovery` / `TransportOpen` に分類し、公開 message を sanitize する。
- [x] **T09 — adapter-only hardware lifecycle**
  - ignored test で no-open discovery、selector aliases、HCI reset/capability/address、
    Classic capability、unplug、process reopen を検査する。
  - CSR8510 A10 で 100 回 open/init/close を実行し、全 iteration の close/join を記録する。
- [x] **T10 — package evidence**
  - Rust 1.87 all-features check/test/build、default/no-default build、rustdoc を通す。
  - default 対 all-features の clean build time と binary/rlib size を記録する。
  - dependency license report と Cargo.lock hash を保存する。

### 6.1 TDD cycle evidence

| state | item | evidence |
|---|---|---|
| refactor-done | T01: no-open adapter discovery | red: `cargo test descriptor_discovery_returns_only_bluetooth_hci_without_opening_or_claiming` は `AdapterInfo`、descriptor probe、classification 未定義の compile error。green: descriptor success/source-failure の unit test 2件と feature-disabled public integration test が成功し、device-level または interface-level `E0/01/01` だけを stable `usb:N` candidate にした。inventory failure は `AdapterDiscovery` と typed source を保持し、公開 message へ source text を出さない。refactor: discovery に渡す capability を descriptor record の取得だけに限定し、USB open/claim method を境界から除外。production `rusb` path にも open/detach/claim はなく、device/config/interface descriptor、bus、取得できた port path、serial index だけを読む。取得不能な port path は実際の空 path と区別して `None` にする。device-level HCI class では config descriptor を要求しない。個別 device の descriptor 読み取り失敗は candidate から除外し、件数だけを `tracing` event に残す。`cargo test --all-targets --all-features --locked` と default は lib 217 passed / 1 ignored と integration/example 全件、clippy `-D warnings`、rustdoc `-D warnings` が成功 |
| refactor-done | T02: USB selector grammar | red: `cargo test usb_selector_accepts_the_supported_bumble_subset --all-features --locked` は `UsbSelector`、`AdapterSelector::as_str` / `parse_usb`、`ErrorKind::TransportOpen` 未定義の compile error。green: supported subset、invalid/unsupported syntax、serial redaction の unit test 3件が成功。index、hex 1〜4桁の VID/PID、serial、occurrence、bus/port path を owned internal typeへ変換し、ASCII 数字、`u8` / `u16` / `usize` overflow、empty segment、非 USB scheme、`pyusb:`、force、SCO、dispatch metadata を `TransportOpen` にした。refactor: VID/PID の選択方法を `First` / `Serial` / `Occurrence` に分けて不可能状態を除き、raw/parsed selector と error の `Debug` / `Display` から serial を除外。`cargo test --all-targets --all-features --locked` と default は lib 220 passed / 1 ignored と integration/example 全件、clippy `-D warnings`、rustdoc `-D warnings` が成功 |
| refactor-done | T03: model-independent config projection | red: `cargo test transport_config_projects_model_protocol_metadata_into_complete_local_name_eir --all-features --locked` は `ControllerConfig::transport_config` 未定義の compile error。green: 3 model の name、`0x002508` Class of Device、Complete Local Name AD、zero-padded 240-byte EIR、Classic/SSP/SC/LE/scan policy と、各 model の Periodic/Direct 同値性を unit test 2件で固定。refactor: generic `ControllerConfig<M, R>` から model metadata だけを owned、非 generic の `TransportConfig` へ投影し、raw AD と Classic EIR を同じ TLV から生成。固定 Bumble revision では raw AD を `DeviceConfiguration::advertising_data`、240 bytes を HCI `WriteExtendedInquiryResponse` へ別々に渡すことを確認。`cargo test --all-targets --all-features --locked` と default は lib 222 passed / 1 ignored と integration/example 全件、clippy `-D warnings` が成功 |
| refactor-done | T04: open returns initialized capabilities | red: `cargo test initialized_capabilities_preserve_identity_versions_and_classic_requirements --all-features --locked` は capability 型、version/USB/Classic metadata、`FakeTransport::with_capabilities` が未定義の compile error。green: capability field と Classic 判定条件、Classic ACL metadata 不在、全ゼロ address、repeated fake open、非 Classic の worker 起動前拒否と cleanup、distinctive address の device-info 応答を unit test 4件で固定。partial-open error は public `TransportOpen`、crate-private `OpenFailed`、typed backend source の三段を保持した。refactor: `RuntimeComponents` の先行 address 注入を削除し、`TransportPort::open` の immutable snapshot だけを protocol 構築へ渡した。version は 5 field の atomic `Option`、Classic ACL metadata は `Option` にして upstream の不在をゼロ値に変換しない。`cargo test --all-features --locked` と default は lib 226 passed / 1 ignored と integration/doc test 全件、`cargo clippy --all-targets --all-features --locked -- -D warnings`、all-features/default build、rustdoc `-D warnings`、`cargo fmt --check`、`git diff --check` が成功 |
| refactor-skipped | T05: Bumble dependency and synchronous initialization | red: `cargo test bumble_initialization_uses_configured_device_and_exact_hci_order --all-features --locked` は `bumble-hci`、`bumble-host`、`bumble-transport` と初期化 module が未定義の compile error。green: `cargo test runtime::transport::bumble_tests --all-features --locked` は 4件成功。split opener だけを置換し、固定 revision の実 `ExternalHost`、`Device::from_config`、`initialize_device`、`send_command` を使って selector、USB metadata、model 由来 `DeviceConfiguration`、Reset/capability/ReadBdAddr/identity/scan-off の command 順序、HCI little-endian address の一度だけの反転、Command Complete の型と status を固定した。初期化途中の sink failure は crate-private `OpenFailed` と typed Bumble source を保持し、有限な scripted source/sink の両方を解放した。これは実 USB reader の cancellation、join、interface release を証明しない。固定 upstream の detached reader と typed error 欠落は T06 blocker のままとした。direct dependency は exact revision の `bumble-hci`、`bumble-host`、`bumble-transport` に限定し、core `bumble` は transitive のみにした。Cargo.lock の package は 27 から 236（+209）へ増え、HEAD と共通する package の版は変えていない。`cargo test --all-targets --all-features --locked` は lib 230 passed / 1 ignored と integration/example 全件、`cargo test --locked` は default で lib 226 passed / 1 ignored と integration/doc test 全件が成功した。`cargo +1.87.0 check --all-targets --all-features --locked`、clippy `-D warnings`、all-features/default build、rustdoc `-D warnings` も成功。production ownership の構造変更は T06 の upstream gate 解消後に行うため、この cycle では refactor を追加していない。実機 USB は未実行 |
| refactor-done | T06: reader wake、cancellation、terminal、join | red: `cargo test bumble_reader_enqueues_before_wake_and_coalesces_activity --all-features --locked` は `ActivityNotifier` を受ける初期化境界と `BumbleSession::poll` / `close` が未定義の compile error。green: fork revision `48f1bc36169b2692d2a61e87eda4223b126dca2b` を固定し、`ExternalHost::new_with_activity_callback` から M2 の bounded `ActivityNotifier` を queue enqueue 後に起動した。controlled source は wall-clock polling を使わず `Condvar` で packet/end/failure/shutdown を制御し、初期化中の複数 wake の coalescing、wake 後の zero-time poll、clean end と typed reader error の terminal wake 1回、同じ source を持つ sticky `SourceTerminated`、explicit/repeated close の cancellation/completion/join、source/sink 各1回の drop を 3 test で固定した。close 後の `BumbleRuntime` drop は host/device を同時に解放し、T07 の Controller 配線や M4 の HID event 変換は追加していない。refactor: `host` と `device` の別々の `Option` を `Option<BumbleRuntime>` にまとめ、不整合な半閉じ状態を除いた。`cargo test runtime::transport::bumble_tests --all-features --locked` は T05 を含む7件成功。`cargo test --all-targets --all-features --locked` は lib 233 passed / 1 ignored と integration/example 全件、`cargo test --locked` は default で lib 226 passed / 1 ignored と integration/doc test 全件が成功した。all-features/default clippy `-D warnings`、Rust 1.87 all-targets/all-features check、all-features/default build、rustdoc `-D warnings`、fmt、diff check も成功。実機 USB は未実行 |
| refactor-done | T07: production Controller open/close | red: `cargo test controller_open_is_idempotent_preserves_open_on_unsupported_pair_and_reopens_after_join --all-features --locked` は `open_controller_runtime` と Controller の runtime install seam が未定義の compile error。green: `BumbleTransportPort` が selector と model 非依存 config を所有し、`TransportPort::open` で T06 の `BumbleSession` を生成する production path を追加した。reader terminal 後の poll/send/drain/disconnect は同じ typed source を返し、close は reader cleanup を続ける。`Controller::open()` は `bumble` feature で HCI 初期化と outer worker start の完了後に成功し、status は `Open` / `connected=false` になる。repeated open は factory を再実行せず、M3 の `pair()` は `UnsupportedCapability` を返して open runtime を維持する。fake transport の drop flag と open/drain/disconnect/close counter により、`close()` と `close_without_neutral()` が cleanup、worker completion、join を終えてから返ること、repeated close、同じ Controller の reopen を固定した。default feature の公開 open は従来どおり side effect 前に unsupported。refactor: Open から所有する内部値を `ReadyRuntime` ではなく `ControllerRuntime` とし、public generic `Controller<M, R>::open()` は sealed reporting dispatch で維持した。到達可能になった M2 の M3 向け `dead_code` 理由を除去し、M4/M5 の pairing/HID/profile 境界と feature-disabled build だけを未到達として区別した。README、crate rustdoc、public open/pair rustdoc も feature ごとの現状へ更新。`cargo test --all-targets --all-features --locked` は lib 233 passed / 1 ignored と integration/example 全件、`cargo test --locked` は default で lib 227 passed / 1 ignored と integration/doc test 全件が成功した。all-features/default clippy `-D warnings`、Rust 1.87 all-targets/all-features check、all-features/default build、rustdoc `-D warnings`、fmt、diff check も成功。USB claim/release、実 adapter reopen、process reopen は未実行で T09 の対象 |
| refactor-skipped | T08: adapter error integration | red: 新しい失敗は発生しなかった。T08 の regression matrix を追加した最初の targeted run で4 testが成功し、T01 の discovery、T02 の selector、T04 の公開 `TransportOpen` mapping、T05 の HCI initialization がすでに要求を実装していた。green: discovery inventory failure は既存 test で `AdapterDiscovery` と typed source を保持する。invalid selector は公開 `Controller::open()` を通して `TransportOpen` となり、runtime を install しない。production path は selector parse を opener より前に実行する。決定的に注入した Bumble の not-found `InvalidSpec`、permission `rusb::Error::Access`、driver `NotSupported`、claim `Busy` は public `Error` → `TransportError::OpenFailed` → typed `bumble_transport::Error` の source chain を保持する。failed Reset は typed Bumble source、incomplete/failed ReadBdAddr は typed initialization source を持つ `OpenFailed` になり、公開および transport error の `Display` / `Debug` に selector/backend text を出さない。production code の変更が不要だったため refactor は追加していない。`cargo test --all-targets --all-features --locked` は lib 237 passed / 1 ignored と integration/example 全件、`cargo test --locked` は default で lib 227 passed / 1 ignored と integration/doc test 全件が成功した。all-features/default clippy `-D warnings`、Rust 1.87 all-targets/all-features check、all-features/default build、rustdoc `-D warnings`、fmt、diff check も成功。T09 は current WinUSB の成功経路を実機確認したが、permission/driver failure を作る device/driver 状態変更は行っていない |
| refactor-done | T09: adapter-only hardware lifecycle | red: `cargo test --features adapter-tests --test adapter_open -- --ignored` は `adapter-tests` feature 不在で失敗した。green: Windows 11 が CSR8510 A10 `0A12:0001` を `WinUSB` / `libwdi` device として認識した状態で、ignored hardware target を実行した。descriptor-only discovery は2回同じ target を返して 0.04 s で成功。candidate index、VID/PID、`#0` occurrence、bus/port path の4 alias は各 open/init/close に成功し、serial descriptor がないため serial alias の実機検査は対象外。crate 内 capability test は local address `00:1B:DC:F9:9F:7D`、HCI `0x06/0x22BB`、LMP `0x06/0x22BB`、company `0x000A`、Classic ACL capability、USB VID/PID を記録して 0.34 s で成功した。別 process 2回の open/init/close は 0.70 s、100回の連続 lifecycle は全 iteration の `close_without_neutral()` 完了後に次へ進み 31.77 s で成功した。物理 unplug は reader terminal と outer worker failure/cleanup を 19.23 s で検出し、挿し直し後は Windows の `OK` 再認識と別 process 2回の reopen 0.71 s を確認した。refactor: `adapter-tests = ["bumble"]` を hardware suite の opt-in 境界にし、production initialization の secret-free `tracing` event に local address、HCI/LMP version、Classic capability、USB VID/PID/bus/address を明示 field として追加した。test 実行時は subscriber output を保存していない。permission/driver failure を作る device/driver 状態変更は未実行で、typed classification は T08 の決定的 test を根拠とする。`cargo test --all-targets --all-features --locked` は lib 236 passed / 2 ignored、hardware target 5 ignored と integration/example 全件、`cargo test --locked` は default で lib 227 passed / 1 ignored と integration/doc test 全件が成功した。all-features/default clippy `-D warnings`、Rust 1.87 all-targets/all-features check、all-features/default build、rustdoc `-D warnings`、fmt、diff check も成功 |
| refactor-skipped | T10: package evidence | red: `cargo deny check` は host に command がなく失敗し、platform filter なしの offline metadata は未取得 target-specific `atomic-polyfill 1.0.3` を要求して停止した。green: Rust 1.87.0 の all-features check/test/build、default/no-default build、rustdoc `-D warnings` が成功した。空の別 target directory と共用 source cache を使う一回の clean release build は default 3.53 s / rlib 2,580,250 bytes、all-features 21.71 s / 4,028,046 bytes。Windows target に絞った metadata inventory は211 package、license metadata 欠落0件で、Bumble 21 package は Apache-2.0、`serialport 4.9.0` は MPL-2.0。`Cargo.lock` SHA-256 は `1B5C4504519933A22B78C8B2CABBAB112A26AF8CE360C3559385DBF7EFEE9BE9`。測定方法、全 license expression、非 MIT/Apache 系 package、未検証範囲は [package evidence](evidence/package-windows-msvc-20260730.md) に保存した。production behavior を変えない evidence item のため refactor は追加していない |

## 7. 設計メモ

### 7.1 dependency

T05 前の optional `bumble` dependency は core-only で I/O API を含まなかった。T05 で同じ
exact revision の `bumble-transport`、`bumble-host`、`bumble-hci` を direct optional
dependency とし、core `bumble` の direct dependency を除いた。core はこれらの transitive
dependency として残る。no-open discovery には `rusb` を direct optional dependency として
使用する。
`bumble-controller` は transport/host の transitive dependency には含まれるが、M4 の
virtual integration まで direct dependency に追加しない。

`cargo tree --all-features --locked -p swbt-rs --depth 1` で確認した direct graph は
`bumble-hci 0.1.0`、`bumble-host 0.1.0`、`bumble-transport 0.1.0`、`rusb 0.9.4`、
`serde_json 1.0.151`、`tracing 0.1.44` である。Bumble 3 crate は T06 で temporary fork の
`48f1bc36169b2692d2a61e87eda4223b126dca2b` を指す。
`cargo tree --no-default-features --locked -p swbt-rs --depth 1` の direct graph は
`serde_json 1.0.151` だけで、Bumble、rusb、tracing は default graph に入らない。

`bumble-transport` は transport ごとの Cargo feature 分割がなく、Tokio、tonic、WebSocket、
audio 関連も依存 graph に入る。T05 の Cargo.lock では package が 27 から 236（+209）へ
増えた。既存 package の版更新は含まない。T10 の Windows clean release build では
all-features の wall time は default の 6.15 倍、rlib は 56.1% 増だった。license inventory
と測定条件は [package evidence](evidence/package-windows-msvc-20260730.md) を正本とする。

### 7.2 ownership

T06 の `BumbleSession` は `BumbleRuntime` として `ExternalHost` / `Device` を同時に所有し、
`TransportCapabilities` と sticky terminal state を保持する。close は reader
cancellation/completion/join 後に runtime 全体を drop する。T07 でこの session を
`BumbleTransportPort` に組み込み、selector と `TransportConfig` を含む open lifecycle を
接続する。generic `M` は持たず、worker thread 以外から `ExternalHost` / `Device` を共有しない。
`Sync` は要求しない。

M2 の factory は local address を open 前に要求しているため、T04 で
`TransportPort::open -> TransportCapabilities` へ変更し、protocol の device-info address は
HCI `ReadBdAddr` 後に構築する。未検証の placeholder address は使わない。

### 7.3 target adapter

2026-07-30 の PnP と T09 実機調査では次を確認した。

- CSR8510 A10 `0A12:0001`: `E0/01/01`、WinUSB / libwdi、状態 OK/Started
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
cargo metadata --offline --all-features --locked `
  --filter-platform x86_64-pc-windows-msvc --format-version 1
Get-FileHash Cargo.lock -Algorithm SHA256
```

`cargo-deny` が導入済みなら `cargo deny check` も実行する。未導入の場合は global tool を
暗黙に追加せず、metadata inventory の範囲と deny/advisory 未実行を evidence に残す。

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
