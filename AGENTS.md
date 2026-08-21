# Web AITuber 作業ルール

## 実運用サーバーの起動

- `target/release/web-aituber.exe`はLAN上のTTSエンジンへ接続するため、サンドボックスから起動しない。必ず権限昇格した`Start-Process`を使い、`-WindowStyle Hidden`とワークスペースの`-WorkingDirectory`を指定する。
- 再起動後は、管理画面のTTS試聴が成功することを確認してから完了とする。
