# 変更履歴

この文書は利用者に見える変更と互換性上の制限を記録する。日付と版は実際に公開した時点で
確定する。

## 未公開

### 0.1.0 候補

- Pro Controller、Joy-Con L、Joy-Con R を Periodic/Direct の型付き API から操作できる。
- Switch HID protocol、profile schema v2、pairing、保存鍵からの reconnect、model-valid input、
  IMU、診断 event、`swbt-probe` を実装した。
- Windows 11 25H2、CSR8510 A10、WinUSB、Switch 2 22.5.0（ユーザ報告）の限定構成で、3 model
  の pairing/reconnect、入力、neutral close、adapter reopen を実機確認した。
- Linux x86_64 は CI の build/test と USB ownership の source audit までで、実 adapter は未検証。
  macOS は対象外。
- M8 の subscriber 観測 interval は 8 ms report period に対して p95 16.6487–17.0223 ms だった。
  15秒 yaw run では横移動、目視カクつきなし、終了後の移動・入力残りなしを確認したが、
  subscriber 時刻は無線送信完了時刻ではない。

### 公開停止条件

0.1.0 は crates.io へ公開していない。固定 Bumble fork の同名 crate は registry へ正規化できない。
crates.io の `bumble@0.1.0` は Google Bumble の別 crate で、必要な `bumble-controller@0.1.0` なども
存在しない。配布境界を再設計し、clean `cargo package --locked` と archive smoke が成功するまで
`publish = false` を維持する。

## 版の方針

0.1.x では、公開型と既存の意味を保つ修正・機能追加を行う。公開 API の削除、型引数の意味変更、
profile や input contract の非互換変更は 0.2.0 とする。0.x 系では Cargo の通常の互換性規則に
加え、利用者が移行を判断できるよう非互換変更をこの文書へ明記する。
