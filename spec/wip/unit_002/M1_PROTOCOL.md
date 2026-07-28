# M1 model-valid input と pure protocol parity 仕様書

## 1. 概要

### 1.1 目的

Python 基準 commit `84d2723b127f70fc78e12f4496f5c40af0ccfb0a` の
model-valid input と NX HID protocol を、filesystem、thread、Bumble に依存しない
Rust の純粋な protocol component へ移植する。M2 の worker と送信処理は、この作業単位が
返す report bytes、effect、candidate next state を transport 受理後に commit する。

### 1.2 起点 / source

| source | 内容 | path |
|---|---|---|
| user request | roadmap 順に移植し、TDD 項目単位で commit する | 対話上の依頼 |
| roadmap | M1 model-valid input / pure protocol と exit criteria | `spec/initial/roadmap.md` |
| architecture | crate-private protocol、typed state、送信受理前の state 境界 | `spec/initial/architecture.md` |
| test policy | fixture provenance、golden、malformed、Miri | `spec/initial/testing.md` |
| migration | Python と Rust の pure protocol shadow | `spec/initial/migration-strategy.md` |
| source baseline | Python `84d2723b` と Bumble `bbac2a68` | `spec/initial/source-baseline.md` |
| completed unit | M0 の model/input/mapping/type foundation | `spec/complete/unit_001/M0_FOUNDATION.md` |

### 1.3 Intent Delta

| 区分 | M0 完了時 | M1 完了時 |
|---|---|---|
| public value | typed button/stick/raw IMU/state | `Rgb24`、`ControllerColors`、IMU 物理値変換を追加 |
| model data | kind/profile/button/stick/mapping | device info、SPI、IMU 校正と mode、model default colors を追加 |
| protocol | 未実装 | `0x30`、`0x01` / `0x10`、`0x21`、virtual SPI、session、IMU encoder |
| parity evidence | model mapping を Python と手動照合 | Python 固定 SHA から生成した commit 済み fixture を Rust test が消費 |
| dependency boundary | Bumble core が常時 direct dependency | default-off `bumble` feature に隔離し、pure protocol graph から除外 |

### 1.4 use case

| actor / boundary | 入力または状態 | 期待する観測結果 | 制約 |
|---|---|---|---|
| M2 ReportSender | `InputState<M>`、timer、session IMU state、時刻 | 49-byte `0x30` と candidate next state | transport 受理前に state を変更しない |
| M2 output handler | raw `0x01` / `0x10` | typed parse result、必要なら 50-byte `0x21` と effect | malformed packet で panic しない |
| subcommand responder | model、typed input、session、request | model-specific ACK/data/prefix | `0x03/30/40/48` は candidate state だけを返す |
| fixture maintainer | clean な Python 固定 checkout | deterministic fixture と provenance | 通常の Rust test は Python を起動しない |
| protocol test | default feature graph | pure protocol unit/golden test | Bumble crate を build/link 対象に含めない |

## 2. 対象範囲

- `Rgb24` と `ControllerColors`
- `ImuFrame` の `0.070 dps/raw`、`1/4096 G/raw` 変換
- model 宣言から得る device type、device info tail、default colors、pairing trigger、
  accepted IMU modes、sensor calibration
- read-only virtual SPI
- 49-byte standard full input report `0x30`
- disabled、standard、quaternion `0x02..=0x05` の 36-byte IMU encoder
- output report `0x01` / `0x10` parser と raw rumble 保持
- connection-scoped protocol session と readiness projection
- subcommand `0x02/03/04/08/10/21/30/40/48` の `0x21` reply
- crate-private `SwitchHidProtocol<M>` facade
- Python fixture generator、source audit、model/semantic input/expected bytes
- malformed corpus、byte-for-byte golden、selected Miri
- default-off `bumble` feature と no-default-features protocol gate

## 3. 対象外

- worker、`ReportSender`、transport acceptance、retry、holdoff、scheduler
- filesystem、pairing profile JSON、key store
- USB、HCI、L2CAP、HIDP framing、SDP、Bumble runtime
- pairing、reconnect、実機、adapter-only、network
- diagnostics、probe、CLI
- subcommand `0x22` と NFC / IR semantic state
- high-level rumble generation
- HID report descriptor と SDP record construction
- protocol module の公開
- IMU の long-run、runtime ACK ordering、hardware parity

## 4. 関連 docs

