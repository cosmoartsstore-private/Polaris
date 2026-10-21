# Tech Stack and Architecture Decisions

Polaris の現行依存関係、process 構成、ビルド・配布契約を記録する。

## Document Control

| Field | Value |
| --- | --- |
| Application | Polaris |
| Scope | runtime構造、依存、設計判断、開発command、NSISの配布・更新・削除 |
| Audience | 構造を保守する開発者、開発環境・配布を管理する人 |
| Source review date | 2026-10-22 |
| Authority | [main.rs](../src-tauri/src/main.rs)、[package.json](../package.json)、[Cargo.toml](../src-tauri/Cargo.toml)、[Cargo.lock](../Cargo.lock)、[tauri.conf.json](../src-tauri/tauri.conf.json)、[installer.nsi](../src-tauri/windows/installer.nsi)、[hooks.nsi](../src-tauri/windows/hooks.nsi) |
| Related documents | [README](../README.md)、[Specification](spec.md)、[Data Storage](database.md) |

## Stack Reference

| Layer | Dependency | Declared version | Resolved version | Authority | Purpose |
| --- | --- | --- | --- | --- | --- |
| Runtime | Rust | edition 2021、`stable` | stable channel。patch固定なし | `src-tauri/Cargo.toml`、`rust-toolchain.toml` | 常駐process |
| Runtime | `windows` | `0.58` | `0.58.0` | `src-tauri/Cargo.toml`、`Cargo.lock` | Win32 process列挙・mutex・message box |
| Runtime | `winreg` | `0.52` | `0.52.0` | 同上 | HKCU読取り |
| Runtime | `dirs` | `6.0` | `6.0.0` | 同上 | user profile解決 |
| Runtime | `anyhow` | `1` | `1.0.102` | 同上 | runtimeエラー |
| Runtime | `tracing` | `0.1` | `0.1.44` | 同上 | ログ出力 |
| Runtime | `tracing-subscriber` | `0.3` | `0.3.23`、`fmt` | 同上 | file writer |
| Build graph | `tauri` | `2.2.4` | `2.10.3` | 同上 | crate依存。runtimeはBuilderを呼ばない |
| Build | `tauri-build` | `2.0.5` | `2.5.6` | 同上 | build-time設定処理 |
| Build | `@tauri-apps/cli` | `^2.10.0` | `2.10.1` | `package.json`、`package-lock.json` | build・NSIS bundler |

宣言値は version range、解決値は現在の lockfile に基づく。transitive な別版の `windows` / `winreg` が lockfile に共存するため、表には直接依存の版を記載する。

## Source Layout

`src-tauri/src/main.rs` が初期化、poll、コピー、logger を所有する。Frontend や DB 用の空 module は追加しない。Rust の test-only include と runtime は区別する。

## Runtime Architecture

Polarisは単一runtime moduleのため、構造説明を本書にまとめる。機能の条件と利用者への結果は [Specification](spec.md)、物理保存と置換・保持は [Data Storage](database.md) を正本とする。

### Process Model and Entry Points

```text
Windows Run / installer完了後起動 / exe直接起動
  → main → acquire_mutex → ready_env → init_logger
  → run_polling_loop → find_vrchat_pid
      → 起動時None / 終了 / PID交代 → backup
          → 対象判定 → サイズ安定待機 → safe_copy
          → 監視へ戻る
NSIS PREINSTALL/PREUNINSTALL → taskkill /F /IM Polaris.exe
```

Tauri Builder、WebView、IPC handler、SQLite、background task poolは起動しない。ToolHelpによるprocess列挙とcopyは同じloopで直列実行する。NSISは別の実行主体である。

### Source Responsibility Map

| Module | Responsibility | Entry | Owned state | Main dependencies |
| --- | --- | --- | --- | --- |
| `main.rs` 初期化 | mutexと保存先・profile・loggerの準備 | `main`、`ready_env` | process寿命のmutex handle、RuntimePaths | Win32、HKCU、dirs、tracing |
| `main.rs` 監視 | 最初に一致するVRChat PIDの追跡 | `run_polling_loop`、`find_vrchat_pid` | 前回PID、同一threadのloop | ToolHelp API |
| `main.rs` copy | 判定、安定待機、一時copy、rename、次回清掃 | `backup`、`safe_copy` | file処理中のpath・metadata | filesystem |
| `main.rs` error/log | 起動失敗MessageBox、file writerとpanic hook | `show_error`、`init_logger` | writer、process global hook | Win32、tracing-subscriber |
| `windows/` | process停止、配置、Run/製品登録、削除 | NSIS template/hook | installerの選択・更新mode | NSIS、Windows registry |
| `build.rs`、Tauri config | headless buildとbundle | `tauri_build::build()` | build-time設定 | tauri-build、CLI、NSIS |
| Cargo/package/toolchain設定 | 直接依存・解決値・target・lint | workspace/package script | runtime状態は持たない | Cargo、npm |
| `*_tests.rs` | test-only include群 | `cfg(test)` | 専用fixture | runtimeとは別の検証用コード |
| icons、docs、生成schema | 配布asset、公開情報、生成物 | bundle参照 | runtime業務状態なし | config、build tooling |

### Composition and State Ownership

DI containerや共有storeはない。RuntimePathsは初期化後に監視・copyへ渡し、追跡PIDはloopのlocal変数として持つ。mutex handleはprocess終了まで保持する。file単位の状態はbackup呼出し中だけ保持し、永続retry queueや処理済みmanifestは持たない。

### Main Data Flows

