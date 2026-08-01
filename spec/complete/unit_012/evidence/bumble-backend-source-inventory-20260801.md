# Bumble backend source / API inventory

- 監査日: 2026-08-01 (JST)
- Bumble source: `niart120/bumble-rs@cb55e2d98dc7b7b0227c43772c9ae184034dd9a1`
- 対象: `swbt-rs` の `bumble` feature が必要とする Bluetooth Classic HID backend
- 状態: T05 green
- red evidence: 監査前の
  `Test-Path spec/wip/unit_012/evidence/bumble-backend-source-inventory-20260801.md` は
  `False`

ここでいう「採用」は、既存 Bumble package を path / Git dependency として残すことではない。
固定 revision の source と test から必要な振る舞いを単一 `swbt-bumble-backend` package の
内部 module へ整理し、Apache-2.0 の派生コードとして履歴と改変表示を保持することを指す。

## 1. 現在の依存境界

`cargo tree --locked --features bumble -e normal` では、自己所有 fork 由来の通常依存は22 packageである。
`swbt-rs` が直接宣言する8 packageと、backendでの扱いは次のとおり。

| current package | production 利用 | current consumer | backendでの扱い |
|---|---|---|---|
| `bumble` | address、UUID、pairing key / key store | `bumble.rs`、`classic.rs`、`sdp.rs`、`profile_key_store.rs` | 必要な値型とClassic bond契約だけを内部化する |
| `bumble-controller` | なし。`swbt-rs` からの直接利用はtest-only | `classic.rs` tests、`virtual_tests.rs` | 通常依存にはしない。Classic仮想peerのbehavior oracleとしてtest移植に使う |
| `bumble-hci` | HCI command / event / ACL codec | `bumble.rs`、`classic.rs` | 必要なpacket codecとcommand/eventだけを内部化する |
| `bumble-hid` | HIDP messageとdevice-side dispatch | `hidp.rs` | `src/lib.rs` のprotocol runtimeを内部化し、既存L2CAP adapterは採用しない |
| `bumble-host` | external controller上のDevice、Classic connection、ACL/L2CAP、pairing event | `bumble.rs`、`classic.rs` | full `Device` を持ち込まず、Classic host stateだけを抽出する |
| `bumble-l2cap` | Classic channel、signaling、SDU queue | `classic.rs` | Classic/basic signalingと現在のERTM交渉処理を内部化し、LE credit channelを除く |
| `bumble-sdp` | SDP PDU、data element、request server | `sdp.rs` | codec / service serverを内部化し、既存L2CAP bindingを採用しない |
| `bumble-transport` | USB open、split source/sink、external reader、controller初期化 | `bumble.rs` | USB HCIとreader lifecycleだけを抽出し、汎用transportと高水準profileを除く |

残る14 packageは `bumble-host` / `bumble-transport` の通常依存から入る。

- `bumble-att`、`bumble-gatt`、`bumble-profiles`: LE/GATT host機能であり不採用。
- `bumble-smp`、`bumble-crypto`: LE SMP / CTKD機能であり不採用。Classic link keyは専用bond型へ縮める。
- `bumble-a2dp`、`bumble-audio`、`bumble-avc`、`bumble-avctp`、`bumble-avdtp`、
  `bumble-avrcp`、`bumble-codecs`、`bumble-rtp`: audio / AV機能であり不採用。
- `bumble-rfcomm`: Classic HIDが利用しないprotocolであり不採用。

## 2. 公開API境界

backendの利用者に、`bumble_*` moduleの型、HCI packet、L2CAP CID、SDP PDUを公開しない。
`swbt-rs::runtime::transport::TransportPort` との対応は次の意味契約に固定する。

