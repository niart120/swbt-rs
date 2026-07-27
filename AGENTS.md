# swbt-rs Agent Guide

## 対話と作業境界

- ユーザとの対話は日本語で行う。
- 事実、仮説、提案、未検証事項を分けて書く。未実行の検証や外部仕様の推測を確認済みとして扱わない。
- 変更前に `git status --short` と現在の branch を確認し、既存変更を破棄しない。
- default branch への直接 commit は、ユーザが明示した場合を除き避ける。

## プロジェクト

`swbt-rs` は Rust 2024 edition の Cargo バイナリ crate である。パッケージ名、版、依存、feature、公開範囲の正本は `Cargo.toml` と `Cargo.lock` に置く。

- Rust toolchain は `rust-toolchain.toml` があればそれに従う。なければ `Cargo.toml` の `rust-version` と CI を確認してから追加する。
- 実装は `src/`、統合テストは `tests/`、例は `examples/`、ベンチマークは `benches/` に置く。
- 公開 API は `pub` にする前に必要性を確認し、利用者に見える型・関数・module には rustdoc を書く。
- `unsafe`、ネットワーク、ファイル操作、実機 I/O、feature flag、プラットフォーム分岐は境界を小さくし、対応する検証と未検証条件を記録する。

## 作業仕様

仕様を使う作業では、`spec/initial/` を初期設計の正本、`spec/wip/unit_XXX/` を着手中、`spec/complete/unit_XXX/` を完了済みの作業単位として扱う。小さい観測や先送り判断は `spec/dev-journal.md` に置く。

作業仕様には、目的、対象範囲、対象外、振る舞い仕様、TDD Test List、対象ファイル、検証、先送り事項、完了チェックリストを含める。仕様ツリーがまだない初期化段階では、必要になるまで作らない。

## Skills

リポジトリ内 skill は `.agents/skills/` を正本とする。呼び出し先と役割は [SKILLS.md](SKILLS.md) を参照する。

- 仕様から実装へ進めるときは `agentic-sdd`、TDD では `tdd-workflow` を使う。
- Rust の公開 API・型・所有権・エラー境界は `rust-api-boundary-review`、rustdoc は `rustdoc-style` を使う。
- crates.io 公開は `crates-io-release` を使う。公開 tag や publish は、この turn で明示承認がある場合だけ実行する。

## Rust の実装規約

- `rustfmt` に従う。手作業で整形規則を再定義しない。
- `clippy` の警告を黙殺するための広い `#[allow(...)]` は追加しない。必要な例外は最小の lint を対象に理由を近くに書く。
- `unwrap()` と `expect()` は、失敗不能性が局所的に証明できるテストまたは起動時の不変条件に限る。ライブラリ境界・入力境界では意味のある `Result` / error 型で返す。
- 文字列を error の唯一の契約にしない。呼び出し側が分岐する失敗は error variant または型で表す。
- 非同期処理は cancellation、timeout、所有権、共有状態を明示する。`Mutex` をまたいで `.await` しない。
- feature flag や OS 固有処理を変えた場合は、影響する feature / target を検証範囲に書く。

## テストと検証

変更範囲に応じて次を選ぶ。すべてを機械的に実行せず、実行した command と検査対象を記録する。

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --all-features
git diff --check
```

- crate の通常利用を feature なしで支える場合は `cargo test` と `cargo build` も確認する。
- `--workspace` は workspace manifest を導入した場合に使う。単一 crate に先回りして付けない。
- benchmark、実機、network、cross compile、Miri、fuzz は対象に含めた場合だけ実行し、未実行なら理由を残す。
- docs / spec / skill だけの変更では、対象文書の事実整合、参照先、仮テキスト残り、skill frontmatter を確認する。製品コードを検査しない `cargo test` や build の成功を文書品質の根拠にしない。
- skill を変更した場合は `C:\Users\train\.codex\skills\.system\skill-creator\scripts\quick_validate.py` で対象 skill を検査する。必要な `pyyaml` はプロジェクト依存に追加せず、隔離したツール環境で供給する。

## Docs / Public Text

- README は利用開始の入口に保つ。agent 運用、作業履歴、実験ログは `AGENTS.md`、`SKILLS.md`、`spec/` に分ける。
- 公開 docs と rustdoc には、利用者が確認できる現在の仕様、手順、制約、エラー条件を書く。
- 文書の説明品質は読んで確認する。固定文言だけを検査する test で意味の正確さを主張しない。
- docs、spec、skill、PR 本文を変更した場合は `docs-quality-review` を使う。

## Git / PR

- commit は一つの論理変更に絞り、Conventional Commits の `<type>(<scope>): <subject>` を使う。
- PR 本文には変更理由、論理単位、実行 command と結果、未実行理由、先送り事項を書く。
- PR merge 後は default branch を同期し、安全な場合だけ作業 branch を削除する。
