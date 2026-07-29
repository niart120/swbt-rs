# swbt-rs

`swbt-rs` は、NX 互換の仮想 Bluetooth HID 入力デバイスを扱う
[`swbt-python`](https://github.com/niart120/swbt-python) を Rust へ移植するプロジェクトです。
Bluetooth stack の実装基盤には
[`bumble-rs`](https://github.com/chaitanyarahalkar/bumble-rs) を使います。

## 現在の状態

このリポジトリは M2 の controller runtime と profile frontend まで実装済みです。Cargo
package は library target `swbt` を提供し、model-valid input、crate 内部の Switch HID
protocol と runtime、公開 controller builder を実装しています。

- pure protocol は `swbt-python` 0.6.0 の固定 commit
  `84d2723b127f70fc78e12f4496f5c40af0ccfb0a` から生成した 45 fixture を直接検査します。
- `ControllerBuilder::build()` は adapter や worker を開始せず、profile path 未指定なら
  一時 controller、既存 path なら controller model を検査した Configured controller を返します。
- `press()`、`release()`、`tap()`、`neutral()`、Periodic `apply()`、Direct `send()` を
  型付き worker command へ接続しています。Ready runtime に対する `close()` と
  `close_without_neutral()` は cleanup の完了を待って worker を join し、cleanup または
  join の失敗を返します。`Drop` は neutral report と pending send の drain を省いた
  bounded best-effort shutdown であり、終了失敗を呼び出し側へ返せません。3 model × 2
  reporting の crate 内 fake-runtime test で Pair→Ready→入力→worker join を検査しています。
- 新しい connection session は input snapshot を neutral へ戻して開始し、接続前または
  前 session の入力状態と stale event を持ち越しません。
- `build()` 直後の Configured controller には Ready runtime がないため、入力操作は
  `ErrorKind::TransportClosed` を返します。
- default feature は空です。`bumble` feature を有効にした場合だけ、基準 commit
  `bbac2a6803b8cab0920ab725a23aa408fc4fed85` の依存を組み込みます。
- 公開 `open()` / `pair()` / `create_profile()` は、concrete Bluetooth transport がない
  現在の build では `ErrorKind::UnsupportedCapability` を返します。`create_profile()` は
  builder、path、identity、target の存在を検査した後、file を作る前に停止します。
  既存 target は上書きしません。
- Bluetooth transport と Bumble の接続は未実装です。
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
