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

## 2026-07-29: blocking tap と public close cancellation

### 現状

初期 API は `tap(&mut self, ...) -> Result<()>` を blocking operation とし、
`Controller<M, R>` は `Send` だが非 `Sync` とする。同時に、tap の delay は worker
scheduler が管理し、close で中断できることを求める。

### 観察

safe な public API では、blocking `tap()` が `&mut self` を保持している間に同じ
controller handle から `close()` を発行できない。public concurrent close を契約にするには、
cloneable cancellation handle、または tap の非 blocking 化が必要になり、どちらも初期 API の
変更になる。

### 方針

M2 は tap を worker deadline として保持し、thread sleep を使わない。transport disconnect、
worker shutdown、command channel termination、crate-private priority close では pending tap を
中断する。public cancellation handle は追加せず、必要性を fake runtime と後続の実 transport で
再評価する。blocking tap の press/release acceptance 契約は維持する。

## 2026-07-29: M2 worker の activity wait 候補

### 現状

worker は bounded command、transport activity、shutdown、Periodic deadline のいずれかまで
待つ必要がある。短い timeout の `TransportPort::poll()` を反復すると実装は小さいが、idle
wake が増え、command、HCI、shutdown latency が同じ poll 量子に縛られる。

### 観察

command 本体とは別に容量 1 の wake channel を置けば、command enqueue、transport activity、
priority shutdown を同じ通知へ coalesce できる。worker は最短 deadline まで
`recv_timeout()` し、起床後に transport を non-blocking drain できる。transport object 自体は
worker thread の単独所有を維持する。

### 方針

M2 の第一候補を bounded command `sync_channel`、容量 1 の wake `sync_channel<()>`、
queue 外の priority shutdown latch とする。fake clock / scripted waiter で invariant を検証し、
実時間 probe で idle CPU、8 ms jitter、command/HCI latency、shutdown latency、fairness を
測定してから queue 容量と command batch 上限を固定する。Bumble 側で activity notifier を
得られない場合は M3 の upstream 方針として扱い、短周期 polling を未測定の恒久 fallback にしない。

## 2026-07-29: M2 runtime で意図的に変える Python state semantics

### 現状

Python 基準断面は quaternion IMU encoding state を transport send 前に更新するため、
rejected input send でも次の quaternion candidate が進み得る。Periodic では connection 前に
commit した input state を最初の normal input へ持ち越す。bootstrap neutral retry は
各 send 完了後から 1 秒を待つため、send latency 分だけ次回 deadline が後ろへずれる。

Rust 初期設計は report bytes と next IMU state を candidate として計算し、transport acceptance
後だけ commit する。各 new connection では protocol state、timer、holdoff と committed input を
neutral baseline へ戻す。worker scheduler は bootstrap retry も absolute deadline で管理する。

### 観察

IMU の rejection rollback と new-session neutral reset は Python の観測結果と一致しない。
前者は timer と committed IMU state を維持し、retry 時の時刻から候補を再計算できる。
fake time も同じなら同じ bytes になる。後者は前 session または connection 前の pressed
state を別の Switch connection へ自動送信しない安全境界になる。
absolute bootstrap retry は send latency による drift を避け、overrun 時の missed retry を
burst 送信しない scheduler 契約とそろう。

### 方針

M2 は Rust 初期設計を採用し、3 件を `rust_spec_delta` の deterministic test で固定する。
Python runtime fixture には基準断面の観測も `baseline_observation` として残し、byte parity
case と混同しない。Periodic の未接続 `apply()` 自体は local snapshot へ commit するが、
new connection の開始時に neutral reset が優先する。利用者へ見える README/rustdoc には
session を越えて input state を持ち越さないことを書く。

## 2026-07-29: M2 transport contract と worker 接続前 dead code

### 現状

T02 は `src/runtime/transport/` に crate-private の transport contract を追加する。fake contract
test は open、send acceptance、non-blocking bounded poll、channel routing、coalescing wake、
queue overflow、terminal source、close を検証する。production caller は T21 の worker core で
接続する。

### 観察

transport module を `cfg(test)` にすると通常 build が contract を検査しない。通常 build に
含めると、worker 接続前は module 全体が未参照になり、`-D warnings` の `dead_code` に失敗する。
個々の型へ許可を散らすと、production caller 接続後も不要な型を見落としやすい。

### 方針

T02 から T21 までは transport module に限り、理由付き
`cfg_attr(not(test), allow(dead_code))` を置く。test build では抑制せず、fake contract の未参照を
隠さない。T21 で worker caller を接続した commit に属性削除を含め、通常 build と clippy で
実参照を確認する。

## 2026-07-29: M2 observed subcommand set と output handler 接続前 dead code

### 現状

T03 は `src/runtime/connection.rs` に crate-private の `ObservedSubcommands` を追加する。
観測集合を使う output handler は T07 で実装する。

### 観察

T03 の test は全 `u8` ID の bit set、重複除去、reset を検査する。通常 build に
connection module を含めると、T07 までは production caller がなく `dead_code` になる。
`cfg(test)` だけにすると通常 build が型を検査しない。

### 方針

T03 から T07 までは connection module に理由付き
`cfg_attr(not(test), allow(dead_code))` を置く。T07 の output handler が `observe()` を使う
commit で module 全体の許可を削除し、未接続の `reset()` と `is_empty()` だけへ許可を狭める。
`reset()` は T17 の session reset、`is_empty()` は T18 の handshake で実参照を作り、それぞれの
commit で method 単位の許可を削除する。各段階で通常 build と clippy を確認する。

