# M6 profile compatibility、reconnect、Pro Direct

- 状態: **着手中**
- milestone: M6
- branch: `feat/unit-007-m6-profile-reconnect`
- 正本:
  - `spec/initial/roadmap.md` 9
  - `spec/initial/api.md` 3.3、4、5、6
  - `spec/initial/architecture.md` 7、12、15.5、16、18、19
  - `spec/initial/migration-strategy.md`
  - `spec/initial/testing.md` 5.9、6、8.5、10、13
- Python 基準断面:
  - repository: `niart120/swbt-python`
  - revision: `84d2723b127f70fc78e12f4496f5c40af0ccfb0a`
- Bumble fork:
  - repository: `https://github.com/niart120/bumble-rs`
  - branch: `fix/external-host-reader-lifecycle`
  - revision: `b8c7cd625bc2ac2f58a4beb4ade1264426969819`
  - public fork と branch push だけを許可範囲とし、upstream PR は作成しない

## 1. 目的

Python schema v2 profile と Rust の typed profile を相互利用可能にし、保存済み Classic link key
による reconnect を production runtime へ接続する。同じ Pro profile を Periodic と Direct の
両方で使い、Windows 11、CSR8510 A10、WinUSB、Switch 2 `22.5.0` の実機で power-cycle
reconnect、Direct input、neutral、期限付き close を確認する。

M6 は profile file の完全性、Bumble key-store の変換、接続経路、Switch UI の観測を別の証拠と
して扱う。JSON が読めることを bond の有効性と同一視せず、command acceptance を UI 反映へ
読み替えない。

## 2. Intent Delta

| 境界 | M5 完了時 | M6 完了後 | 保証 |
|---|---|---|---|
| profile | valid empty schema v2 envelope の create-new | Python/Rust 相互読取、既知 field 検証、未知 field 保持、決定的出力 | model mismatch、壊れた key、複数 peer を adapter open 前に拒否する |
| persistence | create-new のみ | lock 付き atomic replace | interruption 後の target は旧版か新版の完全な document である |
| pairing key | profile に保存しない | Bumble key-store update を profile 全体へ反映 | key material と raw profile を log、error、evidence に出さない |
| connection | fresh pairing のみ | stored-key active/incoming reconnect、明示的 re-pair | invalid bond を暗黙削除せず、fresh pairing へ自動 fallback しない |
| Direct | fake runtime の transaction | same Pro profile の production/virtual Direct connection | idle 時に user input report を周期送信せず、失敗前 snapshot を維持する |

## 3. 対象範囲

- schema v2 raw DTO と `PairingProfile<model::Pro>` の相互変換
- Python profile fixture の Rust 読取
- Rust 出力の Python 読取
- 未知の top-level field と key-store field の lossless 保持
- deterministic JSON、UTF-8、2-space indent、sorted keys、trailing newline
- create-new と lock 付き atomic update
- adapter-default namespace の power-on 後 local address 解決
- current peer 最大1件と key field/hex/address type の検証
- `SwbtProfileKeyStore<M>` と Bumble `KeyStore` の変換
- pairing key update の profile 永続化
- stored-key active/incoming reconnect
- public `reconnect`、`connect`、`try_reconnect`、`try_connect`
- no-bond、timeout、stale bond、明示的 re-pair、clean close
- same Pro profile の Periodic/Direct 再利用
- Pro Direct hardware runner と secret-free evidence
- alpha.2 criteria note

## 4. 対象外

- upstream Bumble PR / issue 作成
- Joy-Con L/R
- explicit local Bluetooth address
- invalid bond の暗黙削除
- automatic infinite reconnect
- long-run jitter、stable diagnostics schema、`swbt-probe`
- Linux、macOS、cross compile
- crates.io publish、tag、GitHub release
- Python repository の変更

## 5. 振る舞い仕様

### 5.1 schema v2

`ProfileDocument::parse_json` は raw document を保持し、少なくとも次を検証する。

- `format == "swbt-pairing-profile"`
- `schema_version == 2`
- `controller_kind` が既知で、typed conversion 時に `M::KIND` と一致する
- `identity` が `adapter-default` または仕様上有効な local-address 形式である
- `key_store_namespaces` は object である
- namespace 名、peer address、address type、key field、hex 長と文字種が有効である
- 各 namespace の current peer は最大1件である

未知 field は読み取りと既知 field の更新を経ても保持する。secret を含み得る parse error と
`Debug` は raw JSON、key value、peer key object を出力しない。

