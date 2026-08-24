# Web AITuber

来場者の質問に、VRMキャラクターがLLMと音声で回答するイベント向けWebアプリです。LANまたは公開URLから複数端末で利用できます。

## 必要なもの

- OpenAI APIキー
- VOICEVOXまたはAivisSpeech Engine
- FFmpeg
- `assets/model.vrm`
- `assets/motions/*.vrma`

配布版の利用にRustは不要です。ソースから起動する場合はRust 1.85以降が必要です。

## セットアップ

`config.json`がない場合は、初回起動時に`config.example.json`から自動生成します。管理用トークンとイベント識別子はランダム値になります。

1. 初回起動後、`config.json`の`public_base_url`、`llm.api_key`、`tts.engine_url`などを環境に合わせて編集し、サーバーを再起動する。
2. VRMを`assets/model.vrm`、VRMAを`assets/motions/`へ配置する。
3. TTSエンジンを起動し、FFmpegをPATHへ追加する。

APIキーと管理用トークンは管理画面に表示されません。`config.json`で管理してください。

## 起動

配布ファイルを展開したフォルダ、またはリポジトリのルートで実行します。

- Windows: `.\web-aituber.exe`
- Linux: `./web-aituber`
- ソースから起動: `cargo run --release`

## 画面

| 用途 | URL |
| --- | --- |
| メイン画面 | `http://127.0.0.1:3000/event/イベント識別子` |
| 簡易入力 | `http://127.0.0.1:3000/event/イベント識別子/input` |
| 食べ物の描画 | `http://127.0.0.1:3000/event/イベント識別子/draw` |
| 管理画面 | `http://127.0.0.1:3000/admin?token=管理用トークン` |

メイン画面では、最初に「表示・音声を開始」を押します。

管理画面では、公開イベントリンクのコピーとQRコード表示、回答の中断、LLM・TTS設定、ユーザー辞書、背景画像、画面オーバーレイ、BGMを管理できます。管理画面のURLは共有しないでください。

## 他の端末から使う

同じLANでは、URLの`127.0.0.1`をサーバーPCのIPv4アドレスへ置き換えます。接続できない場合は、ファイアウォールでTCP 3000番を許可してください。

Cloudflare Tunnelを使う場合は、接続先を`http://127.0.0.1:3000`にします。来場者へは管理画面に表示されたイベント別URLを案内します。

## デバッグ表示

メイン画面のURLに`?debug`を付けます。

```text
http://127.0.0.1:3000/event/イベント識別子?debug
```

`debug`パラメータは値に関係なく有効です。

デバッグ中は、次の情報を表示します。

- WebSocketの接続状態
- 再生中のモーションファイル名と種別
- 要求表情とVRMの対応状況
- 食事動作の状態
- 食事画像アンカーの座標軸と枠

通常運用では`?debug`を付けません。`character.food_prop`を変更した場合は、管理画面で設定を再読み込みしてからメイン画面を再読み込みしてください。

## 開発

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
node --test tests/*.test.cjs
```

`v1.0.0`のようなタグをpushすると、Windows版とLinux版のGitHub Releaseを作成します。配布物に`config.json`、VRM・VRMA、FFmpeg、TTSエンジンは含まれません。

## systemdで起動する場合

systemdなどの外部プロセス管理下では、管理画面からの自己更新は利用できません。アプリケーションによる再起動とsystemdの自動再起動が競合するためです。更新時はサービスを停止し、`config.json`と`assets/`を残して新しい配布物へ置き換えてから起動してください。

```bash
sudo systemctl stop web-aituber
# 実行ファイル、web/、web-aituber-updater、config.example.json、README.mdを新しい配布物へ置き換える
sudo systemctl start web-aituber
```

systemdのユニットでは、配布物を展開したフォルダを`WorkingDirectory`、その中の`web-aituber`を`ExecStart`に指定してください。`systemctl stop`ではSIGTERMを受けて正常終了します。
