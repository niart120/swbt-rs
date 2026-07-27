# swbt-rs Skills

リポジトリ固有の workflow は `.agents/skills/` に置く。ここは呼び出し先の一覧であり、手順の正本は各 `SKILL.md` である。

| Skill | 用途 |
|---|---|
| `agentic-sdd` | 仕様、作業単位、TDD、品質 gate をつないで実装を進める。 |
| `agentic-self-review` | handoff や PR 前に仕様、差分、gate、未検証リスクを整理する。 |
| `spec-format` / `dev-journal` | 作業仕様と小さい設計観測を管理する。 |
| `tdd-workflow` / `tdd-test-list` / `tdd-one-cycle` | 振る舞いベースの TDD を進める。 |
| `refactor-after-green` / `tidy-first` | green 後の構造変更を振る舞い変更から分離する。 |
| `test-desiderata-review` | Rust の unit、integration、doc、feature 検証の役割を見直す。 |
| `rust-api-boundary-review` | 公開 API、所有権、error、非同期、feature の境界を確認する。 |
| `rustdoc-style` | rustdoc と README / docs の公開 API 説明を整合させる。 |
| `docs-quality-review` | docs、spec、skill、PR 本文の根拠と表現を確認する。 |
| `diagnosing-bugs` | 再現困難な不具合や性能退行を診断する。 |
| `crates-io-release` | crates.io 公開の preflight、tag、publish、公開後確認を扱う。 |
| `pr-merge-cleanup` | PR 作成、merge、同期、branch cleanup を行う。 |
