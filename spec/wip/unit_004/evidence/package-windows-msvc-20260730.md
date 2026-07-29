# M3 package evidence: Windows MSVC

## 1. 測定条件

- 測定日: 2026-07-30
- code baseline: `ae8e87f test(transport): verify CSR adapter lifecycle`
- host target: `x86_64-pc-windows-msvc`
- Rust: `rustc 1.87.0 (17067e9ac 2025-05-09)`
- Cargo: `cargo 1.87.0 (99624be9 2025-05-06)`
- Bumble fork revision: `48f1bc36169b2692d2a61e87eda4223b126dca2b`
- package target: library `swbt`。binary target はない
- `Cargo.lock` SHA-256:
  `1B5C4504519933A22B78C8B2CABBAB112A26AF8CE360C3559385DBF7EFEE9BE9`

## 2. Quality gate

| command | result |
|---|---|
| `cargo +1.87.0 check --all-targets --all-features --locked` | success |
| `cargo +1.87.0 test --all-targets --all-features --locked` | lib 236 passed / 2 ignored、hardware target 5 ignored、integration/example 全件成功 |
| `cargo +1.87.0 build --all-features --locked` | success |
| `cargo +1.87.0 build --locked` | success |
| `cargo +1.87.0 build --no-default-features --locked` | success |
| `RUSTDOCFLAGS="-D warnings" cargo +1.87.0 doc --all-features --no-deps --locked` | success |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | success |
| `cargo clippy --all-targets --locked -- -D warnings` | success |
| `cargo fmt --check` / `git diff --check` | success |

ignored hardware test の実行結果は
[`M3_BUMBLE_EXTERNAL_HCI.md`](../M3_BUMBLE_EXTERNAL_HCI.md) の T09 evidence に分けて記録した。

## 3. Clean release build

既存の `target/` は削除せず、空の `CARGO_TARGET_DIR` を構成ごとに作成した。registry と git
source cache は共用している。Rust 1.87.0 で `--release --locked --offline` を一回ずつ実行し、
PowerShell の wall time と生成された `swbt` rlib の file size を測った。測定後に一時
directory は削除した。

| configuration | command | wall time | Cargo reported | rlib size |
|---|---|---:|---:|---:|
| default | `cargo +1.87.0 build --release --locked --offline` | 3.53 s | 3.47 s | 2,580,250 bytes |
| all features | `cargo +1.87.0 build --release --all-features --locked --offline` | 21.71 s | 21.64 s | 4,028,046 bytes |

all-features はこの一回の測定で wall time 6.15 倍、rlib は 1,447,796 bytes、56.1% 増だった。
これは current Windows host の clean compile 記録であり、複数回の統計や CI、別 host の
測定ではない。package に binary target がないため binary size は該当しない。

## 4. License inventory

`cargo-deny` は host に導入されておらず、`cargo deny check` は `no such command: deny` で
実行不能だった。新しい global tool や project dependency は追加せず、次の metadata inventory
を代替 report とした。

```powershell
cargo metadata --offline --all-features --locked `
  --filter-platform x86_64-pc-windows-msvc --format-version 1
```

resolved Windows graph は 211 package、license metadata 欠落は 0 package だった。`Cargo.lock`
全体は 236 package を含むが、この report は Windows target で解決される package だけを
集計している。

| package count | declared license expression |
|---:|---|
| 1 | `(MIT OR Apache-2.0) AND Unicode-3.0` |
| 27 | `Apache-2.0` |
| 1 | `Apache-2.0 / MIT` |
| 1 | `Apache-2.0 AND ISC` |
| 2 | `Apache-2.0 OR ISC OR MIT` |
| 25 | `Apache-2.0 OR MIT` |
| 1 | `BSD-3-Clause` |
| 2 | `ISC` |
| 48 | `MIT` |
| 1 | `MIT AND BSD-3-Clause` |
| 91 | `MIT OR Apache-2.0` |
| 7 | `MIT/Apache-2.0` |
| 1 | `MPL-2.0` |
| 3 | `Unlicense OR MIT` |

Bumble fork 由来の21 package はすべて `Apache-2.0` を宣言している。MIT / Apache-2.0 系以外を
含む package は次のとおり。

| package | version | declared license |
|---|---:|---|
| `unicode-ident` | 1.0.24 | `(MIT OR Apache-2.0) AND Unicode-3.0` |
| `fnv` | 1.0.7 | `Apache-2.0 / MIT` |
| `ring` | 0.17.14 | `Apache-2.0 AND ISC` |
| `rustls` | 0.23.42 | `Apache-2.0 OR ISC OR MIT` |
| `rustls-native-certs` | 0.8.4 | `Apache-2.0 OR ISC OR MIT` |
| `subtle` | 2.6.1 | `BSD-3-Clause` |
| `rustls-webpki` | 0.103.13 | `ISC` |
| `untrusted` | 0.9.0 | `ISC` |
| `matchit` | 0.8.4 | `MIT AND BSD-3-Clause` |
| `serialport` | 4.9.0 | `MPL-2.0` |
| `aho-corasick` | 1.1.4 | `Unlicense OR MIT` |
| `byteorder` | 1.5.0 | `Unlicense OR MIT` |
| `memchr` | 2.8.3 | `Unlicense OR MIT` |

declared metadata に GPL、AGPL、SSPL、license 不明 package はない。`serialport` の `MPL-2.0`
を含む notice/source compliance の最終確認と、機械的な deny policy / SBOM は M9 の release
evidence で行う。この inventory は法的判断や source file 単位の license 監査を置き換えない。

## 5. 未検証範囲

- Windows 以外の target graph は集計していない。platform filter なしの offline metadata は、
  未取得の target-specific `atomic-polyfill 1.0.3` を要求して停止した。
- `cargo-deny` による license allow/deny、advisory、duplicate/source policy は未実行。
- build time は各構成一回で、benchmark としての分布はない。
