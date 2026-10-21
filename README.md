# Polaris

Polaris は、VRChat の終了を検知してゲームログをローカルへコピーする Windows 向けバックグラウンドアプリケーションです。画面、WebView、SQLite、外部 API、認証は使用しません。

文書の実装照合日: 2026-10-22。利用・保守の参照先は [Documentation](#documentation) を参照してください。

## Features

- **Process monitoring** — 1秒間隔で `vrchat.exe` を検出し、終了または PID の変更を検知して3秒後にバックアップを開始します。
- **Startup backup** — Polaris 起動時に VRChat が動いていない場合、残っているログをバックアップします。
- **Log backup** — `output_log_*.txt` のうち0 byteでないファイルを `Data/archive` へコピーします。同名・同サイズの既存コピーは省略し、サイズが変わったログは再コピーします。
- **Temporary copy** — `.polaris_tmp` へ書き込み、コピー結果のサイズを確認してから最終名へ変更します。
- **Single instance** — `Local\Polaris_SingleInstance` によって同じ Windows session での多重起動を防ぎます。
- **Automatic startup** — インストーラーが現在の利用者の Run key に Polaris を登録します。

## Architecture

```text
VRChat の終了 / PID 変更
└─ Polaris.exe                       Rust の常駐 process
   ├─ Win32 ToolHelp API             process の列挙
   ├─ filesystem                     元ログのコピー
   ├─ Windows Registry               インストール先・自動起動
   └─ Data/archive/output_log_*.txt   StellaRecord が同期元として参照
```

Tauri CLI と bundler はビルド・NSIS配布に使います。実行時の入口は `src-tauri/src/main.rs` で、Tauri runtime、画面、IPC command は起動しません。

process・moduleの責務、起動から終了までの経路は [Runtime Architecture](docs/tech-stack.md#runtime-architecture) を参照してください。

## Tech Stack

| Layer | Technology |
| --- | --- |
| Runtime | Rust、edition 2021、`stable` toolchain |
| Process monitoring | `windows 0.58.0`、Win32 ToolHelp API |
| Logging | `tracing 0.1.44`、`tracing-subscriber 0.3.23` |
| Registry | `winreg 0.52.0` |
| Packaging | Tauri CLI `2.10.1`、NSIS、current-user install |

版番号は現在の lockfile の解決値です。宣言値とビルド用依存は [Tech Stack](docs/tech-stack.md) を参照してください。

## Requirements

### Runtime

- Windows x64
- 同じ Windows user の profile に VRChat のログフォルダが存在すること
- インストーラーが登録した `InstallLocation` と、その場所への読み書き権限

### Development

- Node.js と npm
- Rust `stable`、`x86_64-pc-windows-msvc` target、Windows の MSVC 開発環境

Frontend がないため、Vite・React は不要です。利用者向けの WebView も生成しません。

## Installation

NSIS インストーラーを実行します。既定のインストール先は `%LOCALAPPDATA%\CosmoArtsStore\Polaris` で、Program Files と Windows directory 配下は拒否します。

インストールとアンインストールの hook は、実行中の Polaris を `taskkill /F` で終了させます。通常のアンインストールではアプリ本体、運用ログ、製品 registry、自動起動登録を削除し、`Data/archive` のログコピーを残します。`/UPDATE` を指定した削除では運用ログ・Run登録・製品keyの通常削除を省略します。詳細は [Installation Lifecycle](docs/tech-stack.md#installation-lifecycle) を参照してください。

直接 exe を起動する場合もインストール先の registry が必要です。未登録の開発 exe が動作することを前提にしないでください。

## Build and Test

```powershell
# lockfile どおりに Tauri CLI を導入
npm ci --ignore-scripts

# NSIS インストーラーを生成
npm run tauri build

# Rust の検証
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

ビルド成果物は `target/release/bundle/nsis/` に生成されます。コマンドは repository root で実行します。開発前提と各検証の対象は [Tech Stack](docs/tech-stack.md#verification) を参照してください。

## Data and Privacy

| Data | Location | Purpose |
| --- | --- | --- |
| Source logs | `%USERPROFILE%\AppData\LocalLow\VRChat\VRChat\output_log_*.txt` | VRChat の原本 |
| Archived logs | `<install>\Data\archive\output_log_*.txt` | 非圧縮コピー |
| Temporary files | `<install>\Data\archive\*.polaris_tmp` | コピー完了前の一時データ |
| Application log | `<install>\Data\logs\info.log` | Debug は DEBUG 以上、Release は ERROR 以上 |
| Install registry | `HKCU\Software\CosmoArtsStore\Polaris` | `InstallLocation` |
| Startup registry | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` | `Polaris` の起動 command |

元ログは削除しません。同サイズ判定は本文一致の検証ではありません。サイズが異なる既存コピーは新しいコピーの置換成功まで保持します。保存手順、各データの所有者、削除・保持範囲は [Data Storage](docs/database.md) を参照してください。

StellaRecord と Alpheratz は、Polaris の `InstallLocation/Data/archive` をログの読取り先にします。姉妹アプリとの共有はファイルとregistryによるもので、Polarisが相手のprocessやDBを操作することはありません。

## Project Structure

```text
.
├─ src-tauri/
│  ├─ src/main.rs            process 監視・コピー・registry・logger
│  ├─ windows/               NSIS template・hook
│  └─ tauri.conf.json        headless build・配布設定
├─ Cargo.toml / Cargo.lock   workspace・Rust 依存
├─ rust-toolchain.toml       stable toolchain・Windows target
├─ package.json              Tauri CLI command
├─ docs/                     現行の公開文書
└─ AGENT.md                  project-specific routing
```

## Documentation

| Document | Scope |
| --- | --- |
| [Specification](docs/spec.md) | 利用・仕様確認: 監視、起動、コピー、失敗・再試行の結果 |
| [Runtime Architecture](docs/tech-stack.md#runtime-architecture) | 保守: process、module責務、状態所有、実行・終了の経路 |
| [Data Storage](docs/database.md) | 保存・保全: filesystem、registry、置換、削除・保持。SQLiteは使用しない |
| [Tech Stack](docs/tech-stack.md) | 開発・配布: 宣言値・解決値、設計判断、command、NSISの導入・更新・削除 |

## License

Proprietary — Copyright (c) CosmoArtsStore. All rights reserved.