- `spec/initial/roadmap.md`
- `spec/initial/architecture.md`
- `spec/initial/testing.md`
- `spec/initial/migration-strategy.md`
- `spec/initial/type-modeling.md`
- `spec/initial/source-baseline.md`
- `spec/initial/QUALITY_GATES.md`

## 5. 振る舞い仕様

| 振る舞い | 入力・状態 | 期待結果 | 備考 |
|---|---|---|---|
| fixture generation | Python checkout | HEAD が固定 SHA、worktree clean、Python 3.13 以上の場合だけ生成 | timestamp と絶対 path を出力しない |
| RGB value | `0x000000..=0xFFFFFF` | 24-bit value と RGB big-endian bytes を保持 | 範囲外は `InvalidInput` |
| colors | body/buttons/left/right | SPI `0x6050` の 12 bytes | model default と明示 override を区別 |
| physical IMU | finite f64 または raw i16 | Python と同じ ties-to-even と scale | non-finite / overflow は `InvalidInput` |
| virtual SPI | address、size、model/colors | seeded byte または `0xFF` | max `0x1D`、address end `0x80000` |
| input report | typed state、timer、IMU block | `[u8; 49]`、ID `0x30`、model-specific button/stick | unavailable stick side は neutral |
| IMU encoding | mode、3 frames、前 state、`now_ns` | 36 bytes と candidate next state | 時計逆行は elapsed 0 |
| parser `0x01` | 11 bytes 以上 | packet、8-byte rumble、subcommand、残り payload | short は protocol error |
| parser `0x10` | 10 bytes 以上 | packet、8-byte rumble、reply なし | 10 bytes より後は無視 |
| session readiness | report mode と player lights | mode `0x30` supported かつ lights 非 0 の場合だけ ready | unsupported mode を丸めない |
| reply envelope | subcommand、typed state | `[u8; 50]`、ACK 13、ID 14、data 15.. | prefix は現在の state から作る |
| state effect | `0x03/30/40/48` | reply と candidate next session | caller の accept 前は current session 不変 |
| facade | raw output と current state | typed parse、optional prepared reply | filesystem/thread/I/O を行わない |

### 5.1 model policy

| model | device type | device info tail | pairing trigger | default colors |
|---|---:|---|---|---|
| Pro | `0x03` | `03 02` | L / R | `323232 ffffff 00b2ff ff3b30` |
| JoyConL | `0x01` | `01 01` | SL / SR | `00b2ff 323232 00b2ff 00b2ff` |
| JoyConR | `0x02` | `01 01` | SL / SR | `ff3b30 323232 ff3b30 ff3b30` |

3 model とも IMU mode `0x00..=0x05` を受理する。Pro の tail `03 02` と
quaternion mode の実機互換範囲は Python 基準断面の hardware observation を移植するもので、
全 firmware の一般保証とはしない。

### 5.2 subcommand policy

