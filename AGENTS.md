# Web AITuber 作業ルール

## コミット
- 切りの良いところで定期的にコミットする。

## 実運用サーバーの起動

- `target/release/web-aituber.exe`はLAN上のTTSエンジンへ接続するため、サンドボックスから起動しない。必ず権限昇格した`Start-Process`を使い、`-WindowStyle Hidden`とワークスペースの`-WorkingDirectory`を指定する。

