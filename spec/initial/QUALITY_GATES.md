# Quality Gates

## Local Gate

通常の変更:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cargo build --workspace --all-features --locked
git diff --check
```

Cargo metadata / release / public API を触る変更:

```powershell
cargo package -p swbt-core --locked
cargo package -p swbt-rs --locked
```

`swbt-rs`が未公開版の`swbt-core`へ依存する変更では、core archiveの検証を先に完了する。
root packageのclean verificationがregistry上のcoreを要求して失敗する場合は、失敗結果と公開順序を
work unitへ記録し、`cargo package -p swbt-rs --list`で収録対象を検査する。`--no-verify`でも
registry解決が必要なため、root archiveの生成とverificationは`swbt-core`公開後、`swbt-rs`公開前に
必ず再実行する。

`publish = false` の作業単位で package artifact を完了条件に含めない場合も、
command は実行して結果を記録する。失敗は成功扱いにせず、公開を再び有効にする
作業単位の blocker として追跡する。`publish = false` を解除する前には
`cargo package` の成功を必須とする。

## 判定

- command と結果を PR 本文または work unit に記録する。
- 未実行の gate は `not run` とし、理由を書く。
- 対象外の gate は `not applicable` とし、なぜ対象外かを書く。
- warning を確認済みとして握りつぶさない。
- README、docs、spec、skill、PR 本文を変更した場合は `$docs-quality-review` で文言、置き場所、根拠を確認する。
- 公開 API、所有権、error、async、feature、unsafe を変更した場合は `$rust-api-boundary-review` で境界を確認する。
- 公開 API の rustdoc、README/docs の API 説明、doctest を変更した場合は `$rustdoc-style` で契約と文言を確認する。
