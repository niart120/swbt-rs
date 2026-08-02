# profile key canonical uppercase 仕様書

## 1. 概要

### 1.1 目的

pairing profile の key-store namespace と Classic peer 名について、読み込み時の
uppercase 正規化を廃止し、`swbt-python 0.6.0` の writer/runtime が使用する
canonical uppercase 表記だけを受理する。非canonical key は秘密情報を表示せず
`ErrorKind::InvalidProfile` で拒否する。

### 1.2 起点 / source

| source | 内容 | path |
|---|---|---|
| GitHub Issue #31 | 正規化後に同一になる address key が後勝ちで置換され得る問題 | `https://github.com/niart120/swbt-rs/issues/31` |
| user decision | canonical collision 検出ではなく、Python と同じ canonical uppercase のやり取りへそろえ、読み込み時の正規化を廃止する | 2026-08-03 の対話 |
| pinned Python source | profile loader は key を正規化せず、writer と runtime は uppercase address を使用する | `niart120/swbt-python@84d2723b:src/swbt/transport/_pairing_profile.py`, `src/swbt/transport/_bumble_key_store.py` |
| prior work unit | lowercase/uppercase key を受理して uppercase 出力する旧契約 | `spec/complete/unit_017/PROFILE_MODEL_AND_STORE_SIMPLIFICATION.md` |

### 1.3 use case

| actor / boundary | 入力または状態 | 期待する観測結果 | 制約 |
|---|---|---|---|
| Python profile 利用者 | swbt-python 0.6.0 が生成した uppercase namespace / peer | 対応する `PairingProfile<M>` として読み書きできる | schema v2、Classic `/P` peer |
| 非canonical profile 利用者 | lowercase または mixed-case の namespace / peer | `ErrorKind::InvalidProfile` | address、peer、link key を error / source / Debug に含めない |
| Rust profile 利用者 | canonical profile を `to_json_bytes()` で保存 | 入力 key を変換せず uppercase の deterministic JSON を得る | sorted key、2-space indent、末尾 newline を維持 |

## 2. 対象範囲

- key-store namespace の Bluetooth address に canonical uppercase を要求する
- Classic peer の address 部分に canonical uppercase を要求し、`/P` を維持する
- `BluetoothAddressKey` / `ClassicPeerKey` の読み込み時 uppercase 変換を削除する
- 非canonical key を secret-free な `InvalidProfile` として拒否する
- profile unit / public compatibility test と rustdoc / 初期仕様を現行契約へ同期する

## 3. 対象外

- 同一綴りの JSON object member 重複を個別に検出・診断すること
- canonical collision 用の custom `MapAccess`、`DeserializeSeed`、map wrapper
- lowercase profile の自動移行または読み込み時のファイル書き戻し
- profile schema version の変更
- unknown extension、旧Rust profile、LE key field、`/P`なしpeerの再受理
- file store の atomic update、複数writer、runtime lookup、Bumble backend の変更
- USB adapter、Switch UI、実機 pairing / reconnect

## 4. 関連 docs

- `spec/initial/architecture.md`
- `spec/initial/testing.md`
- `spec/initial/roadmap.md`
- `spec/complete/unit_017/PROFILE_MODEL_AND_STORE_SIMPLIFICATION.md`
- `spec/complete/unit_019/CORE_PACKAGE_AND_REQUIRED_RUNTIME_BACKEND.md`

## 5. 振る舞い仕様

| 振る舞い | 入力・状態 | 期待結果 | 備考 |
|---|---|---|---|
| namespace canonical input | uppercase `XX:XX:XX:XX:XX:XX` | 変換せず受理する | Python/Rust writer 出力と一致 |
| namespace noncanonical input | lowercase または mixed-case address | `InvalidProfile` | 同じdocument内のkey順序に依存しない |
| peer canonical input | uppercase `XX:XX:XX:XX:XX:XX/P` | 変換せず受理する | `/P`以外は従来どおり拒否 |
| peer noncanonical input | lowercase または mixed-case addressと`/P` | `InvalidProfile` | link keyをerrorへ含めない |
| canonical compatibility | pinned Python 0.6.0 fixture 6件 | 同じ意味のdeterministic JSONとしてround-tripする | fixtureはcanonical uppercase |
| unsupported repeated member | 同一object内の同一JSON member名 | 個別diagnosticを保証しない | supported profileは一意なmember名を前提とする |

### 5.1 Intent Delta

`unit_017` は uppercase/lowercase のaddress keyを受理し、型付きkeyへ変換する際に
uppercase化する契約を導入した。本unitでは、Python側が生成・利用するcanonical uppercaseを
profile入力の正本とし、Rust固有の寛容な正規化を廃止する。

