# Explicit local address

- 状態: **着手中**
- milestone: explicit local address milestone
- branch: `feat/unit-011-explicit-local-address`
- 正本:
  - `spec/initial/roadmap.md` 13
  - `spec/initial/api.md` 3.4、12
  - `spec/initial/architecture.md` 8、15、16、21
  - `spec/initial/testing.md` 6.2、11、12、13
  - `spec/initial/migration-strategy.md` 5、16、17

## 1. 目的

`ProfileIdentity::LocalAddress` を CSR8510 A10 の volatile BD_ADDR 書換えへ接続し、profile に保存した
identity と controller が実際に報告する local address が一致した場合だけ pairing / reconnect を開始する。
書換え途中の失敗では再試行可能な通常エラーと、物理 power cycle が必要な不確定状態を型付きで区別する。

この unit は `swbt-python` の CSR 実機探索と local-address profile の確定済み挙動を Rust へ移植する。
Rust 側の公開 `LocalAddress` は個別かつ locally administered な address だけを受け付けるため、Python 版の
characterization で使った未割当 universal dummy address は製品経路へ移植しない。

## 2. 着手時点

### 2.1 実装済み

- `LocalAddress` は `XX:XX:XX:XX:XX:XX` を parse し、multicast、universal、予約 inquiry LAP を拒否する
- `ProfileIdentity::LocalAddress` と schema v2 の `exp-local-address` 読込みは存在する
- local-address profile の `Debug` と profile summary は address を表示しない
- pairing key store の namespace は controller が報告した local address から決まる
- Bumble fork revision `b8c7cd625bc2ac2f58a4beb4ade1264426969819` は generic HCI command と Vendor Event の
  decode を持つ
- normal Bumble session は connectable / discoverable を無効にしたまま HCI 初期化し、pairing command 後にだけ
  Switch-facing 動作へ進む

### 2.2 未実装

- create-profile は local address を `UnsupportedCapability` で拒否する
- local-address empty envelope を生成する経路がない
- typed profile から runtime identity を取得する crate-private 境界がない
- CSR BCCMD の build / parse、volatile write、warm reset、USB 再列挙、read-back がない
- Bumble `ExternalHost` は Vendor Event を device event として保持するが、command と一致する Vendor Event を期限付きで
  待つ公開 API と、応答を待たず warm reset を送る公開 API を持たない
- write 開始後の不確定状態を表す public error kind がない
- probe、公開文書、実機 matrix に explicit local address がない

## 3. 対象範囲

- CSR BCCMD opcode `0xFC00` の PSKEY_BDADDR GETREQ / volatile SETREQ / warm reset byte contract
- request type、sequence number、VARID、status を使う Vendor Event response matching
- CSR company identifier `10` の capability gate
- already-active fast path、volatile write、warm reset、USB 再列挙、read-back
- write 開始後の失敗を `AdapterIdentityRecoveryRequired` として分類する recovery boundary
- profile 作成前検査、local-address empty envelope、typed reopen、runtime identity projection
- normal Bumble power-on 後かつ pairing / advertising 前の expected-address guard
- actual local address と一致する key-store namespace
- create-profile と existing profile build/open/reconnect の共通 identity preparation
- `swbt-probe pair` から local address を明示する安全な実機入口
- Python schema v2 の `exp-local-address` と namespace 互換 fixture
- Pro / Joy-Con L / Joy-Con R、Periodic / Direct に共通する model-independent unit / virtual coverage
- Windows 11、CSR8510 A10、WinUSB、Switch 2 22.5.0 の期限付き実機検証
- duplicate address、power cycle、失敗時復旧、対応範囲の公開文書
- Bumble public fork branch の最小 API 追加と固定 revision 更新

## 4. 対象外

- CSR persistent store / EEPROM への書込み
- CSR 以外の vendor command と adapter
- 任意の universal address、multicast address、予約 inquiry LAP
- 複数 adapter / 複数 controller の同時 identity 切替
- duplicate address の自動検出
- write 開始後の自動 restore。状態が確定できないため物理 power cycle を要求する
- macOS、Linux、別 CSR 個体での実機保証
- raw address、link key、raw HCI packet を diagnostics trace に出す機能
- upstream Bumble issue / PR。ユーザ指示により作成しない
- M9 の crates.io dependency namespace 解決、crate publish、tag、release

