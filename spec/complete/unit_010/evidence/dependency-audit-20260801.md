# M9 dependency audit

> 履歴: この記録は単一registry backendへ切り替える前のBumble Git dependency graphを対象とする。
> 現行0.1.0 candidateの判定には`dependency-package-audit-20260802.md`を使う。

## 条件

- 日付: 2026-08-01 JST
- package: `swbt-rs 0.1.0`
- lockfile: repository の `Cargo.lock`
- target: `x86_64-pc-windows-msvc`、`x86_64-unknown-linux-gnu`
- feature: all features
- cargo-deny: `0.20.2`
- cargo-cyclonedx: `0.5.9`
- CycloneDX specification: `1.5`

tool は `target/m9-tools` に版固定で一時導入した。global Cargo dependency と project dependency は
変更していない。

## dependency policy

`deny.toml` は次を release policy とする。

- RustSec advisory を ignore しない。
- unknown registry と unknown Git source を拒否する。
- registry は crates.io index、Git は `https://github.com/niart120/bumble-rs` だけを許可する。
- wildcard dependency を拒否する。
- MIT、Apache-2.0、Unicode-3.0、ISC、BSD-3-Clause、Unlicense を許可する。
- MPL-2.0 は `serialport@4.9.0` だけの例外とする。
- dependency の複数版は可視化するが、この unit では警告とする。主な重複は
  `bumble-transport` の audio、gRPC、WebSocket を含む広い graph に由来する。

実行結果:

```text
advisories ok, bans ok, licenses ok, sources ok
```

Windows resolved graph は root を含む221 packages、Linux は223 packages だった。license metadata
欠落は両 target とも0、Git source は両 target とも Bumble fork 由来22 packagesである。

`serialport@4.9.0` は `MPL-2.0` を宣言し、registry source に `LICENSE.txt` を含む。
`swbt-rs` は `serialport` source を vendoring または改変して crate package へ再配布しない。
この機械検査は法的判断や source file 単位の監査を置き換えない。

## SBOM

`SOURCE_DATE_EPOCH` を source commit timestamp に設定し、target ごとに次を生成した。

```powershell
cargo cyclonedx --all-features --target x86_64-pc-windows-msvc --format json --spec-version 1.5
cargo cyclonedx --all-features --target x86_64-unknown-linux-gnu --format json --spec-version 1.5
```

| file | dependency components | license 欠落 | SHA-256 |
|---|---:|---:|---|
| `swbt-rs-0.1.0-windows.cdx.json` | 220 | 0 | `D646DADDC260A0E9AF89354C849969FA3C4FEFF4F9C966894B40CB3D03A111A1` |
| `swbt-rs-0.1.0-linux.cdx.json` | 222 | 0 | `D22ED5CD3FE5825544CF617F88E46A34A585F2C20AA24AFC549E34359682AF6D` |

各 SBOM の root component は `swbt-rs 0.1.0` である。Bumble 22 components の purl はすべて
fork URL と revision `b8c7cd625bc2ac2f58a4beb4ade1264426969819` を含む。profile path、Bluetooth
address、link key、USB serial、実機 trace は含まない。

## 判定

- advisory: pass as of 2026-08-01
- license metadata/policy: pass
- source allowlist: pass
- SBOM schema/content inspection: pass
- duplicate dependencies: warning、release size/build-time limitationとして維持
