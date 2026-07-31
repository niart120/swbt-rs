# M9 portability と 0.1.0 配布準備 仕様書

## 1. 概要

### 1.1 目的

Windows で実機確認した `swbt-rs` の利用条件と制限を公開文書へ移し、Linux の USB 所有権と
検証水準を明示する。公開 API、配布物、依存ライセンス、変更履歴、脆弱性報告先、release
手順を監査し、0.1.0 を公開できる条件と現在の停止条件を実行結果で固定する。

### 1.2 起点 / source

| source | 内容 | path |
|---|---|---|
| roadmap | M9 portability、release engineering、0.1.0 exit criteria | `spec/initial/roadmap.md` |
| quality gate | Cargo metadata と release 変更時の local gate | `spec/initial/QUALITY_GATES.md` |
| migration | Python backend へ戻すときの排他利用と復旧順序 | `spec/initial/migration-strategy.md` |
| source baseline | Python、Bumble、Rust の固定断面と依存固定方針 | `spec/initial/source-baseline.md` |
| M3 evidence | Windows build 規模と暫定 license inventory | `spec/complete/unit_004/evidence/package-windows-msvc-20260730.md` |
| M5-M8 evidence | Windows 11、CSR8510 A10、Switch 2 22.5.0 の実機結果 | `spec/complete/unit_006` から `spec/complete/unit_009` |

### 1.3 use case

| actor / boundary | 入力または状態 | 期待する観測結果 | 制約 |
|---|---|---|---|
| Windows 利用者 | 専用 CSR8510 A10 と `bumble` feature | driver の準備、adapter 選択、終了後の再利用手順を辿れる | 実機確認済み構成を支援対象とし、他構成へ一般化しない |
| Linux 利用者 | libusb と Bluetooth HCI USB adapter | udev、kernel driver detach/reattach、検証水準を確認できる | CI build は実機動作を証明しない |
| crate 利用者 | model alias と generic API | alias の model/reporting mode、error、feature 条件を rustdoc で確認できる | macOS は初期対象外 |
| release 担当者 | clean checkout と 0.1.0 candidate | package 内容、license/SBOM、gate、公開停止条件を再現できる | tag と publish は当該 turn の明示承認が必要 |

## 2. 対象範囲

- Windows の driver setup、claim/release/unplug、troubleshooting を公開文書へ記録する。
- Linux の libusb/udev、Bumble USB transport の自動 detach/reattach 所有権、支援水準を記録する。
- Windows と Linux の非実機 CI compile/test 境界を固定する。
- macOS を unsupported として明記する。
- public generic API、6 controller alias、model/button/wire mapping、examples を監査する。
- crate へ収録する file を明示し、spec、agent 設定、実機 trace、開発 tool を除外する。
- changelog、security policy、license/SBOM evidence、release runbook、backend rollback を整備する。
- `cargo package` と clean-install smoke を実行し、成功または再現可能な停止条件を記録する。
- release candidate commit と Bumble fork revision を記録できる checklist を作る。

## 3. 対象外

- `cargo publish`、production tag、GitHub Release、publish workflow の実行。
- Bumble upstream への issue または PR 作成。公開 fork への branch push も、この unit で必要な
  Bumble 変更が見つからない限り行わない。
- Bumble workspace の crate を crates.io へ公開すること、またはその namespace を取得すること。
- macOS USB transport と driver ownership の実装・実機確認。
- Linux adapter の実機確認。利用可能な Linux host と専用 adapter がないため、build/test と
  source inspection を hardware evidence と扱わない。
- 明示 local Bluetooth address。`spec/initial/roadmap.md` の独立 milestone に残す。
- M8 で観測した subscriber interval variation の性能修正。0.1.0 の制限として記録する。

## 4. 関連 docs

- `README.md`
- `spec/initial/api.md`
- `spec/initial/testing.md`
- `spec/initial/migration-strategy.md`
- `spec/initial/source-baseline.md`
- `spec/complete/unit_009/M8_IMU_DIAGNOSTICS_PROBE.md`

## 5. 振る舞い仕様