## 2026-07-29: M2 ReportSender と worker 接続前 dead code

### 現状

T05 は `src/runtime/sender.rs` に crate-private の `ReportSender<M>` を追加する。sender は
timer と committed protocol session を所有し、protocol と transport は送信ごとに借りる。
通常コードから sender を所有する worker は T21 で実装する。

### 観察

T05 で M1 protocol の input preparation と session は sender から参照されるが、output parse と
reply preparation を通常コードへ接続するのは T07、sender と transport を所有するのは T21
である。T05 だけで M1 protocol、T02 transport、T05 sender の module-wide `dead_code` 許可を
すべて外すと、まだ着手していない機能の未使用警告を避けるために不要な caller が必要になる。

### 方針

T05 から T21 までは sender module に理由付き
`cfg_attr(not(test), allow(dead_code))` を置く。test build では抑制しない。T07 以降は各機能を
仕様どおり接続し、T21 の worker caller を追加する commit で transport、sender、残る M1
protocol の module-wide 許可を削除する。通常 build と clippy で実参照を確認する。

## 2026-07-29: T31 の worker 再生成と status counter の寿命

### 現状

T26 の `input_reports_accepted` と `replies_accepted` は `ReportSender` を正本とし、接続 session
の開始ではリセットしない。T26 の test は同じ worker 内で session 1 から session 2 へ移っても
累積値を保持する。

### 観察

`GamepadStatus` の counter は controller lifetime の累積値である。T31 で public `close()` 後の
`open()` を、同じ controller に新しい `WorkerCore` と `ReportSender` を作る方式で実装すると、
sender の初期値 0 が既存 projection を上書きし、公開済みの累積値が後退する。

### 方針

T31 で worker の再生成が必要になった場合は、既存 projection の counter を新しい sender へ
seed するか、counter の正本を controller lifetime の所有物へ移す。reopen 前後で値が単調非減少
であることを public orchestration test に含める。T26 の範囲では worker を作り直さず、接続
session 間の保持だけを完了条件とする。

## 2026-07-29: T31 create-profile と worker Ready 完了境界

### 現状

T31 は empty envelope の create-new、同じ store からの typed reopen、transport open、pairing、
protocol Ready、controller 返却の因果順を検査する。既存 `WorkerCore` は handshake と Ready
判定を持つが、`run_worker_loop()` は Ready、timeout、disconnect の結果を同期呼出元へ返す
pairing response をまだ持たない。

### 観察

T31 で実 worker thread を強制すると、create-profile の順序検査に加えて pair command、
one-shot completion、reporting 別 command 型の所有境界まで同じ item へ入る。これは T33 の
fake runtime smoke と入力操作を先取りする。worker を thread 起動前に直接 Ready まで進めると、
open 後の worker 所有と pairing の実行順を検査できない。

### 方針

T31 は crate-private `CreateProfileRuntimeBackend` の
`ensure_supported()`、`open()`、`pair_to_ready()` を同じ production orchestrator へ注入し、
`InspectTarget → CheckBackendCapability → CreateNew → ReadBack → Open → PairStarted →
ProtocolReady` を代表的な Pro/Periodic fake で固定する。成功時は `ReadyRuntime<M, R>` を
controller の先頭 field で所有し、runtime を config/status より先に破棄する。これは実
worker thread や Switch pairing の成功根拠ではない。

T32 は `Opened` の open/pair failure 時 cleanup と empty envelope 残存を固定する。T33 は
sealed reporting 境界へ model 別 command 型を接続し、pair command の one-shot completion で
実 worker thread の Ready を待ってから同じ controller 所有へ移す。`Any` downcast で
`WorkerOwner` を取り出す方式は採用しない。

## 2026-07-29: T32 create-profile runtime attempt の cleanup 所有権

### 現状

同日の「T31 create-profile と worker Ready 完了境界」では、backend の `open()` が
`Opened` を返し、`pair_to_ready()` へ渡す設計としていた。T32 では、
`CreateProfileRuntimeBackend::begin_attempt()` が resource 未取得の attempt を返し、
orchestrator が同じ attempt を open、pair、失敗時 cleanup、成功時 Ready 所有権移譲まで
保持する設計へ更新した。

### 観察

旧設計では `open()` が error を返した場合、途中で取得した resource の owner を
orchestrator が受け取れない。現設計では open 途中に resource を取得した場合も attempt が
残るため、open 失敗と pair 失敗を同じ `cleanup_without_neutral()` 経路へ渡せる。
create-new 競合は runtime open 前に終了する。

T32 の fake が検査するのは、明示 cleanup が without-neutral 経路を選ぶことと一回性である。
T20 の close-without-neutral と同じく neutral だけを省略して bounded drain を残す実 worker
接続は T33 で行う。resource 取得後の早期 Drop では attempt の fallback が resource を
解放する。fake probe は明示 cleanup、fallback、resource drop を別々に数え、各経路の一回性を
検査する。

### 方針

T33 で実 worker と pair 完了通知を attempt へ接続する。cleanup 自身も失敗した場合は、
primary と cleanup の両 source を利用者が辿れる構造化 error 境界を決める。標準の
`Error::source()` は単線なので、cleanup error を非公開 field に置くだけでは完了としない。

T34 の public backend 不在、T35 の worker Drop timeout/detach、M6 の atomic create/replace、
lock、key preservation は、それぞれの既存 item へ残す。
