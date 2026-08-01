# Bumble registry package 公開前検査

- 検査日時: 2026-08-01T13:08:42+09:00
- public fork: `https://github.com/niart120/bumble-rs`
- branch: `feat/swbt-registry-package-names`
- revision: `5fb0f6ddb811d1ad43dffa6e72a5d8cc6096fb07`
- package version: `0.1.0`
- 実行した公開操作: なし

## crates.io 名と owner

`cargo search swbt-bumble --limit 100` の出力は 0 行だった。検査時点では `swbt-bumble`
prefix の crate は検索結果に存在せず、既存 owner も存在しない。crate 名は予約されないため、公開直前に
24 名を再確認する。初回公開に使う crates.io account と、公開後の owner 一覧は現在未確定であり、
明示承認を得る turn で確認する。

## 公開順序

各 layer 内に相互依存はない。crates.io index で直前 layer の `0.1.0` を取得できることを確認してから
次の layer へ進む。

| layer | package |
|---|---|
| 0 | `swbt-bumble`, `swbt-bumble-at`, `swbt-bumble-audio`, `swbt-bumble-avc`, `swbt-bumble-codecs`, `swbt-bumble-crypto`, `swbt-bumble-l2cap`, `swbt-bumble-rtp` |
| 1 | `swbt-bumble-att`, `swbt-bumble-hci`, `swbt-bumble-hid`, `swbt-bumble-rfcomm`, `swbt-bumble-sdp`, `swbt-bumble-smp` |
| 2 | `swbt-bumble-controller`, `swbt-bumble-hfp` |
| 3 | `swbt-bumble-gatt` |
| 4 | `swbt-bumble-profiles` |
| 5 | `swbt-bumble-host` |
| 6 | `swbt-bumble-avctp`, `swbt-bumble-avdtp` |
| 7 | `swbt-bumble-a2dp`, `swbt-bumble-avrcp` |
| 8 | `swbt-bumble-transport` |

`bumble-drivers` と `bumble-pandora` は swbt-rs package の通常依存と package 検証用 dev 依存から
到達しないため公開対象に含めない。

## archive と SHA-256

一時的な `[patch.crates-io]` で未公開 package を同 revision の local path へ解決し、対象 24 package
それぞれに `cargo package --locked` を実行した。全 archive の生成と verify build が成功した。

