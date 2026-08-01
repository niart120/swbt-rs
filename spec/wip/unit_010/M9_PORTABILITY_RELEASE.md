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
- fork 元 `chaitanyarahalkar/bumble-rs` への issue または PR 作成。backend配布境界は自己所有 fork の
  Issue #1で追跡する。
- Bumble workspace の crate を crates.io へ公開すること、またはその namespace を取得すること。
- macOS USB transport と driver ownership の実装・実機確認。
- Linux adapter の実機確認。利用可能な Linux host と専用 adapter がないため、build/test と
  source inspection を hardware evidence と扱わない。
- 明示 local Bluetooth address。この unit の対象外として独立 unit_011 で完了した。
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
| refactor-skipped | T01: 0.1.0 package 候補が配布対象だけを含み、registry 用 dependency metadata を持つ | regression | package | unit_010 candidate は 124 files。開発用 root と実機 trace を除外し、8 Git dependency に `=0.1.0` を追加。manifest 構造の追加 refactor は不要 |
| refactor-done | T02: 6 alias の model/reporting 対応と公開 API 契約を rustdoc と compile 済み example から確認できる | characterization | public API / docs | 既存型契約と全 button wire mapping は green。alias rustdoc に reporting と side-specific input を追記 |
| refactor-done | T03: Windows 利用者が driver claim、close、unplug、backend rollback を手順どおり実施できる | new | docs | WinUSB 専用 adapter、排他 claim、明示 close、reopen、Python rollback を公開 docs に分離 |
| refactor-done | T04: Linux 利用者が udev permission と kernel driver ownership を区別でき、支援水準を誤認しない | new | docs / source audit | `TAG+="uaccess"` と fixed Bumble/libusb の自動 detach/release を記録。hardware は未検証と明記 |
| refactor-skipped | T05: Windows と Linux の CI が all-feature compile/test を実行し、hardware 未検証を置き換えない | regression | CI | `windows-latest` の check/test と既存 Linux jobが PR #11 run `30649158739` で成功。CI 結果を hardware evidence に読み替えず、追加 refactor は不要 |
| refactor-done | T06: resolved dependency graph の license と SBOM inventory が生成され、未知 license と禁止 source を検出できる | new | package / release | cargo-deny policy と Windows/Linux CycloneDX 1.5 SBOM を追加。CI job でも検査 |
| refactor-done | T07: changelog、security policy、hardware matrix、known limitations、source baseline、release/rollback checklist を一続きに辿れる | new | docs / release | 未公開 candidate と明記し、M8 timing variation、registry 名衝突、private vulnerability reporting 未設定を停止条件にした |
| refactor-skipped | T08: clean package archive から default/all-feature target と examples を検証できる | new | package | unit_012で公開済み`swbt-bumble-backend@0.1.1`へ更新し、registryだけを使うarchive verification buildと展開archiveのoffline/all-feature testが成功。検査後の構造変更は不要 |
| refactor-skipped | T09: local gate と public docs review が変更範囲に対して成功し、未実行 hardware/publish を明記する | regression | quality gate | all/default/no-default、MSRV、doc、dependency policy、diff、archive gateがgreen。unit_012のWindows実機回帰は完了し、remote CIはmerge gate、`swbt-rs` publishは別承認事項として残した。検査後の構造変更は不要 |
| refactor-done | T10: Bumble session 統合 test が reader thread の packet 分割順序に依存せず公開 transport event を検査する | regression | test harness / CI | PR #11 run `30649447099` で `CommandStatus` 後に空で返る red を記録。残り期限内の再 poll に変更し、対象100回と全 library testが green |
| refactor-done | T11: registry backend 0.1.1 の現行 dependency graph、license、Windows/Linux SBOM、package archive を再生成できる | regression | package / release | red: 旧evidenceはGit Bumble 22 componentsとWindows 220 / Linux 222 dependenciesを記録し、現行graphと不一致。green: registry sourceだけでcargo-deny、120-file archive、MSRV offline test、33 / 34 component SBOM、license欠落0が成功。refactor: 旧Git allowlistと未使用license例外を削除し、SBOM local path正規化とreference検査をtool化 |
| pending | T12: 非公開脆弱性報告先が実際に有効で、`SECURITY.md` からその入口へ到達できる | new | repository security / docs | GitHub API の有効値と実在URLを確認し、public issueへ秘密情報を出さない契約を維持する |
| pending | T13: workload soak と Python backend への切戻し訓練が、adapter排他・profile非破壊・neutral終了を満たす | acceptance | operational cutover / hardware | unit_012 の連続60秒runをsoakとして評価し、Rust終了後のadapter再利用、Python再接続、入力、neutral closeを新しい非秘密 evidenceへ記録する |