| 振る舞い | 入力・状態 | 期待結果 | 備考 |
|---|---|---|---|
| package file selection | clean 0.1.0 candidate | library source、public docs、examples、tests/fixtures、README、LICENSE、CHANGELOG、SECURITY のみを package 候補に含む | `spec/`、`.agents/`、`.codex/`、`.github/`、`tools/`、実機 trace は除外 |
| registry dependency validation | fixed Git Bumble dependencies | dependency に exact `0.1.0` 版要件を持たせる。registry に同版がない限り `cargo package` と publish を成功扱いにしない | Git source は package 時に registry source へ正規化される |
| Windows lifecycle guidance | WinUSB/libusbK 対応 driver と専用 adapter | open 中の排他所有、明示 close、unplug、元 backend へ戻す順序を実行可能な command とともに示す | OS 全体の Bluetooth adapter を対象にしない |
| Linux ownership guidance | kernel driver が HCI interface を所有 | fixed Bumble revision が `set_auto_detach_kernel_driver(true)` 後に claim し、handle Drop で release/reattach する境界を示す | udev permission は別に必要 |
| platform support label | Windows/Linux/macOS | Windows は限定実機確認、Linux は CI build-tested/実機未検証、macOS は unsupported と表示する | 未検証を supported と書かない |
| public API audit | Pro/Joy-Con L/Joy-Con R × Periodic/Direct | alias が対応する `Controller<M, R>` と feature/error 条件を rustdoc で確認でき、examples が compile する | model-invalid input は型で表現させない |
| release evidence | clean candidate | source revision、Bumble revision、Cargo.lock、license/SBOM、gate、hardware matrix、既知制限、rollback を一つの runbook から辿れる | secret/key/profile raw dataを含めない |
| release authorization | publish-ready candidate | tag/publish はその turn の明示承認がなければ停止する | `publish = false` を package 成功前に解除しない |

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| refactor-skipped | T01: 0.1.0 package 候補が配布対象だけを含み、registry 用 dependency metadata を持つ | regression | package | 120 files。開発用 root と実機 trace を除外し、8 Git dependency に `=0.1.0` を追加。manifest 構造の追加 refactor は不要 |
| refactor-done | T02: 6 alias の model/reporting 対応と公開 API 契約を rustdoc と compile 済み example から確認できる | characterization | public API / docs | 既存型契約と全 button wire mapping は green。alias rustdoc に reporting と side-specific input を追記 |
| refactor-done | T03: Windows 利用者が driver claim、close、unplug、backend rollback を手順どおり実施できる | new | docs | WinUSB 専用 adapter、排他 claim、明示 close、reopen、Python rollback を公開 docs に分離 |
| refactor-done | T04: Linux 利用者が udev permission と kernel driver ownership を区別でき、支援水準を誤認しない | new | docs / source audit | `TAG+="uaccess"` と fixed Bumble/libusb の自動 detach/release を記録。hardware は未検証と明記 |
| todo | T05: Windows と Linux の CI が all-feature compile/test を実行し、hardware 未検証を置き換えない | regression | CI | Windows job を追加し、Ubuntu job の意味を docs と整合させる |
| todo | T06: resolved dependency graph の license と SBOM inventory が生成され、未知 license と禁止 source を検出できる | new | package / release | tool 未導入時に検査を省略せず、導入方法か代替の再現 command を定義する |
| todo | T07: changelog、security policy、hardware matrix、known limitations、source baseline、release/rollback checklist を一続きに辿れる | new | docs / release | M8 timing variation と crates.io dependency blocker を含める |
| deferred | T08: clean package archive から default/all-feature target と examples を検証できる | new | package | `bumble-controller@0.1.0` を含む必要 crate が registry にない。`--no-verify` を成功根拠にしない |
| todo | T09: local gate と public docs review が変更範囲に対して成功し、未実行 hardware/publish を明記する | regression | quality gate | all/default/no-default、MSRV、doc、package、diff を記録する |

## 7. 設計メモ

### 7.1 現在確認済みの事実

- `Cargo.toml` は `publish = false` で、Bumble 8 crate を fork revision
  `b8c7cd625bc2ac2f58a4beb4ade1264426969819` に固定している。
- clean worktree の `cargo package --locked --list` は成功するが、`.agents/`、`spec/`、実機 traceを
  package 候補に含める。
- clean worktree の `cargo package --locked` は、Bumble dependency に version requirement が
  ないため packaging 前検査で停止する。
- 2026-08-01 に crates.io registry を指定して確認したところ、`bumble-transport@0.1.0` と
  `bumble-hci@0.1.0` は存在しない。`bumble@0.2.0` は Google Bumble の別実装であり、固定 fork
  workspace の `bumble@0.1.0` の代替ではない。
- fixed Bumble revision の `bumble-transport/src/usb.rs` は USB handle に
  `set_auto_detach_kernel_driver(true)` を設定してから interface を claim する。
- current CI は `ubuntu-latest` のみで、Windows build/test は local evidence だけである。

### 7.2 判断

- `publish = false` は維持する。必要な Bumble crate が registry に存在し、clean
  `cargo package --locked` と archive smoke が成功するまで解除しない。
- Git dependency には fork workspace と一致する exact version requirement を追加する。これは
  package 正規化に必要だが、未公開 dependency を公開済みに見せるものではない。
- package の `include` を allowlist とし、運用記録や hardware evidence が将来増えても crate に
  混入しない構成にする。
- Linux の detach/reattach code を `swbt-rs` に重複実装しない。Bumble transport が handle と
  interface lifecycle を所有するため、公開 docs と source revision 監査で境界を固定する。
- crates.io blocker の解消は Bumble crate 群の正規公開、または backend 配布境界の再設計を要する。
  fork branch push だけでは registry dependency を満たさない。

### 7.3 public API / model mapping audit

- 公開 controller surface は generic `Controller<M, R>` と6 aliasで過不足がない。新しい dynamic
  controller enum や Bumble 型の公開は不要である。