| package | SHA-256 |
|---|---|
| `swbt-bumble` | `75c3247d992934b77b994a4921b0eec04c7b01d1ec4ca977b69d604383b27adb` |
| `swbt-bumble-a2dp` | `23228fb6156078c0952cb00e0632dfa13d7994cf9a9d5066870e5defb31430b8` |
| `swbt-bumble-at` | `2da32ac1eb033c64c6b9301159feb44ccd26f942cdfa35317e3654739a849a37` |
| `swbt-bumble-att` | `34d66dfd9a479429681c93bef13337258806870755ca50cdfd1f2e90e87f23d6` |
| `swbt-bumble-audio` | `c84665ba112757d1bc9144cad70e51817df9be079dc75c8c59bf682d553954c7` |
| `swbt-bumble-avc` | `a74139ee60527ec39e22d6c03a60bc7ae8e8b0edda26fe6e4b6e7708212f865e` |
| `swbt-bumble-avctp` | `c486df5ea5f9c10eac93e9e67d1279f36274a07c42af895486b5781c3d0e9c93` |
| `swbt-bumble-avdtp` | `f2756214c768b8d894172dd515b2d9a68a88ed40137e98d5972f618ee173e8e2` |
| `swbt-bumble-avrcp` | `314486eee47d20fc91d22c584e54676d9f8dbcd1d32dfcd52b70b8e63dd937e8` |
| `swbt-bumble-codecs` | `0b7ea487584925437b98a5f889f3574313f1b334c0d41cc06dc5ac385e370364` |
| `swbt-bumble-controller` | `26f10ddd0620a2c9e5fe3308f2f0f94c3b5874e39dde71cc4b78dae339ceb6a5` |
| `swbt-bumble-crypto` | `096d63813365a9088838c3e44a318e5007dbf7cc825205b0346c46b8d656ec5f` |
| `swbt-bumble-gatt` | `d0ec2a9d3accabd6c28bfa395c230c7584a0c536071cbe1b3b843949d258f1e3` |
| `swbt-bumble-hci` | `467868e9774eaa75287c173dd19dc3360a6dc352ba6b8e46b55aee23e121876f` |
| `swbt-bumble-hfp` | `9cba78887e6315fcaf5c3c2628c2e2c7359fc256cf6118cdf189df7f8bded7c6` |
| `swbt-bumble-hid` | `f909f2d10383728997adf7f60123db3dd34658f5c9ffea23441662b9ba57ddf0` |
| `swbt-bumble-host` | `77959b36eafee9c6868c8cdac5a547c35e70ebbd7866ac157bad32abcaf2ba54` |
| `swbt-bumble-l2cap` | `97e79acf87586f70989232e319b84770a70517412d2d0314baf975c25a4326d0` |
| `swbt-bumble-profiles` | `9e32c8489aeca4c64daa23cc037b9a55ab3103cf3707ae9e2eadb1198eaf0a73` |
| `swbt-bumble-rfcomm` | `b1f86450945c16b4af0cfe61d3cbbb1f22f7ddb5c6255c43e8a969981018c2a6` |
| `swbt-bumble-rtp` | `abc69bf25cc9e6ae1e4f633c2442b3735ff6024f413c36e142fea561c7fbc06b` |
| `swbt-bumble-sdp` | `309409e38455524d15afd5845a3ab6ccb41ca0077294b7d5fdde996dd5d65749` |
| `swbt-bumble-smp` | `f9a7e86cc8502e65e64a7b4098b9d8f652fbc0b29e3f9e03d450fb5786024e02` |
| `swbt-bumble-transport` | `ddf651611237992fecaddc2a9d139912a40849e847c8537f7a9a5836f9d0c1c4` |

archive 内の Cargo 正規化済み `Cargo.toml` 24 件を検査した。内部依存 79 辺はすべて
`package = "swbt-bumble*"` と `version = "=0.1.0"` を持ち、dependency block に local `path` または
Git source は残っていない。

## 公開前停止点

local path patch を外した `cargo package --locked -p swbt-bumble-hci` は、crates.io に
`swbt-bumble@0.1.0` がないため停止した。これは layer 順の registry publish が必要であることを示す。
local path patch を使った verify は各 package archive の内容と compile を検査するが、公開済み archive
だけから依存を解決する T06 の clean-install smoke を置き換えない。

layer 0 の `swbt-bumble` は `cargo publish --dry-run --locked -p swbt-bumble` が成功した。18 files、
圧縮 63.7 KiB の package と verify build を完了し、upload は dry-run により中止された。crates.io の
状態は変更していない。残りの `swbt-bumble-at`、`swbt-bumble-audio`、`swbt-bumble-avc`、
`swbt-bumble-codecs`、`swbt-bumble-crypto`、`swbt-bumble-l2cap`、`swbt-bumble-rtp` も同じ
`cargo publish --dry-run --locked` が成功した。layer 0 は 8/8 package が公開前検査を通過した。

layer 1–8 の 16 package は、一時 `[patch.crates-io]` で同 revision の未公開下位 package を local path
へ解決した `cargo publish --dry-run --locked` が 16/16 成功した。対象 24 package はすべて publish
metadata、archive、verify build、upload 前処理を検査済みである。layer 1–8 の結果は local patch 補助で
あり、下位 package を crates.io から取得できることは証明しない。

公開 turn では各 package について、公開直前の name availability、対象 `.crate` の checksum、
`cargo publish --dry-run`、公開後の `cargo info` と owner 一覧を確認する。実際の publish は当該 turn の
明示承認なしに行わない。
