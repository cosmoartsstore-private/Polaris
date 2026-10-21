// 画面を伴わない監視・保存・再利用の不足条件を、所有するfixtureとprocessで確認する。

#[test]
fn headless_producer_exports_owned_logs_on_exit() {
    let _process_lock = lock_native_process_tests(); assert_owned_vrchat_processes(&[]);
    let name: Vec<u16> = SINGLE_INSTANCE_MUTEX_NAME.encode_utf16().chain([0]).collect();
    let mutex = OwnedNativeHandle(unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.expect("検証用mutexを取得できません"));
    assert_ne!(unsafe { GetLastError() }, ERROR_ALREADY_EXISTS, "実Polarisが動作しているため開始できません");
    let temporary = TestDir::new();
    let root = if let Some(root) = std::env::var_os("POLARIS_HEADLESS_PRODUCER_FIXTURE_ROOT") {
        let root = PathBuf::from(root).canonicalize().expect("consumer専用fixtureを解決できません");
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("workspaceがありません");
        let boundary = workspace.join(".claude/audit-work/checks").canonicalize().expect("検証境界を解決できません");
        assert!(root.starts_with(&boundary) && root != boundary);
        assert_eq!(fs::read_to_string(root.join("owner.marker")).expect("所有markerがありません").trim(), "polaris-headless-consumer-fixture-v1"); root
    } else {
        fs::create_dir(temporary.path("source")).expect("専用sourceを作成できません");
        fs::write(temporary.path("source/output_log_owned_consumer.txt"), b"owned producer body\r\n").expect("原本を書き込めません");
        fs::write(temporary.path("expected-files.txt"), b"output_log_owned_consumer.txt\n").expect("期待名を書き込めません"); temporary.0.clone()
    };
    assert!(!root.join("Data/archive").exists(), "完成済みのconsumer fixtureは上書きしません");
    let sources = startup_file_snapshot(&root.join("source"));
    let mut names: Vec<String> = fs::read_to_string(root.join("expected-files.txt")).expect("期待名を読めません").lines().map(str::trim).filter(|name| !name.is_empty()).map(str::to_owned).collect();
    assert!(!names.is_empty());
    for name in &names { assert!(!name.contains(['/', '\\']) && name.starts_with("output_log_") && name.to_ascii_lowercase().ends_with(".txt")); assert!(!sources[Path::new(name)].is_empty()); }
    names.sort(); assert!(!names.windows(2).any(|pair| pair[0] == pair[1]));
    let executable = root.join("VRChat.exe"); assert!(!executable.exists());
    fs::copy(std::env::current_exe().expect("test exeがありません"), &executable).expect("専用process imageを作成できません");
    let test = "tests::actual_polling_loop_handles_owned_exit_and_pid_replacement";
    let mut session = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-first", &root); session.wait_for_marker(&root.join("owned-vrchat-first.started"));
    assert_owned_vrchat_processes(&[session.0.id()]);
    let mut monitor = OwnedTestChild::start(test, "owned-consumer-monitor", &root); monitor.wait_for_marker(&root.join("monitor.started"));
    std::thread::sleep(Duration::from_millis(2_200)); assert!(!root.join("Data/archive").exists());
    let exited = std::time::Instant::now(); stop_owned_monitor_child(&mut session); assert_owned_vrchat_processes(&[]);
    for name in &names { wait_owned_archive(&root.join("Data/archive").join(name), &sources[Path::new(name)]); }
    let wait_ms = exited.elapsed().as_millis(); assert!(wait_ms >= 3_000);
    let mut actual: Vec<_> = fs::read_dir(root.join("Data/archive")).expect("公開先を列挙できません").map(|entry| entry.expect("公開entryがありません").file_name().into_string().expect("公開名がUTF-8ではありません")).collect();
    actual.sort(); assert_eq!(actual, names); assert_eq!(startup_file_snapshot(&root.join("source")), sources);
    assert!(monitor.0.try_wait().expect("監視状態を取得できません").is_none()); stop_owned_monitor_child(&mut monitor);
    assert_eq!(fs::read(root.join("monitor.completed")).expect("監視完了がありません"), b"completed"); assert_owned_vrchat_processes(&[]);
    fs::write(root.join("producer.receipt.json"), format!("{{\"monitor_wait_ms\":{wait_ms},\"source_bytes_preserved\":true,\"published_files\":{},\"owned_session_completed\":true,\"owned_monitor_completed\":true}}", names.len())).expect("producer記録を保存できません");
    fs::remove_file(executable).expect("所有process imageを削除できません"); drop(mutex);
}

