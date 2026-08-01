# Bumble registry package 実験の不採用判断

- 判断日: 2026-08-01 (JST)
- 状態: **不採用**
- crates.io 公開件数: 0
- 自己所有 fork: `https://github.com/niart120/bumble-rs`
- 実験 branch: `feat/swbt-registry-package-names`
- 実験 revision: `5fb0f6ddb811d1ad43dffa6e72a5d8cc6096fb07`
- 採用した暫定 revision: `cb55e2d98dc7b7b0227c43772c9ae184034dd9a1`
- backend 追跡先: [Issue #1](https://github.com/niart120/bumble-rs/issues/1)
- remote/local branch cleanup: 2026-08-01 に削除済み

## 判断

fork workspace 24 packageを `swbt-bumble*` 名でcrates.ioへ公開する案は採用しない。
`swbt-rs` が利用するBluetooth Classic HIDの実装だけを、単一の
`swbt-bumble-backend` crateへ整理する。

## 根拠

- swbt-rsが利用しないprotocolを含む24 packageの版管理、脆弱性対応、yank、問い合わせ対応まで
  自己所有fork側で引き受ける配布境界になる。
- 生成した `.crate` archiveにworkspace rootの `LICENSE` と `NOTICE` が含まれていなかった。
  package metadataの `license = "Apache-2.0"` だけを再配布条件の充足として扱わない。
- READMEとpackage metadataは、swbt-rs専用の非公式forkであることを十分に区別していなかった。
- 24 packageの公開は、swbt-rsが必要とする実装範囲より大きい保守責任を固定する。

## 実験で確認した事実

改名、内部dependency alias、9 layerの公開順、24 packageのarchive checksum、
`cargo publish --dry-run` の結果は
[`abandoned-registry-package-preflight-20260801.md`](abandoned-registry-package-preflight-20260801.md)
に残す。これは不採用案のraw evidenceであり、現在のrelease candidateが公開可能であることを
示す証跡ではない。

## 採用した移行状態

- 自己所有forkの `main` は実動作修正3 commitだけを含む `cb55e2d` へfast-forwardした。
- `swbt-rs` の8 direct dependencyは元の `bumble*` package名と `cb55e2d` へ戻した。
- default/all-featureのcheck、test、build、Clippyは成功した。
- clean `cargo package --locked --list` は成功した。
- clean `cargo package --locked` はcrates.ioに `bumble-controller@0.1.0` がないため停止した。
- current dependencyとrelease文書から参照を外した後、実験branchのremote/local refを削除した。
- 削除後の `git ls-remote --heads origin main feat/swbt-registry-package-names` は
  `main@cb55e2d` だけを返した。

## 後続

後続の対象範囲、TDD Test List、packageと実機の完了条件は
[`../BUMBLE_BACKEND_BOUNDARY.md`](../BUMBLE_BACKEND_BOUNDARY.md) を正本とする。
