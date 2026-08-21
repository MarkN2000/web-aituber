# 実装方針

## ファイル構成

開始時点では以下を目安とする。

```text
.github/
└─ workflows/
   └─ release.yml
Cargo.toml
config.example.json
src/
├─ main.rs
├─ config.rs
├─ state.rs
├─ protocol.rs
├─ routes.rs
├─ pipeline.rs
├─ llm.rs
├─ tts.rs
└─ audio.rs

web/
├─ input.html
├─ draw.html
├─ main.html
├─ admin.html
├─ style.css
├─ draw.css
├─ history.css
└─ js/
   ├─ input.js
   ├─ draw.js
   ├─ sources.js
   ├─ main.js
   ├─ admin.js
   ├─ history.js
   ├─ audio-queue.js
   ├─ vrm-viewer.js
   └─ motion.js

assets/
├─ model.vrm
└─ motions/
   ├─ idle.vrma
   └─ その他の待機・感情モーション
```

実際の設定は、`config.example.json`を複製した`config.json`に記載する。`config.json`にはAPIキーと管理用トークンを含むため、Gitの管理対象にしない。管理画面はLLMとTTSの運用中に変更する項目だけを取得・保存し、APIキー、管理用トークン、待受設定、FFmpegパス、キャラクター設定をブラウザへ返さない。

管理画面からの設定保存は、読み込み元の設定ファイルを改めて読み、編集可能項目だけを適用して検証する。これにより、画面に出さない項目の外部変更を古い実行中設定で上書きしない。設定ファイルと同じディレクトリの一時ファイルへ全体を書き込んで同期した後、既存ファイルのアクセス権を保ったまま原子的に置換する。Linuxでは同一ファイルシステム内のrename、Windowsでは既存ファイルを置換できるOS APIを使用する。保存に成功してから実行中の設定を切り替え、保存とファイルからの再読み込みは直列に処理する。

バックエンドはHTTP・WebSocketから `pipeline` を呼び出し、`pipeline` が `llm`、`tts`、`audio` の処理順序を管理する。フロントエンドの `main` は、音声再生、VRM表示、モーションを各モジュールへ委譲する。

## 分割方針

- 1ファイルには基本的に一つの役割を持たせる。
- 異なる理由で変更される処理が混ざった場合に分割する。
- 同じ処理が実際に重複するまで汎用化しない。
- 行数のみを理由にファイルを分割しない。
- 必要性が確認されていない抽象化、設定、互換処理は追加しない。

ファイル構成とコード量は目安であり、固定の制約とはしない。全体は数千行程度を想定し、規模が増えた場合は仕様にない機能や抽象化が加わっていないか確認する。

## 外部ツールとアセット

- サーバーは`0.0.0.0:3000`で待ち受け、同一ローカルネットワークとCloudflare Tunnelの両方から接続できるようにする。
- WAVからWebM/Opusへの変換にはFFmpegを使用する。
- VRMモデルは `assets/model.vrm` に配置する。
- VRMAは `assets/motions/` に配置する。
- メイン画面は、表示開始ボタンが押された後に音声再生とAudioContextを開始する。
- LLMはResponses APIのWeb検索開始イベントだけをフィラー生成の合図にし、最終回答は1回だけ処理する。
- 食事投稿はWeb検索と会話履歴を使用せず、画像の感想生成だけをLLMへ依頼する。
- 食事投稿はVRM表示用の256×256px透過WebPとAI入力用の128×128px白背景WebPを受け付け、前者だけをメモリ内の一時URLで配信してVRMの右手へ追加したQuadへ貼り付け、後者だけをLLMへ送信する。
