# Data Storage

Polaris の filesystem・registry・運用ログの保存契約を記録する。共通の文書配置では `database.md` を使うが、Polaris は SQLite database を持たない。

## Document Control

| Field | Value |
| --- | --- |
| Application | Polaris |
| Scope | filesystem・registry・運用logの配置、置換、保全、削除・保持 |
| Audience | データを保全する利用者、保存・連携を保守する開発者 |
| Source review date | 2026-10-22 |
| Authority | [main.rs](../src-tauri/src/main.rs)、[installer.nsi](../src-tauri/windows/installer.nsi)、[hooks.nsi](../src-tauri/windows/hooks.nsi) |
| Related documents | [README](../README.md)、[Specification](spec.md)、[Tech Stack](tech-stack.md) |

## Storage

```text
<install>/
├─ Polaris.exe
└─ Data/
   ├─ archive/
   │  ├─ output_log_*.txt
   │  └─ *.polaris_tmp
   └─ logs/info.log
```

保存先は HKCU の `InstallLocation` を正本とする。現在の exe の場所や既定 LocalAppData への runtime fallback はない。

| Resource | Owner | Location and format | Identity | Write unit | Retention and deletion |
| --- | --- | --- | --- | --- | --- |
| Source logs | VRChatが生成、Polarisが読取り | profile配下の生ログ | filename | Polarisは更新しない | Polaris runtime/installerの削除対象外 |
| Archive logs | Polarisが書込み、StellaRecord/Alpheratzが読取り | `InstallLocation/Data/archive`、原文bytes | 元filename。skipは非0サイズ一致 | fileごとの一時copy→rename | 通常uninstallでも保持。新copyで同名を置換 |
| Temporary copies | Polaris | 同じarchive内の `.polaris_tmp` | コピー先の拡張子を置換 | copy中のfile | 次backup冒頭で直下の一致拡張子を削除試行 |
| Application log | Polaris | `Data/logs/info.log`、追記 | 固定path | tracing writerの書込み | runtimeにrotation/上限なし。通常uninstallで削除 |
| Install/Run registration | NSIS | 現userのHKCU | 製品key・Run値名 | registry値 | 通常uninstallで削除。更新時は専用分岐 |

StellaRecordはPolaris製品keyへ `ArchivePath` と `CapacityThresholdBytes` を保存するが、Polaris runtimeはその値を採用しない。runtimeのarchiveは `InstallLocation/Data/archive` から解決する。これらのkeyの存在を保存先変更機能と解釈しない。

## Relationships

元ログのファイル名と archive のファイル名を1対1で対応させる。別のファイル名への世代保存、SQLite table、外部キー、index、schema version は定義しない。

## Records and Keys

| Record | Key / format | Meaning |
| --- | --- | --- |
| Source log | `output_log_*.txt` | VRChat の原本 |
| Archive log | 元ログと同じ名前 | 非圧縮コピー。内容を正規化しない |
| Temporary copy | コピー先の拡張子を `polaris_tmp` に変更 | コピー後の検証前のファイル |
| Install location | `HKCU\Software\CosmoArtsStore\Polaris\InstallLocation`、string | インストール先の絶対パス |
| Startup command | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Polaris`、string | 引用符で囲んだ exe path |
| Application log | `Data/logs/info.log` | 追記方式の tracing 出力 |

## Query Semantics

`is_vrchat_log` は名前の `output_log_` prefix と `txt` 拡張子を確認する。ファイル名の日付・時刻は解析しない。`is_same_size` は元とコピーのサイズが等しく0でないことだけを確認する。

ログ本文の hash、行数、更新時刻は保存せず、同サイズの内容更新は検出しない。圧縮アーカイブの本文一致と再取込判定は StellaRecord の契約である。StellaRecordとAlpheratzは本アプリの保存先 `InstallLocation/Data/archive` から元logを読む。

## Write Transactions

filesystem 操作に database transaction はない。`safe_copy` は一時ファイルへのコピー量を確認してから rename で既存コピーを置換する。安定待機、コピー、renameが失敗した場合は、置換前のarchiveと元logを保持する。

途中終了で残った `.polaris_tmp` は次回バックアップ開始時に削除を試みる。copy自体やrenameの失敗で一時fileが残る場合があり、清掃の失敗も無視して続行する。今回の失敗対象は次の起動時backupまたは監視eventで再評価する。常時retry queueやbackup manifestは持たない。

## Initialization and Compatibility

runtime 初期化は registry と既存 directory の存在を検証する。archive directory はバックアップ時に作成する。旧保存先や旧 registry の探索・移行は runtime にはない。installer の以前の配置復元は NSIS template の管理範囲である。

## Deletion and Retention

| Operation | Original input | Derived data | Settings | Partial failure and recovery |
| --- | --- | --- | --- | --- |
| 通常backup | 元ログを保持 | 成功fileだけ置換。既存copyは置換まで保持 | 変更なし | file失敗は残りを継続。全file一体の巻戻しはない |
| 一時file整理 | 元ログを保持 | archive直下の一致拡張子だけ削除試行 | 変更なし | 清掃失敗は無視。次backupで再評価 |
| 導入・更新 | 元ログを保持 | 同名processを停止して本体を配置。archiveを移行・統合しない | NSISがInstallLocation/Runを登録 | 配置・登録失敗は中断。全操作のrollbackではない |
| 通常uninstall | 元ログを保持 | archiveを保持。本体・運用log・bundle ID領域を削除 | Polaris製品key・Run値を削除 | 削除済み部分は戻さず中断。残したuninstallerで再試行 |
| `/UPDATE`付きuninstall | 元ログを保持 | 本体等を削除するが通常の運用log削除分岐を省略 | 通常のRun値・製品key削除を省略 | 更新の全段階が一体のtransactionではない |

`Data` とinstall rootは空の場合だけ削除する。archiveが残ることと、通常uninstall後に製品registryから同じ場所を再発見できることは別である。退避先と元の配置を保全してから再導入する。

## Backup and Restore

Polaris を終了してから `Data/archive` をフォルダ単位でコピーする。未完了の `.polaris_tmp` は完成したログとして扱わない。

専用の復元 UI はない。別のインストール先へ移す場合は、installer が登録した保存先の `Data/archive` へコピーする。通常のアンインストールでは archive を残し、運用ログと Run key・製品 registry は削除する。

## References

- [Specification](spec.md#current-constraints)
- [Copy implementation](../src-tauri/src/main.rs)
- [Installation and retention](../src-tauri/windows/installer.nsi)
