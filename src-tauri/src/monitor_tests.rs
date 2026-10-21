// 実監視loopに所有するprocessの終了・PID変更を渡し、利用者のVRChatを操作しない。
static NATIVE_PROCESS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// 共有値は排他用のunit値だけである。先行caseのpanic後も解除を待ち、後続case固有の前提条件を確認する。
fn lock_native_process_tests() -> std::sync::MutexGuard<'static, ()> {
    NATIVE_PROCESS_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// 所有するmutex・eventのhandleを解放し、検証終了後に同じ名前を残さない。
struct OwnedNativeHandle(windows::Win32::Foundation::HANDLE);
impl Drop for OwnedNativeHandle { fn drop(&mut self) { let _ = unsafe { CloseHandle(self.0) }; } }

// 製品と同じexe名を使う所有fixtureだけを、独立したCIM照会で列挙する。
fn owned_vrchat_pids_from_independent_query() -> Vec<u32> {
    use std::os::windows::process::CommandExt;
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("workspaceがありません");
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", r#"@(Get-CimInstance Win32_Process -Filter "Name = 'VRChat.exe'" | Where-Object { if (-not $_.ExecutablePath) { return $false }; $parent = Split-Path -Parent $_.ExecutablePath; $leaf = Split-Path -Leaf $parent; $tempOwned = (Split-Path -Parent $parent) -eq $env:POLARIS_PROCESS_TEST_TEMP -and $leaf.StartsWith('polaris-test-'); $proofOwned = $parent.StartsWith($env:POLARIS_PROCESS_TEST_CHECKS, [StringComparison]::OrdinalIgnoreCase) -and (Test-Path -LiteralPath (Join-Path $parent 'owner.marker')) -and (Get-Content -LiteralPath (Join-Path $parent 'owner.marker') -Raw -Encoding UTF8).Trim() -eq 'polaris-headless-consumer-fixture-v1'; $tempOwned -or $proofOwned }) | ForEach-Object { $_.ProcessId }; exit 0"#])
        .env("POLARIS_PROCESS_TEST_TEMP", std::env::temp_dir().to_string_lossy().trim_end_matches(['/', '\\']))
        .env("POLARIS_PROCESS_TEST_CHECKS", workspace.join(".claude").join("audit-work").join("checks"))
        .creation_flags(0x0800_0000).output().expect("独立したprocess照会を実行できません");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).lines().map(|line| line.trim().parse().expect("PIDを読めません")).collect()
}

fn assert_owned_vrchat_processes(expected: &[u32]) {
    let mut actual = owned_vrchat_pids_from_independent_query();
    let mut expected = expected.to_vec(); actual.sort_unstable(); expected.sort_unstable(); assert_eq!(actual, expected, "利用者のVRChat状態が変わったため検証できません");
}

fn stop_owned_monitor_child(child: &mut OwnedTestChild) {
    drop(child.0.stdin.take()); let status = child.finish(); let diagnostic = child.diagnostic();
    assert!(status.success() && diagnostic.contains("test result: ok. 1 passed; 0 failed;"), "専用childの正常完了を確認できません: {diagnostic}");
}

