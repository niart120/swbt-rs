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

## 2026-07-29: M1 pure protocol と M2 runtime の接続前 dead code

### 現状

M1 は `src/protocol/` に crate-private の純粋変換を実装する。最初の production caller は M2 の worker / sender であり、M1 中は unit test からだけ呼ばれる。

### 観察

`protocol` を `cfg(test)` にすると通常 build が M1 の実装を検査しない。通常 build に含めると、M2 接続前の module 全体が未参照として `dead_code` になる。

### 方針

M1 中は、実装済みで unit test がある protocol module に限り `cfg_attr(not(test), allow(dead_code, reason = "..."))` を置く。test build では抑制せず、未検査 item を隠さない。M2 が runtime caller を追加した commit で属性を削除し、通常 build の参照関係を gate で確認する。

## 2026-07-29: observed subcommand の収集と診断投影

### 現状

Python 基準断面の session は、接続中に受信した subcommand ID を重複なしで保持する。runtime は reply 構築前に ID を記録するため、unsupported command や reply 送信失敗でも観測済み ID は残る。一方、M1 fixture generator の session projection と M1 TDD item 11 はこの集合を対象にしていない。

### 観察

この集合は reply bytes や readiness の計算には使わず、接続ごとの trace と診断に使う。M1 の pure session へ先に追加すると、fixture で検証していない runtime の記録順と rollback 契約まで完了したように見える。

### 方針

M1 item 11 は report mode、player lights、IMU、vibration、readiness に限定する。M2 の output handler で、parse 済み subcommand ID を reply 構築前に接続単位の集合へ記録し、reply 失敗時にも戻さない契約をテストする。M8 はその集合を安定した diagnostics event へ投影する。
