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

- `format == "swbt.profile"`
- `schema_version == 2`
- `controller_kind` が既知で、typed conversion 時に `M::KIND` と一致する
- `identity` が `adapter-default` または仕様上有効な local-address 形式である
- `key_store.namespaces` は object である
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
| refactor-done | T02 Rust profile を決定的 JSON として出力し、pinned Python reader が読めることを検査する | new | integration | 未知 field 保持、2-space/sorted/trailing newline |
| refactor-done | T03 existing profile を lock 付き atomic replace し、競合と interruption 後も旧版か新版を読める | new/edge | unit/integration | create-new no-replace は維持 |
| refactor-done | T04 Bumble key object を adapter-default namespace の単一 peer として lossless に取得・更新・明示削除する | new | unit | `SwbtProfileKeyStore<M>` |
| refactor-done | T05 production pairing の key-store update を profile へ保存し、永続化失敗を worker/public error へ伝える | new/regression | integration | raw key 非出力 |
| refactor-done | T06 virtual Classic で stored key の outgoing/incoming reconnect が同じ session の Ready へ到達する | new | integration | active/incoming 両経路 |
| refactor-done | T07 public connection API が no-bond、timeout、stale bond、明示 re-pair を仕様どおり分類する | new/edge | unit/integration | 暗黙削除・fallback なし |
| refactor-done | T08 同じ Pro profile を Periodic/Direct で再利用し、Direct idle、send failure、tap、neutral close の契約を満たす | new/regression | integration | 既存単体挙動を profile 接続面で検査 |
| todo | T09 hardware runner で Periodic reconnect、power-cycle reconnect、Direct input と clean close を記録する | new | hardware | UI 観測を別 record にする |
| todo | T10 completion gate と alpha.2 criteria note を確定する | new | docs/package | Rust 1.87、各 feature 組合せ、未検証事項 |

### 6.1 TDD cycle evidence