| backend operation / type | 入出力と所有権 | `swbt-rs` の対応 |
|---|---|---|
| `open` | USB selector、controller / HID設定、任意の明示local address、bond store、activity callbackを受け、readerとsessionを所有する | `TransportPort::open` |
| `Capabilities` | local address、HCI/LMP version、Classic ACL buffer、USB metadataをbackend固有の値型で返す | `TransportCapabilities` |
| `start_pairing` | discoverable/connectable windowを開始し、最初のpeerだけを採用する | `TransportPort::start_pairing` |
| `start_reconnect` | 保存済みpeer/keyを使い、incomingまたはactive reconnectを開始する | `TransportPort::start_reconnect` |
| `poll` | timeout付きでHCI/Classic/L2CAP/SDP/HIDPを駆動し、domain-neutral eventを返す | `TransportEvent` へ変換する |
| `send_interrupt` / `drain_interrupt` | HID interrupt SDUの受理とhost-side ACL flushを管理する | 同名の`TransportPort` operation |
| `disconnect` / `close` | HID channel、ACL、USB readerの順で停止し、reader cancellation / joinを完了する | 同名の`TransportPort` operation |
| `BondStore` | peer addressをkeyに `ClassicBond` をload/upsert/deleteする。pathやprofile schemaを知らない | `profile_key_store.rs` が実装する |
| `ClassicBond` | 16-byte link key、link-key type、認証状態だけを持つ | profile内のBumble互換key JSONへ変換する |

`TransportConfig`、controller model、reporting mode、profile file DTOは `swbt-rs` に残す。
backendはHID service recordを構築するための値だけを `OpenOptions` 相当の設定で受ける。
明示local addressのCSR command sequenceはbackend内部で完結させ、generic HCI型を公開APIへ出さない。

## 3. 採用するsourceと振る舞い

### 3.1 core values / bond

| source | 採用対象 | 除外対象 | 移植後の検証元 |
|---|---|---|---|
| `bumble/src/address.rs` | public/random addressの6-byte表現、parse/display、address type | advertising address policy | `bumble/tests/acceptance.rs`、current profile/reconnect tests |
| `bumble/src/uuid.rs` | 16/32/128-bit UUIDとSDP byte order | well-known UUID catalog | `bumble/tests/acceptance.rs`、`sdp.rs` tests |
| `bumble/src/keys.rs` | Classic link keyの値、store failure contract | `JsonKeyStore`、MemoryKeyStore production利用、LTK/IRK/CSRK解決処理 | `bumble/tests/key_store.rs`、`profile_key_store.rs` tests |

`PairingKeys` 全体をbackendの公開型にしない。既存profileにあるLE用fieldと未知extensionは
`swbt-rs` 側が保持し、backendとの往復ではClassic link keyだけを置換する。

### 3.2 HCI codec

採用元は `bumble-hci/src/codes.rs`、`command.rs`、`event.rs`、`packet.rs`、
`return_parameters.rs`、`metadata.rs`、`metadata_tables.rs` である。必要な観測面は次のとおり。

- controller初期化: `Reset`、`ReadLocalSupportedCommands`、`ReadLocalVersionInformation`、
  `ReadLocalExtendedFeatures`、`SetEventMask`、`LeSetEventMask`、`ReadBufferSize`、`ReadBdAddr`。
- NX identity: `WriteLocalName`、`WriteClassOfDevice`、`WriteSimplePairingMode`、
  `WriteExtendedInquiryResponse`、`WriteDefaultLinkPolicySettings`、`WriteScanEnable`。
- CSR identity: generic vendor command、Vendor Event、応答なしwarm reset。
- Classic接続: connection request/complete、accept/reject/create、role change、disconnect、
  authentication、encryption、completed-packet flow control。
- SSP/link key: PIN、IO capability、user confirmation/passkey、OOB、simple-pairing complete、
  link-key request/notificationのcommand/event。
- data path: HCI ACL fragmentation/reassemblyとCommand Complete / Command Status credit。

Android/Zephyr vendor definition、LE advertising/GATT/ISO/SCO command surfaceは不採用。
T06では生成済みfileを丸ごと採用しただけで完了とせず、上記variantをbackend内部codecへ固定する。
behavior oracleは `bumble-hci/tests/acceptance.rs`、`acl_fragments.rs`、
`controller_return_parameters.rs`、`generated_commands.rs`、`generated_events.rs` と
`swbt-rs/src/runtime/transport/bumble_tests.rs` のexact command sequenceである。

### 3.3 USB HCI / external reader