#[test]
fn safe_copy_replaces_archive_atomically_for_existing_and_new_readers() {
    use std::io::Read;
    use std::os::windows::fs::OpenOptionsExt;
    let directory = TestDir::new(); let source = directory.path("source.txt"); let archive = directory.path("archive.txt");
    let previous = b"previous complete archive\r\n"; let expected = "新規readerに公開する完成本文🙂\n".as_bytes();
    fs::write(&source, expected).expect("原本を書き込めません"); fs::write(&archive, previous).expect("旧archiveを書き込めません");
    let mut reader = fs::OpenOptions::new().read(true).share_mode(7).open(&archive).expect("既存readerを開けません");
    safe_copy(&source, &archive).expect("完成したarchiveへ置換できません");
    let mut retained = Vec::new(); reader.read_to_end(&mut retained).expect("既存readerから読めません");
    assert_eq!(retained, previous); assert_eq!(fs::read(&archive).expect("新規readerから読めません"), expected);
    assert_eq!(fs::read(&source).expect("原本を読めません"), expected); assert!(!archive.with_extension("polaris_tmp").exists());
}

#[test]
fn backup_accepts_empty_folder_and_preserves_binary_body_and_long_unicode_name() {
    let directory = TestDir::new(); let paths = RuntimePaths { vrchat_log_folder: directory.path("source"), archive_folder: directory.path("archive") };
    fs::create_dir(&paths.vrchat_log_folder).expect("sourceを作成できません"); backup(&paths);
    assert_eq!(fs::read_dir(&paths.archive_folder).expect("空archiveを列挙できません").count(), 0);
    let name = format!("output_log_{}.TxT", "日本語🙂".repeat(25)); let expected = b"\0\xff\xfe\r\nowned binary body\0";
    fs::write(paths.vrchat_log_folder.join(&name), expected).expect("binary原本を書き込めません"); backup(&paths);
    assert_eq!(fs::read(paths.archive_folder.join(&name)).expect("binary本文を読めません"), expected);
    assert_eq!(fs::read(paths.vrchat_log_folder.join(&name)).expect("原本を読めません"), expected);
    assert_eq!(fs::read_dir(&paths.archive_folder).expect("archiveを列挙できません").count(), 1);
}

// 属性を変えた所有fileは、panic時も元の属性へ戻してからfixtureを解放する。
struct RestoreOwnedPermissions<'a> { path: &'a Path, previous: fs::Permissions }
impl<'a> RestoreOwnedPermissions<'a> {
    fn readonly(path: &'a Path) -> Self {
        let previous = fs::metadata(path).expect("所有fileの属性を読めません").permissions();
        let mut readonly = previous.clone(); readonly.set_readonly(true); fs::set_permissions(path, readonly).expect("readonly属性を設定できません");
        Self { path, previous }
    }
}
impl Drop for RestoreOwnedPermissions<'_> { fn drop(&mut self) { let _ = fs::set_permissions(self.path, self.previous.clone()); } }

