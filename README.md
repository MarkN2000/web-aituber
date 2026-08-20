# Web AITuber

イベント来場者の質問を受け付け、表示端末のVRMキャラクターがLLMとTTSで回答するローカルサーバーです。入力端末と表示端末は、Cloudflare Tunnelの公開URLへブラウザから接続します。

## 必要なもの

- Rust
- FFmpeg（PATHに追加するか、設定で実行ファイルの絶対パスを指定）
- VOICEVOXまたはAivisSpeech Engine
- テキスト・画像入力対応のOpenAI Chat Completions互換API
- `assets/model.vrm`
- `assets/motions/*.vrma`（待機・感情モーション）

## 設定

1. `config.example.json`を`config.json`としてコピーする。
2. LLM、TTS、表示用トークン、キャラクター設定を編集する。
3. VRMとVRMAを設定したパスへ配置する。

`llm.api_url`にはChat Completions互換エンドポイントの完全なURLを指定します。`ffmpeg_path`は通常`ffmpeg`のままで構いません。PATHにない場合は`ffmpeg.exe`の絶対パスを指定してください。

AivisSpeechを使用する場合、通常の接続先は`http://127.0.0.1:10101`です。`speaker_id`には、AivisSpeech Engineの`GET /speakers`で確認できる使用モデル・スタイルのIDを指定します。

設定ファイルを別の場所に置く場合は、環境変数`APP_CONFIG_FILE`へそのパスを指定します。

## 起動

```powershell
cargo run --release
```

- 入力画面: `http://127.0.0.1:3000/`
- 表示画面: `http://127.0.0.1:3000/display?token=設定した表示用トークン`

表示画面では最初に「表示開始」を押します。

イベント公開時は、CloudflareのNamed Tunnelの接続先を`http://127.0.0.1:3000`に設定します。公開URLの`/`を入力端末へ案内し、`/display?token=...`は表示端末だけで使用します。

## 確認

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
```

音声まで確認する場合は、TTS EngineとFFmpegを起動・配置した状態で質問を投稿してください。
