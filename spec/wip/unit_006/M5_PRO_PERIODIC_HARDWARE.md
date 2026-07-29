# M5 Pro Periodic fresh pairing

- 状態: **着手中**
- milestone: M5
- branch: `feat/unit-006-m5-pro-periodic-hardware`
- 正本:
  - `spec/initial/roadmap.md` 8
  - `spec/initial/architecture.md` 15、16、18、19
  - `spec/initial/testing.md` 6、12
- Python 基準断面:
  - repository: `niart120/swbt-python`
  - revision: `84d2723b127f70fc78e12f4496f5c40af0ccfb0a`
- Bumble fork:
  - repository: `https://github.com/niart120/bumble-rs`
  - branch: `fix/external-host-reader-lifecycle`
  - revision: `48f1bc36169b2692d2a61e87eda4223b126dca2b`
  - public fork と branch push だけを許可範囲とし、upstream PR は作成しない

## 1. 目的

Windows 11、CSR8510 A10、WinUSB、Switch 2、`ProController`、Periodic reporting の
組合せで、public `create_profile()` から fresh pairing と NX readiness へ到達し、型付き入力、
neutral、明示 close を実機で確認する。

M5 は M4 で検査した Classic SDP/HID packet path を production USB runtime へ接続する最初の
Switch 実機 milestone である。local transport acceptance、HCI/packet trace、Switch UI の観測を
別の証拠として記録し、相互に代用しない。

## 2. Intent Delta

| 境界 | M4 完了時 | M5 完了後 | 保証 |
|---|---|---|---|
| public `create_profile()` | feature 有効時も backend unavailable | valid target へ empty envelope を create-new し、USB open、fresh pair、Ready を一つの所有権で実行 | target 検査と backend availability 後、USB open より前に valid profile が存在する |
| production pair driver | create-profile 経路は pair enqueue 後に unsupported | worker が transport pairing を開始し、外部注入なしで readiness を待つ | production hook は command を上書きせず、test orchestration と分離する |
| file create-new | target の read-only inspection だけ | same-directory temporary file を complete write / flush / sync 後に no-replace で公開 | existing file、directory、symlink、race を置換しない |
|実機証拠| adapter lifecycle と virtual HID | fresh SSP、HID readiness、Periodic input、neutral、close/reopen | report acceptance と Switch UI 反映を分けて記録する |
| profile contents | schema v2 empty envelope | pairing 失敗時も valid empty envelope。M5 success 後の再利用可能 key persistence は保証しない | Python lossless compatibility、atomic update、stored-key reconnect は M6 |

## 3. 現物監査

### 3.1 確認済み

- `create::create_profile` は capability check、empty envelope create-new、typed reopen、
  transport open、pair-to-ready、Ready controller transfer の順序を crate-private test で固定済み。
- `FileProfileCreateTarget` は `symlink_metadata` による存在検査だけを実装し、production
  `ProfileCreatePort` は存在しない。
- `ConcreteRuntimeBackend` と `ConcreteRuntimeAttempt` は partial open、pair failure、
  terminal worker、cleanup、ownership transfer を決定的 test で検査済み。
- public `ControllerBuilder::create_profile()` は production backend を構築せず、
  feature 有効時も `reject_unavailable_backend` へ進む。
- `open_bumble_runtime` は production `BumbleTransportPort` を構築するが、create-profile 用
  `PairDriver` は未接続である。
- M4 の production Bumble port は pairing window、SDP、HID control/interrupt、
  interrupt send/drain/disconnect/close を所有する。

### 3.2 証拠境界

hardware runner が成功として機械判定できるのは次だけである。

- public `create_profile()` が Ready controller を返した
- lifecycle、report mode、player lights、accepted counter が readiness と整合する
- A、L+R、dual sticks、IMU を含む typed state command が成功した
- neutral command と explicit close が成功した
- profile file が valid JSON / typed Pro envelope として読める
- process が timeout 内に終了し、次 run で adapter を再度 open できる

Switch UI 上の A、L+R、stick 反映と neutral 残存なしは人が観測する。runner の command 成功を
UI 成功へ読み替えない。実機 metadata は test 開始前に再取得し、roadmap 記載の firmware
`22.1.0` を現在値として仮定しない。

## 4. 対象範囲