| ID | ACK | data / effect |
|---|---:|---|
| `0x02` | `0x82` | firmware、device type、marker、caller-supplied BD_ADDR、tail |
| `0x03` | `0x80` | requested report mode と supported 判定 |
| `0x04` | `0x83` | L/R/ZL/ZR/SL/SR/HOME の elapsed ticks |
| `0x08` | `0x80` | data なし |
| `0x10` | `0x90` | request prefix 5 bytes と SPI read |
| `0x21` | `0xA0` | Python 基準の 34-byte MCU config |
| `0x30` | `0x80` | player lights |
| `0x40` | `0x80` | IMU mode と encoding epoch reset |
| `0x48` | `0x80` | vibration enabled |

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| refactor-skipped | 固定 SHA 以外から fixture を生成せず、全 fixture が repository、commit、generator、model、semantic input、expected result を持つ | characterization | fixture | red: fixture 不在で compile 失敗。green: 45 cases、audit 2 passed。再生成前後の SHA-256 一致。追加の構造変更なし |
| refactor-done | `Rgb24` が 24-bit 境界を保証し、`ControllerColors` が 4 色を RGB 順の 12 bytes にする | new | public value | red: root import 不在。green: 3 passed。SPI byte 検査を公開 contract test へ統合し、重複 unit test を除去 |
| refactor-skipped | IMU の `0.070 dps/raw` と `1/4096 G/raw` が ties-to-even、finite、i16 overflow 契約を守る | new | public value | red: physical constructor/method 不在。green: 7 passed、MSRV pass。変換 helper は小さく追加の構造変更なし |
| refactor-skipped | 各 `M::SPEC` が device info、default colors、校正、IMU modes、pairing trigger を一意に投影する | new | model | red: `ModelSpec::protocol` 不在。green: 3 model の metadata projection 1 passed、MSRV pass。model macro 内の単一宣言として実装済みで追加の構造変更なし |
| refactor-done | virtual SPI が model seed、custom colors、erased range、最大長、境界 error を再現する | new | protocol unit | red: SPI 型不在。green: 3 passed、MSRV pass。固定長 29-byte read と疎な seed 投影にし、SPI テストを専用 module へ分離 |
| refactor-skipped | neutral state が決定的な 49-byte `0x30` と candidate next timer を生成し、timer が wrap する | new | protocol unit | red: neutral encoder 不在。green: 3 model と `0xFF -> 0x00` を含む 2 passed、MSRV pass。専用 module 内の小さい固定長変換で追加の構造変更なし |
| refactor-done | 3 model の全 supported button と stick が Python fixture と一致し、SL/SR と unavailable side を誤配置しない | characterization | protocol unit | red: generic encoder 不在。green: 全 model button fixture と Pro/Joy-Con stick を含む 4 passed、MSRV pass。neutral encoder を generic encoder へ委譲し、wire stick accessor は crate-private |
| refactor-skipped | disabled / standard IMU が zero block、3 raw frames、順序、candidate next state を生成する | new | protocol unit | red: IMU state/mode/encoder 不在。green: zero reset と3 frame signed LE fixture の 2 passed、MSRV pass。固定長変換は小さく追加の構造変更なし |
| todo | quaternion modes が identity、正負回転、3 sample、時刻逆行、reset candidate を Python と一致させる | characterization | protocol unit | mode `0x02..=0x05` |
| todo | parser が `0x01` / `0x10` の field を保持し、empty、unknown、truncated、arbitrary bytes で panic しない | new | protocol unit | raw rumble を解釈しない |
| todo | session が report mode、lights、IMU、vibration、readiness、unsupported mode を接続単位で保持する | new | protocol unit | lifecycle/session ID は M2 |
| todo | `0x21` envelope と `0x02/04/08/21` が model 別 bytes と typed state prefix を生成する | characterization | protocol unit | state を変更しない command |
| todo | `0x10` SPI reply が request prefix と requested data を返し、session を変更しない | new | protocol unit | payload 5 bytes 未満は error |
| todo | `0x03/30/40/48` が valid candidate state を返し、invalid payloadでは current state を保つ | new | protocol unit | accept 前に commit しない |
| todo | `SwitchHidProtocol<M>` が `0x01` を reply/effectへ、`0x10` を raw rumble/no-replyへ合成し、I/Oを行わない | new | protocol unit | protocol facade |
| todo | 全 committed fixture が Rust assertion に消費され、protocol test が Bumble を含まない graph と selected Miri で通る | characterization | package | coverage audit と isolation gate |

## 7. 設計メモ

- `protocol` は `mod protocol;` として crate-private にする。
- parser は borrowed payload と固定長 rumble を使い、不要な allocation と nullable DTO を避ける。
- state-changing reply は `PreparedReply { bytes, next_session }` を返す。M2 が送信受理後に
  next state を commit する。M1 は transport 受理を模擬しない。
- input encoding は explicit timer、session IMU state、時刻から bytes と next encoding state を返す。
- `button_wire_position(M::KIND, ButtonKind)` だけを wire mapping の入口にする。
- Bumble は optional dependency とし、default feature graph から外す。`--all-features` gate で
  pinned dependency の build も維持する。
- M1 は pure quaternion encoder parity まで実装する。M8 は runtime ACK ordering、long-run、
  diagnostics、model 別実機検証を担当する。
- M1 は model-specific pure fixture を持つ。M7 は Joy-Con の runtime、virtual link、実機統合を担当する。

## 8. 対象ファイル

| path | change | 内容 |
|---|---|---|
| `Cargo.toml` / `Cargo.lock` | modify | fixture test dependency、default-off Bumble feature |
| `src/lib.rs` | modify | public value re-export と private protocol module |
| `src/input/imu.rs` | modify | physical conversion |
| `src/model/` | modify | protocol metadata の単一宣言 |
| `src/profile/` | modify | `Rgb24` / `ControllerColors` |
| `src/protocol/` | new | parser、encoder、session、SPI、subcommand、facade |
| `tools/generate_python_fixtures.py` | new | fixed-baseline fixture generator |
| `tests/fixtures/python-v0.6.0/protocol/` | new | provenance と golden fixture |
| `tests/protocol_fixture_audit.rs` | new | fixture schema / coverage audit |
| `.github/workflows/ci.yml` | modify | no-default-features protocol gate |
| `README.md` | modify | M1 の現在面と feature/gate |
| `spec/dev-journal.md` | modify | M2 接続時に解除する一時的な dead-code 境界 |
| `spec/wip/unit_002/M1_PROTOCOL.md` | new / modify | 作業仕様と検証記録 |

