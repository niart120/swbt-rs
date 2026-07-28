# swbt-rs

`swbt-rs` は、NX 互換の仮想 Bluetooth HID 入力デバイスを扱う
[`swbt-python`](https://github.com/niart120/swbt-python) を Rust へ移植するプロジェクトです。
Bluetooth stack には
[`bumble-rs`](https://github.com/chaitanyarahalkar/bumble-rs) の利用を検討しています。

## 現在の状態

このリポジトリは M0 の基盤実装段階です。Cargo package は library target `swbt` を公開し、
仕様・テスト・品質確認を作業単位ごとに進めます。

- Bluetooth transport、HID protocol、controller API は未実装です。
- core `bumble` は基準 commit `bbac2a6803b8cab0920ab725a23aa408fc4fed85` に固定しています。
- transport 用の Bumble crate と統合処理は未実装です。
- Bluetooth adapter や対象機器を使う実機検証は未実施です。

## 開発

MSRV は Rust 1.87 です。現在のローカル確認は次の command で実行できます。

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --all-features
git diff --check
```

作業境界とリポジトリ固有の手順は [AGENTS.md](AGENTS.md) と
[SKILLS.md](SKILLS.md) にあります。

## ライセンス

MIT ライセンスです。全文は [LICENSE](LICENSE) にあります。
