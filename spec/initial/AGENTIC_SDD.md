# Agentic SDD

## Constitution

この repo の Constitution は次のファイルで構成する。

- `AGENTS.md`
- `SKILLS.md`
- `spec/initial/*.md`
- 対象の `spec/wip/unit_XXX/*.md`

ユーザが追加で与えた指示は Intent Delta として扱う。既存仕様と矛盾する場合は、どの指示を優先するかを作業前に明示する。

## 作業単位

- 1つの work unit は 1 PR でレビューできる範囲にする。
- 仕様なしで大きな実装へ入らない。
- 変更範囲、対象外、検証 command、未検証事項を work unit に残す。
- 完了済みの work unit は `spec/complete` へ移す。

## Gate

標準 gate:

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --all-features
git diff --check
```

Cargo metadata、public API、CI、release に触れる変更では、標準 gate を省略しない。公開前の package 内容確認が必要な場合は、clean worktree で `cargo package` を追加する。
