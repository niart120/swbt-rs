# swbt-rs

`swbt-rs` は、NX 互換の仮想 Bluetooth HID 入力デバイスを扱う
[`swbt-python`](https://github.com/niart120/swbt-python) を Rust へ移植するプロジェクトです。
Bluetooth stack の実装基盤には
[`bumble-rs`](https://github.com/chaitanyarahalkar/bumble-rs) と、reader lifecycle 修正を含む
[一時 fork](https://github.com/niart120/bumble-rs/tree/fix/external-host-reader-lifecycle)
を使います。

## 現在の状態

このリポジトリは M8 の IMU、安定 diagnostics event、`swbt-probe` まで実装済みです。Cargo package は
library target `swbt` を提供し、
model-valid input、crate 内部の Switch HID protocol と runtime、公開 controller builder、
descriptor-only adapter discovery を実装しています。

0.1.0 は未公開です。固定 Bumble fork の同名 crate を crates.io dependency へ正規化できないため、
現在は repository source からだけ build できます。配布境界の再設計、clean `cargo package --locked`、
archive smoke が完了するまで `publish = false` を維持します。利用者向けの変更は
[変更履歴](CHANGELOG.md)、脆弱性報告時の注意は[セキュリティ方針](SECURITY.md)に記録しています。

- pure protocol は `swbt-python` 0.6.0 の固定 commit
  `84d2723b127f70fc78e12f4496f5c40af0ccfb0a` から生成した 55 fixture を直接検査します。
- `ControllerBuilder::build()` は adapter や worker を開始せず、profile path 未指定なら
  一時 controller、既存 path なら controller model を検査した Configured controller を返します。
- `press()`、`release()`、`tap()`、`neutral()`、Periodic `apply()`、Direct `send()` を
  型付き worker command へ接続しています。open runtime に対する `close()` と
  `close_without_neutral()` は cleanup の完了を待って worker を join し、cleanup または
  join の失敗を返します。`Drop` は neutral report と pending send の drain を省いた
  bounded best-effort shutdown であり、終了失敗を呼び出し側へ返せません。3 model × 2
  reporting の crate 内 fake-runtime test で Pair→Ready→入力→worker join を検査しています。
- 新しい connection session は input snapshot を neutral へ戻して開始し、接続前または
  前 session の入力状態と stale event を持ち越しません。
- `build()` 直後の Configured controller には open runtime がないため、入力操作は
  `ErrorKind::TransportClosed` を返します。
- default feature は空です。`bumble` feature を有効にした場合だけ、reader shutdown と join、ACL パケットが
  host queue を離れた状態の判定、CSR command 用の Vendor Event 応答待ちと応答を待たない command 送信を追加した
  一時 fork の commit `cb55e2d98dc7b7b0227c43772c9ae184034dd9a1` と `rusb` を組み込みます。
- `list_adapters()` は `bumble` feature で USB device/config/interface descriptor を読み、
  Bluetooth HCI class の candidate を返します。device open、driver detach、interface claim、
  HCI command は行いません。feature 無効時は `ErrorKind::UnsupportedCapability` を返します。
- `bumble` feature の公開 `open()` は USB HCI adapter を claim し、HCI 初期化と worker
  起動を完了して lifecycle `Open` を返します。同じ controller に対する repeated open は
  adapter や worker を追加せず成功し、close 後は同じ controller を reopen できます。
- feature 無効時の `open()` は `ErrorKind::UnsupportedCapability` を返します。
  `pair()` は open runtime がなければ `ErrorKind::TransportClosed`、open runtime では
  bounded worker command として pairing window の開始から NX readiness まで待ちます。
  timeout と Ready 前の disconnect は成功に変換しません。
- `reconnect()` は profile の保存済み Classic bond 1件を使い、失敗時に bond を削除したり
  fresh pairing へ切り替えたりしません。`connect()` は `NoBond` かつ
  `allow_pairing = true` の場合だけ `pair()` へ進みます。`try_reconnect()` と
  `try_connect()` は no-bond、timeout、Ready 前 disconnect を値として返し、profile、
  protocol、worker の失敗は error のまま返します。
- model 固有 HID descriptor と SDP record、Classic pairing window、NoInputNoOutput SSP
  policy、SDP/HID control/interrupt session は crate 内の Bumble `LocalLink` packet path
  で検査しています。production USB runtime の poll/send/cleanup に同じ Classic session
  を接続し、Windows 11 25H2、CSR8510 A10、Switch 2 system version 22.5.0
  （ユーザ報告）で実機 pairing、保存鍵からの reconnect、Periodic/Direct 入力を確認しています。
  他の OS、adapter、system version と長時間の信頼性は未検証です。
- `bumble` feature の `create_profile()` は既存 target を置換せず、valid empty envelope を
  USB open より先に保存してから pairing と NX readiness を待ちます。feature 無効時は file を
  作らず `ErrorKind::UnsupportedCapability` を返します。pairing key は profile 全体を
  atomic update して保存します。保存鍵を使う active/incoming reconnect は virtual Classic
  packet path で同一 session の Ready まで検査済みです。実機では同じ Pro profile から
  active reconnect した Periodic 2件と Direct 1件が Ready、入力、neutral、close、adapter
  reopen、profile 完全一致まで成功しました。このうち Periodic 1件はユーザが Switch 2 を
  power-cycle した後の実行ですが、power-cycle 操作自体は機械証跡では確認していません。
- CSR8510 A10 の claim/reset、100回の open/init/close、unplug/reopen は確認済みです。
  M5 の fresh pairing 20回中8回が same-session Ready に到達し、人手観測を行った5回では
  A、L+R、左右スティックが Switch UI に反映され、neutral 後の入力残りはありませんでした。
  20回は修正途中の失敗も含む履歴であり、成功率を製品の信頼性としては扱いません。
- M6 の reconnect 証跡は修正途中の失敗を含む14 run です。最終実装の独立した反復試験では
  ないため、3件の成功を長期信頼性や成功率の根拠にはしません。人手観測を行った3件では
  A、L+R、左右スティックが Switch UI に反映され、neutral 後の入力残りはありませんでした。
- M7 ではJoy-Con Lをfresh Periodic Pairから同じprofileのDirect reconnectまで確認しました。
  Joy-Con RはNFC/IR MCU state subcommand `0x22`の互換replyを追加した後、fresh Periodic Pair、
  Periodic reconnect、Direct reconnectがReady、左右固有入力、neutral close、adapter reopen、
  profile検査を通過しました。人手観測ではJoy-Con LのD-pad、L+ZL、SL+SR、left stickと、
  Joy-Con RのABXY、R+ZR、SL+SR、right stickが反映され、neutral後の入力残りはありませんでした。
  修正・診断途中のrunを含むため、長期信頼性や成功率の根拠にはしません。
- M8 では3 modelのstandard/quaternion IMU fixture、version 1の安定diagnostics event、秘密値を
  出さないprofile検査と`swbt-probe`を追加しました。Pro Periodicの60秒runは5,343件の
  non-neutral report、neutral close、profile完全一致、adapter reopenを確認しました。別の15秒
  pure yaw runではSwitch画面の横移動、目視カクつきなし、終了後の移動・入力残りなしを確認しました。
  subscriber観測intervalのp95はそれぞれ17.0223 msと16.6487 msで、8 ms目標に対する揺れは
  M9のrelease制限として残しています。単発runを信頼性や成功率の根拠にはしません。

## 対応環境と USB 準備

実機確認済みの構成は Windows 11、CSR8510 A10 (`0A12:0001`)、WinUSB、Switch 2
system version 22.5.0（ユーザ報告）です。Linux は CI の build/test までで、USB adapter を使った
pair/reconnect は未検証です。macOS は初期対象外です。

driver と udev の準備、claim/release の所有権、既知の制限は
[対応環境と USB adapter](docs/platform-support.md) にまとめています。adapter が見つからない、
permission/claim に失敗する、実行中に unplug した、Python backend へ戻す場合は
[トラブルシューティング](docs/troubleshooting.md) を参照してください。

## Pro Periodic 実機 runner

M5 の実機確認には
[`examples/pro_periodic_hardware.rs`](examples/pro_periodic_hardware.rs) を使います。Switch の
「持ちかた／順番を変える」画面を開き、WinUSB driver を割り当てた CSR8510 A10 を接続してから
実行します。次の例は run 1、pair timeout 60 秒です。profile path は実行前に存在していては
いけません。

```powershell
$runStamp = Get-Date -Format yyyyMMdd-HHmmss
$profilePath = Join-Path $env:TEMP "swbt-m5-$runStamp-run-01.json"
$evidencePath = Join-Path $env:TEMP "swbt-m5-$runStamp-run-01.ndjson"
cargo run --locked --example pro_periodic_hardware --features bumble -- `
  --adapter usb:0a12:0001 `
  --profile $profilePath `
  --pair-timeout-secs 60 `
  --run 1 | Tee-Object -FilePath $evidencePath
```

runner は A と L+R を各 500 ms、左右 stick を独立に4方向へ各500 ms、non-neutral IMU を
1秒送った後、neutral、close、profile 検査、adapter reopen を実行します。標準出力は
schema `swbt.m5.pro-periodic` version 1 の NDJSON です。adapter selector、profile path、
raw profile、key material、USB serial、error source は出力しません。report accepted counter と
command 成功は Switch UI の変化を証明しないため、`ui_observed` は `null` のまま出力し、人の
観測結果は別に記録します。終了 event は `runner_complete` で、`success` が run の機械判定です。

## Pro profile reconnect / Direct 実機 runner

M6 の既存 Pro profile からの再接続には
[`examples/pro_profile_hardware.rs`](examples/pro_profile_hardware.rs) を使います。`--profile` には
schema v2 の既存 profile を指定します。runner は実行前後の byte 完全一致を成功条件として
検査します。次の例は Direct reconnect、timeout 60秒です。

```powershell
$profilePath = Join-Path $env:TEMP 'existing-swbt-pro-profile.json'
$evidencePath = Join-Path $env:TEMP 'swbt-m6-direct-run-01.ndjson'
cargo run --locked --example pro_profile_hardware --features bumble -- `
  --adapter usb:0a12:0001 `
  --profile $profilePath `
  --mode direct `
  --setup normal `
  --connect-timeout-secs 60 `
  --run 1 | Tee-Object -FilePath $evidencePath
```

`--mode periodic` は Periodic、`--setup post-power-cycle` はユーザが事前に power-cycle した run の
分類です。runner 自身は電源操作を行わず、その操作を機械検証もしません。`--setup stale-bond` は
`--stale-source-profile` の正しい profile から、link key を1 nibbleだけ変えた別の create-new
target を作る失敗試験です。正しい profile と同じ path は拒否します。

成功 run は A と L+R、左右 stick、neutral、close、adapter reopen を実行します。Direct は
Ready 後の500 ms idle で user input report が0件であることも検査します。標準出力は schema
`swbt.m6.pro-profile` version 1 の NDJSON で、path、raw profile、peer address、key material は
出力しません。`ui_observed` は `null` とし、Switch UI の人手観測は別 record にします。

## Joy-Con L/R実機runner

M7のJoy-Con確認には
[`examples/joycon_profile_hardware.rs`](examples/joycon_profile_hardware.rs) を使います。fresh Pairは
Periodicだけを受け付け、Directは同じmodelの既存schema v2 profileからreconnectします。次の例は
Joy-Con Rのfresh Periodic Pairです。profile pathは実行前に存在していてはいけません。

```powershell
$runStamp = Get-Date -Format yyyyMMdd-HHmmss
$profilePath = Join-Path $env:TEMP "swbt-m7-joycon-r-$runStamp.json"
$evidencePath = Join-Path $env:TEMP "swbt-m7-joycon-r-$runStamp.ndjson"
cargo run --locked --example joycon_profile_hardware --features bumble -- `
  --adapter usb:0a12:0001 `
  --profile $profilePath `
  --model right `
  --mode periodic `
  --connection pair `
  --timeout-secs 120 `
  --run 1 | Tee-Object -FilePath $evidencePath
```

`--model left`はJoy-Con L、`--connection reconnect --mode direct`は既存profileのDirect再接続です。
runnerはLでD-pad、L+ZL、SL+SR、left stick、RでABXY、R+ZR、SL+SR、right stickを送り、
neutral、close、adapter reopen、profile model、反対側model拒否を検査します。ABXYは各200 ms、
同時押しとstickは各500 msです。DirectではReady後のidleにuser input reportが増えないことも
確認します。標準出力はschema `swbt.m7.joycon-profile` version 1のNDJSONで、path、raw profile、
peer address、key materialは出力しません。UI結果は別recordにします。

## diagnosticsとswbt-probe

runtimeは`swbt::diagnostics` targetへschema `swbt.diagnostics` version 1の`tracing` eventを出します。
session、lifecycle、report mode、committed IMU mode、subcommand、transport受理数、切断理由、分類済み
worker failureを記録し、profile path、Bluetooth address、link key、USB serial、raw packet、error source
chainは安定fieldへ含めません。受理数はtransportがreportを受理した回数であり、無線到達やSwitch画面の
変化を証明しません。

`probe` featureは`bumble`を含み、`swbt-probe` binaryを有効にします。主な入口は次のとおりです。

```powershell
cargo run --locked --features probe --bin swbt-probe -- adapters
cargo run --locked --features probe --bin swbt-probe -- open --adapter usb:0
cargo run --locked --features probe --bin swbt-probe -- profile inspect .\profile.json
cargo run --locked --features probe --bin swbt-probe -- profile verify .\profile.json
cargo run --locked --features probe --bin swbt-probe -- pair --controller pro --profile .\new-profile.json --trace .\pair-trace.ndjson
$localAddress = Read-Host 'locally administered address (XX:XX:XX:XX:XX:XX)'
cargo run --locked --features probe --bin swbt-probe -- pair --controller pro --profile .\new-local-profile.json --trace .\local-pair-trace.ndjson --local-address $localAddress
cargo run --locked --features probe --bin swbt-probe -- reconnect --controller pro --profile .\profile.json --trace .\reconnect-trace.ndjson
```

`pair`のprofileと全接続commandのtraceはcreate-newで、既存fileを上書きしません。controllerは
`pro`、`joycon-l`、`joycon-r`、reconnectのreportingは`periodic`または`direct`です。Pro Periodic
reconnectだけは`--imu-seconds 1..3600`を受け、固定IMU入力、neutral report、close、profile不変、
adapter reopenを検査します。traceの`trace_elapsed_ns`はstatus投影後にsubscriberがeventを観測した
時刻であり、無線送信完了時刻ではありません。実機runと測定境界は
[M8 実機証跡](https://github.com/niart120/swbt-rs/blob/main/spec/complete/unit_009/evidence/pro-imu-diagnostics-windows-20260801/SUMMARY.md)
に記録しています。

`pair --local-address`はCSR8510 A10の揮発address書換え専用です。個別かつローカル管理のaddressだけを
受け付け、指定なしは従来どおりadapter-defaultです。作成したprofileは指定addressをidentityとして
保存するため、1個のdongle・1個のaddress・1個のprofileを一組として扱ってください。成功NDJSONは
`identity_kind`を`local_address`または`adapter_default`として出しますが、address値は標準出力、
標準エラー、traceへ出しません。`adapter_identity_recovery_required`で終了した場合は再試行せず、dongleを
物理的に抜き差しして揮発書換えを解除してから次の操作へ進みます。

## 開発

MSRV は Rust 1.87 です。現在のローカル確認は次の command で実行できます。

```powershell
cargo fmt --all --check
cargo check --all-targets --all-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
cargo test --lib protocol:: --no-default-features --locked
cargo tree --no-default-features --edges normal --locked
cargo test --doc --all-features --locked
cargo build --all-features --locked
git diff --check
```

`cargo tree --no-default-features --edges normal --locked` の直接依存は `atomic-write-file`、
`fs2`、`serde_json`、`tracing` で、Bumble と `rusb` を含みません。selected Miri は nightly の
`miri` component を導入した環境で次の command を実行します。

```powershell
cargo +nightly miri test --lib --no-default-features --locked protocol::
```

現在利用できる model-valid input 型は
[examples/type_model.rs](examples/type_model.rs) で確認できます。

## ライセンス

MIT ライセンスです。全文は [LICENSE](LICENSE) にあります。
