---
name: spec-format
description: "この Rust repo の作業仕様を spec/wip/unit_XXX または spec/complete/unit_XXX に作成、更新、完了移動する skill。ユーザが仕様書、spec、設計書、作業仕様、TDD Test List、spec/initial に基づく実装単位の整理を依頼したとき、または spec/ 配下を扱うときに使う。"
---

# 仕様書構成様式

作業仕様を `spec/wip` / `spec/complete` で管理する。仕様書は単なる設計文書ではなく、対象範囲、TDD 状態、検証結果、先送り事項を束ねる作業単位でもある。

## 配置

| 状態 | path |
|---|---|
| 着手中 | `spec/wip/unit_XXX/FEATURE_NAME.md` |
| 完了済み | `spec/complete/unit_XXX/FEATURE_NAME.md` |

- `FEATURE_NAME.md` は UPPER_SNAKE_CASE にする。
- 連番は `spec/wip/unit_*` と `spec/complete/unit_*` を確認して次を選ぶ。
- 完了時は directory ごと `spec/complete` へ移す。
- `spec/initial/` は初期設計の正本として扱い、作業仕様の完了管理には使わない。

## テンプレート

新規作成時は `references/template.md` を読む。テンプレート内の guide comment は出力に残さない。

## 記述ルール

- source、use case、対象範囲、対象外を分ける。
- 事実、推論、未検証仮説を分ける。
- TDD Test List は観測可能な振る舞いで書く。実装ファイル名や内部構造だけを item にしない。
- 検証には実行 command、結果、未実行理由を残す。
- 先送り事項には観測、先送り理由、後続の置き場を書く。何もなければ `none` と書く。
- Cargo metadata、CI、release workflow、public API に触れる場合は `cargo build --all-features` と必要に応じた `cargo package` を検証へ含める。

## Workflow

1. `AGENTS.md`、`SKILLS.md`、関連する `spec/initial/*.md` を読む。
2. 既存 `spec/wip` / `spec/complete` を確認する。
3. 新規か更新か完了移動かを決める。
4. 新規なら `references/template.md` を使って作成する。
5. TDD で進める作業では `tdd-test-list` と接続する。
6. 実装後は検証結果と checklist を更新し、完了条件が揃った場合だけ `spec/complete` へ移す。

## 完了移動の条件

- checklist が更新されている。
- 検証 command と結果、または未実行理由がある。
- package / release / public API に触れた場合の gate が記録されている。
- 先送り事項が `none` か、後続 source として使える粒度になっている。
