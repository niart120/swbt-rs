# Dev Journal

swbt-rs の設計観測、未解決事項、先送り判断の記録。

仕様書へ昇格できる粒度になったら `spec/wip/unit_XXX` へ移す。

## 2026-07-29: release target と milestone 順序の不一致

### 現状

`spec/initial/roadmap.md` の主依存列は M7 Joy-Con の後に M8 diagnostics / probe を置く。release target は `0.1.0-alpha.2` に diagnostics と `swbt-probe` を含め、Joy-Con は後続の `0.1.0-beta.1` で追加する。

### 観察

主依存列をそのまま実装すると、alpha.2 の対象外である Joy-Con を完了しなければ alpha.2 の diagnostics / probe へ進めない。M0 の開始条件には影響しないが、M6 到達後の release 判定と次 unit 選択が曖昧になる。

### 方針

M6 完了前に、M8 の diagnostics / probe を M6 後へ分離するか、release target と milestone 名を更新する作業仕様へ昇格する。判断前に主依存列を暗黙に並べ替えない。
