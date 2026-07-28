# swbt-rs 初期仕様

このディレクトリは、`swbt-rs` の実装開始時点で合意する公開 API、型モデル、内部構造、実装順序、検証方法、Python 版からの移行方法を定義する。

## 読む順序

1. [source-baseline.md](source-baseline.md) — 参照したリポジトリ断面と判断の前提
2. [type-modeling.md](type-modeling.md) — controller model、reporting mode、モデル固有入力能力の型表現
3. [api.md](api.md) — Rust 利用者に公開する型と振る舞い
4. [architecture.md](architecture.md) — レイヤ、所有権、Bumble 統合、状態機械
5. [testing.md](testing.md) — compile-fail、差分テスト、仮想 Bluetooth、実機検証、CI
6. [migration-strategy.md](migration-strategy.md) — `swbt-python` との互換範囲と段階移行
7. [roadmap.md](roadmap.md) — 実装単位、依存関係、完了条件

## 文書内の状態表記

| 表記 | 意味 |
|---|---|
| **決定** | 初期実装が従う設計判断。変更時は仕様更新を先に行う |
| **基準実装** | 参照元の最新断面で確認した既存挙動 |
| **要検証** | 実 API または実機で未確認。完了条件を満たすまで保証しない |
| **先送り** | 初期リリースの対象外。公開 API に先回りして入れない |

## 仕様の優先順位

矛盾がある場合は、次の順で解消する。

1. `spec/complete/unit_連番/` に移された、より新しい作業単位の仕様
2. このディレクトリの初期仕様
3. `README.md` とコードコメント
4. 参照元リポジトリの現在実装

型関係については [type-modeling.md](type-modeling.md) を正本とする。`api.md`、`architecture.md`、`testing.md` に型名や能力境界の記述がある場合も、同文書と一致させる。

実装が初期仕様から意図的に外れる場合は、同じ変更で仕様も更新する。