## 5. 振る舞い仕様

### 5.1 profile と事前検査

- `ProfileIdentity::LocalAddress(address)` は `bumble` feature が利用可能な場合だけ create-profile capability gate を通る
- path 必須、identity、target existence の検査順を維持する
- target absent の場合、`exp-local-address` と uppercase address を持つ valid schema v2 empty envelope を adapter open 前に
  create-new する
- concurrent target creation は `ProfileAlreadyExists` とし、既存 file を置換しない
- typed reopen は persisted identity を runtime config へ渡す。caller が渡した一時値を正本にしない
- feature なしでは filesystem / adapter side effect 前に `UnsupportedCapability` を返す

### 5.2 pure CSR contract

- PSKEY_BDADDR は CSR BCCMD channel `0xC2`、VARID `0x7003`、PSKEY `0x0001` を使う
- GETREQ は type `0x0000`、SETREQ は `0x0002`、response は双方とも GETRESP `0x0001` とする
- volatile store selector は `0x0008` だけを生成し、persistent selector を production API に持たせない
- response は channel、response type、sequence number、VARID が request と一致する場合だけ受理する
- status word が非 0、短い response、不一致 response は成功にしない
- address packing と warm reset bytes は Python 基準断面と BlueZ 由来 fixture に一致させる

### 5.3 identity preparation

- adapter open 後、HCI Reset を送らず local version と current BD_ADDR を読む
- company identifier が `10` でなければ write 前に `UnsupportedCapability` を返す
- current address が target と一致すれば write / warm reset / reopen を行わない
- 不一致なら volatile SETREQ の成功 response を待ち、CSR warm reset を送って session を閉じる
- selector を使って期限まで再 open を繰り返し、HCI Reset なしで BD_ADDR を read-back する
- read-back が target と一致した場合だけ normal Bumble session の初期化へ進む
- write を送る前の open/read/capability failure は通常の分類済み error とする
- write を送った後の send、response、reset、close、再列挙、read-back、不一致は
  `ErrorKind::AdapterIdentityRecoveryRequired` とする。利用者は dongle を抜き差しして元 identity を確認するまで再試行しない

### 5.4 expected-address guard と runtime

- normal Bumble session は既存どおり HCI Reset と capability read を行う
- local-address profile では normal power-on が返した local address を persisted target と照合する
- 不一致なら connectable / discoverable、pairing、reconnect、worker spawn へ進まない
- identity preparation で write が発生した後の guard 不一致は physical recovery required とする
- adapter-default profile は現行 open path と error semantics を維持する
- key-store namespace は guard 済みの actual local address と一致する
- local-address existing profile の Periodic / Direct reconnect は同じ preparation と guard を使う

### 5.5 error と秘密情報

- `AdapterIdentityRecoveryRequired` は caller が分岐できる public `ErrorKind` とする
- public message は USB dongle の物理 power cycle と再確認が必要なことを示す
- `Display`、`Debug`、source chain、related error、diagnostics、probe error record に target/current address、selector、path、
  raw packet、key を含めない
- internal stage は closed enum とし、人向け文字列を分岐契約にしない

### 5.6 probe と実機

- `swbt-probe pair` は local address の明示指定を受け、指定なしは adapter-default のままにする
- local address は stdout / stderr / trace に出さず、identity kind だけを記録する
- 実機 run は fresh local-address profile で Pro Periodic pairing、保存鍵 reconnect、Direct reconnect、neutral close、
  adapter reopen、profile bytes / namespace を確認する
- Switch UI 観測と machine evidence を分離する
- 書換えを伴う各実機 run の終了後は dongle を物理 power cycle し、adapter-default address への復帰を read-only で確認する
- failure injection または実機 failure が write 後に起きた場合、その run を中止し、power cycle と元 address 復帰確認まで
  次の write を行わない

### 5.7 duplicate address guidance

