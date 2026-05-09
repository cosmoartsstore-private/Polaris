# Polaris

VRChat のログファイルを自動バックアップするバックグラウンドサービス。VRChat の終了を検知し、ログファイルをローカルアーカイブフォルダへコピーします。

## CosmoArtsStore アプリ連携

```
VRChat 終了 → [Polaris] ログを archive/ へコピー → [StellaRecord] .tar.zst に圧縮・解析・閲覧
```

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Runtime | Rust (Edition 2021), Tauri v2 (headless / フロントエンドなし) |
| Process Monitoring | Win32 ToolHelp API (CreateToolhelp32Snapshot) |
| Logging | tracing + tracing-subscriber (ファイル出力) |
| Registry | winreg |
| Installer | NSIS |

## Features

- **VRChat プロセス監視** — 1 秒間隔のポーリングで VRChat の起動・終了を検知 (EAC 安全)
- **自動バックアップ** — VRChat 終了時にログファイル (`output_log_*.txt`) をアーカイブフォルダへコピー
- **起動時バックアップ** — サービス起動時に未アーカイブのログを即座にバックアップ
- **冪等コピー** — アーカイブ済みファイルはスキップ
- **単一インスタンス制御** — Global Mutex による多重起動防止
- **自動起動** — NSIS インストーラが `HKCU\...\Run` に登録

## Project Structure

```
src-tauri/
  ├── src/main.rs       # 全ロジックを含む単一バイナリ
  ├── windows/          # NSIS インストーラスクリプト・フック
  └── icons/            # アプリアイコン
package.json            # Tauri CLI 設定
```

## Getting Started

### Prerequisites

- [Node.js](https://nodejs.org/) (v18+)
- [Rust](https://rustup.rs/)
- [Tauri CLI](https://tauri.app/)
- Windows 11 (動作確認済み)

### Build

```bash
npm install
npm run tauri build
```

> **Note:** Polaris はフロントエンドを持たないヘッドレスアプリです。ビルド後の exe を直接実行してテストします。

## Data Layout (Installed)

```
$INSTDIR/
  └── Data/
      ├── archive/      # バックアップされた VRChat ログ (アンインストール時も保護)
      └── logs/         # info.log
```

## Settings Storage

```
HKCU\Software\CosmoArtsStore\Polaris
```

## Development Notes

- `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` で unwrap / expect / panic を禁止
- Windows サブシステムによりコンソールウィンドウは非表示
- ログ・エラーメッセージは日本語

## License

Proprietary — CosmoArtsStore
