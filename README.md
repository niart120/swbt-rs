# swbt-rs

`swbt-rs` は、NX 互換の仮想 Bluetooth HID 入力デバイスを扱う
[`swbt-python`](https://github.com/niart120/swbt-python) を Rust へ移植するプロジェクトです。
Bluetooth stack の実装基盤には
[`bumble-rs`](https://github.com/chaitanyarahalkar/bumble-rs) と、reader lifecycle 修正を含む
[一時 fork](https://github.com/niart120/bumble-rs/tree/fix/external-host-reader-lifecycle)
を使います。

## 現在の状態

このリポジトリは M4 の仮想 Classic HID 経路まで実装済みです。Cargo package は
library target `swbt` を提供し、
model-valid input、crate 内部の Switch HID protocol と runtime、公開 controller builder、
descriptor-only adapter discovery を実装しています。

- pure protocol は `swbt-python` 0.6.0 の固定 commit
  `84d2723b127f70fc78e12f4496f5c40af0ccfb0a` から生成した 45 fixture を直接検査します。
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
- default feature は空です。`bumble` feature を有効にした場合だけ、reader shutdown と
  join、および ACL パケットがホスト側の待ち行列を離れた状態の判定を追加した一時 fork の commit
  `b8c7cd625bc2ac2f58a4beb4ade1264426969819` と `rusb` を組み込みます。
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
- model 固有 HID descriptor と SDP record、Classic pairing window、NoInputNoOutput SSP
  policy、SDP/HID control/interrupt session は crate 内の Bumble `LocalLink` packet path
  で検査しています。production USB runtime の poll/send/cleanup に同じ Classic session
  を接続し、Windows 11 25H2、CSR8510 A10、Switch 2 system version 22.5.0
  （ユーザ報告）で実機 pairing と入力を確認しています。他の OS、adapter、system
  version と長時間の信頼性は未検証です。
- `bumble` feature の `create_profile()` は既存 target を置換せず、valid empty envelope を
  USB open より先に保存してから pairing と NX readiness を待ちます。feature 無効時は file を
  作らず `ErrorKind::UnsupportedCapability` を返します。pairing key の file 更新と既存
  profile からの reconnect は未実装です。
- CSR8510 A10 の claim/reset、100回の open/init/close、unplug/reopen は確認済みです。
  M5 の fresh pairing 20回中8回が same-session Ready に到達し、人手観測を行った5回では
  A、L+R、左右スティックが Switch UI に反映され、neutral 後の入力残りはありませんでした。
  20回は修正途中の失敗も含む履歴であり、成功率を製品の信頼性としては扱いません。

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

`cargo tree --no-default-features --edges normal --locked` の通常依存は `serde_json` の
依存木だけで、Bumble を含みません。selected Miri は nightly の `miri` component を
導入した環境で次の command を実行します。

```powershell
cargo +nightly miri test --lib --no-default-features --locked protocol::
```

作業境界とリポジトリ固有の手順は [AGENTS.md](AGENTS.md) と
[SKILLS.md](SKILLS.md) にあります。

現在利用できる model-valid input 型は
[examples/type_model.rs](examples/type_model.rs) で確認できます。

## ライセンス

MIT ライセンスです。全文は [LICENSE](LICENSE) にあります。