| source | 採用対象 | 除外対象 | 移植後の検証元 |
|---|---|---|---|
| `bumble-transport/src/common.rs` | USB用error、`PacketSource` / `PacketSink` / shutdown trait | serial、gRPC、WebSocket error | `bumble-transport/tests/specs.rs` |
| `bumble-transport/src/usb.rs` | USB selector、interface/endpoint分類、auto-detach、claim/release、HCI command/event/ACL read-write、shutdown | SCO isochronous transferと直接`libusb1-sys` callback | `bumble-transport/tests/usb.rs`、adapter tests |
| `bumble-transport/src/dispatch.rs` | `SplitOpenedTransport` の所有権契約だけ | scheme dispatch、TCP/UDP/serial/PTY/VHCI/WebSocket | `bumble_tests.rs` scripted split transport |
| `bumble-transport/src/command_channel.rs` | blocking command response型の意味 | package間公開型 | `bumble_tests.rs` initialization/error tests |
| `bumble-transport/src/host.rs` | `ExternalHost` reader queue、activity、direct command、device drive、shutdown/join | LE pairing、GATT client、CTKD、audio/ISO helper | reader lifecycle testsとcurrent 100-run session test |

normal dependencyは `rusb`（`vendored`）を残す。SCOを除くため `libusb1-sys` をbackendの直接依存にせず、
`serialport`、`tokio`、`tonic`、`prost`、`tungstenite`、`regex` を追加しない。
backend自身のsourceに `unsafe` を持ち込まない。

### 3.4 Classic host / L2CAP

`bumble-host/src/lib.rs` は10,105行、`bumble-transport/src/host.rs` は4,299行あり、
full moduleの再配置は単一backend化ではない。次のsymbol/behaviorだけを新しい内部`classic_host`へ抽出する。

- HCI host transportのcommand、ACL send、event drain。
- Classic controller bufferとcompleted-packet credit。
- incoming/active ACL connection、peer address、role、disconnect。
- connectable/discoverable、Classic SSP、authentication/encryption、link-key persistence。
- connectionごとのClassic L2CAP manager、server登録、accepted channel、SDU queue、output flush。
- transport loss時のordered flushとterminal event。

`bumble-host/src/configuration.rs` のGATT database、LE privacy、advertising、SMP managerは採用しない。
backend専用configurationはClassic/HID値だけを持つ。`HostTransport` にあるSCO/ISO/LE pump methodと
`bumble-controller::LocalLink` 実装もproductionから外す。

L2CAPは `bumble-l2cap/src/lib.rs` のframe/signaling codec、`classic.rs`、`ertm.rs` を採用し、
`le_credit.rs` を除外する。現在のNX pathはBasic modeだが、peerが送るretransmission/FCS optionを
現在と同じく解釈・拒否または交渉するため、ERTM option処理は残す。

behavior oracle:

- `bumble-host/tests/classic_channels.rs`
- `bumble-host/tests/configured_l2cap.rs`
- `bumble-host/tests/device_events.rs`
- `bumble-l2cap/tests/classic_channels.rs`
- `bumble-l2cap/tests/complete_signaling.rs`
- `bumble-l2cap/tests/information_signaling.rs`
- `swbt-rs/src/runtime/transport/classic.rs` tests
- `swbt-rs/src/runtime/transport/virtual_tests.rs`

`bumble-host/tests/classic_ctkd.rs` と `bumble-smp` testsは、採用しないCTKD/LE SMPのoracleであり、
backend testへ移植しない。

### 3.5 SDP / HIDP

| boundary | 採用source | 除外source | behavior oracle |
|---|---|---|---|
| SDP | `bumble-sdp/src/lib.rs`、`pdu.rs`、`service.rs` | `l2cap.rs`。channel lifecycleはClassic sessionが所有する | `bumble-sdp/tests/service.rs`、`sdp.rs` tests |
| HIDP | `bumble-hid/src/lib.rs` | `l2cap.rs`。SDU送受信はClassic sessionが所有する | `bumble-hid/tests/protocol.rs`、`hidp.rs` tests |