- 利用者自身が管理する locally administered address を使い、同じ radio 範囲で重複させない
- 同じ address を複数 dongle で同時に使わない
- profile と local address を一対一で管理し、別 identity の key namespace を混在させない
- profile 削除は dongle の volatile identity を復元しない。復元は物理 power cycle と read-back で確認する

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| pending | T01 CSR GETREQ / volatile SETREQ / warm reset bytes、address decode、response matching、status failure を固定 fixture で検査する | new/characterization | pure unit | adapter I/O なし |
| pending | T02 Bumble `ExternalHost` が generic command に一致する Vendor Event を期限付きで返し、warm reset command を応答待ちなしで送れることを fork 側 test で固定する | new | dependency unit | public fork branch。upstream PR なし |
| pending | T03 identity preparation の already-active、rewrite→re-enumerate→read-back、非 CSR、write 前 failure、write 後 recovery-required を fake port で検査する | new/safety | transport unit | closed stage、bounded retry |
| pending | T04 local-address empty profile を adapter open 前に create-new / typed reopen し、feature-disabled と concurrent target の順序を検査する | new/regression | profile/controller | persisted identity が正本 |
| pending | T05 create/reconnect の normal power-on expected-address guard、actual namespace、adapter-default 無変更を fake / Bumble session で検査する | new/regression | runtime/transport | pairing可視化前に拒否 |
| pending | T06 Python schema v2 local-address fixture と、3 model・Periodic/Direct の identity projection を検査する | compatibility | integration/virtual | address/key非表示 |
| pending | T07 probe の local-address 引数、identity-kind output、usage/error/redaction と公開 docs を確定する | new/docs | CLI/public | raw addressを出力しない |
| pending | T08 CSR8510 A10 で Pro Periodic pair/reconnect、Direct reconnect、neutral cleanup、power-cycle recovery を期限付きで記録する | hardware | Windows/Switch | machine/UI分離 |
| pending | T09 completion gate、rustdoc/API review、dependency revision/source baseline、self-review を確定する | gate | package/docs | publish/tagなし |

各 item は red、green、必要な refactor を完了してから、その item だけを Conventional Commit で commit する。
Bumble fork 側 T02 は fork repository の専用 branch に独立 commit を作り、その commit SHA を本仕様と `Cargo.toml` に固定する。

## 7. 対象ファイル

- `Cargo.toml`
- `Cargo.lock`
- `src/error.rs`
- `src/profile/identity.rs`
- `src/profile/document.rs`
- `src/controller/create.rs`
- `src/controller/config.rs`
- `src/controller/runtime.rs`
- `src/runtime/transport/csr.rs`
- `src/runtime/transport/identity.rs`
- `src/runtime/transport/bumble.rs`
- `src/runtime/transport/mod.rs`
- `src/bin/swbt-probe.rs`
- `tests/`
- `docs/platform-support.md`
- `docs/troubleshooting.md`
- `README.md`（入口の変更が必要な場合だけ）
- `spec/wip/unit_011/evidence/`
- Bumble fork の `bumble-transport/src/host.rs` と対応 test

実装中に責務境界が確定した場合は filename を調整し、この一覧へ反映する。

## 8. 検証

TDD item ごとの targeted test に加え、完了時に変更範囲に応じて次を実行する。

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo test --all-features
cargo build
cargo build --all-features
cargo doc --all-features --no-deps
git diff --check
```

Bumble fork は変更 package の unit test、`cargo fmt --check`、対象 package の clippy を実行する。hardware test は
固定 timeout、adapter、driver、console version、run index、cleanup、power-cycle recovery を evidence に残す。

## 9. 先送り事項

- non-CSR identity backend
- persistent local address
- duplicate address detection
- Linux / macOS hardware matrix
- upstream issue / PR と official revision への復帰
- crates.io へ配布可能な Bumble dependency namespace

## 10. 完了チェックリスト

- [ ] T01–T09 が `refactor-done` または理由付き `refactor-skipped`
- [ ] test item ごとの commit がある
- [ ] local-address create-profile が `UnsupportedCapability` ではなく実装済み
- [ ] local-address existing profile の reconnect が実装済み
- [ ] expected-address guard が pairing / advertising 前に働く
- [ ] write 後 failure が typed recovery error になる
- [ ] physical power cycle と元 identity 復帰を実機確認した
- [ ] profile namespace が expected local address と一致する
- [ ] address / key / selector / path / raw packet が公開 error と trace に出ない
- [ ] duplicate address guidance と対応範囲が公開 docs にある
- [ ] Bumble fork branch / commit / 差分 / rollback を記録した
- [ ] default / all-features gate が成功した
- [ ] `agentic-self-review` の指摘を採否記録した
- [ ] `spec/complete/unit_011/` へ移動した
- [ ] PR merge 後に default branch を同期し、作業 branch を削除した

