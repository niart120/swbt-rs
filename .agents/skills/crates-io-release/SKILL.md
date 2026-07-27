---
name: crates-io-release
description: "この Rust repo の crates.io release を計画・実行する workflow skill。ユーザが crates.io 公開、version 更新、release PR、tag、GitHub Actions publish、公開後確認、release 手順確認を依頼したときに使う。"
---

# crates.io Release

`spec/publishing.md` があれば release runbook の正本として使う。手順詳細をこの skill に重複させない。

## 手順

1. `spec/publishing.md`、`Cargo.toml`、`Cargo.lock`、CI、git 状態を確認する。
2. `cargo package --allow-dirty` は検査に使わない。clean worktree で `cargo package` を実行して公開内容を確認する。
3. release PR、merge、default branch 同期、branch cleanup は `pr-merge-cleanup` に委譲する。
4. local gate、package 検査、publish、公開後の取得確認を分けて記録する。

## 停止条件

- この turn の明示承認なしに `cargo publish`、production tag push、publish workflow の実行をしない。
- `spec/publishing.md` または publish workflow が必要なのに存在しない場合は停止する。
- candidate version、tag、registry token / Trusted Publishing、local gate、CI が runbook と矛盾する場合は停止する。
- crates.io に同じ版がある場合は停止する。

## 報告

version、release branch / PR、tag、workflow run、crates.io URL、実行 gate、公開後確認、停止条件を報告する。
