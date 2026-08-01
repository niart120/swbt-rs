# M9 registry backend dependency / package audit

## 条件

- 日付: 2026-08-02 JST
- branch: `feat/unit-010-release-readiness`
- package: `swbt-rs 0.1.0`
- archive source commit: `6e27b0d6b8680330e1677a0cabf8e78f9a2bc0e6`
- `Cargo.lock` SHA-256:
  `40109791FB91C479AF355F4B1A07F59A3E7F3680F35C8E5CF0E311A3D021629F`
- target: `x86_64-pc-windows-msvc`、`x86_64-unknown-linux-gnu`
- feature: all features
- cargo-deny: `0.20.2`
- cargo-cyclonedx: `0.5.9`
- CycloneDX specification: `1.5`

検査toolは`target/m9-tools`へ版固定で一時導入した。project dependencyとglobal Cargo
installationは変更していない。

## red

2026-08-01のdependency evidenceは、22個のBumble Git componentを含むWindows 220 / Linux 222
dependency componentsを記録していた。現行`Cargo.lock`にはGit sourceがなく、registryの
`swbt-bumble-backend@0.1.1`だけを解決するため、旧SBOMとlicense/source判定を現行release candidateの
証拠には使えなかった。

`deny.toml`にも旧graph専用のBumble Git allowlist、`serialport@4.9.0`のMPL-2.0例外、未使用のISC
許可が残っていた。初回`cargo-deny`はpolicy自体をpassしたが、後二者を未使用として警告した。

## dependency policy

旧graph専用のallowlistと例外を削除した。現行policyは次を検査する。

- RustSec advisoryをignoreしない。
- unknown registryとすべてのGit sourceを拒否する。
- registryはcrates.io indexだけを許可する。
- wildcard dependencyを拒否する。
- 現行graphで使用するMIT、Apache-2.0、Unicode-3.0、BSD-3-Clause、Unlicenseを許可する。

`target\m9-tools\bin\cargo-deny.exe --locked check`は警告なしで
`advisories ok, bans ok, licenses ok, sources ok`となった。license metadataに基づく機械検査であり、
法的判断やsource file単位の監査を置き換えない。

## package archive

| 検査 | 結果 |
|---|---|
| `cargo package --locked --list` | 120 files。`spec/`、`tools/`、`.agents/`、`.codex/`、`.github/`、`target/`、実機traceを含まない |
| `cargo package --locked --allow-dirty` | archive生成とverification build成功。1.4 MiB、圧縮258.0 KiB |
| archive SHA-256 | `70631EBE47592C517C646B3C029BDCE927919C313D81E4081FE5DFED91819D06` |
| 展開archiveのMSRV offline test | `cargo +1.87.0 test --all-targets --all-features --locked --offline --quiet`成功。library 271 passed / 1 ignored、hardware 5 ignored、その他target成功 |

archive内の`.cargo_vcs_info.json`は上記source commitを記録する。このhashはT11時点の監査値であり、
後続の公開文書変更を含む最終candidate archiveのhashではない。最終gateではclean branchからarchiveを
再生成する。

## SBOM

source commit timestampを`SOURCE_DATE_EPOCH=1785605818`としてtargetごとに生成した。

```powershell
target\m9-tools\bin\cargo-cyclonedx.exe cyclonedx --all-features `
  --target x86_64-pc-windows-msvc --format json --spec-version 1.5 `
  --override-filename swbt-rs-0.1.0-windows-registry.cdx
target\m9-tools\bin\cargo-cyclonedx.exe cyclonedx --all-features `
  --target x86_64-unknown-linux-gnu --format json --spec-version 1.5 `
  --override-filename swbt-rs-0.1.0-linux-registry.cdx
```

`cargo-cyclonedx`がroot componentへ埋め込むworkstation pathは、
`tools/normalize-cyclonedx.ps1`で`pkg:cargo/swbt-rs@0.1.0`とtarget subpathへ正規化した。同toolは
CycloneDX 1.5、local source文字列の不在、dependency referenceの整合を検査してからUTF-8 JSONを
保存する。

| file | dependency components | license欠落 | 未解決dependency ref | SHA-256 |
|---|---:|---:|---:|---|
| `swbt-rs-0.1.0-windows-registry.cdx.json` | 33 | 0 | 0 | `75E77D3342B56C6944C0E5995E528043C1A67129B65A093BF48B05D0EE313034` |
| `swbt-rs-0.1.0-linux-registry.cdx.json` | 34 | 0 | 0 | `98ECD0C13A66C7F3A7D3714A759C9AFECD0E7620D94EF3918705D0F3783D2486` |

両SBOMのrootは`swbt-rs 0.1.0`である。backend componentはregistry source、version `0.1.1`、
Apache-2.0、archive checksum
`1cc2c8d7d9c8cecfd203cd039fb3c3f8a9c39b072230f977b1e12e526b1bc667`を記録する。Git package source、
workstation path、profile、Bluetooth address、link key、USB serial、token、authorization値の一致は0件だった。

## 判定

- registry-only dependency source: pass
- advisory / license metadata / source policy: pass
- package file selection / archive verification: pass
- archive MSRV offline test: pass
- Windows / Linux SBOM structure、license metadata、秘密情報検査: pass
- production publish、tag、GitHub Release: 未実行。T11の対象外
