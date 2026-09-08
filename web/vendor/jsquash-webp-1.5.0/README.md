# WebPエンコーダー

`@jsquash/webp` 1.5.0 の非SIMDエンコーダーと既定設定を無改変で同梱する。
SIMD検出・デコーダー・npm依存は不要なため含めない。アプリ側のWorkerから直接初期化する。

- 上流: https://github.com/jamsinclair/jSquash
- 配布物: https://registry.npmjs.org/@jsquash/webp/-/webp-1.5.0.tgz
- 配布物SHA-512（Base64）: `KggLoj2MnRSfIqTeKe1EmbljTX2vuV7mh79k89PCL1pyqiDULcPM1L47twxXt0hkb68F70bXiL31MxsuoZtKFw==`
- `webp_enc.js` / `webp_enc.wasm`: 配布物の `codec/enc/` から取得
- `meta.js` / `LICENSE`: 配布物の直下から取得
- `LICENSE.codec.md`: 配布物の `codec/` から取得

更新時は配布物の整合性を確認し、ディレクトリ名とWorkerの参照先を更新する。
ライセンスは `LICENSE`（Apache-2.0）と `LICENSE.codec.md`（libwebp）を参照。
