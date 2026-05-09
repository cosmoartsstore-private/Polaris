#![windows_subsystem = "windows"]

use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Result};
use tracing::{debug, error};
use tracing_subscriber::fmt;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
use winreg::RegKey;

struct RuntimePaths {
    archive_folder: PathBuf,
    vrchat_log_folder: PathBuf,
}

fn main() {
    acquire_mutex();

    let runtime_paths = match ready_env() {
        Ok(paths) => paths,
        Err(e) => {
            show_error(&e.to_string());
            std::process::exit(0);
        }
    };

    run_polling_loop(&runtime_paths);
}

// ── ミューテックス(多重起動防止)処理 ───────────────────────────────────────────────
fn acquire_mutex() {
    let name: Vec<u16> = "Local\\Polaris_SingleInstance\0".encode_utf16().collect();

    let handle = match unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr())) } {
        Ok(h) => h,
        Err(e) => {
            show_error(&format!("ミューテックスを作成できませんでした: {e}"));
            std::process::exit(0);
        }
    };

    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        let _ = unsafe { CloseHandle(handle) };
        std::process::exit(0);
    }

    // ハンドルは意図的に解放しない。プロセス存続中ミューテックスを保持する。
    let _ = handle;
}

// ── ポーリング監視 ──────────────────────────────────────────────────

/// 1秒間隔でVRChatプロセスを監視し、終了・再起動を検知してバックアップを実行する。
fn run_polling_loop(runtime_paths: &RuntimePaths) -> ! {
    let mut tracked_pid = find_vrchat_pid();

    // 起動時にVRChatが動いていなければ、前回終了分をバックアップ
    if tracked_pid.is_none() {
        backup(runtime_paths);
    }

    loop {
        std::thread::sleep(Duration::from_secs(1));

        let current_pid = find_vrchat_pid();

        let exited = match (tracked_pid, current_pid) {
            (Some(_), None) => true,                       // 終了
            (Some(old), Some(new)) if old != new => true,  // 再起動
            _ => false,
        };

        if exited {
            debug!("VRChatの終了を検知。バックアップを開始します。");
            // 書き込み完了待ち
            std::thread::sleep(Duration::from_secs(3));
            backup(runtime_paths);
        }

        tracked_pid = current_pid;
    }
}

/// `CreateToolhelp32Snapshot` でプロセス一覧を取得し、VRChatのPIDを返す。
fn find_vrchat_pid() -> Option<u32> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;

    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    #[allow(clippy::cast_possible_truncation)]
    {
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    }

    let found = unsafe {
        if Process32FirstW(snapshot, &raw mut entry).is_ok() {
            loop {
                let nul_pos = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..nul_pos]);

                if name.eq_ignore_ascii_case("vrchat.exe") {
                    break Some(entry.th32ProcessID);
                }

                if Process32NextW(snapshot, &raw mut entry).is_err() {
                    break None;
                }
            }
        } else {
            None
        }
    };

    let _ = unsafe { CloseHandle(snapshot) };
    found
}

// ── バックアップ処理 ───────────────────────────────────────

/// 未バックアップのログをアーカイブする。(既存アーカイブとサイズが一致しない場合、再バックアップ）
fn backup(runtime_paths: &RuntimePaths) {
    let log_folder = &runtime_paths.vrchat_log_folder;
    let archive_folder = &runtime_paths.archive_folder;

    if let Err(e) = std::fs::create_dir_all(archive_folder) {
        error!(
            "archiveフォルダを作成できませんでした [{}]: {e}",
            archive_folder.display()
        );
        return;
    }

    // 前回クラッシュで残った一時ファイルを掃除
    clean_temp_files(archive_folder);

    let entries = match std::fs::read_dir(log_folder) {
        Ok(entries) => entries,
        Err(e) => {
            error!(
                "ログフォルダを開けませんでした [{}]: {e}",
                log_folder.display()
            );
            return;
        }
    };

    for entry in entries.flatten() {
        let source = entry.path();

        if !is_vrchat_log(&source) {
            continue;
        }

        let file_name = source.file_name().and_then(|n| n.to_str());
        let Some(file_name) = file_name else { continue };

        // 0バイトの場合バックアップ対象外
        if std::fs::metadata(&source).map(|m| m.len()).unwrap_or(0) == 0 {
            continue;
        }

        let archive = archive_folder.join(file_name);

        // 同サイズの場合スキップ
        if archive.exists() && is_same_size(&source, &archive) {
            continue;
        }

        // 同サイズ以外の場合再バックアップ (失敗時スキップ)
        if archive.exists() {
            let src = std::fs::metadata(&source).map(|m| m.len()).unwrap_or(0);
            let arc = std::fs::metadata(&archive).map(|m| m.len()).unwrap_or(0);
            debug!("サイズ不一致のため再バックアップ [{file_name}] (元: {src}, アーカイブ: {arc})");
            if let Err(e) = std::fs::remove_file(&archive) {
                debug!("アーカイブの削除に失敗 [{file_name}]: {e}");
                continue;
            }
        }

        // ファイルサイズが安定するまで待機 (書き込み途中を回避)
        if wait_for_stable_size(&source).is_none() {
            debug!("ファイルサイズが安定しませんでした [{file_name}]");
            continue;
        }

        // 一時ファイル経由の安全なコピー
        if let Err(e) = safe_copy(&source, &archive) {
            error!("バックアップに失敗 [{file_name}]: {e}");
        }
    }
}

