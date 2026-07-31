# トラブルシューティング

この文書では USB adapter の列挙、open/claim、実行中の切断、保存済み profile の再接続、backend
切替を扱います。profile JSON には Bluetooth link key が含まれます。issue、ログ、チャットへ raw
profile を添付しないでください。

## 最初の切り分け

descriptor 列挙と HCI open を分けて実行します。

```powershell
cargo run --locked --features probe --bin swbt-probe -- adapters
cargo run --locked --features probe --bin swbt-probe -- open --adapter usb:0a12:0001
```

`adapters` は device を open/claim しません。ここで candidate が0件なら、USB 接続、VID/PID、
Bluetooth HCI class、Windows PnP または Linux `lsusb` を確認します。`adapters` が成功して `open` が
失敗する場合は、driver/permission、別 process の claim、HCI 初期化応答を調べます。

## Windows の open/claim 失敗

1. `Get-PnpDevice` で対象 `VID_0A12&PID_0001` が `OK` か確認する。
2. Device Manager または Zadig で対象だけが WinUSB を使用しているか確認する。
3. `swbt-python`、別の `swbt-probe`、USB capture tool が終了していることを確認する。
4. Rust process の終了後に adapter を抜き差しし、`swbt-probe open` を再実行する。

WinUSB は同時 open を支えません。claim 失敗を再試行 loop で隠さず、adapter を所有する process を
終了させます。主 Bluetooth adapter の driver を変更して解決しようとしないでください。

## Linux の permission/claim 失敗

`Access` 系の source がある場合は、[対応環境と USB adapter](platform-support.md) の udev rule、
local session、抜き差し後の device node permission を確認します。`sudo` で application 全体を常用して
permission 問題を回避しません。

`Busy` 系の source がある場合は、同じ adapter を開く別 process がないか確認します。固定 Bumble
transport は claim 時の kernel-driver auto detach を要求します。close 後にも再利用できない場合は
Rust process の終了を確認し、adapter を抜き差しして `swbt-probe open` を実行します。

## 実行中に unplug した

unplug は公開 `ErrorKind::WorkerFailed` になり、source chain に transport termination が残ります。
次の順で復旧します。

1. controller operation の error を保持する。
2. `close()` を呼び、返った cleanup/join error も保持する。
3. process が終了したことを確認する。
4. adapter を挿し直し、OS の再列挙を待つ。
5. `swbt-probe open --adapter usb:0a12:0001` で別 process から reopen する。

unplug 後の input report 到達や neutral は保証されません。Switch UI に入力が残った場合は controller
session を切断し、再接続後の session が neutral から開始することを確認します。

## reconnect できない

profile を表示せず、schema と model だけを検査します。

```powershell
cargo run --locked --features probe --bin swbt-probe -- profile verify .\profile.json
```

- `NoBond`: usable Classic link key がない。`connect()` は `allow_pairing = true` の場合だけ fresh pairing
  へ進む。
- `ProfileControllerMismatch`: profile の controller kind と使用する alias が違う。profile を別 model
  として書き換えず、正しい typed controller を選ぶ。
- `ConnectionTimeout` / `ConnectionFailed`: stored bond を自動削除しない。同じ失敗から fresh pairingへ
  暗黙に切り替えない。
- malformed profile / key store: raw key をログへ出さず、元 profile の copy を保全してから原因を
  調査する。

## 正常終了

入力を残さない通常終了は `close()` を使います。

```rust
controller.neutral()?;
controller.close()?;
```

`close()` 自体も trailing neutral を試みますが、先に `neutral()` を成功確認すると application の
操作境界を記録できます。`close_without_neutral()` は adapter-only probe や、neutral を送れない
失敗後の cleanup に限定します。`Drop` は終了失敗を返せないため、正常終了の根拠にしません。

## `swbt-python` backend へ戻す

Rust と Python の backend を同じ adapter に対して同時に開きません。rollback は次の順序です。

1. Rust controller の `close()` と process 終了を確認する。
2. `swbt-probe open` で adapter が再利用可能か確認する。失敗する場合だけ抜き差しする。
3. profile の copy を取り、`swbt-probe profile verify` で schema v2 と controller kind を確認する。
4. [source baseline](https://github.com/niart120/swbt-rs/blob/main/spec/initial/source-baseline.md) の
   Python 基準 commit `84d2723b127f70fc78e12f4496f5c40af0ccfb0a` と、その repository の
   lockfile から環境を再現する。
5. Python 側で同じ model の profile を読み、reconnect、neutral input、明示 close を確認する。
6. Rust へ戻す場合も Python process の終了と adapter release を先に確認する。

rollback で profile を変換または再生成しません。backend ごとの結果、OS、adapter、driver、console
version を記録し、profile path、Bluetooth address、link key は証跡から除外します。
