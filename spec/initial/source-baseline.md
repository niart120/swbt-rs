# 参照断面と前提

- 状態: **決定**
- 調査日: 2026-07-28 (JST)
- 対象: `swbt-python`、`bumble-rs`、`swbt-rs` の既定ブランチ最新コミット

## 1. 固定した参照断面

ブランチ名は移動するため、設計・差分テスト・依存更新では次の commit SHA を使う。

| リポジトリ | 既定ブランチ | 基準 commit | commit date (UTC) | 確認した状態 | この仕様での役割 |
|---|---|---|---|---|---|
| [`niart120/swbt-python`](https://github.com/niart120/swbt-python) | `main` | [`84d2723b127f70fc78e12f4496f5c40af0ccfb0a`](https://github.com/niart120/swbt-python/commit/84d2723b127f70fc78e12f4496f5c40af0ccfb0a) | 2026-07-26 | `v0.6.0` 反映済み | 公開挙動、NX HID protocol、profile schema、実機知見の基準実装 |
| [`chaitanyarahalkar/bumble-rs`](https://github.com/chaitanyarahalkar/bumble-rs) | `main` | [`bbac2a6803b8cab0920ab725a23aa408fc4fed85`](https://github.com/chaitanyarahalkar/bumble-rs/commit/bbac2a6803b8cab0920ab725a23aa408fc4fed85) | 2026-07-16 | ACL packet boundary flag 修正後 | Bluetooth host、HCI transport、Classic L2CAP、SDP、HIDP、pairing の実装基盤 |
| [`niart120/swbt-rs`](https://github.com/niart120/swbt-rs) | `main` | [`8297809caa5510dd8a1d6dbcdd2067d15e429721`](https://github.com/niart120/swbt-rs/commit/8297809caa5510dd8a1d6dbcdd2067d15e429721) | 2026-07-27 | Rust 2024 の最小 binary crate、実装未着手 | この仕様を実装する対象 |

今後「Python 基準断面」「Bumble 基準断面」と記す場合は、それぞれ上表の SHA を指す。

## 2. Python 基準断面で確認した契約

`swbt-python` は package version `0.6.0`、Python `>=3.13`、`bumble==0.0.233` である。Rust 版が参照する主な契約は次の通り。

- Pro Controller、Joy-Con L、Joy-Con R を、周期送信型と直接送信型に分ける
- 公開 API に Bumble の型を露出しない
- `Button`、12-bit `Stick`、3 frame の `IMUFrame`、完全状態の `InputState` を持つ
- 周期送信型は local state を先に確定し、report loop が後続 tick で送る
- 直接送信型は report が transport に受理された後だけ state を確定する
- link 接続だけでは `connect()` を完了せず、report mode と player lights を含む protocol readiness を待つ
- `0x30` input report と `0x21` subcommand reply を同じ送信直列化点へ通す
- `profile_path` は controller shape、adapter identity、pairing key store を同じ JSON envelope に保存する
- profile format は `format = "swbt.profile"`、`schema_version = 2`
- `adapter-default` identity と、明示的な locally administered address identity を区別する
- raw HID、NFC/amiibo、IR camera、高水準 rumble、複数 controller 管理は初期公開 API に含めない

Python の class 階層や `asyncio` 呼び出し形式そのものは互換対象にしない。互換対象は意味、wire bytes、失敗時の state、profile データである。

## 3. Bumble 基準断面で確認した能力

`bumble-rs` workspace は version `0.1.0`、Rust `1.87` 以上、Apache-2.0 である。core は同期 state machine と明示的な `poll` / queue / listener を採用している。

利用する候補 crate は次の通り。

| crate | 利用目的 |
|---|---|
| `bumble` | address、key store、共通データ型 |
| `bumble-hci` | HCI command/event と ACL packet |
| `bumble-host` | `Device`、Classic 接続、L2CAP channel 管理 |
| `bumble-transport` | USB 等の HCI transport、`ExternalHost` |
| `bumble-l2cap` | Classic dynamic channel と PSM |
| `bumble-hid` | HIDP message codec、device-side request dispatch |
| `bumble-sdp` | SDP PDU、service record、server |
| `bumble-smp` | pairing policy 型 |
| `bumble-controller` | software controller と virtual link による統合テスト |

確認済みの部品:

- `usb` / `pyusb` を含む transport specification と `open_split_transport`
- external HCI reader thread を持つ `ExternalHost`
- `DeviceConfiguration` による Classic 有効化と incoming connection の自動受入
- PSM ごとの Classic L2CAP server 登録、accepted channel 取得、SDU 送受信
- HID control PSM `0x0011`、interrupt PSM `0x0013` の codec と device runtime
- Classic pairing session と link key の保存
- SDP server と L2CAP binding
- software controller / virtual link

## 4. Bumble 統合の初期確認事項

次は公式基準断面を選んだ時点の確認事項である。1–4 は M4–M7 と unit_011 までに一体動作を確認した。
5 の依存量と registry 配布は M9 / unit_012 の制約として残る。

1. **HIDP と `bumble-host::Device` の接続方法**
   `bumble-hid::L2capTransport` は `bumble_l2cap::ChannelManager` を直接受け取る。一方、external controller の標準経路では `bumble_host::Device` が channel manager を所有し、SDU 単位の API を公開する。初期実装は `bumble_hid::Message` / `DeviceRuntime` と `Device` の SDU API を結ぶ薄い adapter を置く。上流 API が整えば置換する。

2. **SDP PSM `0x0001` の server lifecycle**
   service record 自体だけでなく、incoming SDP channel の受入、continuation state、切断時 cleanup を同じ worker loop で駆動する必要がある。

3. **Switch 側からの Classic 接続と pairing 順序**
   `classic_accept_any`、Classic SSP、stored link key、HID channel open の順序を virtual link と実機の両方で確認する。

4. **明示 local Bluetooth address の設定と復旧**
   unit_011 で CSR8510 A10 の identity read/write、warm reset 後の readback、pair/reconnect、失敗時 recovery、power-cycle 復旧を実装・実機確認した。他 chipset は未検証である。

5. **依存量**
   `bumble-transport` は現断面で audio、gRPC、WebSocket 等も直接依存する。初期実装では correctness を優先して受け入れ、build 時間・配布サイズを測定する。削減は上流 feature 分割または transport 部分の切り出しとして別変更にする。

## 5. 依存固定方針

`bumble-rs` は branch や tag ではなく、次の exact revision に固定する。

```toml
# 実際の direct dependency は architecture.md の境界に合わせて選ぶ。
bumble = { git = "https://github.com/chaitanyarahalkar/bumble-rs", rev = "bbac2a6803b8cab0920ab725a23aa408fc4fed85" }
bumble-host = { git = "https://github.com/chaitanyarahalkar/bumble-rs", rev = "bbac2a6803b8cab0920ab725a23aa408fc4fed85" }
bumble-transport = { git = "https://github.com/chaitanyarahalkar/bumble-rs", rev = "bbac2a6803b8cab0920ab725a23aa408fc4fed85" }
```

同じ repository の Bumble crate はすべて同じ revision にそろえる。`Cargo.lock` は repository に commit する。

### 0.1.0 candidate の依存差分

0.1.0 candidate は、上記の公式基準断面から public fork
[`niart120/bumble-rs`](https://github.com/niart120/bumble-rs) の exact revision
[`5fb0f6ddb811d1ad43dffa6e72a5d8cc6096fb07`](https://github.com/niart120/bumble-rs/commit/5fb0f6ddb811d1ad43dffa6e72a5d8cc6096fb07)
へ進めている。差分は次の5 commit。

| commit | 目的 |
|---|---|
| `48f1bc3` | external transport reader の shutdown lifecycle を追加 |
| `b8c7cd6` | ACL output が host queue を離れた状態を公開 |
| `cb55e2d` | generic HCI command の Vendor Event 応答待ちと、応答を待たない command 送信を追加 |
| `2f5c853` | 配布対象 24 package を `swbt-bumble*` 名へ変更し、内部 dependency alias と exact version を追加 |
| `5fb0f6d` | package 名を変えても既存 Rust import 名を保つ `[lib] name` を追加 |

先頭2 commit は M3-M9 の CI、仮想 Bluetooth test、Windows 実機試験で使った依存差分である。3つ目は
unit_011 の dependency unit test と、CSR8510 A10 での明示 local-address pair / reconnect 実機試験を通した。
後続2 commit は Rust source を変えず、fork workspace test と対象 24 package の archive verify を通した。
この candidate revision は公式基準断面そのものを書き換えない。Bumble upstream への PR は作成していない。
0.1.0 release commit は merge 前には確定せず、公開承認後に `spec/publishing.md` の手順で main SHA、
Cargo.lock hash、Bumble revision を同時に記録する。

revision 更新は専用 PR で行い、最低限次を実行する。

- protocol golden tests
- virtual Bluetooth integration tests
- profile compatibility tests
- adapter-only smoke test
- 変更された Bumble API と wire behavior の確認
- direct dependency と transitive dependency の差分記録

`main` を直接追従する設定、複数 revision の混在、ローカル path override の commit は禁止する。

## 6. Rust toolchain と crate 形態

**決定:** `swbt-rs` の実効 MSRV は Bumble 基準断面に合わせて Rust `1.87` とする。`Cargo.toml` に `rust-version = "1.87"` を記載し、CI で MSRV を検証する。

**決定:** package 名は当面 `swbt-rs` を維持し、library target 名を `swbt` とする。利用者コードは `use swbt::...` を使う。現在の `src/main.rs` だけの構成は、`src/lib.rs` を正本とする library-first 構成へ変更する。CLI は後続 milestone で `src/bin/swbt-probe.rs` として追加する。

## 7. ライセンス gate

`swbt-python` と `swbt-rs` は MIT、Bumble 基準断面は Apache-2.0 である。

**決定:** `swbt-rs` は MIT で配布する。repository の `LICENSE` と `Cargo.toml` の `license = "MIT"` を正本とする。公開前には Bumble を含む依存関係の license inventory を確認する。

## 8. 基準断面の更新方法

基準 SHA を更新するときは、この文書だけを書き換えない。次を同じ作業単位で行う。

1. upstream changelog と該当 diff を確認する
2. wire fixture と profile fixture を再生成する必要があるか判断する
3. API / architecture / testing / roadmap / migration strategy の影響箇所を更新する
4. CI と adapter-only test を実行する
5. 実機互換性に影響する場合は hardware matrix を更新する

更新後も過去 fixture の provenance には、生成元の旧 SHA を残す。