## 7. 設計メモ

### 7.1 unit_010時点とunit_012 T07で確認済みの事実

- unit_010時点の`Cargo.toml`はBumble 8 direct dependencyを元の`bumble*` package名のまま
  自己所有 fork revision `cb55e2d98dc7b7b0227c43772c9ae184034dd9a1` に固定していた。
- unit_010時点のclean `cargo package --locked` は、crates.io に `bumble-controller@0.1.0` が
  ないため registry 解決で停止した。
- unit_012 T07では初回公開版`swbt-bumble-backend = "=0.1.0"`だけをregistryから解決した。T08で
  legacy LE event maskとACL credit待ちを修正した0.1.1へ更新し、clean package verification buildと
  展開archiveのoffline/all-feature testが成功した。
- current `Cargo.toml` は `publish = false` を維持している。
- `swbt-bumble-backend` 0.1.1 の `src/usb.rs` は USB handle に
  `set_auto_detach_kernel_driver(true)` を設定してから interface を claim する。
- current CI は Ubuntu と Windows を含む。main へ入った PR #12 は全 9 job が成功した。
  unit_012 branchも同じremote checkをmerge gateとして通す。

### 7.2 判断

- `publish = false` は維持する。registry archive gateとunit_012のWindows実機回帰は解消したが、
  release candidateのremote CI、dependency/license/SBOM再監査、非公開脆弱性報告先の停止条件を
  先に解消する。
- unit_010時点のGit dependencyにはfork workspaceと一致するexact version requirementを追加した。
  unit_012 T07でGit dependencyを削除し、backendのexact registry versionへ置き換えた。
- package の `include` を allowlist とし、運用記録や hardware evidence が将来増えても crate に
  混入しない構成にする。
- Linux の detach/reattach code を `swbt-rs` に重複実装しない。backend transport がhandleと
  interface lifecycle を所有するため、公開 docs と source revision 監査で境界を固定する。
- crates.io用の恒久境界はunit_012で単一`swbt-bumble-backend`とし、source/API inventory、実装、
  単体archive、初回公開、registry archiveだけを使うswbt-rs smokeまで完了した。
- T11で現行registry graphのWindows/Linux SBOMを再生成し、旧Git graphのevidenceを履歴として分離した。
  `deny.toml`はGit sourceをすべて拒否し、backend 0.1.1のregistry checksumをSBOMでも固定する。
- GitHub Private Vulnerability Reporting が無効の間は、恒久的な非公開報告先がない。0.1.0 の公開前に
  有効化し、`SECURITY.md` を実際の報告先へ更新する。

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
| `spec/initial/source-baseline.md` | modify | 0.1.0 candidate の fork revision と差分 |
| `spec/publishing.md` | new | 0.1.0 release runbook と停止条件 |
| `tools/normalize-cyclonedx.ps1` | new | 生成SBOMのroot local path正規化とreference検査 |
| `spec/wip/unit_010/evidence/` | new | package list、license/SBOM、source audit、gate の非秘密 evidence |
| `spec/wip/unit_010/M9_PORTABILITY_RELEASE.md` | new | 本作業仕様 |

## 9. 検証

