# Web AITuber

イベント来場者の質問を受け付け、VRMキャラクターがLLMとTTSで回答するローカルサーバーです。各端末はCloudflare Tunnelの公開URLへブラウザから接続します。

## 配布版の実行に必要なもの

- FFmpeg（PATHに追加するか、設定で実行ファイルの絶対パスを指定）
- VOICEVOXまたはAivisSpeech Engine
- テキスト・画像入力・Web検索対応のOpenAI Responses API
- `assets/model.vrm`
- `assets/motions/*.vrma`（待機・感情モーション）

配布版の利用にRustは不要です。ソースからビルドする場合だけRust 1.85以降が必要です。

## 設定

1. `config.example.json`を`config.json`としてコピーする。
2. LLM、TTS、管理用トークン、キャラクター設定を編集する。
3. VRMとVRMAを設定したパスへ配置する。

`character.food_prop`には、食事投稿の画像を表示するQuadの位置、回転、大きさをVRMの右手ローカル座標で指定します。使用するモデルに合わせてイベント前に調整してください。

`llm.api_url`にはResponses APIエンドポイントの完全なURLを指定します。OpenAIでは`https://api.openai.com/v1/responses`です。`llm.food_reaction_prompt`には食事投稿だけに追加する感想の書き方を指定します。`llm.search_fillers`にはWeb検索を開始した場合だけ順番に読み上げる短い文の一覧を指定します。`ffmpeg_path`は通常`ffmpeg`のままで構いません。PATHにない場合はFFmpeg実行ファイルの絶対パスを指定してください。

起動後は管理画面から、LLM APIのURL、モデル名、通常・食事用プロンプト、検索中フィラー、TTSエンジンURL、使用する話者・スタイルを編集して保存できます。保存した内容は次に処理を開始する投稿から反映されます。APIキーと管理用トークンは管理画面へ表示されないため、`config.json`で管理してください。

Web検索の使用はLLMが質問内容から判断します。検索時も最終回答は1回だけ表示・読み上げし、回答末尾の地球儀アイコンから出典URLを確認できます。

AivisSpeechを使用する場合、通常の接続先は`http://127.0.0.1:10101`です。`speaker_id`には、AivisSpeech Engineの`GET /speakers`で確認できる使用モデル・スタイルのIDを指定します。

設定ファイルを別の場所に置く場合は、環境変数`APP_CONFIG_FILE`へそのパスを指定します。

## 配布版の起動

Windowsでは次を実行します。

```powershell
.\web-aituber.exe
```

Linuxでは次を実行します。

```bash
./web-aituber
```

## ソースからの起動

```text
cargo run --release
```

## 接続先

- メイン画面: `http://127.0.0.1:3000/`
- 簡易入力画面: `http://127.0.0.1:3000/input`
- 食べ物の描画画面: `http://127.0.0.1:3000/draw`
- 管理画面: `http://127.0.0.1:3000/admin?token=設定した管理用トークン`

メイン画面では最初に「表示・音声を開始」を押します。メイン画面と簡易入力画面は認証なしで複数端末から利用できます。管理画面では運用状態の確認と中断、AI・音声設定の編集、TTSの話者一覧取得と試聴ができます。管理画面のURLは管理端末だけで使用します。

## ローカルネットワークからの接続

サーバーPCとスマートフォンを同じWi-Fiへ接続し、Windowsでは`ipconfig`、Linuxでは`ip address`でサーバーPCのIPv4アドレスを確認します。IPv4アドレスが`192.168.1.20`の場合、スマートフォンから次のURLを開きます。

- メイン画面: `http://192.168.1.20:3000/`
- 簡易入力画面: `http://192.168.1.20:3000/input`
- 食べ物の描画画面: `http://192.168.1.20:3000/draw`

サーバーは`0.0.0.0:3000`で待ち受けます。OSのファイアウォールでは、会場で使用するネットワークに限りTCP 3000番への受信を許可してください。`0.0.0.0`は待受設定用のため、ブラウザのURLには使用しません。

イベント公開時は、CloudflareのNamed Tunnelの接続先を`http://127.0.0.1:3000`に設定します。公開URLの`/`または`/input`を来場者へ案内します。

## 確認

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
node --test tests/*.test.cjs
```

音声まで確認する場合は、TTS EngineとFFmpegを起動・配置した状態で質問を投稿してください。

## リリース

`v1.0.0`のようなバージョンタグをGitHubへpushすると、GitHub ActionsがWindows x64版とLinux x64版をビルドし、GitHub Releaseへ次のファイルを添付します。

- `web-aituber-windows-x64.zip`
- `web-aituber-linux-x64.tar.gz`

```powershell
git tag v1.0.0
git push origin v1.0.0
```

配布物に`config.json`、VRM・VRMA、FFmpeg、TTSエンジンは含まれません。