Rust の正規化出力は UTF-8、2-space indent、object key の辞書順、末尾改行1個とする。
Python fixture を Rust で読み、Rust 出力を pinned Python reader で読む。相互読取は field の
意味と未知 field の保持を検査し、文字列の完全一致だけを互換性の根拠にしない。

### 5.2 atomic persistence

create-new は M5 の no-replace 契約を維持する。update は次の順序で行う。

1. profile path に対する排他 lock を取得する。競合時は待ち続けず typed error を返す
2. target を再読取し、呼び出し側が基準にした document と同じ世代であることを確認する
3. same-directory temporary file に complete normalized JSON を書く
4. `flush` と `sync_all` を完了する
5. target を atomic replace する
6. 対応 OS では parent directory を同期する
7. temporary file と lock を解放する

書込み失敗、replace 前 interruption、競合更新では既存 target を壊さない。自動 backup と世代
履歴は作らない。

### 5.3 Bumble key-store

`SwbtProfileKeyStore<M>` は `PairingProfile<M>` と Bumble `KeyStore` の間だけを変換する。

- adapter-default は adapter power-on 後に得た local controller address を namespace に使う
- namespace の peer は0件または1件
- `get` / `get_all` は検証済み key だけを返す
- `update` は current peer を1件へ置換し、profile document 全体を atomic update する
- `delete` は明示呼出しだけで実行し、reconnect failure から暗黙に呼ばない
- Bumble key-store error は worker で握り潰さず public typed error へ接続する
- link key、LTK、IRK、CSRK、peer key object を trace と error message に含めない

### 5.4 connection

`reconnect(timeout)` は usable bond がない場合に `NoBond`、期限内に protocol Ready へ到達しない
場合に timeout を返す。stored peer への outgoing Classic connection と、同じ stored peer からの
incoming connection の両方を受理する。

`connect(options)` の順序:

1. usable bond があれば reconnect
2. bond がなく `allow_pairing = true` なら pairing
3. bond がなく `allow_pairing = false` なら `NoBond`
4. stored key を使った通信が失敗しても bond を削除せず、pairing へ自動 fallback しない

`try_*` は no-bond、timeout、接続失敗を `ConnectionResult` として返し、worker terminal や
profile corruption を成功結果へ変換しない。`pair()` は empty profile または一時 controller の
明示的 re-pair 入口であり、existing bonded profile を暗黙初期化しない。

新 session は input snapshot、handshake、report mode、player lights、IMU、timer、HID channel
を reset する。旧 session event は破棄する。成功条件は同じ session の ACL、両 HID channel、
bootstrap neutral、report mode reply、非0 player lights replyである。

### 5.5 Direct

Direct は protocol Ready 後も user input report の周期送信を開始しない。`send` は candidate
`ProInputState` が transport に受理された場合だけ snapshot を commit し、acceptance 前の失敗
では直前 snapshot を維持する。helper と `tap` は同じ transaction 規則に従う。

`close()` は接続中なら neutral を1件受理させ、host queue flush、HID/ACL/HCI/worker cleanup を
期限付きで続ける。Periodic で作成・更新した Pro profile と Direct で更新した Pro profile は、
reporting mode 用 field を追加せず相互利用できる。

### 5.6 実機証拠

hardware runner は既存 profile path、adapter selector、mode、timeout、run index を明示入力と
する。profile raw JSON と key material は出力しない。

実機で次を別 run として記録する。

- Periodic で stored-key reconnect、A、L+R、左右 stick、neutral、close
- Switch 2 power-cycle 後の stored-key reconnect
- 同じ profile の Direct reconnect
- Direct idle 中の user input report 0件
- Direct の A、L+R、左右 stick、neutral、close
- stale bond を模した失敗で profile が変更・削除されないこと

Switch UI の入力反映と残留入力なしは人が観測し、runner の機械結果とは別 record に保存する。

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| refactor-skipped | T01 Python schema v2 fixture を lossless に typed Rust profile として読み、model mismatch、壊れた key、複数 peer を拒否する | new/edge | unit/integration | adapter open 前の検証、secret-free error を含む |
| todo | T02 Rust profile を決定的 JSON として出力し、pinned Python reader が読めることを検査する | new | integration | 未知 field 保持、2-space/sorted/trailing newline |
| todo | T03 existing profile を lock 付き atomic replace し、競合と interruption 後も旧版か新版を読める | new/edge | unit/integration | create-new no-replace は維持 |
| todo | T04 Bumble key object を adapter-default namespace の単一 peer として lossless に取得・更新・明示削除する | new | unit | `SwbtProfileKeyStore<M>` |
| todo | T05 production pairing の key-store update を profile へ保存し、永続化失敗を worker/public error へ伝える | new/regression | integration | raw key 非出力 |
| todo | T06 virtual Classic で stored key の outgoing/incoming reconnect が同じ session の Ready へ到達する | new | integration | active/incoming 両経路 |
| todo | T07 public connection API が no-bond、timeout、stale bond、明示 re-pair を仕様どおり分類する | new/edge | unit/integration | 暗黙削除・fallback なし |
| todo | T08 同じ Pro profile を Periodic/Direct で再利用し、Direct idle、send failure、tap、neutral close の契約を満たす | new/regression | integration | 既存単体挙動を profile 接続面で検査 |
| todo | T09 hardware runner で Periodic reconnect、power-cycle reconnect、Direct input と clean close を記録する | new | hardware | UI 観測を別 record にする |
| todo | T10 completion gate と alpha.2 criteria note を確定する | new | docs/package | Rust 1.87、各 feature 組合せ、未検証事項 |

