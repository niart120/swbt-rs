# セキュリティ方針

## 対象版

公開済みの版はまだない。`main` と未公開の 0.1.0 候補だけを調査対象とする。公開後は、支援中の
版をこの表へ追加する。

| 版 | 状態 |
|---|---|
| 未公開の `main` | 調査対象 |
| crates.io 公開版 | なし |

## 報告方法

この repository では GitHub Private Vulnerability Reporting を有効にしている。脆弱性は
[Report a vulnerability](https://github.com/niart120/swbt-rs/security/advisories/new) から非公開で
報告する。脆弱性の詳細を public issue、discussion、pull request、ログへ投稿しないこと。

GitHub の非公開報告画面を利用できない場合は、詳細を含めずに
[repository owner の GitHub profile](https://github.com/niart120) から連絡手段を確認する。機密性の
ない通常の不具合だけを public issue で報告する。

## 報告に含めない情報

profile JSON には Bluetooth address と link key が含まれる。raw profile、link key、USB serial、
Bluetooth address、未編集の packet trace を添付しないこと。再現に必要な場合も、非公開の報告先が
確立した後に、秘密値を除去した最小の情報だけを共有する。

## 対応範囲

調査では影響する controller model、reporting mode、OS、adapter、driver、再現手順を確認する。
修正後は protocol/profile 互換性、秘密値を出さない診断、関連する build/test を検査する。公開版が
存在する場合は、影響版、回避策、修正版を release note と advisory へ記録する。