| command | result | notes |
|---|---|---|
| `cargo package --locked --list` | success | unit_012 T07 candidate 120 files。`.agents/`、`.codex/`、`.github/`、`spec/`、`tools/`、実機 trace を含まない |
| unit_010 `cargo package --locked` | failed (resolved in unit_012) | crates.io に `bumble-controller@0.1.0` がなく停止した履歴。単一backendへの切り替え前は成功扱いにしない |
| unit_012 T07 `cargo package --locked --allow-dirty` | success | registry backendだけで120 files / 1.4 MiB（圧縮257.8 KiB）のarchiveとverification buildを生成 |
| unit_012 T07 展開archive offline/all-feature test | success | library 271 passed / 1 ignored、hardware 5 ignored、他target success |
| `cargo fmt --all --check` | success | Rust source の整形 |
| `cargo +1.87.0 check --all-targets --all-features --locked` | success | MSRV で全 target/feature compile |
| `cargo check --all-targets --all-features --locked` | success | current toolchain で全 target/feature compile |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | success | warning なし |
| `cargo test --all-targets --all-features --locked` | success | library 300 passed / 2 ignored、probe 9 passed、統合/example test success、hardware 5 ignored |
| `cargo test --all-targets --locked` | success | default feature library 256 passed / 1 ignored、統合/example test success |
| `cargo test --lib protocol:: --no-default-features --locked` | success | 65 passed |
| `cargo build --all-features --locked` | success | all-feature build |
| `cargo build --no-default-features --locked` | success | Bumble-free build |
| GitHub Actions PR #11 initial head `627ba7e` | success | run `30649158739` の9 jobが成功。Windows 3m10s、dependency-policy 22s |
| GitHub Actions PR #12 merge head `b61476f` | success | unit_011 の Windows、Linux、MSRV、dependency-policy を含む9 jobが成功 |
| unit_012 旧Bumble package実験 | abandoned | 24 archiveのdry-runは実施したが、配布・保守境界とLICENSE/NOTICE収録が不適切なため公開案を不採用とした |
| `cargo test --test controller_type_contract --locked` | success | 2 passed。6 alias と builder の2型軸を検査 |
| `cargo test --lib model::tests --no-default-features --locked` | success | 3 passed。model metadata と全 button wire mapping を検査 |
| `cargo check --examples --all-features --locked` | success | public/hardware examples を compile |
| `cargo test --doc --all-features --locked` | success | 1 passed |
| `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked` | success | warning なし |
| `Get-Command Get-PnpDevice` | success | Windows driver 確認 command の入口を確認 |
| fixed Bumble `bumble-transport/src/usb.rs` source audit | success | auto detach、configuration、claim、alternate setting、handle ownership を確認 |
| Linux adapter hardware test | not run | 専用 Linux host/adapter がなく、CI と source audit で代替しない |
| `cargo-deny 0.20.2 --locked check` | success | advisories、bans、licenses、sources pass。複数版は warning |
| `cargo-cyclonedx 0.5.9` Windows/Linux all-features | success | CycloneDX 1.5、220/222 dependency components、license 欠落0 |
| T11 `cargo-deny 0.20.2 --locked check` | success | registry-only graphでadvisories、bans、licenses、sources pass。警告なし |
| T11 `cargo package --locked --allow-dirty` | success | 120 files / 1.4 MiB（圧縮258.0 KiB）、verification build成功 |
| T11 展開archive MSRV offline test | success | library 271 passed / 1 ignored、hardware 5 ignored、他target success |
| T11 `cargo-cyclonedx 0.5.9` Windows/Linux all-features | success | CycloneDX 1.5、33/34 dependency components、license欠落0、local pathなし、dependency ref整合 |
| fixture/package secret audit | success | JSON fixture 4件の provenance と合成 key 3件を確認。代表 credential pattern 0件、実機 profile/trace の収録なし |
| release docs placeholder scan | success | `[TODO]`、`TBD`、`xxx` の残存なし |
| release docs relative-link audit | success | README、CHANGELOG、SECURITY、platform/troubleshooting、publishing の local link 解決を確認 |
| `cargo tree --no-default-features --edges normal --locked` | success | Bumble/rusb を含まず、直接依存は atomic-write-file、fs2、serde_json、tracing |
| Bumble session targeted test | success | 修正後1回と同じ compiled binary の100回反復が成功。全 library 300 passed / 2 ignored、Clippy success |
| `git diff --check` | success | whitespace error なし |

## 10. 先送り事項

- `swbt-rs` crates.io publish: backend 0.1.1公開とregistry archive gateは完了した。残る停止条件と
  公開時の明示承認は`spec/publishing.md`と`spec/complete/unit_012/BUMBLE_BACKEND_BOUNDARY.md`で追跡する。
- Linux hardware: 専用 Linux host/adapter を用いた pair/reconnect/close/reattach は未実行。
  `docs/platform-support.md` で build-tested と hardware-verified を分け、後続 evidence の完了条件を置く。
- macOS: roadmap どおり unsupported。USB transport と driver ownership の調査を別 unit とする。
- local Bluetooth address: unit_011 で実装し、CSR8510 A10 の pair/reconnect/power-cycle を確認した。

## 11. チェックリスト

- [x] 対象範囲と対象外を確認した
- [x] TDD Test List を作成した
- [x] TDD Test List の各 item を更新した
- [x] Windows/Linux/macOS の支援水準を公開文書へ反映した
- [x] public API、alias、model/mapping、examples を監査した
- [x] package file list、license/SBOM、security、changelog を検査した
- [x] release/rollback runbook と blocker を検査した
- [x] 検証結果または未実行理由を記録した
- [x] package / release / public API に触れる gate を記録した
- [x] registry backend 0.1.1 の dependency/license/SBOM/package evidence を再生成した
- [ ] GitHub の非公開脆弱性報告先を有効化し、`SECURITY.md` を更新した
- [ ] workload soak と backend rollback rehearsal の結果を記録した