fn wait_owned_archive(path: &Path, expected: &[u8]) {
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    while fs::read(path).ok().as_deref() != Some(expected) {
        assert!(std::time::Instant::now() < deadline, "実監視loopが専用archiveを保存しません: {}", path.display());
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn write_native_receipt(variable: &str, file_name: &str, content: &str) {
    if let Some(path) = std::env::var_os(variable) {
        let path = PathBuf::from(path); let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("検証workspaceがありません");
        let boundary = workspace.join(".claude/audit-work/checks").canonicalize().expect("検証成果物の親を取得できません");
        assert_eq!(path.file_name().and_then(|name| name.to_str()), Some(file_name));
        assert!(path.parent().expect("成果物の親がありません").canonicalize().expect("成果物の親を解決できません").starts_with(boundary));
        fs::write(path, content).expect("監視時点の測定値を保存できません");
    }
}

#[test]
fn actual_polling_loop_handles_owned_exit_and_pid_replacement() {
    let test = "tests::actual_polling_loop_handles_owned_exit_and_pid_replacement";
    let mode = std::env::var("POLARIS_NATIVE_TEST_MODE").unwrap_or_default();
    if mode.starts_with("owned-vrchat-") {
        let root = PathBuf::from(std::env::var_os("POLARIS_NATIVE_TEST_ROOT").expect("専用fixtureがありません"));
        assert!(matches!(mode.as_str(), "owned-vrchat-first" | "owned-vrchat-second" | "owned-vrchat-third"));
        fs::write(root.join(format!("{mode}.started")), std::process::id().to_string()).expect("専用processの開始を記録できません");
        let mut input = String::new(); std::io::stdin().read_line(&mut input).expect("専用processの終了通知を読めません"); return;
    }
    if matches!(mode.as_str(), "owned-monitor" | "owned-idle-monitor" | "owned-consumer-monitor") {
        let root = PathBuf::from(std::env::var_os("POLARIS_NATIVE_TEST_ROOT").expect("専用fixtureがありません"));
        let archive_folder = root.join(if mode == "owned-consumer-monitor" { "Data/archive" } else { "archive" });
        let paths = RuntimePaths { vrchat_log_folder: root.join("source"), archive_folder };
        assert_eq!(find_vrchat_pid().is_none(), mode == "owned-idle-monitor", "監視開始時の専用process条件が一致しません"); init_logger(&root).expect("専用loggerを初期化できません");
        std::thread::spawn(move || run_polling_loop(&paths)); fs::write(root.join("monitor.started"), b"started").expect("実loopの開始を記録できません");
        let mut input = String::new(); std::io::stdin().read_line(&mut input).expect("監視childの終了通知を読めません");
        fs::write(root.join("monitor.completed"), b"completed").expect("実loopの終了を記録できません"); return;
    }
    assert!(mode.is_empty()); let _process_lock = lock_native_process_tests();
    assert_owned_vrchat_processes(&[]);
    let name: Vec<u16> = SINGLE_INSTANCE_MUTEX_NAME.encode_utf16().chain([0]).collect();
    let mutex = OwnedNativeHandle(unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.expect("検証用mutexを取得できません"));
    assert_ne!(unsafe { GetLastError() }, ERROR_ALREADY_EXISTS, "実Polarisまたは他の検証が動作しているため開始できません");
    let directory = TestDir::new(); let executable = directory.path("VRChat.exe"); fs::copy(std::env::current_exe().expect("test exeを取得できません"), &executable).expect("専用process imageを作成できません");
    let source = directory.path("source"); fs::create_dir(&source).expect("専用source directoryを作成できません");
    let first_source = source.join("output_log_owned_exit.txt"); let first_archive = directory.path("archive/output_log_owned_exit.txt");
    let second_source = source.join("output_log_owned_restart.txt"); let second_archive = directory.path("archive/output_log_owned_restart.txt");
    fs::write(&first_source, "終了前の専用ログ\n".as_bytes()).expect("終了用logを書き込めません");
    let mut first = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-first", &directory.0); first.wait_for_marker(&directory.path("owned-vrchat-first.started"));
    assert_owned_vrchat_processes(&[first.0.id()]); assert_eq!(find_vrchat_pid(), Some(first.0.id()));
    let mut monitor = OwnedTestChild::start(test, "owned-monitor", &directory.0); monitor.wait_for_marker(&directory.path("monitor.started"));
    // 実pollを2回通過させ、稼働中にarchiveが作られないことを確認する。
    std::thread::sleep(Duration::from_millis(2_200)); assert_eq!(find_vrchat_pid(), Some(first.0.id())); assert!(!first_archive.exists());
    let exited_at = std::time::Instant::now(); stop_owned_monitor_child(&mut first); assert_owned_vrchat_processes(&[]); assert_eq!(find_vrchat_pid(), None);
    wait_owned_archive(&first_archive, "終了前の専用ログ\n".as_bytes()); let exit_wait_ms = exited_at.elapsed().as_millis(); assert!(exit_wait_ms >= 3_000, "終了後の3秒待機を通していません");
    assert_eq!(fs::read(&first_source).expect("元logを読めません"), "終了前の専用ログ\n".as_bytes());
    let mut second = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-second", &directory.0); second.wait_for_marker(&directory.path("owned-vrchat-second.started"));
    assert_owned_vrchat_processes(&[second.0.id()]); fs::write(&second_source, "PID変更前の専用ログ\n".as_bytes()).expect("再起動用logを書き込めません");
    std::thread::sleep(Duration::from_millis(2_200)); assert_eq!(find_vrchat_pid(), Some(second.0.id())); assert!(!second_archive.exists(), "未起動→稼働の変化でコピーしてはいけません");
    let replaced_at = std::time::Instant::now();
    // 後続processの開始後に先行processを終了し、Some(old)→Some(new)の間にNoneを挟まない。
    let mut third = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-third", &directory.0); third.wait_for_marker(&directory.path("owned-vrchat-third.started"));
    assert_owned_vrchat_processes(&[second.0.id(), third.0.id()]); stop_owned_monitor_child(&mut second);
    assert_owned_vrchat_processes(&[third.0.id()]); assert_eq!(find_vrchat_pid(), Some(third.0.id()));
    wait_owned_archive(&second_archive, "PID変更前の専用ログ\n".as_bytes()); let replacement_wait_ms = replaced_at.elapsed().as_millis(); assert!(replacement_wait_ms >= 3_000, "PID変更後の3秒待機を通していません");
    assert_eq!(fs::read(&first_archive).expect("先行archiveを読めません"), "終了前の専用ログ\n".as_bytes());
    assert_eq!(fs::read(&first_source).expect("先行元logを読めません"), "終了前の専用ログ\n".as_bytes());
    assert_eq!(fs::read(&second_source).expect("再起動元logを読めません"), "PID変更前の専用ログ\n".as_bytes());
    let log = fs::read_to_string(directory.path("logs/info.log")).expect("実loggerの検知記録を読めません");
    assert_eq!(log.matches("VRChatの終了を検知。バックアップを開始します。").count(), 2, "終了とPID変更を別々に検知する必要があります");
    stop_owned_monitor_child(&mut monitor); assert_eq!(fs::read(directory.path("monitor.completed")).expect("loop終了記録がありません"), b"completed");
    stop_owned_monitor_child(&mut third); assert_owned_vrchat_processes(&[]); assert_eq!(find_vrchat_pid(), None);
    assert!(!directory.path("archive/output_log_owned_exit.polaris_tmp").exists()); assert!(!directory.path("archive/output_log_owned_restart.polaris_tmp").exists());
    // runnerの専用成果物だけへ時点の測定値を保存し、通常のcargo testでは保存先を要求しない。
    write_native_receipt("POLARIS_MONITOR_TEST_RECEIPT_PATH", "polaris-native-monitor-evidence.json",
        &format!("{{\"exit_wait_ms\":{exit_wait_ms},\"replacement_wait_ms\":{replacement_wait_ms},\"detections\":2,\"source_bytes_preserved\":true}}"));
    drop(mutex);
}

#[test]
fn actual_polling_loop_retries_archive_lock_failure_on_next_exit() {
    use std::io::Read;
    use std::os::windows::fs::OpenOptionsExt;
    let _process_lock = lock_native_process_tests(); assert_owned_vrchat_processes(&[]);
    let name: Vec<u16> = SINGLE_INSTANCE_MUTEX_NAME.encode_utf16().chain([0]).collect();
    let mutex = OwnedNativeHandle(unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.expect("検証用mutexを取得できません"));
    assert_ne!(unsafe { GetLastError() }, ERROR_ALREADY_EXISTS, "実Polarisまたは他の検証が動作しているため開始できません");
    let directory = TestDir::new(); let executable = directory.path("VRChat.exe"); fs::copy(std::env::current_exe().expect("test exeを取得できません"), &executable).expect("専用process imageを作成できません");
    fs::create_dir(directory.path("source")).expect("専用sourceを作成できません"); fs::create_dir(directory.path("archive")).expect("専用archiveを作成できません");
    let source = directory.path("source/output_log_locked.txt"); let archive = directory.path("archive/output_log_locked.txt");
    let other_source = directory.path("source/output_log_other.txt"); let other_archive = directory.path("archive/output_log_other.txt");
    let expected = "更新済みの専用ログ\n".as_bytes(); let previous = b"previous";
    fs::write(&source, expected).expect("専用logを書き込めません"); fs::write(&archive, previous).expect("旧archiveを書き込めません");
    fs::write(&other_source, b"other owned log").expect("別logを書き込めません"); assert_ne!(expected.len(), previous.len());
    let mut locked = fs::OpenOptions::new().read(true).share_mode(0).open(&archive).expect("専用archiveをlockできません");
    let test = "tests::actual_polling_loop_handles_owned_exit_and_pid_replacement";
    let mut first = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-first", &directory.0); first.wait_for_marker(&directory.path("owned-vrchat-first.started"));
    assert_owned_vrchat_processes(&[first.0.id()]); assert_eq!(find_vrchat_pid(), Some(first.0.id()));
    let mut monitor = OwnedTestChild::start(test, "owned-monitor", &directory.0); monitor.wait_for_marker(&directory.path("monitor.started"));
    std::thread::sleep(Duration::from_millis(2_200)); assert!(!other_archive.exists());
    let failed_at = std::time::Instant::now(); stop_owned_monitor_child(&mut first); assert_owned_vrchat_processes(&[]);
    let failure = "バックアップに失敗 [output_log_locked.txt]: リネームに失敗:"; let log_path = directory.path("logs/info.log");
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    while !fs::read_to_string(&log_path).is_ok_and(|log| log.contains(failure)) {
        assert!(monitor.0.try_wait().expect("監視childの状態を取得できません").is_none(), "保存失敗後に監視が終了しました");
        assert!(std::time::Instant::now() < deadline, "実archive lockの失敗を検知しません"); std::thread::sleep(Duration::from_millis(20));
    }
    let failure_wait_ms = failed_at.elapsed().as_millis(); assert!(failure_wait_ms >= 3_000);
    wait_owned_archive(&other_archive, b"other owned log");
    let mut bytes = Vec::new(); locked.read_to_end(&mut bytes).expect("所有するlockから旧archiveを読めません"); assert_eq!(bytes, previous); drop(locked);
    assert_eq!(fs::read(&source).expect("故障後の元logを読めません"), expected); assert_eq!(fs::read(&archive).expect("故障後の旧archiveを読めません"), previous);
    // lock解除だけでは再試行せず、通常の次の終了eventを待つ現行契約を確認する。
    std::thread::sleep(Duration::from_millis(2_200)); assert_eq!(fs::read(&archive).expect("次event前のarchiveを読めません"), previous);
    let mut second = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-second", &directory.0); second.wait_for_marker(&directory.path("owned-vrchat-second.started"));
    assert_owned_vrchat_processes(&[second.0.id()]); assert_eq!(find_vrchat_pid(), Some(second.0.id()));
    std::thread::sleep(Duration::from_millis(2_200)); assert_eq!(fs::read(&archive).expect("稼働中のarchiveを読めません"), previous);
    let retried_at = std::time::Instant::now(); stop_owned_monitor_child(&mut second); assert_owned_vrchat_processes(&[]); wait_owned_archive(&archive, expected);
    let retry_wait_ms = retried_at.elapsed().as_millis(); assert!(retry_wait_ms >= 3_000);
    assert_eq!(fs::read(&source).expect("再試行後の元logを読めません"), expected); assert_eq!(fs::read(&other_source).expect("別元logを読めません"), b"other owned log");
    assert_eq!(fs::read(&other_archive).expect("別archiveを読めません"), b"other owned log");
    assert!(!archive.with_extension("polaris_tmp").exists()); assert!(!other_archive.with_extension("polaris_tmp").exists());
    let log = fs::read_to_string(&log_path).expect("監視記録を読めません"); assert_eq!(log.matches(failure).count(), 1);
    assert_eq!(log.matches("VRChatの終了を検知。バックアップを開始します。").count(), 2);
    assert!(monitor.0.try_wait().expect("再試行後の監視状態を取得できません").is_none()); stop_owned_monitor_child(&mut monitor);
    assert_eq!(fs::read(directory.path("monitor.completed")).expect("監視完了markerがありません"), b"completed"); assert_eq!(find_vrchat_pid(), None);
    write_native_receipt("POLARIS_MONITOR_RETRY_TEST_RECEIPT_PATH", "polaris-native-monitor-retry-evidence.json",
        &format!("{{\"failure_wait_ms\":{failure_wait_ms},\"retry_wait_ms\":{retry_wait_ms},\"detections\":2,\"failures\":1,\"previous_archive_preserved\":true,\"source_bytes_preserved\":true,\"next_event_retry_succeeded\":true}}"));
    drop(mutex);
}

#[test]
fn actual_polling_loop_retries_source_lock_failure_on_next_exit() {
    use std::io::Read;
    use std::os::windows::fs::OpenOptionsExt;
    let process_lock = lock_native_process_tests(); assert_owned_vrchat_processes(&[]);
    let name: Vec<u16> = SINGLE_INSTANCE_MUTEX_NAME.encode_utf16().chain([0]).collect();
    let mutex = OwnedNativeHandle(unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.expect("検証用mutexを取得できません"));
    assert_ne!(unsafe { GetLastError() }, ERROR_ALREADY_EXISTS, "実Polarisまたは他の検証が動作しているため開始できません");
    let directory = TestDir::new(); let executable = directory.path("VRChat.exe"); fs::copy(std::env::current_exe().expect("test exeを取得できません"), &executable).expect("専用process imageを作成できません");
    fs::create_dir(directory.path("source")).expect("専用sourceを作成できません"); fs::create_dir(directory.path("archive")).expect("専用archiveを作成できません");
    let source = directory.path("source/output_log_source_locked.txt"); let archive = directory.path("archive/output_log_source_locked.txt");
    let other_source = directory.path("source/output_log_other.txt"); let other_archive = directory.path("archive/output_log_other.txt");
    let expected = "読込故障後に再試行する専用ログ\n".as_bytes(); let previous = "旧archiveを保持する\n".as_bytes(); let other = "他の正常なログ\n".as_bytes();
    fs::write(&source, expected).expect("専用logを書き込めません"); fs::write(&archive, previous).expect("旧archiveを書き込めません"); fs::write(&other_source, other).expect("別logを書き込めません");
    assert_ne!(expected.len(), previous.len()); let mut locked = fs::OpenOptions::new().read(true).share_mode(0).open(&source).expect("コピー元だけをlockできません");
    assert_eq!(fs::metadata(&source).expect("lock中のsizeを取得できません").len(), expected.len() as u64);
    let test = "tests::actual_polling_loop_handles_owned_exit_and_pid_replacement";
    let mut first = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-first", &directory.0); first.wait_for_marker(&directory.path("owned-vrchat-first.started")); assert_owned_vrchat_processes(&[first.0.id()]);
    let mut monitor = OwnedTestChild::start(test, "owned-monitor", &directory.0); monitor.wait_for_marker(&directory.path("monitor.started"));
    std::thread::sleep(Duration::from_millis(2_200)); assert_eq!(fs::read(&archive).expect("稼働中の旧archiveを読めません"), previous); assert!(!other_archive.exists());
    let failed_at = std::time::Instant::now(); stop_owned_monitor_child(&mut first); assert_owned_vrchat_processes(&[]);
    let failure = "バックアップに失敗 [output_log_source_locked.txt]: 一時ファイルへのコピーに失敗:"; let log_path = directory.path("logs/info.log");
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    while !fs::read_to_string(&log_path).is_ok_and(|log| log.contains(failure)) {
        assert!(monitor.0.try_wait().expect("監視状態を取得できません").is_none(), "元logの読込故障後に監視が終了しました");
        assert!(std::time::Instant::now() < deadline, "実監視からの元logコピー失敗を検知しません"); std::thread::sleep(Duration::from_millis(20));
    }
    let failure_wait_ms = failed_at.elapsed().as_millis(); assert!(failure_wait_ms >= 3_500, "終了待機と安定待ちを通していません");
    wait_owned_archive(&other_archive, other);
    // 保持違反で検証を打ち切らず、同じmonitorの次eventによる復旧まで確認してから保持をassertする。
    let previous_archive_preserved = fs::read(&archive).ok().as_deref() == Some(previous);
    let mut bytes = Vec::new(); locked.read_to_end(&mut bytes).expect("所有するlockから元logを読めません"); assert_eq!(bytes, expected); drop(locked);
    assert_eq!(fs::read(&source).expect("故障後の元logを読めません"), expected); let after_failure = fs::read(&archive).ok();
    std::thread::sleep(Duration::from_millis(2_200)); assert_eq!(fs::read(&archive).ok(), after_failure, "lock解除だけで再試行しています");
    let mut second = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-second", &directory.0); second.wait_for_marker(&directory.path("owned-vrchat-second.started")); assert_owned_vrchat_processes(&[second.0.id()]);
    std::thread::sleep(Duration::from_millis(2_200)); assert_eq!(fs::read(&archive).ok(), after_failure, "実process稼働中に再試行しています");
    let retried_at = std::time::Instant::now(); stop_owned_monitor_child(&mut second); assert_owned_vrchat_processes(&[]); wait_owned_archive(&archive, expected);
    let retry_wait_ms = retried_at.elapsed().as_millis(); assert!(retry_wait_ms >= 3_500);
    assert_eq!(fs::read(&source).expect("再試行後の元logを読めません"), expected); assert_eq!(fs::read(&other_source).expect("別元logを読めません"), other); assert_eq!(fs::read(&other_archive).expect("別archiveを読めません"), other);
    assert!(!archive.with_extension("polaris_tmp").exists()); assert!(!other_archive.with_extension("polaris_tmp").exists());
    let log = fs::read_to_string(&log_path).expect("監視記録を読めません"); assert_eq!(log.matches(failure).count(), 1); assert_eq!(log.matches("VRChatの終了を検知。バックアップを開始します。").count(), 2);
    assert!(monitor.0.try_wait().expect("復旧後の監視状態を取得できません").is_none()); stop_owned_monitor_child(&mut monitor);
    assert_eq!(fs::read(directory.path("monitor.completed")).expect("監視完了markerがありません"), b"completed"); assert_eq!(find_vrchat_pid(), None);
    write_native_receipt("POLARIS_MONITOR_SOURCE_RETRY_TEST_RECEIPT_PATH", "polaris-native-monitor-source-retry-evidence.json",
        &format!("{{\"failure_wait_ms\":{failure_wait_ms},\"retry_wait_ms\":{retry_wait_ms},\"detections\":2,\"failures\":1,\"previous_archive_preserved\":{previous_archive_preserved},\"source_bytes_preserved\":true,\"other_archive_preserved\":true,\"same_monitor_next_event_retry_succeeded\":true}}"));
    drop(mutex);
    // 所有processの終了後に排他guardを解放し、保持違反のpanicを別caseのlockへ伝播させない。
    drop(process_lock);
    assert!(previous_archive_preserved, "元logの読込故障によって次event前の旧archiveが消失しました");
}

#[test]
fn actual_polling_loop_retries_growing_source_on_next_exit() {
    use std::io::Write;
    let _process_lock = lock_native_process_tests(); assert_owned_vrchat_processes(&[]);
    let name: Vec<u16> = SINGLE_INSTANCE_MUTEX_NAME.encode_utf16().chain([0]).collect();
    let mutex = OwnedNativeHandle(unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.expect("検証用mutexを取得できません"));
    assert_ne!(unsafe { GetLastError() }, ERROR_ALREADY_EXISTS, "実Polarisまたは他の検証が動作しているため開始できません");
    let directory = TestDir::new(); let executable = directory.path("VRChat.exe"); fs::copy(std::env::current_exe().expect("test exeを取得できません"), &executable).expect("専用process imageを作成できません");
    fs::create_dir(directory.path("source")).expect("専用sourceを作成できません"); fs::create_dir(directory.path("archive")).expect("専用archiveを作成できません");
    let source = directory.path("source/output_log_growing.txt"); let archive = directory.path("archive/output_log_growing.txt");
    let other_source = directory.path("source/output_log_other.txt"); let other_archive = directory.path("archive/output_log_other.txt"); let preserved = directory.path("archive/保持する保存済みログ.txt");
    let initial = "書込開始前の専用ログ\n".as_bytes(); let appended = "継続書込の行\n".as_bytes(); let other = "別の終了済みログ\n".as_bytes(); let previous = "保持する既存archive\n".as_bytes();
    fs::write(&source, initial).expect("専用logを書き込めません"); fs::write(&other_source, other).expect("別logを書き込めません"); fs::write(&preserved, previous).expect("既存archiveを書き込めません");
    let test = "tests::actual_polling_loop_handles_owned_exit_and_pid_replacement";
    let mut first = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-first", &directory.0); first.wait_for_marker(&directory.path("owned-vrchat-first.started"));
    assert_owned_vrchat_processes(&[first.0.id()]); assert_eq!(find_vrchat_pid(), Some(first.0.id()));
    let mut monitor = OwnedTestChild::start(test, "owned-monitor", &directory.0); monitor.wait_for_marker(&directory.path("monitor.started"));
    std::thread::sleep(Duration::from_millis(2_200)); assert!(!archive.exists()); assert!(!other_archive.exists());
    let mut writer = fs::OpenOptions::new().append(true).open(&source).expect("専用logの書込handleを開けません"); let stop = AtomicBool::new(false);
    let (started, received) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let guard = StopGrowth(&stop); let should_stop = &stop;
        let worker = scope.spawn(move || {
            let mut append_count = 0_u64; let deadline = std::time::Instant::now() + Duration::from_secs(15);
            while !should_stop.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
                writer.write_all(appended).expect("専用logの書込を継続できません"); writer.flush().expect("専用logの書込を反映できません"); append_count += 1;
                if append_count == 1 { started.send(()).expect("書込開始を通知できません"); }
                std::thread::sleep(Duration::from_millis(20));
            }
            append_count
        });
        received.recv_timeout(Duration::from_secs(15)).expect("専用logの書込が開始されません");
        let failed_at = std::time::Instant::now(); stop_owned_monitor_child(&mut first); assert_owned_vrchat_processes(&[]);
        let failure = "ファイルサイズが安定しませんでした [output_log_growing.txt]"; let log_path = directory.path("logs/info.log");
        let deadline = std::time::Instant::now() + Duration::from_secs(12);
        while !fs::read_to_string(&log_path).is_ok_and(|log| log.contains(failure)) {
            assert!(monitor.0.try_wait().expect("監視childの状態を取得できません").is_none(), "書込継続中に監視が終了しました");
            assert!(std::time::Instant::now() < deadline, "書込継続による実backupの中止を検知しません"); std::thread::sleep(Duration::from_millis(20));
        }
        let failure_wait_ms = failed_at.elapsed().as_millis(); assert!(failure_wait_ms >= 6_000, "終了後の待機と安定待ち上限を通していません");
        assert!(!archive.exists()); assert!(!archive.with_extension("polaris_tmp").exists()); wait_owned_archive(&other_archive, other);
        assert_eq!(fs::read(&preserved).expect("既存archiveを読めません"), previous);
        drop(guard); let append_count = worker.join().expect("専用logの書込workerが失敗しました"); assert!(append_count > 1);
        let mut expected = initial.to_vec(); for _ in 0..append_count { expected.extend_from_slice(appended); }
        assert_eq!(fs::read(&source).expect("書込停止後の元logを読めません"), expected);
        // 入力の成長停止だけでは公開せず、同じ監視processの次の終了eventで再試行する。
        std::thread::sleep(Duration::from_millis(2_200)); assert!(!archive.exists()); assert!(!archive.with_extension("polaris_tmp").exists());
        let mut second = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-second", &directory.0); second.wait_for_marker(&directory.path("owned-vrchat-second.started"));
        assert_owned_vrchat_processes(&[second.0.id()]); assert_eq!(find_vrchat_pid(), Some(second.0.id()));
        std::thread::sleep(Duration::from_millis(2_200)); assert!(!archive.exists());
        let retried_at = std::time::Instant::now(); stop_owned_monitor_child(&mut second); assert_owned_vrchat_processes(&[]); wait_owned_archive(&archive, &expected);
        let retry_wait_ms = retried_at.elapsed().as_millis(); assert!(retry_wait_ms >= 3_500, "終了後の待機と安定確認を通していません");
        assert_eq!(fs::read(&source).expect("再試行後の元logを読めません"), expected); assert_eq!(fs::read(&other_source).expect("別元logを読めません"), other);
        assert_eq!(fs::read(&other_archive).expect("別archiveを読めません"), other); assert_eq!(fs::read(&preserved).expect("既存archiveを読めません"), previous);
        assert!(!archive.with_extension("polaris_tmp").exists()); assert!(!other_archive.with_extension("polaris_tmp").exists());
        let log = fs::read_to_string(&log_path).expect("監視記録を読めません"); assert_eq!(log.matches(failure).count(), 1);
        assert_eq!(log.matches("VRChatの終了を検知。バックアップを開始します。").count(), 2);
        assert!(monitor.0.try_wait().expect("再試行後の監視状態を取得できません").is_none()); stop_owned_monitor_child(&mut monitor);
        assert_eq!(fs::read(directory.path("monitor.completed")).expect("監視完了markerがありません"), b"completed"); assert_eq!(find_vrchat_pid(), None);
        write_native_receipt("POLARIS_MONITOR_GROWING_RETRY_TEST_RECEIPT_PATH", "polaris-native-monitor-growing-retry-evidence.json",
            &format!("{{\"failure_wait_ms\":{failure_wait_ms},\"retry_wait_ms\":{retry_wait_ms},\"detections\":2,\"failures\":1,\"growth_writes\":{append_count},\"saved_bytes\":{},\"no_partial_archive\":true,\"other_archive_preserved\":true,\"source_bytes_preserved\":true,\"no_immediate_retry_after_growth_stopped\":true,\"same_monitor_next_event_retry_succeeded\":true}}", expected.len()));
    });
    drop(mutex);
}

