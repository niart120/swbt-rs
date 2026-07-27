# Quality Gates

## Local Gate

通常の変更:

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --all-features
git diff --check
```

Cargo metadata / release / public API を触る変更:

```powershell
cargo package
```

## 判定

- command と結果を PR 本文または work unit に記録する。
- 未実行の gate は `not run` とし、理由を書く。
- 対象外の gate は `not applicable` とし、なぜ対象外かを書く。
- warning を確認済みとして握りつぶさない。
- README、docs、spec、skill、PR 本文を変更した場合は `$docs-quality-review` で文言、置き場所、根拠を確認する。
- 公開 API、所有権、error、async、feature、unsafe を変更した場合は `$rust-api-boundary-review` で境界を確認する。
- 公開 API の rustdoc、README/docs の API 説明、doctest を変更した場合は `$rustdoc-style` で契約と文言を確認する。