/// `output_log_*.txt` 一致判定
fn is_vrchat_log(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name.starts_with("output_log_")
        && path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
}

/// 同名ファイルのサイズが一致し、かつ 0 でないか判定
fn is_same_size(a: &Path, b: &Path) -> bool {
    let size_a = std::fs::metadata(a).map(|m| m.len()).unwrap_or(0);
    let size_b = std::fs::metadata(b).map(|m| m.len()).unwrap_or(0);
    size_a == size_b && size_a > 0
}

/// 500ms間隔で最大6回サイズを確認。連続一致でOK判定
fn wait_for_stable_size(path: &Path) -> Option<u64> {
    const MAX_CHECKS: u32 = 6;
    let mut prev = std::fs::metadata(path).ok()?.len();

    for _ in 0..MAX_CHECKS {
        std::thread::sleep(Duration::from_millis(500));
        match std::fs::metadata(path).map(|m| m.len()) {
            Ok(current) if current == prev => return Some(current),
            Ok(current) => prev = current,
            Err(_) => {} // 一時的なI/Oエラーの場合リトライ
        }
    }

    None
}

/// 一時ファイルにコピー → サイズ検証 → リネーム
fn safe_copy(source: &Path, destination: &Path) -> Result<()> {
    let temp_path = destination.with_extension("polaris_tmp");

    let bytes_copied = std::fs::copy(source, &temp_path)
        .map_err(|e| anyhow!("一時ファイルへのコピーに失敗: {e}"))?;

    let tmp_size = std::fs::metadata(&temp_path).map(|m| m.len()).unwrap_or(0);

    if bytes_copied != tmp_size || bytes_copied == 0 {
        let _ = std::fs::remove_file(&temp_path);
        return Err(anyhow!(
            "サイズ検証失敗 (コピー: {bytes_copied}, 書込: {tmp_size})"
        ));
    }

    std::fs::rename(&temp_path, destination).map_err(|e| anyhow!("リネームに失敗: {e}"))
}

/// archiveフォルダ内に残っているtmpファイルを削除
fn clean_temp_files(archive_folder: &Path) {
    let Ok(entries) = std::fs::read_dir(archive_folder) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "polaris_tmp") {
            let _ = std::fs::remove_file(&path);
        }
    }
}

// ── 初期化・Utils ──────────────────────────────────────────

/// インストールパス取得 & ロガー初期化
fn ready_env() -> Result<RuntimePaths> {
    let install_path: String = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Software\\CosmoArtsStore\\Polaris", KEY_READ)
        .and_then(|key| key.get_value("InstallLocation"))
        .map_err(|_| {
            anyhow!("インストール情報を取得できませんでした。再インストールしてください。")
        })?;

    let install_dir = PathBuf::from(install_path);
    if !install_dir.is_dir() {
        return Err(anyhow!(
            "インストール先フォルダが見つかりません。再インストールしてください。"
        ));
    }

    let Some(profile_dir) = dirs::home_dir() else {
        return Err(anyhow!(
            "VRChatのログ排出先を特定できないため終了します。"
        ));
    };

    let vrchat_log_folder = profile_dir
        .join("AppData")
        .join("LocalLow")
        .join("VRChat")
        .join("VRChat");
    if !vrchat_log_folder.is_dir() {
        return Err(anyhow!(
            "VRChatのログ排出先が見つからないため終了します。"
        ));
    }

    let data_dir = install_dir.join("Data");
    let runtime_paths = RuntimePaths {
        archive_folder: data_dir.join("archive"),
        vrchat_log_folder,
    };

    init_logger(&data_dir)?;

    Ok(runtime_paths)
}

/// ファイルロガー初期化 & パニック時のログ出力設定
fn init_logger(data_dir: &Path) -> Result<()> {
    let logs_dir = data_dir.join("logs");
    let _ = std::fs::create_dir_all(&logs_dir);
    let log_path = logs_dir.join("info.log");

    let max_level = if cfg!(debug_assertions) {
        tracing::Level::DEBUG
    } else {
        tracing::Level::ERROR
    };

    fmt()
        .with_max_level(max_level)
        .with_ansi(false)
        .with_target(false)
        .with_writer(move || -> Box<dyn io::Write> {
            match OpenOptions::new().create(true).append(true).open(&log_path) {
                Ok(file) => Box::new(file),
                Err(_) => Box::new(io::sink()),
            }
        })
        .try_init()
        .map_err(|e| anyhow!("ロガーを初期化できませんでした: {e}"))?;

    std::panic::set_hook(Box::new(|info| {
        let loc = info.location().map_or_else(
            || "unknown".to_string(),
            |l| format!("{}:{}", l.file(), l.line()),
        );
        error!("予期しないエラーが発生しました [{loc}]: {info}");
    }));

    Ok(())
}

/// エラーメッセージをメッセージボックスで表示
fn show_error(message: &str) {
    let title: Vec<u16> = "Polaris\0".encode_utf16().collect();
    let body: Vec<u16> = format!("{message}\0").encode_utf16().collect();
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(body.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}
