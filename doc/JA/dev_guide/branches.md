# ブランチの命名と運用方針

* 基本的に開発は`main`ブランチ一本で行う(モノレポ開発)。どうしてもブランチを切らないと作業しにくい場合のみ`feature-*`ブランチか`issue-*`ブランチを作成する。

## main

* メイン開発ブランチ
* 以下の条件を満たす必要がある

* コンパイルが成功する

## beta (現在のところは作らない)

* 最新のベータリリース
* 以下の条件を満たす必要がある

* コンパイルが成功する
* 全てのテストが成功する

## feature-*

* 特定の一機能を開発するブランチ
* mainを切って作る

* 条件なし

## issue-*

* 特定のissueを解決するブランチ

* 条件なし