#[test]
fn backup_preserves_readonly_files_and_recovers_after_attribute_repair() {
    let directory = TestDir::new(); let paths = RuntimePaths { vrchat_log_folder: directory.path("source"), archive_folder: directory.path("archive") };
    fs::create_dir(&paths.vrchat_log_folder).expect("sourceを作成できません"); fs::create_dir(&paths.archive_folder).expect("archiveを作成できません");
    let readonly_source = paths.vrchat_log_folder.join("output_log_readonly_source.txt"); let copied = paths.archive_folder.join("output_log_readonly_source.txt");
    let source = paths.vrchat_log_folder.join("output_log_readonly_target.txt"); let archive = paths.archive_folder.join("output_log_readonly_target.txt");
    let orphan = paths.archive_folder.join("orphan.polaris_tmp"); let previous = b"previous complete log\n"; let expected = b"replacement after readonly attribute repair\r\n";
    fs::write(&readonly_source, b"readonly source bytes\n").expect("readonly原本を書き込めません");
    fs::write(&copied, b"").expect("属性復元用の宛先を準備できません");
    let copied_permissions = fs::metadata(&copied).expect("宛先属性を読めません").permissions(); let _copied_guard = RestoreOwnedPermissions { path: &copied, previous: copied_permissions };
    let _source_guard = RestoreOwnedPermissions::readonly(&readonly_source);
    fs::write(&source, expected).expect("更新原本を書き込めません"); fs::write(&archive, previous).expect("旧archiveを書き込めません");
    let target_guard = RestoreOwnedPermissions::readonly(&archive);
    backup(&paths);
    assert_eq!(fs::read(&copied).expect("readonly原本のコピーを読めません"), b"readonly source bytes\n");
    assert!(fs::metadata(&readonly_source).expect("原本属性を読めません").permissions().readonly());
    assert_eq!(fs::read(&archive).expect("readonly旧archiveを読めません"), previous); assert_eq!(fs::read(&source).expect("原本を読めません"), expected);
    drop(target_guard); backup(&paths); assert_eq!(fs::read(&archive).expect("属性修復後のarchiveを読めません"), expected);
    fs::write(&orphan, b"incomplete owned bytes").expect("未完了fileを書き込めません"); let orphan_guard = RestoreOwnedPermissions::readonly(&orphan);
    assert!(fs::metadata(&orphan).expect("未完了fileの属性を読めません").permissions().readonly());
    backup(&paths); assert!(!orphan.exists(), "標準権限で削除できる所有tmpを残してはいけません");
    assert_eq!(fs::read(&archive).expect("完成archiveを読めません"), expected); drop(orphan_guard);
    assert_eq!(fs::read(&readonly_source).expect("readonly原本を読めません"), b"readonly source bytes\n"); assert_eq!(fs::read(&source).expect("更新原本を読めません"), expected);
}

#[test]
fn backup_preserves_archive_when_source_disappears_during_stable_wait() {
    let directory = TestDir::new(); let paths = RuntimePaths { vrchat_log_folder: directory.path("source"), archive_folder: directory.path("archive") };
    fs::create_dir(&paths.vrchat_log_folder).expect("sourceを作成できません"); fs::create_dir(&paths.archive_folder).expect("archiveを作成できません");
    let source = paths.vrchat_log_folder.join("output_log_disappeared.txt"); let archive = paths.archive_folder.join("output_log_disappeared.txt"); let offline = directory.path("owned-offline.txt");
    let previous = b"previous completed archive"; let expected = b"complete original restored after metadata failure\r\n";
    fs::write(&source, expected).expect("原本を書き込めません"); fs::write(&archive, previous).expect("旧archiveを書き込めません");
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| { std::thread::sleep(Duration::from_millis(150)); fs::rename(&source, &offline).expect("所有原本を一時退避できません"); });
        let started = std::time::Instant::now(); backup(&paths); worker.join().expect("退避workerが終了しません");
        assert!(started.elapsed() >= Duration::from_secs(3), "安定待機開始後のmetadata失敗を通過していません");
    });
    assert_eq!(fs::read(&archive).expect("旧archiveを読めません"), previous); assert_eq!(fs::read(&offline).expect("退避原本を読めません"), expected);
    assert!(!archive.with_extension("polaris_tmp").exists()); fs::rename(&offline, &source).expect("原本を戻せません"); backup(&paths);
    assert_eq!(fs::read(&archive).expect("復旧後のarchiveを読めません"), expected); assert_eq!(fs::read(&source).expect("復旧後の原本を読めません"), expected);
}