監視は前回PIDと現在PIDを比較し、初回未検出または終了/PID交代でbackupへ渡す。backupはarchive作成→前回temp清掃→source列挙→名前・サイズ判定→安定待機→temp copy→renameの順で進む。fileごとの失敗は残りのfileへ進み、処理後に監視を再開する。対象条件と省略規則は [Log Backup](spec.md#log-backup)、置換途中の保存結果は [Write Transactions](database.md#write-transactions) を参照する。

### Interfaces and Events

アプリ間のcustom IPC/eventはない。Polarisは製品HKCUからInstallLocationを読み、filesystemへraw copyを出力する。StellaRecordとAlpheratzは同じ保存先を読むが、Polarisがそのprocessを起動したりDBを更新する経路はない。保存先とkeyの詳細は [Records and Keys](database.md#records-and-keys) を参照する。

### Concurrency and Lifecycle

監視、3秒待機、copyを一つのloopで行うため、copy中に新しいprocess eventを取得しない。Local mutexは同一Windows session内の排他である。通常loopに自然終了条件はなく、停止UI、終了IPC、終了時copy完了待機はない。NSIS hookは同名exeを強制終了する。一時fileと既存archiveの結果は保存仕様で扱う。

### External Boundaries

runtimeが読むのはWin32のprocess一覧、現userの製品registry、VRChat profile内の元ログである。書込み先は解決したData配下のarchiveと運用logで、元ログや姉妹アプリのDBは更新しない。Run登録、製品登録、本体配置、uninstallはNSISの責務であり、runtimeのloopやmutexとは別に動作する。

## Runtime Decisions

### ADR-001: Headless Polling Process

1秒間隔の ToolHelp process 列挙を使う。VRChat の memory、network、plugin へ接続せず、監視とバックアップを1つの loop で追いやすくする。Windows Service や Tauri WebView として起動しない。

### ADR-002: Plain Log Copies

元ログを非圧縮でコピーし、StellaRecordの圧縮・解析とAlpheratzの写真world補完へ入力を提供する。両consumerはPolarisのInstallLocation配下の `Data/archive` を読む。内容判定の制約は [Specification](spec.md#current-constraints) を参照する。

### ADR-003: Installer-owned Paths

runtime の保存先は installer の HKCU 登録を使う。任意の作業 directory へ自動保存せず、登録が欠けている場合は初期化を拒否する。

## Build and Packaging Contract

```powershell
npm ci --ignore-scripts
npm run tauri build
```

Tauri 設定は `frontendDist: null`、`app.windows: []`、NSIS、Japanese、`installMode: currentUser`。ビルド結果は `target/release/bundle/nsis/` に生成する。

既定の配置は `%LOCALAPPDATA%\CosmoArtsStore\Polaris`。hook は置換・削除前に同名 exe を強制終了し、導入後に HKCU Run key へ登録する。通常の削除では `Data/archive` を保持する。

NSIS template には WebView2 の検出・導入 section が存在する。画面のない runtime と、installer の配布処理を区別する。署名を設定した配布契約は現在の設定にない。

## Verification

Rust workspace は `unwrap_used`、`expect_used`、`panic` を deny にする。これらの lint は runtime の全失敗を防ぐ保証ではない。

| Purpose | Command | Configuration and scope | CI gate |
| --- | --- | --- | --- |
| 依存導入 | `npm ci --ignore-scripts` | rootのlockfileに従うTauri CLI導入 | workflow定義なし |
| 配布build | `npm run tauri build` | Windows MSVC、NSIS成果物 | workflow定義なし |
| Rust test | `cargo test --locked --workspace` | Debug、test-only include群を含む。OSのprocess/registry等を使うfixtureがある | workflow定義なし |
| Rust lint | `cargo clippy --locked --workspace --all-targets -- -D warnings` | workspace全target | workflow定義なし |

開発にはNode.js/npm、Rust stableと `x86_64-pc-windows-msvc` target、WindowsのMSVC開発環境が必要である。rootにJavaScript test/coverage scriptやCI workflowはない。検証commandの結果と、実VRChat監視・installer操作・姉妹アプリ連携の確認範囲を区別する。

## Installation Lifecycle

| Stage | Behavior | Failure and retained state |
| --- | --- | --- |
| 通常導入 | current-userの既定配置はLocalAppData。Program Files/Windows配下を拒否し、製品登録とRun値を保存 | 本体配置・登録失敗は中断。全操作のrollbackではない |
| 更新 | `/UPDATE`と再実行UIの既存版処理を区別する。本体の置換前にhookでprocess停止 | archiveの自動移行・統合はない。既存配置のデータ寿命は保存仕様を参照 |
| 完了後起動 | GUIは完了画面の選択、silent/passiveは `/R` 指定時 | Run登録は次回Windows起動用で、完了後起動とは別 |
| 通常uninstall | process停止後、本体・運用log・bundle ID領域・Run/製品登録を削除。archiveは保持 | 必須削除失敗で中断。先に終えた削除は戻さず、uninstallerを後段まで保持 |
| `/UPDATE`付きuninstall | 本体等を処理し、運用log・Run値・製品keyの通常削除分岐を省略 | すべての設定・本体が常に不変という意味ではない |

fileとregistryごとの保持範囲は [Deletion and Retention](database.md#deletion-and-retention) を参照する。installer templateのWebView2導入処理とruntimeの無通信・headless構成を同一視しない。

## Current Distribution Constraints

installer hook による終了はコピー処理の完了待ちを行わない。保存先を変更する再インストールでは、以前の archive を runtime が自動統合しない。移行時は旧 archive を利用者が明示的に保全する。

## References

- [Rust workspace](../Cargo.toml)
- [Rust toolchain](../rust-toolchain.toml)
- [Installer hooks](../src-tauri/windows/hooks.nsi)
- [Installer template](../src-tauri/windows/installer.nsi)