- `tests/controller_type_contract.rs` は6 aliasが3 modelと2 reporting modeの直積に一致することを
  compile 時に検査する。
- `src/model/tests.rs` は各 model の全 button を Python 基準 wire position と比較し、reserved bitを
  使用しないことを検査する。
- `examples/pro_profile_hardware.rs` と `examples/joycon_profile_hardware.rs` は6 aliasを実際の builder、
  input、close 経路で使用し、`cargo check --examples --all-features --locked` で compile する。
- alias rustdoc は Periodic の後続 tick と Direct の Ready/transport acceptance を区別し、Joy-Conは
  入力できる側だけを明記する。型 signature と error contract は変更しない。

## 8. 対象ファイル

| path | change | 内容 |
|---|---|---|
| `Cargo.toml` | modify | package metadata/include と Git dependency version requirement |
| `src/controller/mod.rs` | modify | 6 public alias の rustdoc |
| `tests/controller_type_contract.rs` | modify | alias/model/reporting の既存 compile/runtime 契約補強が必要な場合のみ |
| `.github/workflows/ci.yml` | modify | Windows compile/test gate |
| `README.md` | modify | platform support と公開 docs への入口 |
| `docs/platform-support.md` | new | Windows/Linux/macOS、driver、udev、hardware matrix、known limitations |
| `docs/troubleshooting.md` | new | claim/release/unplug と backend rollback |
| `CHANGELOG.md` | new | 0.1.0 の利用者向け変更履歴 |
| `SECURITY.md` | new | 脆弱性報告と秘密情報を含む報告の扱い |
| `deny.toml` | new | license/advisory/source policy |
| `spec/publishing.md` | new | 0.1.0 release runbook と停止条件 |
| `spec/wip/unit_010/evidence/` | new | package list、license/SBOM、source audit、gate の非秘密 evidence |
| `spec/wip/unit_010/M9_PORTABILITY_RELEASE.md` | new | 本作業仕様 |

## 9. 検証

| command | result | notes |
|---|---|---|
| `cargo package --locked --list` | success | 120 files。`.agents/`、`.codex/`、`.github/`、`spec/`、`tools/`、実機 trace を含まない |
| `cargo package --locked` | failed (tracked blocker) | manifest 検査と packaging 開始後、crates.io に `bumble-controller@0.1.0` がなく停止 |
| `cargo fmt --all --check` | not run | 実装後に実行 |
| `cargo check --all-targets --all-features --locked` | not run | 実装後に current Rust と MSRV 1.87 で実行 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | not run | 実装後に実行 |
| `cargo test --all-targets --all-features --locked` | not run | 実装後に実行 |
| `cargo test --all-targets --locked` | not run | default feature 利用を検査 |
| `cargo test --lib protocol:: --no-default-features --locked` | not run | Bumble-free 境界を検査 |
| `cargo build --all-features --locked` | not run | 実装後に実行 |
| `cargo build --no-default-features --locked` | not run | 実装後に実行 |
| `cargo test --test controller_type_contract --locked` | success | 2 passed。6 alias と builder の2型軸を検査 |
| `cargo test --lib model::tests --no-default-features --locked` | success | 3 passed。model metadata と全 button wire mapping を検査 |
| `cargo check --examples --all-features --locked` | success | public/hardware examples を compile |
| `cargo test --doc --all-features --locked` | success | 1 passed |
| `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked` | success | warning なし |
| `Get-Command Get-PnpDevice` | success | Windows driver 確認 command の入口を確認 |
| fixed Bumble `bumble-transport/src/usb.rs` source audit | success | auto detach、configuration、claim、alternate setting、handle ownership を確認 |
| Linux adapter hardware test | not run | 専用 Linux host/adapter がなく、CI と source audit で代替しない |
| `cargo deny check` | not run | tool availability と policy 作成後に実行 |
| `git diff --check` | not run | 各 cycle と全体 gate で実行 |

## 10. 先送り事項

- crates.io publish: Bumble fork の必要 crate が registry に存在せず、`cargo package` を検証できない。
  `spec/publishing.md` に registry dependency gate と再開条件を置く。
- Linux hardware: 専用 Linux host/adapter を用いた pair/reconnect/close/reattach は未実行。
  `docs/platform-support.md` で build-tested と hardware-verified を分け、後続 evidence の完了条件を置く。
- macOS: roadmap どおり unsupported。USB transport と driver ownership の調査を別 unit とする。
- local Bluetooth address: roadmap の explicit local address milestone に残す。

## 11. チェックリスト

- [x] 対象範囲と対象外を確認した
- [x] TDD Test List を作成した
- [ ] TDD Test List の各 item を更新した
- [ ] Windows/Linux/macOS の支援水準を公開文書へ反映した
- [ ] public API、alias、model/mapping、examples を監査した
- [ ] package file list、license/SBOM、security、changelog を検査した
- [ ] release/rollback runbook と blocker を検査した
- [ ] 検証結果または未実行理由を記録した
- [x] package / release / public API に触れる gate を記録した
