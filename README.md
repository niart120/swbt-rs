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
  join を追加した一時 fork の commit
  `48f1bc36169b2692d2a61e87eda4223b126dca2b` と `rusb` を組み込みます。
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
  を接続していますが、実 adapter と Switch を使う pairing は未検証です。
- `bumble` feature の `create_profile()` は既存 target を置換せず、valid empty envelope を
  USB open より先に保存してから pairing と NX readiness を待ちます。feature 無効時は file を
  作らず `ErrorKind::UnsupportedCapability` を返します。pairing key の file 更新と既存
  profile からの reconnect は未実装です。
- CSR8510 A10 の claim/reset、100回の open/init/close、unplug/reopen は確認済みです。
  Switch 実機の pairing と入力反映は未検証です。

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