#[test]
fn actual_monitor_ignores_unrelated_process_and_waits_for_current_pid_exit() {
    let _process_lock = lock_native_process_tests(); assert_owned_vrchat_processes(&[]); assert_startup_mutex_available();
    let directory = TestDir::new(); fs::create_dir(directory.path("source")).expect("sourceを作成できません");
    let source = directory.path("source/output_log_session.txt"); let archive = directory.path("archive/output_log_session.txt");
    fs::write(&source, b"startup complete\n").expect("起動時原本を書き込めません");
    let test = "tests::actual_polling_loop_handles_owned_exit_and_pid_replacement";
    let mut monitor = OwnedTestChild::start(test, "owned-idle-monitor", &directory.0); monitor.wait_for_marker(&directory.path("monitor.started")); wait_owned_archive(&archive, b"startup complete\n");
    let expected = b"updated body waiting for owned session exit\n"; fs::write(&source, expected).expect("更新原本を書き込めません");
    let unrelated_root = directory.path("unrelated"); fs::create_dir(&unrelated_root).expect("無関係process領域を作成できません");
    let executable = directory.path("Harmless.exe"); fs::copy(std::env::current_exe().expect("test exeがありません"), &executable).expect("無関係process imageを作成できません");
    let mut unrelated = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-first", &unrelated_root); unrelated.wait_for_marker(&unrelated_root.join("owned-vrchat-first.started"));
    std::thread::sleep(Duration::from_millis(2_200)); assert_eq!(find_vrchat_pid(), None); assert_eq!(fs::read(&archive).expect("稼働中のarchiveを読めません"), b"startup complete\n");
    stop_owned_monitor_child(&mut unrelated); std::thread::sleep(Duration::from_millis(2_200)); assert_eq!(fs::read(&archive).expect("無関係process終了後のarchiveを読めません"), b"startup complete\n");
    let executable = directory.path("VRChat.exe"); fs::copy(std::env::current_exe().expect("test exeがありません"), &executable).expect("対象process imageを作成できません");
    let mut session = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-second", &directory.0); session.wait_for_marker(&directory.path("owned-vrchat-second.started"));
    std::thread::sleep(Duration::from_millis(2_200)); assert_eq!(find_vrchat_pid(), Some(session.0.id())); assert_eq!(fs::read(&archive).expect("同PID稼働中のarchiveを読めません"), b"startup complete\n");
    stop_owned_monitor_child(&mut session); wait_owned_archive(&archive, expected); assert_eq!(fs::read(&source).expect("原本を読めません"), expected);
    stop_owned_monitor_child(&mut monitor); assert_owned_vrchat_processes(&[]);
    let log = fs::read_to_string(directory.path("logs/info.log")).expect("監視logを読めません"); assert_eq!(log.matches("VRChatの終了を検知。バックアップを開始します。").count(), 1);
}

#[test]
fn actual_monitor_handles_session_restart_during_the_exit_delay() {
    let _process_lock = lock_native_process_tests(); assert_owned_vrchat_processes(&[]); assert_startup_mutex_available();
    let directory = TestDir::new(); fs::create_dir(directory.path("source")).expect("sourceを作成できません"); fs::create_dir(directory.path("archive")).expect("archiveを作成できません");
    let source = directory.path("source/output_log_session.txt"); let archive = directory.path("archive/output_log_session.txt");
    let expected = b"completed during delay\r\n"; let next = b"following complete session body\r\n"; fs::write(&source, expected).expect("原本を書き込めません"); fs::write(&archive, b"previous").expect("旧archiveを書き込めません");
    let executable = directory.path("VRChat.exe"); fs::copy(std::env::current_exe().expect("test exeがありません"), &executable).expect("専用process imageを作成できません");
    let test = "tests::actual_polling_loop_handles_owned_exit_and_pid_replacement";
    let mut first = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-first", &directory.0); first.wait_for_marker(&directory.path("owned-vrchat-first.started"));
    let mut monitor = OwnedTestChild::start(test, "owned-monitor", &directory.0); monitor.wait_for_marker(&directory.path("monitor.started")); std::thread::sleep(Duration::from_millis(2_200));
    stop_owned_monitor_child(&mut first); let deadline = std::time::Instant::now() + Duration::from_secs(4); let log_path = directory.path("logs/info.log");
    while !fs::read_to_string(&log_path).is_ok_and(|log| log.contains("VRChatの終了を検知。バックアップを開始します。")) { assert!(std::time::Instant::now() < deadline); std::thread::sleep(Duration::from_millis(20)); }
    let mut second = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-second", &directory.0); second.wait_for_marker(&directory.path("owned-vrchat-second.started"));
    assert_eq!(find_vrchat_pid(), Some(second.0.id())); assert_eq!(fs::read(&archive).expect("待機中の旧archiveを読めません"), b"previous");
    wait_owned_archive(&archive, expected); fs::write(&source, next).expect("次session原本を書き込めません"); std::thread::sleep(Duration::from_millis(2_200));
    assert_eq!(fs::read(&archive).expect("次session稼働中のarchiveを読めません"), expected); stop_owned_monitor_child(&mut second); wait_owned_archive(&archive, next);
    assert_eq!(fs::read(&source).expect("次session原本を読めません"), next); stop_owned_monitor_child(&mut monitor); assert_owned_vrchat_processes(&[]);
    assert_eq!(fs::read_to_string(log_path).expect("監視logを読めません").matches("VRChatの終了を検知。バックアップを開始します。").count(), 2);
}

