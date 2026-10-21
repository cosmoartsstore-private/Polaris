# Specification

Polaris の現行 process 監視とログコピーの動作を記録する。

## Document Control

| Field | Value |
| --- | --- |
| Application | Polaris |
| Scope | process監視、ログコピー、失敗・再試行、姉妹アプリへの入力提供 |
| Audience | 利用者、機能・運用を確認する開発者 |
| Source review date | 2026-10-22 |
| Authority | [main.rs](../src-tauri/src/main.rs)、[tauri.conf.json](../src-tauri/tauri.conf.json)、[installer.nsi](../src-tauri/windows/installer.nsi) |
| Related documents | [README](../README.md)、[Data Storage](database.md)、[Tech Stack](tech-stack.md) |

## Scope

同じ Windows user の VRChat ログを、インストール先の `Data/archive` へ非圧縮でコピーする。StellaRecord の圧縮・解析と、Alpheratz の訪問区間によるワールド名補完に元ログを提供する。

画面、通知領域アイコン、Windows Service 登録、SQLite、外部 API、認証、クラウド同期、元ログ削除は提供しない。

## Terms

| Term | Meaning |
| --- | --- |
| Source log | VRChat profile 配下の `output_log_*.txt` |
| Archive | `Data/archive` の非圧縮ログコピー。`.tar.zst` は生成しない |
| Tracked PID | 前回の poll で検出した `vrchat.exe` の PID |
| Stable size | 500 ms 間隔で隣り合う2回の取得サイズが一致した状態 |

## Runtime Architecture

監視とコピーは同じRust processのloopで直列実行する。初期化、module責務、状態所有、停止の実経路は [Runtime Architecture](tech-stack.md#runtime-architecture) を参照する。

## Startup and Shutdown

1. `Local\Polaris_SingleInstance` を取得する。既存 instance があれば終了する。
2. `HKCU\Software\CosmoArtsStore\Polaris\InstallLocation` を読む。
3. インストール先と `%USERPROFILE%\AppData\LocalLow\VRChat\VRChat` の存在を確認する。
4. `Data/logs/info.log` への logger を設定する。
5. VRChat が起動していなければ、その時点の元ログをバックアップする。
6. 各pollの前に1秒待機し、監視を続ける。コピー処理や終了検知後の3秒待機も同じloop内で行うため、全体が厳密な1秒周期になるわけではない。

初期化できない場合は Windows のメッセージボックスへエラーを表示して終了する。現在の終了 code は0であり、成功・失敗の区別には使えない。

通常動作に停止UIや終了IPCはない。installer hookは同名processを強制終了し、コピーの完了を待たない。終了時の一時ファイルと既存archiveの扱いは [Write Transactions](database.md#write-transactions) を参照する。

## Features

| Feature | Entry and input | Decision and output | Persistence | Cancellation and failure |
| --- | --- | --- | --- | --- |
| Process Monitoring | 起動と各poll。検出した `vrchat.exe` のPID | 起動時未検出、終了、PID交代でコピーを開始 | 追跡PIDはprocessメモリーだけ | 常駐の停止操作なし。列挙失敗は未検出と同じ扱い |
| Log Backup | 監視からの自動開始。元フォルダ直下の候補 | 同名・非0同サイズを省略し、安定サイズ候補を一時コピー後に置換 | file単位。複数file一括のrollbackなし | file失敗は残りを継続。次のbackup契機で再評価 |
| Application Integration | 姉妹アプリが保存先を読む | 生ログの提供。相手側で圧縮・解析・写真補完 | 相手のDBをPolarisは変更しない | 読取り側の失敗・再試行は各アプリの契約 |

### Process Monitoring

ToolHelpのprocess一覧から、exe名を大文字小文字無視で `vrchat.exe` と比較し、最初に一致したPIDを追跡する。processのメモリーやネットワーク通信には接続しない。

前回が `Some(pid)`、今回が `None` の場合、または両方が存在して PID が変わった場合に終了・再起動と判定する。3秒待ってバックアップし、その後のpollへ進む。待機後のPID再取得は行わず、バックアップ中も追加のpollは行わない。検出した全起動・終了をqueueへ保存する方式ではない。

### Log Backup

1. `Data/archive` を作成し、残っている `.polaris_tmp` を整理する。
2. VRChat ログフォルダの直下だけを列挙する。
3. 名前の先頭が `output_log_`、拡張子が大文字小文字無視で `txt`、サイズが0でないファイルを対象にする。
4. 同名のコピーがあり、元とコピーのサイズが等しく0でなければ省略する。
5. サイズが異なる既存コピーは、完成した新しいコピーの置換まで保持する。同名のdirectoryがある場合はそのファイルを省略する。
6. 元ログのサイズを500 ms間隔、最大6回確認する。連続一致しなければ省略する。
7. `.polaris_tmp` へコピーし、返されたコピー量と一時ファイルサイズが一致して0でないことを確認する。
8. 一時ファイルを最終名へ rename し、既存コピーを置換する。失敗時は既存コピーと元logを保持する。

### Application Integration

StellaRecord は `Data/archive` のログコピーを同期・圧縮し、SQLite解析を担当する。Alpheratz は同じ保存先から訪問ログを読み、撮影時刻に対応する未知ワールド名を補完する。Polaris 自身は姉妹アプリの process を起動せず、DB に書き込まない。

## Persistence and Events

filesystem と registry は [Data Storage](database.md) を参照する。アプリ間イベントや Tauri event は送出しない。

Release は ERROR 以上、Debug は DEBUG 以上を保存する。ログファイルを開けない場合は出力を破棄して処理を続ける。現在の logger にローテーションや容量上限はない。

## Concurrency and Failure Handling

- 監視とコピーは単一 loop で直列実行する。
- フォルダ作成・列挙失敗は記録して今回のバックアップを終了する。
- ファイルごとのコピー失敗は記録し、残るファイルの処理を続ける。
- mutex handle は process 終了まで保持する。
- ファイルの取得失敗や安定待ちの失敗は、そのファイルを省略する。常時 retry queue は持たない。
- directory作成・列挙失敗を解消しても即時の再実行は行わない。次回起動時の未検出、VRChat終了、PID交代が再評価の契機になる。

## External Effects and Validation

- 元ログは読み取りだけ行う。
- バックアップの自動書込み・一時ファイル整理は解決したarchiveフォルダに限定する。運用logは別に `Data/logs` へ書き込む。
- installer が現在の利用者の製品 registry と Run key を管理する。
- インストール・アンインストールの hook は Polaris を強制終了する。
- runtime は外部通信を行わない。installer には WebView2 導入 section が残っており、runtime の構成と installer の処理は区別する。

## Current Constraints

- 同名・同サイズなら本文が異なってもコピーし直さない。
- process 列挙の失敗は VRChat 未検出と同じ `None` になる。
- `Local` mutex は同じ Windows session の排他であり、複数 session を排他しない。
- polling とサイズ安定待ちは、書込み完了を完全に保証する契約ではない。

## References

- [Runtime implementation](../src-tauri/src/main.rs)
- [Installer hooks](../src-tauri/windows/hooks.nsi)
- [Installer template](../src-tauri/windows/installer.nsi)
