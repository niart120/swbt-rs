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

## 2026-07-29: Bumble git 依存と crates.io package 名の衝突

### 現状

M0 は `chaitanyarahalkar/bumble-rs` の commit `bbac2a6803b8cab0920ab725a23aa408fc4fed85` に `bumble` を固定する。Cargo は git dependency を package へ含める際に version requirement を要求し、git 指定を除去する。

### 観察

基準断面の version `0.1.0` を併記して `cargo package` を実行すると、検証は crates.io の別物である PyO3 系 `bumble 0.1.0` を取得して成功する。これは Rust の `bumble-rs` を検証した証拠にならない。version requirement を外すと Cargo は package 作成を明示的に拒否する。

### 方針

M0 では exact git revision を優先し、`publish = false` を設定する。M9 の package / release 作業までに、`bumble-rs` の crate 公開名、upstream publish、または依存境界の変更を作業仕様へ昇格する。別 package を使った `cargo package` 成功を release gate にしない。