#[test]
fn actual_monitor_recovers_same_archive_after_owned_process_interruption() {
    use std::io::Read;
    use std::os::windows::fs::OpenOptionsExt;
    let _process_lock = lock_native_process_tests(); assert_owned_vrchat_processes(&[]); assert_startup_mutex_available();
    let directory = TestDir::new(); fs::create_dir(directory.path("source")).expect("sourceを作成できません"); fs::create_dir(directory.path("archive")).expect("archiveを作成できません");
    let source = directory.path("source/output_log_interrupted.txt"); let archive = directory.path("archive/output_log_interrupted.txt"); let unrelated = directory.path("archive/retained.bin");
    let expected = b"complete original after owned process interruption\r\n"; let previous = b"previous archive";
    fs::write(&source, expected).expect("原本を書き込めません"); fs::write(&archive, previous).expect("旧archiveを書き込めません"); fs::write(&unrelated, b"retained").expect("対象外archiveを書き込めません");
    let mut locked = fs::OpenOptions::new().read(true).share_mode(0).open(&archive).expect("所有archiveをlockできません");
    let executable = directory.path("VRChat.exe"); fs::copy(std::env::current_exe().expect("test exeがありません"), &executable).expect("専用process imageを作成できません");
    let test = "tests::actual_polling_loop_handles_owned_exit_and_pid_replacement";
    let mut session = OwnedTestChild::start_executable(&executable, test, "owned-vrchat-first", &directory.0); session.wait_for_marker(&directory.path("owned-vrchat-first.started"));
    let mut monitor = OwnedTestChild::start(test, "owned-monitor", &directory.0); monitor.wait_for_marker(&directory.path("monitor.started")); std::thread::sleep(Duration::from_millis(2_200)); stop_owned_monitor_child(&mut session);
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    while !fs::read_to_string(directory.path("logs/info.log")).is_ok_and(|log| log.contains("リネームに失敗")) { assert!(monitor.0.try_wait().expect("監視状態を取得できません").is_none()); assert!(std::time::Instant::now() < deadline); std::thread::sleep(Duration::from_millis(20)); }
    assert_eq!(fs::read(archive.with_extension("polaris_tmp")).expect("未公開の完成tmpを読めません"), expected);
    monitor.0.kill().expect("所有monitorを中断できません"); let status = monitor.finish(); assert!(!status.success()); assert!(!directory.path("monitor.completed").exists());
    let mut old = Vec::new(); locked.read_to_end(&mut old).expect("旧archiveを読めません"); assert_eq!(old, previous); drop(locked);
    assert_eq!(fs::read(&source).expect("中断後の原本を読めません"), expected); assert_eq!(fs::read(&unrelated).expect("対象外archiveを読めません"), b"retained");
    fs::rename(directory.path("monitor.started"), directory.path("interrupted.started")).expect("開始記録を保持できません");
    let mut recovered = OwnedTestChild::start(test, "owned-idle-monitor", &directory.0); recovered.wait_for_marker(&directory.path("monitor.started")); wait_owned_archive(&archive, expected);
    assert!(!archive.with_extension("polaris_tmp").exists()); assert_eq!(fs::read(&source).expect("復旧後の原本を読めません"), expected); assert_eq!(fs::read(&unrelated).expect("復旧後の対象外を読めません"), b"retained");
    assert!(recovered.0.try_wait().expect("復旧後の監視状態を取得できません").is_none()); stop_owned_monitor_child(&mut recovered); assert_eq!(fs::read(directory.path("monitor.completed")).expect("復旧完了がありません"), b"completed"); assert_owned_vrchat_processes(&[]);
}