## 9. 検証

| command | result | notes |
|---|---|---|
| `cargo test --test protocol_fixture_audit` | pass | red は fixture file 不在。green は provenance / case schema の 2 passed |
| Python 3.13 generator の連続実行と SHA-256 比較 | pass | 45 cases。固定 checkout、clean、Bumble 非 import を generator 内でも検査 |
| `cargo test --test colors_contract` | pass | red は `Rgb24` / `ControllerColors` root import 不在。green / refactor 後は 3 passed |
| `cargo +1.87 test --test imu_contract --locked` | pass | red は physical conversion API 不在。green は raw/scale/rounding/error/preservation を含む 7 passed |
| `cargo +1.87 test --locked model::tests::protocol_metadata_is_projected_from_each_model_declaration` | pass | red は `ModelSpec::protocol` 不在。green は model 別 device info、色、校正、IMU modes、pairing trigger を検査する 1 passed |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass | TDD item 4 時点。protocol metadata は crate-private のまま警告なし |
| `cargo +1.87 test --locked protocol::tests::spi` | pass | red は `VirtualSpiFlash` / `ProtocolError` 不在。green / refactor 後は model seed、override、最大長、end-exclusive、overflow の 3 passed |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | pass | TDD item 5 時点。通常 build と unit test build の双方で警告なし |
| `cargo +1.87 test --locked protocol::tests::input_report` | pass | red は `encode_neutral_0x30` 不在。green は全 model の 49 bytes、candidate timer、wrap、共有 mutation 不在の 2 passed |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | pass | TDD item 6 時点。通常 build と unit test build の双方で警告なし |
| `cargo +1.87 test --locked --lib protocol::tests::input_report` | pass | red は `encode_0x30` 不在。green / refactor 後は全 model button、SL/SR、custom stick、Joy-Con unavailable side を含む 4 passed |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | pass | TDD item 7 時点。公開 API 追加なし、通常 build と unit test build の双方で警告なし |
| `cargo +1.87 test --locked --lib protocol::tests::imu` | pass | red は IMU state / mode / encoder 不在。green は disabled zero/reset と standard 3-frame signed LE、完全な `0x30` fixture の 2 passed |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | pass | TDD item 8 時点。quaternion は未実装のまま、通常 build と unit test build の双方で警告なし |
| TDD item commands | not run | 各 item の red / green / refactor を追記する |
| `cargo fmt --all --check` | not run | final gate |
| `cargo +1.87 check --all-targets --all-features --locked` | not run | MSRV |
| `cargo check --all-targets --all-features --locked` | not run | stable |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | not run | static gate |
| `cargo test --all-targets --all-features --locked` | pass | TDD item 8 時点で unit 12、integration 28、example 0 passed。final gate で再実行する |
| `cargo test --lib protocol:: --no-default-features --locked` | not run | Bumble-free protocol |
| `cargo tree --no-default-features --edges normal` | not run | Bumble 不在を検査 |
| `cargo +nightly miri test --lib protocol::` | not run | selected Miri |
| `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked` | not run | public docs |
| `cargo build --locked` / `cargo build --all-features --locked` | not run | feature 組合せ |
| `cargo package` | not run | M0 から継続する既知の release blocker を再確認 |
| `git diff --check` | not run | whitespace |
| GitHub required checks | not run | PR 作成後 |

## 10. 先送り事項

- worker、ReportSender、send acceptance、rollback と ACK ordering は M2。
- Bumble transport、HIDP、SDP、virtual link は M3 以降。
- Joy-Con runtime / hardware integration は M7。
- quaternion long-run、diagnostics、probe、hardware evidence は M8。
- Bumble git dependency の package 公開問題は M9。

## 11. チェックリスト

- [x] 対象範囲と対象外を確認した
- [ ] TDD Test List をすべて `refactor-done` または `refactor-skipped` にした
- [ ] Python fixture の provenance と coverage を確認した
- [ ] 検証結果または未実行理由を記録した
- [ ] public API / feature / package gate を記録した