#[test]
fn actual_polling_loop_recovers_archive_directory_failure_on_pid_replacement() {
    let _process_lock = lock_native_process_tests(); assert_owned_vrchat_processes(&[]);
    let name: Vec<u16> = SINGLE_INSTANCE_MUTEX_NAME.encode_utf16().chain([0]).collect();
    let mutex = OwnedNativeHandle(unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.expect("検証用mutexを取得できません"));
    assert_ne!(unsafe { GetLastError() }, ERROR_ALREADY_EXISTS, "実Polarisまたは他の検証が動作しているため開始できません");
    let directory = TestDir::new(); let executable = directory.path("VRChat.exe");
    fs::copy(std::env::current_exe().expect("test exeを取得できません"), &executable).expect("専用process imageを作成できません");
    fs::create_dir(directory.path("source")).expect("専用sourceを作成できません");
    let expected = "保存先の修復後に利用する日本語ログ🙂\r\n".as_bytes();
    let other = b"other complete log\n"; let occupied = b"owned occupied archive path\n";
    let source = directory.path("source/output_log_recovered.txt"); let archive = directory.path("archive/output_log_recovered.txt");
    let other_source = directory.path("source/output_log_other.TXT"); let other_archive = directory.path("archive/output_log_other.TXT");
    fs::write(&source, expected).expect("専用logを書き込めません"); fs::write(&other_source, other).expect("別の専用logを書き込めません");
    fs::write(directory.path("source/ignored.txt"), b"ignored source\n").expect("対象外入力を書き込めません");
    fs::write(directory.path("archive"), occupied).expect("保存先を占有する専用fileを書き込めません");
    let test = "tests::actual_polling_loop_handles_owned_exit_and_pid_replacement";
    let mut first = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-first", &directory.0);
    first.wait_for_marker(&directory.path("owned-vrchat-first.started")); assert_owned_vrchat_processes(&[first.0.id()]);
    let mut monitor = OwnedTestChild::start(test, "owned-monitor", &directory.0); monitor.wait_for_marker(&directory.path("monitor.started"));
    std::thread::sleep(Duration::from_millis(2_200));
    assert_eq!(fs::read(directory.path("archive")).expect("稼働中の占有fileを読めません"), occupied);
    let failed_at = std::time::Instant::now(); stop_owned_monitor_child(&mut first); assert_owned_vrchat_processes(&[]);
    let log_path = directory.path("logs/info.log"); let failure = "archiveフォルダを作成できませんでした";
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    while !fs::read_to_string(&log_path).is_ok_and(|log| log.contains(failure)) {
        assert!(monitor.0.try_wait().expect("監視状態を取得できません").is_none(), "保存先の作成失敗で監視が終了しました");
        assert!(std::time::Instant::now() < deadline, "保存先の作成失敗を記録しません"); std::thread::sleep(Duration::from_millis(20));
    }
    let failure_wait_ms = failed_at.elapsed().as_millis(); assert!(failure_wait_ms >= 3_000);
    assert_eq!(fs::read(directory.path("archive")).expect("失敗後の占有fileを読めません"), occupied);
    assert_eq!(fs::read(&source).expect("失敗後の元logを読めません"), expected); assert_eq!(fs::read(&other_source).expect("失敗後の別logを読めません"), other);
    fs::remove_file(directory.path("archive")).expect("所有する保存先の故障を除去できません");
    // 保存先の修復だけでは即時再試行しない。同じ監視processで次のPID交代へ接続する。
    std::thread::sleep(Duration::from_millis(2_200)); assert!(!directory.path("archive").exists());
    let mut second = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-second", &directory.0);
    second.wait_for_marker(&directory.path("owned-vrchat-second.started")); assert_owned_vrchat_processes(&[second.0.id()]);
    std::thread::sleep(Duration::from_millis(2_200)); assert!(!directory.path("archive").exists());
    let mut third = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-third", &directory.0);
    third.wait_for_marker(&directory.path("owned-vrchat-third.started")); assert_owned_vrchat_processes(&[second.0.id(), third.0.id()]);
    let retried_at = std::time::Instant::now(); stop_owned_monitor_child(&mut second); assert_owned_vrchat_processes(&[third.0.id()]);
    assert_eq!(find_vrchat_pid(), Some(third.0.id())); wait_owned_archive(&archive, expected); wait_owned_archive(&other_archive, other);
    let retry_wait_ms = retried_at.elapsed().as_millis(); assert!(retry_wait_ms >= 3_500);
    assert_eq!(fs::read(&source).expect("再試行後の元logを読めません"), expected); assert_eq!(fs::read(&other_source).expect("再試行後の別logを読めません"), other);
    assert_eq!(fs::read(directory.path("source/ignored.txt")).expect("対象外の原本を読めません"), b"ignored source\n");
    assert_eq!(fs::read_dir(directory.path("archive")).expect("再試行後のarchiveを列挙できません").count(), 2);
    let log = fs::read_to_string(&log_path).expect("監視記録を読めません"); assert_eq!(log.matches(failure).count(), 1);
    assert_eq!(log.matches("VRChatの終了を検知。バックアップを開始します。").count(), 2);
    assert!(monitor.0.try_wait().expect("復旧後の監視状態を取得できません").is_none()); stop_owned_monitor_child(&mut monitor);
    assert_eq!(fs::read(directory.path("monitor.completed")).expect("監視完了markerがありません"), b"completed");
    stop_owned_monitor_child(&mut third); assert_owned_vrchat_processes(&[]); assert_eq!(find_vrchat_pid(), None);
    write_native_receipt("POLARIS_MONITOR_ARCHIVE_DIRECTORY_TEST_RECEIPT_PATH", "polaris-native-monitor-archive-directory-evidence.json",
        &format!("{{\"failure_wait_ms\":{failure_wait_ms},\"retry_wait_ms\":{retry_wait_ms},\"detections\":2,\"directory_creation_failures\":1,\"occupied_file_preserved_on_failure\":true,\"source_bytes_preserved\":true,\"no_immediate_retry_after_repair\":true,\"same_monitor_pid_replacement_retry_succeeded\":true,\"copied_files\":2,\"ignored_source_not_archived\":true,\"temporary_files_absent\":true}}"));
    drop(mutex);
}