#[test]
fn native_main_recovers_after_owned_install_and_source_configuration_are_repaired() {
    if let Some(scope) = startup_child_scope("headless-repair") {
        crate::dirs::set_test_profile(Some(scope.root.join("profile")));
        assert!(ready_env().err().expect("未登録を受け付けてはいけません").to_string().contains("インストール情報"));
        let (settings, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey("Software\\CosmoArtsStore\\Polaris").expect("所有設定を作成できません");
        settings.set_value("InstallLocation", &scope.root.join("missing").to_str().expect("専用pathを読めません")).expect("不在設定を保存できません");
        assert!(ready_env().err().expect("不存在のinstall先を受け付けてはいけません").to_string().contains("インストール先フォルダ"));
        settings.set_value("InstallLocation", &scope.root.to_str().expect("専用pathを読めません")).expect("修復設定を保存できません");
        assert!(ready_env().err().expect("不存在の原本先を受け付けてはいけません").to_string().contains("VRChatのログ排出先"));
        assert!(!scope.root.join("Data").exists()); prepare_startup_idle_data(&scope.root);
        fs::write(scope.root.join("repair.failures.json"), b"{\"initialization_failures\":3,\"data_created_before_repair\":false}").expect("失敗条件を記録できません");
        observe_startup_idle_main(&scope, "設定修復後の起動検証ログ"); return;
    }
    let _process_lock = lock_native_process_tests(); assert_owned_vrchat_processes(&[]); assert_startup_mutex_available();
    let input = startup_input_desktop_name(); let directory = TestDir::new(); let desktop = StartupDesktop::new(&directory.0); let registry = StartupRegistry::new(&directory.0);
    let child = StartupProcess::start("tests::native_main_recovers_after_owned_install_and_source_configuration_are_repaired", "headless-repair", &directory.0, &desktop); child.wait_before(&directory.0);
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !directory.path("startup.ready.txt").exists() { assert!(child.running(), "修復後のmainが開始しません: {}", child.diagnostic()); assert!(std::time::Instant::now() < deadline); std::thread::sleep(Duration::from_millis(20)); }
    assert_startup_idle_data(&directory.0); assert!(desktop.windows().is_empty());
    assert_eq!(fs::read(directory.path("repair.failures.json")).expect("失敗条件がありません"), b"{\"initialization_failures\":3,\"data_created_before_repair\":false}");
    std::thread::sleep(Duration::from_millis(2_200)); fs::write(directory.path("startup.stop.txt"), b"stop").expect("終了通知を保存できません");
    assert_eq!(child.finish(), 0); assert!(child.diagnostic().contains("test result: ok. 1 passed; 0 failed;")); assert_startup_idle_data(&directory.0); assert!(desktop.windows().is_empty());
    drop(child); let registry_path = registry.path.clone(); drop(registry); assert!(RegKey::predef(HKEY_CURRENT_USER).open_subkey(registry_path).is_err()); desktop.handle.close();
    assert_eq!(startup_input_desktop_name(), input); assert_startup_mutex_available();
}

// 親のpanic時にもstdinのEOFで所有ACLを復元し、復元processの終了まで待つ。
struct OwnedAclChild(OwnedTestChild);
impl Drop for OwnedAclChild {
    fn drop(&mut self) {
        drop(self.0.0.stdin.take()); let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while self.0.0.try_wait().ok().flatten().is_none() && std::time::Instant::now() < deadline { std::thread::sleep(Duration::from_millis(20)); }
    }
}

#[test]
fn backup_preserves_data_on_owned_access_denial_and_retries_after_acl_restore() {
    use std::os::windows::process::CommandExt;
    for kind in ["file", "directory"] {
        let directory = TestDir::new(); let paths = RuntimePaths { vrchat_log_folder: directory.path("source"), archive_folder: directory.path("archive") };
        fs::create_dir(&paths.vrchat_log_folder).expect("sourceを作成できません"); fs::create_dir(&paths.archive_folder).expect("archiveを作成できません");
        let source = paths.vrchat_log_folder.join("output_log_denied.txt"); let archive = paths.archive_folder.join("output_log_denied.txt"); let other = paths.archive_folder.join("output_log_other.txt");
        let previous = b"previous complete archive"; let expected = b"complete source after owned ACL restoration\r\n";
        fs::write(&source, expected).expect("原本を書き込めません"); fs::write(&archive, previous).expect("旧archiveを書き込めません");
        fs::write(paths.vrchat_log_folder.join("output_log_other.txt"), b"other source").expect("別原本を書き込めません"); fs::write(paths.archive_folder.join("retained.bin"), b"retained").expect("対象外archiveを書き込めません");
        let script = directory.path("owned-acl.ps1"); let marker = directory.path("acl.ready");
        fs::write(&script, r"$ErrorActionPreference = 'Stop'
$target = $env:POLARIS_OWNED_ACL_TARGET
$kind = $env:POLARIS_OWNED_ACL_KIND
$section = [Security.AccessControl.AccessControlSections]::Access
if ($kind -eq 'file') { $acl = [IO.File]::GetAccessControl($target) } else { $acl = [IO.Directory]::GetAccessControl($target) }
$saved = $acl.GetSecurityDescriptorSddlForm($section)
try {
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $rule = New-Object Security.AccessControl.FileSystemAccessRule($sid, [Security.AccessControl.FileSystemRights]::ReadData, [Security.AccessControl.AccessControlType]::Deny)
    $acl.AddAccessRule($rule)
    if ($kind -eq 'file') { [IO.File]::SetAccessControl($target, $acl) } else { [IO.Directory]::SetAccessControl($target, $acl) }
    [IO.File]::WriteAllText($env:POLARIS_OWNED_ACL_MARKER, 'ready')
    [void][Console]::ReadLine()
} finally {
    if ($kind -eq 'file') { $restored = New-Object Security.AccessControl.FileSecurity } else { $restored = New-Object Security.AccessControl.DirectorySecurity }
    $restored.SetSecurityDescriptorSddlForm($saved, $section)
    if ($kind -eq 'file') { [IO.File]::SetAccessControl($target, $restored) } else { [IO.Directory]::SetAccessControl($target, $restored) }
    [Console]::WriteLine('permissions-restored')
}
").expect("所有ACL手順を書き込めません");
        let target = if kind == "file" { &source } else { &paths.vrchat_log_folder };
        let process = std::process::Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"]).arg(&script)
            .env("POLARIS_OWNED_ACL_TARGET", target).env("POLARIS_OWNED_ACL_KIND", kind).env("POLARIS_OWNED_ACL_MARKER", &marker)
            .creation_flags(0x0800_0000).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().expect("所有ACL processを開始できません");
        let mut child = OwnedAclChild(OwnedTestChild(process)); child.0.wait_for_marker(&marker);
        if kind == "file" { assert!(fs::read(&source).is_err()); } else { assert!(fs::read_dir(&paths.vrchat_log_folder).is_err()); }
        backup(&paths); assert_eq!(fs::read(&archive).expect("アクセス拒否後の旧archiveを読めません"), previous);
        if kind == "file" { assert_eq!(fs::read(&other).expect("正常な別logを読めません"), b"other source"); } else { assert!(!other.exists()); }
        assert_eq!(fs::read(paths.archive_folder.join("retained.bin")).expect("対象外archiveを読めません"), b"retained");
        drop(child.0.0.stdin.take()); let status = child.0.finish(); let diagnostic = child.0.diagnostic(); assert!(status.success() && diagnostic.contains("permissions-restored"), "所有ACLを復元できません: {diagnostic}");
        assert_eq!(fs::read(&source).expect("ACL復元後の原本を読めません"), expected); backup(&paths);
        assert_eq!(fs::read(&archive).expect("再試行後のarchiveを読めません"), expected); assert_eq!(fs::read(&other).expect("再試行後の別logを読めません"), b"other source");
        assert_eq!(fs::read(&source).expect("再試行後の原本を読めません"), expected); assert_eq!(fs::read(paths.archive_folder.join("retained.bin")).expect("再試行後の対象外を読めません"), b"retained");
    }
}
