# 対応環境と USB adapter

`swbt-rs` は Bluetooth HCI USB adapter を OS の Bluetooth 機能から一時的に切り離し、NX 互換
controller の専用 adapter として使います。キーボード、マウス、ヘッドセットに使用中の主 adapter
ではなく、取り外せる専用 adapter を用意してください。

## 支援水準

| platform | 支援水準 | 確認した範囲 |
|---|---|---|
| Windows 11 25H2 x86_64 | 限定構成で実機確認済み | CSR8510 A10 `0A12:0001`、WinUSB、Switch 2 22.5.0（ユーザ報告）で Pro/Joy-Con L/Joy-Con R の pairing、保存鍵 reconnect、Periodic/Direct input、neutral close、adapter reopen |
| Linux x86_64 | build-tested、実機未検証 | GitHub Actions の Ubuntu runner で all-feature check/test、Bumble USB ownership の固定 source audit |
| macOS | unsupported | USB transport と driver ownership を未調査 |

Windows の結果は上表の組合せに限ります。別の Bluetooth chipset、Windows driver、console system
version、Linux distribution へ同じ実機結果を一般化しません。

## 共通の確認

`bumble` feature が USB transport を有効にします。descriptor の列挙は device を open/claim
しません。

```powershell
cargo run --locked --features probe --bin swbt-probe -- adapters
```

出力は candidate 件数と VID/PID だけです。selector、USB serial、Bluetooth address は表示しません。
対象を特定できたら、open と close の最小確認を実行します。

```powershell
cargo run --locked --features probe --bin swbt-probe -- open --adapter usb:0a12:0001
```

この command は HCI 初期化後に `close_without_neutral()` を完了してから終了します。controller を
pairing しません。

### CSR8510 A10 の揮発 local address

`swbt-probe pair --local-address <XX:XX:XX:XX:XX:XX>`は、CSR8510 A10で確認するための限定入口です。
個別かつローカル管理のaddressを受け付けます。別chipsetへの書換え、永続PSKEY書換え、複数dongleで
同じaddressを同時に使う運用は対象外です。

明示addressを持つprofileでは、通常のHCI初期化より前に現在addressとcontroller versionを読みます。
既に一致する場合は書き換えません。不一致ならvolatile SETREQとwarm resetを送り、USB再列挙とread-backが
成功した後だけ通常初期化、key-store設定、pairingへ進みます。通常初期化後にもexpected-address guardを
行います。成功NDJSONとtraceに出すのは`identity_kind`だけで、address値は出しません。

書換え開始後の失敗は`adapter_identity_recovery_required`です。この場合は次のwriteを行わず、dongleを
物理的に抜き差ししてください。profileを削除してもdongleの揮発identityは元に戻りません。実機検証では
書換えを伴うrunごとにpower cycleし、元のadapter addressへ戻ったことをread-onlyで確認します。

## Windows

### driver

Windows で libusb から汎用 USB device を開くには対応 driver が必要です。libusb は WinUSB を推奨し、
WinUSB の制約に当たる場合だけ libusbK を候補にしています。M3-M8 の実機確認は WinUSB だけで
実施しました。根拠と制約は [libusb Windows backend の説明](https://github.com/libusb/libusb/wiki/Windows)
を参照してください。

1. 専用 adapter だけを接続する。
2. PowerShell で VID/PID と現在の driver を確認する。

   ```powershell
   Get-PnpDevice -PresentOnly |
     Where-Object InstanceId -Match 'VID_0A12&PID_0001' |
     Format-Table Status, Class, FriendlyName, InstanceId
   ```

3. WinUSB でない場合は、libusb が案内する最新の Zadig で対象 device を選び、USB ID が
   `0A12:0001` であることを再確認してから WinUSB を割り当てる。別 device や主 Bluetooth adapter
   の driver は変更しない。
4. `swbt-probe adapters` と `swbt-probe open` を順に実行する。

WinUSB は複数 application からの同時利用を支えません。Rust controller、`swbt-probe`、
`swbt-python`、別の libusb tool を同じ adapter に対して同時に開かないでください。

### claim と release

`Controller::open()` は adapter を open/claim し、reader と worker を所有します。通常終了では
`Controller::close()` または `close_without_neutral()` が reader shutdown、worker join、USB handle
の解放まで待ちます。明示 close が戻る前に別 process を開始しないでください。

物理 unplug は transport termination です。実機試験では unplug 後に worker を回収し、挿し直した
adapter を別 process から reopen できました。unplug を通常の close 代わりには使いません。

## Linux

### udev permission

`rusb` は vendored libusb を使用しますが、USB device node の permission は OS の設定に従います。
最初に対象 adapter を確認します。

```bash
lsusb -d 0a12:0001
```

systemd-logind の local session に限定して access を与える例です。`/etc/udev/rules.d/70-swbt-csr8510.rules`
を root 権限で作成します。

```udev
SUBSYSTEM=="usb", ATTR{idVendor}=="0a12", ATTR{idProduct}=="0001", TAG+="uaccess"
```

rule を reload した後、adapter を抜き差しします。

```bash
sudo udevadm control --reload-rules
```

`MODE="0666"` で全利用者へ書き込みを許可しません。service account や remote session で使用する場合は、
administrator が専用 group と `MODE="0660"` を定義してください。udev の `OWNER`、`GROUP`、`MODE`、
`TAG` の意味は [systemd udev manual](https://www.freedesktop.org/software/systemd/man/devel/udev.html) に従います。

### kernel driver detach と reattach

固定 Bumble revision `cb55e2d98dc7b7b0227c43772c9ae184034dd9a1` の
`bumble-transport/src/usb.rs` は、USB handle に `set_auto_detach_kernel_driver(true)` を設定してから
HCI interface を claim します。libusb の契約では、auto detach が有効な handle は claim 時に kernel
driver を detach し、interface release 時に attach します。詳細は
[libusb device handling API](https://libusb.sourceforge.io/api-1.0/group__libusb__dev.html) を参照してください。

`swbt-rs` はこの処理を重複実装しません。明示 close によって Bumble runtime と USB handle を drop
することが ownership の終端です。platform が auto detach を支えない場合、Bumble は
`NotSupported` を受け入れて claim を試みるため、kernel driver が有効なままなら open は失敗します。

Linux で実行した実機証跡はありません。CI success は Rust/libusb の compile と仮想・純粋 test の
結果であり、udev permission、実 adapter の detach/reattach、pair/reconnect を証明しません。

## 既知の制限

- Windows 実機確認は CSR8510 A10 と WinUSB の1構成に限る。
- M8 の Pro Periodic 60秒 run では、8 ms report period に対する subscriber 観測 interval の p95 が
  17.0223 ms だった。別の15秒 yaw run は横移動が反映され、目視カクつきと終了後の移動・入力残りは
  観測されなかった。subscriber 時刻は無線送信完了時刻ではないため、両者を同じ測定として扱わない。
- pairing/reconnect の修正途中 run を含む履歴を成功率や長期信頼性として扱わない。
- 明示 local Bluetooth address、macOS、複数 controller 管理は 0.1.0 の対象外。
- `Drop` は bounded best-effort shutdown で、neutral report と終了失敗の通知を保証しない。正常系は
  明示 `close()` を使う。
