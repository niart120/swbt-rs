# swbt-rs

`swbt-rs` は、NX 互換の仮想 Bluetooth HID 入力デバイスを扱う
[`swbt-python`](https://github.com/niart120/swbt-python) を Rust へ移植するプロジェクトです。
Bluetooth stack の実装基盤には
[`bumble-rs`](https://github.com/chaitanyarahalkar/bumble-rs) を使います。

## 現在の状態

このリポジトリは M2 の controller runtime 基盤を実装中です。Cargo package は library
target `swbt` を提供し、model-valid input、crate 内部の Switch HID protocol と runtime、
公開 controller builder を実装しています。

- pure protocol は `swbt-python` 0.6.0 の固定 commit
  `84d2723b127f70fc78e12f4496f5c40af0ccfb0a` から生成した 45 fixture を直接検査します。
- `ControllerBuilder::build()` は adapter や worker を開始せず、profile path 未指定なら
  一時 controller、既存 path なら controller model を検査した Configured controller を返します。
- `press()`、`release()`、`tap()`、`neutral()`、Periodic `apply()`、Direct `send()` を
  型付き worker command へ接続しています。Ready runtime に対する `close()` と
  `close_without_neutral()` は priority shutdown、cleanup、worker join を実行します。
  3 model × 2 reporting の crate 内 fake-runtime test で Pair→Ready→入力→worker join を
  検査しています。
- `build()` 直後の Configured controller には Ready runtime がないため、入力操作は
  `ErrorKind::TransportClosed` を返します。
- default feature は空です。`bumble` feature を有効にした場合だけ、基準 commit
  `bbac2a6803b8cab0920ab725a23aa408fc4fed85` の依存を組み込みます。
- Bluetooth transport と Bumble の接続、公開 `open()` / `pair()` / `create_profile()` は
  未実装です。
- Bluetooth adapter や対象機器を使う実機検証は未実施です。

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

`cargo tree --no-default-features --edges normal --locked` の通常依存は `swbt-rs` だけになり、
Bumble を含みません。selected Miri は nightly の `miri` component を導入した環境で次の
command を実行します。

```powershell
cargo +nightly miri test --lib --no-default-features --locked protocol::
```

作業境界とリポジトリ固有の手順は [AGENTS.md](AGENTS.md) と
[SKILLS.md](SKILLS.md) にあります。

現在利用できる model-valid input 型は
[examples/type_model.rs](examples/type_model.rs) で確認できます。

## ライセンス

MIT ライセンスです。全文は [LICENSE](LICENSE) にあります。