SDP PSM `0x0001`、HID control `0x0011`、HID interrupt `0x0013` と、model別service record、
continuation state、reverse channel order、peer MTU、malformed PDUの現行契約を維持する。

### 3.6 swbt-owned orchestration seam

次のsourceはBumble由来ではないが、backend側へ移すかbackend公開APIで完全に包む必要がある。

| current swbt source | T06 ownership |
|---|---|
| `src/runtime/transport/bumble.rs` | USB/external host/session部分をbackendへ移し、`TransportPort` adapterだけをswbtに残す |
| `src/runtime/transport/classic.rs` | Classic session stateをbackendへ移し、event変換だけをswbtに残す |
| `src/runtime/transport/hidp.rs` | backend内部HIDP runtimeへ移す |
| `src/runtime/transport/sdp.rs` | backend内部SDP/HID serviceへ移す |
| `src/runtime/transport/csr.rs`、`identity.rs` | explicit local-address workflowをbackend open optionの内部処理にする |
| `src/runtime/transport/profile_key_store.rs` | swbtに残し、backend `BondStore` adapterへ変更する |

これによりbackendは `swbt-rs` に依存せず、`swbt-rs` もBumble内部型に依存しない。

## 4. test移行単位

| test unit | 必須観測 |
|---|---|
| codec unit | HCI command/event/ACL、L2CAP signaling、SDP continuation、HIDP encode/decode |
| scripted external host | exact initialization order、unrelated packet queue、terminal propagation、reader cancellation/join |
| Classic host | pairing/reconnect、link-key load/store、ACL credit、reverse channel order、disconnect cleanup |
| backend integration | pair → SDP → HID → NX handshake、Periodic/Direct input、malformed packet isolation |
| swbt adapter | backend error/event/capabilitiesと既存domain型の写像、profile extension保持 |
| hardware regression | Pro/Joy-Con、Periodic/Direct、explicit local address、power-cycle、stale bond、reader cleanup |

`bumble-controller` を通常依存または公開APIへ残さない。仮想Classic検証は、既存
`bumble-controller/tests/classic.rs` と `swbt-rs` virtual testsをoracleにしたtest-only peer fixtureへ移す。
test fixtureはbackend package内へ含めてもよいが、production moduleから到達可能にしない。

## 5. T06 package開始条件

T06では自己所有 fork の新しい作業branchに `swbt-bumble-backend/` を作り、次を最初のredとする。

1. normal dependencyにfork package、Git dependency、local path dependencyがない。
2. `cargo package --locked` が成功し、archiveに `LICENSE`、`NOTICE`、README、改変表示がある。
3. USB scripted host、Classic pairing/L2CAP、SDP、HIDPのtestがsource抽出前は失敗する。
4. public APIに `bumble_*` 型、HCI packet、L2CAP/SDP/HIDP内部型がない。
5. production dependencyにaudio、AV、ATT、GATT、SMP、LE advertising、RFCOMM、gRPC、
   WebSocket、serial transportがない。

初期package構造の候補は `core`、`hci`、`usb`、`external_host`、`classic_host`、`l2cap`、
`sdp`、`hidp`、`bond` とする。file配置はT06のgreenを作る過程で変更できるが、上記の所有境界は変えない。

## 6. 検証記録

| command / check | result |
|---|---|
| inventory artifact precondition | red: `Test-Path ...bumble-backend-source-inventory-20260801.md` returned `False` |
| `cargo tree --locked --features bumble -e normal` | current fork closure 22 package、direct 8 packageを確認 |
| production/test import audit | productionはcore/HCI/HID/host/L2CAP/SDP/transport、controller直接利用はtest-only |
| fork manifest audit | host/transportの不要protocol通常依存と、USB SCOの`libusb1-sys`直接利用を確認 |
| source symbol audit | address/key/UUID、HCI、ExternalHost、Device Classic、L2CAP、SDP、HIDPの定義元を確認 |
| license audit | fork root `LICENSE` はApache-2.0、`NOTICE` はbumble-rsとGoogle Bumble由来を記録 |
| Rust test/build | not run: T05はsource inventoryだけを変更し、製品codeとCargo metadataを変更しない |
