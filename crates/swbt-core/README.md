# swbt-core

`swbt-core` は、`swbt` の controller model、model-valid input、pairing profile、
Nintendo Switch HID protocol 実装を Bluetooth backend から分離した package です。

Bluetooth adapter を開かずに型付き入力や profile JSON を扱う利用者は、次の依存だけを追加します。

```toml
[dependencies]
swbt-core = "0.1"
```

公開 API は `error`、`input`、`model`、`profile` module と、それらの主要型の crate-root
再公開です。protocol engine と wire metadata は `swbt-rs` runtime を支える rustdoc 非表示の
内部境界であり、安定した利用者向け API ではありません。通常依存は `serde` と `serde_json` だけで、
Bumble、`rusb`、`tracing`、profile file writer を含みません。

controller runtime、USB adapter discovery、profile file writer が必要な場合は
`swbt-rs` package（library 名 `swbt`）を使用します。