### 6.1 TDD cycle evidence

| phase | item | evidence |
|---|---|---|
| refactor-skipped | T01 | red: `cargo +1.87.0 test profile::document_tests --locked` は namespace 内部を未検証のため `namespace_shape_and_known_key_fields_are_validated_without_secret_echo` が失敗。green: pinned Python `0.6.0` / commit `84d2723b127f70fc78e12f4496f5c40af0ccfb0a` の synthetic Classic link-key fixture を typed Pro profile として読み、namespace/peer address、単一 peer、既知 numeric/key field、hex、metadata type の不正を fixed secret-free error で拒否する7 test が成功。Rust 1.87 all-feature test は272 passed / 2 ignored、stable clippy all-target/all-feature `-D warnings`、rustfmt、diff check が成功。validation helper は raw document と typed key-store adapter の間に留まり、T04 の Bumble conversion を先取りしないため追加 refactor を省略 |

## 7. 対象ファイル

- `src/profile/`
- `src/connection.rs`
- `src/controller/`
- `src/runtime/`
- `src/runtime/transport/`
- `tests/fixtures/`
- `tests/profile_compat/` または同等の integration test
- `examples/`
- `README.md`、crate rustdoc、公開 API rustdoc
- `evidence/` の M6 secret-free 実機要約
- 本作業仕様

Bumble fork の追加変更が必要な場合は、swbt 側の失敗 test と fork 側の最小 test を先に作る。
変更は既存の許可済み public fork branch へ push できるが、upstream PR は作らない。

## 8. 検証

TDD item ごとに対象 test を red/green 同一 command で実行する。完了 gate:

```powershell
cargo +1.87.0 check --all-targets --all-features --locked
cargo +1.87.0 test --all-features --locked
cargo +1.87.0 test --locked
cargo +1.87.0 test --no-default-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo clippy --all-targets --locked -- -D warnings
cargo +1.87.0 build --all-features --locked
cargo +1.87.0 build --locked
cargo +1.87.0 build --no-default-features --locked
cargo +1.87.0 doc --all-features --no-deps --locked
cargo fmt --check
git diff --check
```

Python compatibility test は pinned revision の reader を使い、実行 command と Python version を
記録する。hardware、network、cross compile、publish は対象に含めたものだけを実行し、未実行
条件を T10 に残す。

## 9. 先送り事項

- Joy-Con L/R profile と実機: M7
- long-run jitter、stable diagnostics、probe: M8
- Linux、macOS、release: M9
- explicit local address: 独立 milestone
- Bumble upstream contribution: ユーザが明示的に許可するまで実施しない

## 10. 完了チェックリスト

- [ ] T01-T10 が個別 commit で完了している
- [ ] Python profile を typed Rust が lossless に読む
- [ ] Rust profile を pinned Python reader が読む
- [ ] update interruption 後の target が旧版または新版として有効である
- [ ] lock contention を typed error として返す
- [ ] adapter-default namespace と単一 peer 制約を守る
- [ ] pairing key update が profile へ永続化される
- [ ] active/incoming stored-key reconnect が virtual test を通る
- [ ] no-bond、timeout、stale bond を区別し、invalid bond を暗黙削除しない
- [ ] same Pro profile を Periodic/Direct で利用する
- [ ] Direct idle で周期 user input report を送らない
- [ ] Direct send failure で直前 snapshot を維持する
- [ ] power-cycle reconnect と Direct input を実機確認する
- [ ] key material、raw profile、secret が error、trace、evidence に残らない
- [ ] alpha.2 criteria、未実行条件、residual risk を記録する
- [ ] upstream PR を作成していない
- [ ] self-review と completion gate を通す
- [ ] `spec/complete/unit_007/` へ移動する
