# swbt-core

`swbt-core` は、`swbt` の controller model、model-valid input、pairing profile、
Nintendo Switch HID protocol 実装を Bluetooth backend から分離した package です。

controller runtime、USB adapter discovery、profile file writer が必要な場合は
`swbt-rs` package（library 名 `swbt`）を使用します。