- production file profile create-new
- `bumble` feature の public `create_profile()` backend
- create-profile production pair driver
- feature-disabled と preflight error の既存契約維持
- invalid selector を使う USB side-effect なしの production failure test
- Pro Periodic hardware runner と secret-free evidence
- fresh pairing 1 run の protocol/readiness 証拠
- A、L+R 500 ms、dual sticks、IMU、neutral、close の入力 sequence
- fresh pairing 20 run の成功率、hang、leak、stale input、profile validity
- alpha.1 note draft

## 5. 対象外

- upstream PR / issue 作成
- explicit local Bluetooth address
- Python profile lossless compatibility
- pairing key の filesystem 永続化と atomic update
- stored-key reconnect、power-cycle reconnect、invalid bond recovery
- Direct reporting
- Joy-Con L/R 実機
- Switch UI の自動画像認識
- long-run jitter、stable diagnostics schema、probe CLI
- Linux、macOS、cross compile、release publish

## 6. 振る舞い仕様

### 6.1 production file create-new

`FileProfileStore::create_new(path, bytes)` は次の順序を守る。

1. parent directory を作成する
2. target と同じ directory に衝突しない temporary file を create-new する
3. complete bytes を write、flush、`sync_all` する
4. target が存在しない場合だけ temporary file の内容を一度に公開する
5. temporary file を削除する
6. target が race で作成済みなら `AlreadyExists` を返し、相手の内容を変更しない

成功後の target bytes は `ProfileDocument::parse_json` と
`PairingProfile<model::Pro>::try_from` を通る。途中失敗を成功に変換しない。

### 6.2 public create-profile

処理順:

```text
builder validation
target inspection
backend availability
empty profile create-new
typed reopen
USB transport open
pair command enqueue
production pair continuation
same-session NX Ready
return ProController
```

`bumble` feature 無効時は target を作らず `UnsupportedCapability` を返す。feature 有効時は
syntactically invalid USB selector でも backend 自体は利用可能なので、valid empty profile を
作成した後、USB access 前に `TransportOpen` を返す。これを profile-before-open の production
回帰 test に使う。

open、pair、readiness の失敗では runtime cleanup を続行し、empty profile を削除しない。
existing target は一切置換しない。

### 6.3 hardware runner

runner は `adapter selector`、`profile path`、`pair timeout`、`run index` を明示入力として受ける。
固定の production sequence:

1. profile path が absent であることを確認
2. public `create_profile()` を開始
3. Ready 後の status と neutral snapshot を記録
4. A を 500 ms
5. L+R を 500 ms
6. left/right stick を独立に4方向へ各500 ms
7. non-neutral IMU samples を1秒、続けて neutral IMU
8. explicit `neutral()`
9. status と snapshot を記録
10. `close()`、process 終了
11. profile document の shape と key 値を含まない要約を記録

runner は link key、LTK、IRK、CSRK、raw profile JSON、USB serial を標準出力へ出さない。
protocol trace は HCI opcode、event kind、PSM、HID report ID、subcommand ID、session transition、
accepted counter に限定する。

### 6.4 20 run

各 run は新しい profile path を使う。Switch 側の fresh pairing 画面準備と UI 観測は run ごとに
人が行う。次を表形式で記録する。

- run index
- observed OS、dongle VID/PID、driver、console firmware
- pair start / Ready / close の時刻と所要時間
- readiness status
- typed command 結果
- A、L+R、stick、neutral の UI 観測
- profile validity
- hang、worker/reader terminal、cleanup failure の有無

20 run 中に失敗した場合は成功率へ含め、原因を隠して再実行分へ置き換えない。修正が必要なら
失敗 run の evidence を残し、同じ TDD item 内で再現 test を先に追加する。

## 7. TDD Test List

- [ ] **T01 — production profile create-new**
  - real filesystem で parent create、exact bytes、typed reopen を検査する。
  - existing file/directory/symlink と create-new race を置換しない。
  - temporary file が成功後と既知 failure 後に残らない。
- [ ] **T02 — production create-profile wiring**
  - `bumble` feature で public `create_profile()` を concrete backend へ接続する。
  - production pair continuation を no-op hook とし、worker の pairing command を駆動する。
  - feature-disabled backend unavailable は target を作らない。
- [ ] **T03 — production failure ordering**
  - invalid USB selector で valid empty profile が open failure より先に残る。
  - typed `TransportOpen` source、cleanup、secret redaction を検査する。
  - existing target と dangling symlink の no-replace 契約を feature 有効時にも維持する。