- canonical keyは変換せず保持する。
- 非canonical keyは別namespace/peerとして保持せず、入力境界で拒否する。
- case違いの複数keyは、collision検出ではなく非canonical keyの拒否によって順序非依存で失敗する。
- 同一綴りのJSON member重複は本unitの診断契約に含めない。

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| refactor-skipped | T01 canonical uppercase namespaceは受理し、lowercase / mixed-case namespaceはkey順序に依存せずsecret-freeな`InvalidProfile`になる | behavior / regression / edge | public integration | key変換を削除し、既存address parserへcanonical検査だけを追加した |
| refactor-skipped | T02 canonical uppercase Classic `/P` peerは受理し、lowercase / mixed-case peerはkey順序に依存せずsecret-freeな`InvalidProfile`になる | behavior / regression / edge | public integration | `/P`、16-byte link key、最大1 peerを維持する |

status は `todo`、`red`、`green`、`refactor-done`、`refactor-skipped`、`deferred` を使う。

### 6.1 TDD cycle evidence

| phase | item | evidence |
|---|---|---|
| red | T01 | `cargo test -p swbt-core --test profile_compat typed_profile_requires_canonical_uppercase_namespace --locked`は、現行parserがlowercase namespaceを受理して`PairingProfile`を返したため期待どおり失敗した |
| green | T01 | namespace parserがuppercase addressだけを変換せず保持するよう変更した。同じcommandは成功し、後続self-reviewでは非canonical key単独とcanonical key前後の両順序も成功した |
| refactor-skipped | T01 | canonical検査はT02でも使う一つのpredicateへ置いた。namespace item内で追加の構造変更は不要と判断した |
| red | T02 | `cargo test -p swbt-core --test profile_compat typed_profile_requires_canonical_uppercase_peer --locked`は、現行parserがlowercase peerを受理して`PairingProfile`を返したため期待どおり失敗した |
| green | T02 | peer parserがuppercase addressと`/P`だけを変換せず保持するよう変更した。同じcommand、namespace test、pinned Python fixture 6件は成功し、後続self-reviewではcanonical key前後の両順序も成功した |
| refactor-skipped | T02 | T01で追加したcanonical address predicateをそのまま再利用できた。map deserializerや追加のkey型は不要と判断した |

## 7. 設計メモ

Tidy decision:

- classification: mixed
- action: tidy-first
- reason: 既存integration testがnamespaceとpeerのlowercase正規化を一つのcaseで検査している。T01とT02を独立したred/greenにするため、観測結果を変えずtest caseを先に分離する。
- verification: 分離前後で既存lowercase正規化testとPython fixture round-tripを成功させる。

canonical uppercase検査は既存address parserへ置き、profile model、nested `BTreeMap`、serde deriveを
変更しない。key typeの`Deserialize`は形式検査とcanonical表記検査だけを行い、文字列変換をしない。

## 8. 対象ファイル

| path | change | 内容 |
|---|---|---|
| `crates/swbt-core/src/profile/document.rs` | modify | namespace / peer keyのcanonical uppercase検査、正規化削除、rustdoc更新 |
| `crates/swbt-core/tests/profile_compat.rs` | modify | namespace / peerの受理契約分離とPython fixture回帰 |
| `src/lib.rs` | modify | crate-level profile入力契約を同期 |
| `spec/initial/architecture.md` | modify | canonical uppercase入力境界を明記 |
| `spec/initial/testing.md` | modify | 非canonical key拒否と互換testを明記 |
| `spec/initial/roadmap.md` | modify | M6 profile契約をcanonical uppercaseへ同期 |
| `spec/wip/unit_021/PROFILE_KEY_CANONICAL_UPPERCASE.md` | new / modify | Intent Delta、TDD状態、検証結果 |

## 9. 検証

| command | result | notes |
|---|---|---|
| `cargo test -p swbt-core typed_profile_normalizes_bluetooth_addresses_to_uppercase --locked` | success | 着手前baseline。lowercase namespace / peerを同時に正規化する既存test |
| `cargo test -p swbt-core --test profile_compat typed_profile_writes_deterministic_python_json --locked` | success | 着手前baseline。pinned Python fixture 6件 |
| itemごとのfocused `cargo test` | not run | T01 / T02 red-greenで実行する |
| `cargo test -p swbt-core --all-targets --locked` | not run | profile core回帰 |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | not run | workspace lint gate |
| `cargo test --workspace --all-targets --all-features --locked` | not run | workspace回帰 |
| `cargo fmt --all --check` | not run | Rust formatting |
| `git diff --check` | not run | whitespace検査 |
| hardware / USB / Switch UI | not run | parser入力契約だけの変更であり対象外 |

## 10. 先送り事項

- none

## 11. チェックリスト

- [x] 対象範囲と対象外を確認した
- [x] TDD Test Listを更新した
- [ ] T01 / T02をitem単位でred / green / refactor / commitした
- [ ] pinned Python 0.6.0 fixture 6件の互換性を確認した
- [ ] public rustdocと`spec/initial`を現行契約へ同期した
- [ ] 検証結果または未実行理由を記録した
- [x] package / release / public APIに触れる場合のgateを記録した
- [ ] docs-quality-reviewとagentic-self-reviewを完了した
- [ ] `spec/complete/unit_021`へ移動した