#[test]
fn actual_polling_loop_recovers_source_directory_failure_on_next_exit() {
    let _process_lock = lock_native_process_tests(); assert_owned_vrchat_processes(&[]);
    let name: Vec<u16> = SINGLE_INSTANCE_MUTEX_NAME.encode_utf16().chain([0]).collect();
    let mutex = OwnedNativeHandle(unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.expect("検証用mutexを取得できません"));
    assert_ne!(unsafe { GetLastError() }, ERROR_ALREADY_EXISTS, "実Polarisまたは他の検証が動作しているため開始できません");
    let directory = TestDir::new(); let executable = directory.path("VRChat.exe");
    fs::copy(std::env::current_exe().expect("test exeを取得できません"), &executable).expect("専用process imageを作成できません");
    fs::create_dir(directory.path("source")).expect("専用sourceを作成できません"); fs::create_dir(directory.path("archive")).expect("専用archiveを作成できません");
    let expected = "元logフォルダ復旧後に利用する本文🙂\n".as_bytes(); let other = b"new source after directory restoration\n";
    let previous = b"previous complete archive\n"; let retained = "保持する無関係なarchive\n".as_bytes(); assert_ne!(expected.len(), previous.len());
    let source = directory.path("source/output_log_recovered.txt"); let archive = directory.path("archive/output_log_recovered.txt");
    let other_source = directory.path("source/output_log_new.txt"); let other_archive = directory.path("archive/output_log_new.txt");
    let unrelated = directory.path("archive/output_log_retained.txt");
    fs::write(&source, expected).expect("専用logを書き込めません"); fs::write(&other_source, other).expect("別logを書き込めません");
    fs::write(&archive, previous).expect("旧archiveを書き込めません"); fs::write(&unrelated, retained).expect("保持対象のarchiveを書き込めません");
    let test = "tests::actual_polling_loop_handles_owned_exit_and_pid_replacement";
    let mut first = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-first", &directory.0);
    first.wait_for_marker(&directory.path("owned-vrchat-first.started")); assert_owned_vrchat_processes(&[first.0.id()]);
    let mut monitor = OwnedTestChild::start(test, "owned-monitor", &directory.0); monitor.wait_for_marker(&directory.path("monitor.started"));
    std::thread::sleep(Duration::from_millis(2_200)); assert_eq!(fs::read(&archive).expect("稼働中の旧archiveを読めません"), previous); assert!(!other_archive.exists());
    // 原本を削除せず、所有するfolderの名前だけを一時変更して実read_dirの失敗を起こす。
    fs::rename(directory.path("source"), directory.path("source-offline")).expect("所有する元logフォルダを一時退避できません");
    let failed_at = std::time::Instant::now(); stop_owned_monitor_child(&mut first); assert_owned_vrchat_processes(&[]);
    let log_path = directory.path("logs/info.log"); let failure = "ログフォルダを開けませんでした";
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    while !fs::read_to_string(&log_path).is_ok_and(|log| log.contains(failure)) {
        assert!(monitor.0.try_wait().expect("監視状態を取得できません").is_none(), "元logの列挙失敗で監視が終了しました");
        assert!(std::time::Instant::now() < deadline, "元logの列挙失敗を記録しません"); std::thread::sleep(Duration::from_millis(20));
    }
    let failure_wait_ms = failed_at.elapsed().as_millis(); assert!(failure_wait_ms >= 3_000);
    assert_eq!(fs::read(&archive).expect("列挙失敗後の旧archiveを読めません"), previous); assert_eq!(fs::read(&unrelated).expect("列挙失敗後の別archiveを読めません"), retained);
    assert_eq!(fs::read(directory.path("source-offline/output_log_recovered.txt")).expect("退避中の元logを読めません"), expected);
    assert_eq!(fs::read(directory.path("source-offline/output_log_new.txt")).expect("退避中の別logを読めません"), other); assert!(!other_archive.exists());
    fs::rename(directory.path("source-offline"), directory.path("source")).expect("所有する元logフォルダを元に戻せません");
    std::thread::sleep(Duration::from_millis(2_200)); assert_eq!(fs::read(&archive).expect("次event前の旧archiveを読めません"), previous); assert!(!other_archive.exists());
    let mut second = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-second", &directory.0);
    second.wait_for_marker(&directory.path("owned-vrchat-second.started")); assert_owned_vrchat_processes(&[second.0.id()]);
    std::thread::sleep(Duration::from_millis(2_200)); assert_eq!(fs::read(&archive).expect("次session稼働中の旧archiveを読めません"), previous); assert!(!other_archive.exists());
    let retried_at = std::time::Instant::now(); stop_owned_monitor_child(&mut second); assert_owned_vrchat_processes(&[]);
    wait_owned_archive(&archive, expected); wait_owned_archive(&other_archive, other);
    let retry_wait_ms = retried_at.elapsed().as_millis(); assert!(retry_wait_ms >= 3_500);
    assert_eq!(fs::read(&source).expect("再試行後の元logを読めません"), expected); assert_eq!(fs::read(&other_source).expect("再試行後の別logを読めません"), other);
    assert_eq!(fs::read(&unrelated).expect("再試行後の保持対象archiveを読めません"), retained);
    assert_eq!(fs::read_dir(directory.path("archive")).expect("再試行後のarchiveを列挙できません").count(), 3);
    let log = fs::read_to_string(&log_path).expect("監視記録を読めません"); assert_eq!(log.matches(failure).count(), 1);
    assert_eq!(log.matches("VRChatの終了を検知。バックアップを開始します。").count(), 2);
    assert!(monitor.0.try_wait().expect("復旧後の監視状態を取得できません").is_none()); stop_owned_monitor_child(&mut monitor);
    assert_eq!(fs::read(directory.path("monitor.completed")).expect("監視完了markerがありません"), b"completed"); assert_eq!(find_vrchat_pid(), None);
    write_native_receipt("POLARIS_MONITOR_SOURCE_DIRECTORY_TEST_RECEIPT_PATH", "polaris-native-monitor-source-directory-evidence.json",
        &format!("{{\"failure_wait_ms\":{failure_wait_ms},\"retry_wait_ms\":{retry_wait_ms},\"detections\":2,\"source_directory_failures\":1,\"previous_archive_preserved_on_failure\":true,\"unrelated_archive_preserved\":true,\"source_bytes_preserved\":true,\"no_immediate_retry_after_repair\":true,\"same_monitor_next_exit_retry_succeeded\":true,\"copied_files\":2,\"temporary_files_absent\":true}}"));
    drop(mutex);
}