- [ ] **T04 — Pro Periodic hardware runner**
  - public API だけで pairing、status、A、L+R、sticks、IMU、neutral、close を実行する。
  - output schema と timeout を固定し、key material と raw profile を出力しない。
- [ ] **T05 — single fresh pairing**
  - current hardware metadata と Switch firmware を記録する。
  - fresh SSP から same-session NX Ready までの trace と所要時間を記録する。
  - 失敗時も valid empty profile と adapter reopen を確認する。
- [ ] **T06 — input reflection and cleanup**
  - A UI 反映、L+R 500 ms、dual sticks を人の観測として記録する。
  - typed IMU command、neutral snapshot、trailing neutral、close/drain、再open を機械観測する。
- [ ] **T07 — 20-run clean pairing**
  - 20 run の成功率と各 run の所要時間を保存する。
  - hang、leak、stale input、neutral 残存を0件にする。
  - failure を除外せず、必要な再現 test と修正を同じ evidence に結ぶ。
- [ ] **T08 — completion gate and alpha.1 note**
  - Rust 1.87、all/default/no-default、clippy、test、build、rustdoc、fmt、diff check を通す。
  - alpha.1 note draft、未実行条件、residual risk、upstream PR 未作成を記録する。

### 7.1 TDD cycle evidence

| phase | item | evidence |
|---|---|---|
| pending | T01 | red/green/refactor 後に追記する |
| pending | T02 | red/green/refactor 後に追記する |
| pending | T03 | red/green/refactor 後に追記する |
| pending | T04 | red/green/refactor 後に追記する |
| pending | T05 | 実機実行後に追記する |
| pending | T06 | 実機実行と UI 観測後に追記する |
| pending | T07 | 20 run 後に追記する |
| pending | T08 | completion gate と self-review 後に追記する |

## 8. 対象ファイル

- `README.md`
- `src/lib.rs`
- `src/controller/`
- `src/profile/`
- `src/runtime/`
- `tests/`
- `examples/`
- `spec/wip/unit_006/`

TDD cycle で必要性を確認したファイルだけを変更する。profile key compatibility と reconnect の
実装を M5 へ混ぜない。

## 9. 検証

通常 gate:

```powershell
cargo +1.87.0 check --all-targets --all-features --locked
cargo +1.87.0 test --all-targets --all-features --locked
cargo test --all-targets --locked
cargo test --all-targets --no-default-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo clippy --all-targets --locked -- -D warnings
cargo +1.87.0 build --all-features --locked
cargo +1.87.0 build --locked
cargo +1.87.0 build --no-default-features --locked
$env:RUSTDOCFLAGS="-D warnings"
cargo +1.87.0 doc --all-features --no-deps --locked
cargo fmt --all -- --check
git diff --check
```

hardware command は T04 で確定し、selector と profile path を明示する。実機実行前に
`list_adapters()`、Windows PnP、driver、Switch firmware、pairing 画面を確認する。

## 10. 先送り事項

- filesystem pairing key persistence、Python compatibility、atomic update、stored-key reconnect:
  M6
- Pro Direct と Periodic profile reuse: M6
- Joy-Con L/R hardware: M7
- stable diagnostics schema、long-run timing、probe: M8
- Linux、macOS、license/SBOM、release publish: M9
- explicit local address: 独立 milestone

## 11. 完了チェックリスト

- [ ] T01-T08 がすべて完了している
- [ ] public create-profile が empty envelope を USB open より先に保存する
- [ ] feature-disabled と existing target の no-side-effect 契約を維持した
- [ ] pairing 失敗後も valid typed Pro profile が残る
- [ ] production pair が unsupported hook で停止しない
- [ ] single fresh pairing が same-session NX Ready へ到達した
- [ ] A、L+R 500 ms、dual sticks の UI 反映を観測した
- [ ] IMU command、neutral、drain、close、再open を検査した
- [ ] 20 run の成功率と failure を除外せず記録した
- [ ] 20 run で hang、leak、stale input、neutral 残存が0件だった
- [ ] hardware metadata と current Switch firmware を記録した
- [ ] report acceptance と Switch UI 反映を別 evidence として記録した
- [ ] alpha.1 note draft を作成した
- [ ] upstream PR を作成していない
- [ ] placeholder、未根拠の完了表現、secret を含む evidence が残っていない
- [ ] self-review で未実行条件と residual risk を明記した
- [ ] `spec/complete/unit_006/` へ移動した