| phase | item | evidence |
|---|---|---|
| refactor-skipped | T01 | red: `cargo +1.87.0 test profile::document_tests --locked` は namespace 内部を未検証のため `namespace_shape_and_known_key_fields_are_validated_without_secret_echo` が失敗。green: pinned Python `0.6.0` / commit `84d2723b127f70fc78e12f4496f5c40af0ccfb0a` の synthetic Classic link-key fixture を typed Pro profile として読み、namespace/peer address、単一 peer、既知 numeric/key field、hex、metadata type の不正を fixed secret-free error で拒否する7 test が成功。Rust 1.87 all-feature test は272 passed / 2 ignored、stable clippy all-target/all-feature `-D warnings`、rustfmt、diff check が成功。validation helper は raw document と typed key-store adapter の間に留まり、T04 の Bumble conversion を先取りしないため追加 refactor を省略 |
| refactor-done | T02 | red: `cargo +1.87.0 test --test profile_compat --locked` は crate root に公開 `PairingProfile` がなく compile error。green: `PairingProfile<M>::from_json` と `to_json_bytes` を公開し、raw DTO と field mutation は非公開のまま、未知 top-level/key metadata の保持、反復出力一致、UTF-8、2-space indent、sorted keys、末尾改行を integration test で検査。manual ignored writer が作った Rust profile を Python 3.13、pinned repository HEAD `84d2723b127f70fc78e12f4496f5c40af0ccfb0a` の `PairingProfile.load` が読み、Pro、adapter-default、namespace 1、peer 1を確認。key value は出力していない。Rust 1.87 all-feature lib は272 passed / 2 ignored、profile compatibility は1 passed / 1 manual ignored。stable clippy、all-feature rustdoc `-D warnings`、rustfmt、diff check が成功。refactor: integration fixture setup を共通 helper に抽出し、公開 profile module/crate docs を byte API と filesystem persistence の境界に合わせた |
| refactor-done | T03 | red: `cargo +1.87.0 test profile::store::tests --locked` は `ProfileUpdatePort` と `update` が未定義で compile error。green: existing regular file の OS 排他 lock を非待機で取得し、lock 後に再読取した bytes が caller の expected bytes と一致する場合だけ、same-directory temporary file の complete write、flush、`sync_all`、Windows 対応 atomic replace、parent sync を行う。stale writer と lock contention は `WouldBlock` で target を変更せず、commit 前に atomic writer を破棄した場合は旧 profile、commit 後は新 profile が typed Pro として有効な6 test が成功。`atomic-write-file 0.3.0` と `fs2 0.4.3` を MSRV 1.87 で固定し、lockfile の既存 `http 1.4.2` と `rustls 0.23.42` は更新前 version を維持。Rust 1.87 all-feature lib は275 passed / 2 ignored、default lib は242 passed / 1 ignored、all-feature build、stable all/default clippy、rustfmt、diff check が成功。refactor: lock contention の OS error を一つの helper で `WouldBlock` へ正規化し、filesystem update 契約を T04 が利用する `ProfileUpdatePort` に分離した |
| refactor-done | T04 | red: `cargo +1.87.0 test profile_key_store --all-features --locked` は `SwbtProfileKeyStore` が未定義で compile error。green: power-on 後 local address を adapter-default namespace に解決し、その namespace の0件または1件だけを Bumble `PairingKeys` と変換する3 test が成功。同じ peer の update は既知 key field だけを置換して未知 peer metadata を保持し、別 peer の update は current peer を1件へ置換する。delete は明示呼出しだけで行い、欠落 peer を secret-free `NotFound`、不正名を secret-free `InvalidAddress` とする。read、parse、serialize、atomic update error は path、peer、key value を含まない固定文言へ変換し、adapter `Debug` も path と namespace を伏せる。Rust 1.87 all-feature lib は278 passed / 2 ignored、default lib は242 passed / 1 ignored、all/default build、stable all/default clippy、all-feature rustdoc `-D warnings`、rustfmt、diff check が成功。refactor: raw document の未知 field 保持操作を profile 層、Bumble JSON 変換と error sanitization を transport adapter 層へ分離した。T05 が production `Device` へ adapter を接続するまで、該当内部入口だけに理由付き dead-code 許可を置く |
| refactor-done | T05 | red: `cargo +1.87.0 test production_profile_key_store --all-features --locked` は production factory、profile-aware initializer、`InvalidKeyStore` transport/public variant が未定義で compile error。green: persistent profile だけを typed factory へ投影し、controller power-on 後の local address を得てから `SwbtProfileKeyStore<M>` を Bumble `Device` へ設定する。scripted HCI `LinkKeyNotification` が schema v2 profile の該当 namespace/current peer を atomic update することを検査した。排他 lock を保持した有効な secret-bearing profile では target を変更せず、path、peer、key value、未知 secret を含まない sticky `InvalidKeyStore` terminal を返す。worker は pending pairing response に同分類を保持し、公開 `Controller::pair()` は `ErrorKind::InvalidKeyStore` を返す。Rust 1.87 all-feature lib は283 passed / 2 ignored、default lib は244 passed / 1 ignored、all/default build、stable all/default clippy、all-feature rustdoc `-D warnings`、rustfmt、diff check が成功。refactor: profile path の型付き factory、Bumble key-store trait object wrapper、transport terminal、worker pairing response、公開 error mapping を各所有層へ分離した。reader thread の enqueue と zero-time poll の競合は activity wake を待つ test 同期へ修正した |
| refactor-done | T06 | red: `cargo +1.87.0 test stored_key_active_and_incoming_reconnect --all-features --locked` は `RuntimeCommand::Reconnect`、transport reconnect 入口、再接続 trace が未定義で compile error。green: virtual Classic の active では swbt が Central として発呼し、incoming では保存 peer 1件だけからの要求を Peripheral として受理する。両経路で両端の key-store `get` が保存 link key を読み、fresh `pair_classic` / CTKD を開始せず、暗号化した同じ ACL session 上の SDP、control/interrupt HID、bootstrap neutral、report-mode reply、非0 player-light replyを経て Ready へ到達した。fresh pairing completion は0、reconnect completion は各経路1である。production Bumble transport も power-on 後 namespace の usable Classic bond 1件だけを選び、active reconnect と同時 incoming 受理 window を開始する。Rust 1.87 all-feature は284 passed / 2 ignored、default は244 passed / 1 ignored、all/default build、stable all/default clippy、all-feature rustdoc `-D warnings`、rustfmt、diff check が成功。refactor: pairing と reconnect の接続 window を Classic session 内で分離し、worker の pending state を接続コマンド共通の名前へ変更した |
| refactor-done | T07 | red: `cargo +1.87.0 test public_reconnect_classifies_recoverable_and_terminal_failures --all-features --locked` は公開 `ConnectOptions` / `ConnectionPath` / `ConnectionStatus`、`ErrorKind::NoBond`、`Controller::reconnect` / `connect` / `try_*` が未定義で compile error。green: `reconnect` は no-bond、timeout、Ready 前 disconnect をそれぞれ `NoBond`、`ConnectionTimeout`、`ConnectionFailed` とし、invalid key-store と worker terminal を回復可能な成功値へ変換しない。`connect` は必ず reconnect を先に実行し、`NoBond` かつ `allow_pairing = true` の場合だけ明示 pairing へ進む。stale bond の disconnect では2番目に用意した pairing script を消費せず、bond の暗黙削除 API も呼ばない。`pair` は reconnect 判定を通らない明示 re-pair 入口を維持する。`try_*` は no-bond、timeout、Ready 前 disconnect だけを `ConnectionResult` に変換し、成功 path を `Reconnected` / `Paired` として返す。Rust 1.87 all-feature は287 passed / 2 ignored、default は247 passed / 1 ignored、all/default build、stable all/default clippy、all-feature rustdoc `-D warnings`、rustfmt、diff check が成功。refactor: worker が pending connection command の Pair/Reconnect 種別を保持し、同じ readiness failure を公開 API ごとの error 文脈へ写像するよう分離した。README と crate rustdoc から reconnect 未実装の記述を除き、virtual 検証済みと hardware power-cycle 未検証を分けた |
| refactor-done | T08 | red: `cargo +1.87.0 test same_pro_profile_reconnects_periodic_then_direct --all-features --locked` は file-backed reconnect harness と次回 interrupt send の失敗注入が未定義で compile error。green: schema v2 の同一 Pro profile を production `SwbtProfileKeyStore` 経由で Periodic、Direct の順に stored-key reconnect し、両 session が Ready へ到達した。Direct Ready 後の idle 5 tick では user input 0件、受理された A は snapshot へ反映、次の L+R send failure は直前 snapshot を維持した。0 ms の L+R tap は A+L+R、A の2 report、明示 neutral と close は neutral 2 report を順に送り、profile bytes と未知 reporting sentinel は両 mode の close 後も不変だった。Rust 1.87 all-feature は288 passed / 2 ignored、default は247 passed / 1 ignored、all/default build、all-feature check、stable 1.96.1 all/default clippy、all-feature rustdoc `-D warnings`、rustfmt、diff check が成功。追加で実行した非 gate の Rust 1.87 clippy は T08 外の既存 `src/adapter.rs` に `needless_borrows_for_generic_args` 8件を検出して停止した。refactor: memory/file key-store の観測 wrapper を共通化し、全 HID report の位置と user input 0x30 の位置を別 helper に分離した |

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
- [x] same Pro profile を Periodic/Direct で利用する
- [x] Direct idle で周期 user input report を送らない
- [x] Direct send failure で直前 snapshot を維持する
- [ ] power-cycle reconnect と Direct input を実機確認する
- [ ] key material、raw profile、secret が error、trace、evidence に残らない
- [ ] alpha.2 criteria、未実行条件、residual risk を記録する
- [ ] upstream PR を作成していない
- [ ] self-review と completion gate を通す
- [ ] `spec/complete/unit_007/` へ移動する
