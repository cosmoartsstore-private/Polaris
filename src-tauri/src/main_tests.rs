// 専用childのprofile入力だけを差し替える。未設定caseは実Windows照会へ渡す。
#[cfg(test)]
mod dirs {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    static TEST_PROFILE: OnceLock<Option<PathBuf>> = OnceLock::new();
    pub fn home_dir() -> Option<PathBuf> { TEST_PROFILE.get().cloned().unwrap_or_else(::dirs::home_dir) }
    pub(super) fn set_test_profile(profile: Option<PathBuf>) {
        if let Some(current) = TEST_PROFILE.get() { assert_eq!(current, &profile, "専用childのprofile入力を変更してはいけません"); return; }
        assert!(TEST_PROFILE.set(profile).is_ok(), "専用childのprofile入力を同時に変更してはいけません");
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    /// 入力の成長を止め、panic時もscopeのjoinで所有する書込workerを待てるようにする。
    struct StopGrowth<'a>(&'a AtomicBool);
    impl Drop for StopGrowth<'_> { fn drop(&mut self) { self.0.store(true, Ordering::Relaxed); } }

    #[link(name = "advapi32")]
    extern "system" {
        fn RegOverridePredefKey(key: isize, replacement: isize) -> i32;
    }

    /// 子processのHKCUだけを専用keyへ向け、利用者のインストール設定を変更しない。
    struct TestRegistry {
        path: String,
        original: RegKey,
        isolated: RegKey,
    }

    impl TestRegistry {
        fn new() -> Self {
            let original = RegKey::predef(HKEY_CURRENT_USER);
            let path = format!("Software\\Polaris.Tests\\fixture-{}", std::process::id());
            let (isolated, _) = original.create_subkey(&path).expect("専用registry keyを作成できません");
            assert_eq!(unsafe { RegOverridePredefKey(HKEY_CURRENT_USER, isolated.raw_handle()) }, 0);
            Self { path, original, isolated }
        }
    }

    impl Drop for TestRegistry {
        fn drop(&mut self) {
            let _ = unsafe { RegOverridePredefKey(HKEY_CURRENT_USER, 0) };
            let _ = self.original.delete_subkey_all(&self.path);
        }
    }

    // 各テストが作成した一時領域だけを所有し、実際のログ・archive・registryへ触れない。
    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).expect("現在時刻を取得できません").as_nanos();
            let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("polaris-test-{}-{timestamp}-{id}", std::process::id()));
            fs::create_dir(&path).expect("テスト用directoryを作成できません");
            Self(path)
        }

        fn path(&self, name: &str) -> PathBuf { self.0.join(name) }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let Ok(target) = self.0.canonicalize() else { return }; let Ok(parent) = std::env::temp_dir().canonicalize() else { return };
            if target.parent() == Some(parent.as_path()) && target.file_name().is_some_and(|name| name.to_string_lossy().starts_with("polaris-test-")) { let _ = fs::remove_dir_all(target); }
        }
    }

    /// test専用childだけを所有し、失敗時にも停止完了後にfixtureを破棄する。
    struct OwnedTestChild(std::process::Child);

    impl OwnedTestChild {
        fn start(test: &str, mode: &str, root: &Path) -> Self {
            Self::start_executable(&std::env::current_exe().expect("test exeの場所を取得できません"), test, mode, root)
        }

        fn start_executable(executable: &Path, test: &str, mode: &str, root: &Path) -> Self {
            use std::os::windows::process::CommandExt;
            let child = std::process::Command::new(executable)
                .args(["--exact", test, "--test-threads=1", "--nocapture"]).env("POLARIS_NATIVE_TEST_MODE", mode).env("POLARIS_NATIVE_TEST_ROOT", root)
                .creation_flags(0x0800_0000).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().expect("専用childを起動できません");
            Self(child)
        }

        fn wait_for_marker(&mut self, marker: &Path) {
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            while !marker.is_file() {
                assert!(self.0.try_wait().expect("childの状態を取得できません").is_none(), "marker保存前にchildが終了しました");
                assert!(std::time::Instant::now() < deadline, "childの開始markerを待機できません"); std::thread::sleep(Duration::from_millis(20));
            }
        }

        fn finish(&mut self) -> std::process::ExitStatus {
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            loop {
                if let Some(status) = self.0.try_wait().expect("childの状態を取得できません") { return status; }
                assert!(std::time::Instant::now() < deadline, "専用childが終了しません"); std::thread::sleep(Duration::from_millis(20));
            }
        }

        fn diagnostic(&mut self) -> String {
            use std::io::Read;
            let mut text = String::new();
            if let Some(mut stream) = self.0.stdout.take() { let _ = stream.read_to_string(&mut text); }
            if let Some(mut stream) = self.0.stderr.take() { let _ = stream.read_to_string(&mut text); }
            text
        }
    }

    impl Drop for OwnedTestChild {
        fn drop(&mut self) { if self.0.try_wait().ok().flatten().is_none() { let _ = self.0.kill(); let _ = self.0.wait(); } }
    }

    // test binaryだけが所有fixtureをprocess検出へ接続する。配布runtimeの全体照会は変更しない。
    pub(super) fn is_owned_test_vrchat_pid(pid: u32) -> bool {
        use windows::core::PWSTR;
        use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
        let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else { return false };
        let mut buffer = vec![0_u16; 32_768]; let mut length = u32::try_from(buffer.len()).expect("path bufferが大きすぎます");
        let found = unsafe { QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &raw mut length) };
        let _ = unsafe { CloseHandle(handle) }; if found.is_err() { return false; }
        let Ok(image) = String::from_utf16(&buffer[..usize::try_from(length).expect("image pathの長さが不正です")]) else { return false };
        let Some(root) = Path::new(&image).parent().and_then(|path| path.canonicalize().ok()) else { return false };
        let temp = std::env::temp_dir().canonicalize().expect("temp境界を取得できません");
        if root.parent() == Some(temp.as_path()) && root.file_name().is_some_and(|name| name.to_string_lossy().starts_with("polaris-test-")) { return true; }
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("workspaceがありません");
        let boundary = workspace.join(".claude/audit-work/checks").canonicalize().expect("検証境界を取得できません");
        root.starts_with(boundary) && fs::read_to_string(root.join("owner.marker")).is_ok_and(|text| text.trim() == "polaris-headless-consumer-fixture-v1")
    }

    #[test]
    fn native_mutex_rejects_second_process_and_releases_on_exit() {
        if let Ok(mode) = std::env::var("POLARIS_NATIVE_TEST_MODE") {
            let root = PathBuf::from(std::env::var_os("POLARIS_NATIVE_TEST_ROOT").expect("専用childのfixtureがありません"));
            acquire_mutex(); fs::write(root.join(format!("{mode}.acquired")), b"acquired").expect("mutex取得を記録できません");
            let mut input = String::new(); std::io::stdin().read_line(&mut input).expect("親の終了通知を読めません"); return;
        }
        let _process_lock = lock_native_process_tests();
        let directory = TestDir::new(); let test = "tests::native_mutex_rejects_second_process_and_releases_on_exit";
        let mut first = OwnedTestChild::start(test, "first", &directory.0); first.wait_for_marker(&directory.path("first.acquired"));
        let mut second = OwnedTestChild::start(test, "second", &directory.0); let status = second.finish(); assert!(status.success(), "{}", second.diagnostic()); assert!(!directory.path("second.acquired").exists());
        assert!(first.0.try_wait().expect("保持childの状態を取得できません").is_none()); drop(first.0.stdin.take()); let status = first.finish(); assert!(status.success(), "{}", first.diagnostic());
        let mut third = OwnedTestChild::start(test, "third", &directory.0); third.wait_for_marker(&directory.path("third.acquired")); drop(third.0.stdin.take()); let status = third.finish(); assert!(status.success(), "{}", third.diagnostic());
        assert_eq!(fs::read(directory.path("first.acquired")).expect("最初の取得記録を読めません"), b"acquired"); assert_eq!(fs::read(directory.path("third.acquired")).expect("再取得記録を読めません"), b"acquired");
    }

    #[test]
    fn native_vrchat_lookup_matches_windows_process_query() {
        let _process_lock = lock_native_process_tests();
        let pids = owned_vrchat_pids_from_independent_query();
        let found = find_vrchat_pid(); assert_eq!(found.is_some(), !pids.is_empty()); if let Some(pid) = found { assert!(pids.contains(&pid), "独立したWindows照会にPIDがありません: {pid}"); }
    }

    #[test]
    fn actual_polling_loop_preserves_running_session_or_copies_absent_startup() {
        if std::env::var("POLARIS_NATIVE_TEST_MODE").as_deref() == Ok("poll") {
            let directory = TestDir::new(); let paths = RuntimePaths { vrchat_log_folder: directory.path("logs"), archive_folder: directory.path("archive") };
            fs::create_dir(&paths.vrchat_log_folder).expect("専用log directoryを作成できません"); let source = paths.vrchat_log_folder.join("output_log_fixture.txt"); let archive = paths.archive_folder.join("output_log_fixture.txt"); fs::write(&source, b"private session log").expect("専用logを書き込めません");
            let initial = find_vrchat_pid(); std::thread::spawn(move || run_polling_loop(&paths));
            // productionの1秒pollを2回通過させ、専用childの正常終了でcoverageもflushする。
            std::thread::sleep(Duration::from_millis(3_200)); assert_eq!(find_vrchat_pid(), initial, "確認中に利用者のVRChat状態が変わりました");
            if initial.is_some() { assert!(!archive.exists(), "動作中sessionを起動時にコピーしてはいけません"); } else { assert_eq!(fs::read(&archive).expect("未起動時のbackupがありません"), b"private session log"); }
            assert_eq!(fs::read(&source).expect("専用の元logを読めません"), b"private session log");
            let receipt = PathBuf::from(std::env::var_os("POLARIS_NATIVE_TEST_ROOT").expect("専用childの記録先がありません")).join("poll.completed");
            fs::write(receipt, format!("{initial:?}")).expect("監視loopの実施記録を保存できません"); return;
        }
        let _process_lock = lock_native_process_tests();
        let directory = TestDir::new(); let mut child = OwnedTestChild::start("tests::actual_polling_loop_preserves_running_session_or_copies_absent_startup", "poll", &directory.0); let status = child.finish(); assert!(status.success(), "{}", child.diagnostic());
        let initial = fs::read_to_string(directory.path("poll.completed")).expect("監視loopの完了記録がありません"); assert!(initial == "None" || initial.starts_with("Some("), "実際の監視対象PIDを記録する必要があります");
    }

    include!("monitor_tests.rs");
    include!("startup_tests.rs");
    include!("headless_tests.rs");

    #[test]
    fn backup_keeps_source_when_archive_target_is_a_directory() {
        let directory = TestDir::new(); let paths = RuntimePaths { vrchat_log_folder: directory.path("logs"), archive_folder: directory.path("archive") }; fs::create_dir(&paths.vrchat_log_folder).expect("専用log directoryを作成できません"); fs::create_dir_all(paths.archive_folder.join("output_log_a.txt")).expect("置換不可のdirectoryを作成できません");
        let source = paths.vrchat_log_folder.join("output_log_a.txt"); let unrelated = paths.archive_folder.join("output_log_a.txt/keep.txt"); fs::write(&source, b"source").expect("専用logを書き込めません"); fs::write(&unrelated, b"keep").expect("別fileを書き込めません"); backup(&paths);
        assert_eq!(fs::read(&source).expect("元logを読めません"), b"source"); assert_eq!(fs::read(&unrelated).expect("別fileを読めません"), b"keep"); assert!(!paths.archive_folder.join("output_log_a.polaris_tmp").exists());
    }

    #[test]
    fn exclusive_source_cannot_publish_partial_copy_and_can_be_retried() {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = TestDir::new(); let source = directory.path("source.txt"); let destination = directory.path("archive.txt"); fs::write(&source, b"private source").expect("元logを書き込めません");
        let locked = fs::OpenOptions::new().read(true).share_mode(0).open(&source).expect("元logをlockできません"); let failure = safe_copy(&source, &destination).expect_err("排他lock中のsourceを読んではいけません"); assert!(failure.to_string().contains("一時ファイルへのコピーに失敗")); assert!(!destination.exists()); let remaining = fs::read_dir(&directory.0).expect("検証用directoryを列挙できません").map(|entry| entry.map(|path| path.file_name().to_string_lossy().into_owned())).collect::<std::io::Result<Vec<_>>>().expect("検証用file名を取得できません"); assert_eq!(remaining, ["source.txt"]);
        drop(locked); safe_copy(&source, &destination).expect("lock解除後にコピーできません"); assert_eq!(fs::read(&destination).expect("コピー先を読めません"), b"private source"); assert_eq!(fs::read(&source).expect("元logを読めません"), b"private source");
        let mut remaining = fs::read_dir(&directory.0).expect("検証用directoryを列挙できません").map(|entry| entry.map(|path| path.file_name().to_string_lossy().into_owned())).collect::<std::io::Result<Vec<_>>>().expect("検証用file名を取得できません"); remaining.sort(); assert_eq!(remaining, ["archive.txt", "source.txt"]);
    }

    #[test]
    fn safe_copy_rejects_unavailable_parent_without_creating_directories() {
        let directory = TestDir::new(); let source = directory.path("source.txt"); let destination = directory.path("missing/archive.txt"); fs::write(&source, b"private source").expect("元logを書き込めません");
        let failure = safe_copy(&source, &destination).expect_err("未作成の親directoryへコピーしてはいけません"); assert!(failure.to_string().contains("一時ファイルへのコピーに失敗")); assert!(!directory.path("missing").exists()); assert_eq!(fs::read(&source).expect("元logを読めません"), b"private source");
    }

    #[test]
    fn vrchat_log_name_requires_exact_prefix_and_text_extension() {
        for name in ["output_log_2026.txt", "output_log_2026.TXT", "folder/output_log_2026.Txt"] {
            assert!(is_vrchat_log(Path::new(name)), "{name}");
        }
        for name in ["", "other.txt", "Output_log_2026.txt", "output_log_2026", "output_log_2026.txt.bak", "output_log_2026.csv"] {
            assert!(!is_vrchat_log(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn equal_nonempty_sizes_are_recognized_without_comparing_contents() {
        let directory = TestDir::new();
        let source = directory.path("source.txt");
        let archive = directory.path("archive.txt");
        fs::write(&source, b"abc").expect("元ファイルを書き込めません");
        fs::write(&archive, b"xyz").expect("archiveを書き込めません");
        assert!(is_same_size(&source, &archive));
        fs::write(&archive, b"longer").expect("archiveを更新できません");
        assert!(!is_same_size(&source, &archive));
    }

    #[test]
    fn empty_or_missing_files_are_not_equal_nonempty_sizes() {
        let directory = TestDir::new();
        let empty = directory.path("empty.txt");
        let missing = directory.path("missing.txt");
        fs::write(&empty, []).expect("空ファイルを作成できません");
        assert!(!is_same_size(&empty, &empty));
        assert!(!is_same_size(&empty, &missing));
        assert!(!is_same_size(&missing, &missing));
    }

    #[test]
    fn stable_size_returns_current_size_and_missing_file_returns_none() {
        let directory = TestDir::new();
        let source = directory.path("source.txt");
        fs::write(&source, b"stable").expect("元ファイルを書き込めません");
        assert_eq!(wait_for_stable_size(&source), Some(6));
        assert_eq!(wait_for_stable_size(&directory.path("missing.txt")), None);
    }

    #[test]
    fn growing_source_is_not_stable_and_backup_does_not_publish_partial_log() { check_growing_source_backup(None); }

    #[test]
    fn backup_preserves_previous_archive_when_source_keeps_growing() { check_growing_source_backup(Some("保存済みのログ本文\n".as_bytes())); }

    // 同じ継続変動条件を、新規archiveと保存済みarchiveの独立した契約で確認する。
    fn check_growing_source_backup(previous: Option<&[u8]>) {
        let directory = TestDir::new(); let paths = RuntimePaths { vrchat_log_folder: directory.path("logs"), archive_folder: directory.path("archive") };
        fs::create_dir(&paths.vrchat_log_folder).expect("専用log directoryを作成できません"); let source = paths.vrchat_log_folder.join("output_log_owned.txt");
        let archive = paths.archive_folder.join("output_log_owned.txt"); let temporary = paths.archive_folder.join("output_log_owned.polaris_tmp");
        if let Some(bytes) = previous { fs::create_dir(&paths.archive_folder).expect("専用archiveを作成できません"); fs::write(&archive, bytes).expect("旧archiveを書き込めません"); assert_eq!(fs::read(&archive).expect("旧archiveを読めません"), bytes); }
        let contents = b"owned ongoing log\n"; fs::write(&source, contents).expect("専用logを書き込めません");
        let writer = fs::OpenOptions::new().write(true).open(&source).expect("専用logの書込handleを開けません"); let stop = AtomicBool::new(false);
        let (started, received) = std::sync::mpsc::channel(); let initial = u64::try_from(contents.len()).expect("専用logのsizeが大きすぎます");
        let (observed_archive, stable_wait_ms, backup_wait_ms, final_size) = std::thread::scope(|scope| {
            let guard = StopGrowth(&stop); let should_stop = &stop;
            let worker = scope.spawn(move || {
                let mut size = initial; let deadline = std::time::Instant::now() + Duration::from_secs(15);
                // 安定待ちの上限が壊れた場合も、入力の成長終了でassertへ進める。
                while !should_stop.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
                    size += 1; writer.set_len(size).expect("専用logの書込を継続できません");
                    if size == initial + 1 { started.send(()).expect("書込開始を通知できません"); }
                    std::thread::sleep(Duration::from_millis(20));
                }
                size
            });
            received.recv_timeout(Duration::from_secs(15)).expect("専用logの書込が開始されません"); let waiting = std::time::Instant::now();
            assert_eq!(wait_for_stable_size(&source), None, "書込継続中のlogを安定済みとしてはいけません"); assert!(waiting.elapsed() >= Duration::from_secs(3));
            let stable_wait_ms = waiting.elapsed().as_millis();
            if let Some(bytes) = previous { assert_ne!(fs::metadata(&source).expect("更新中logのsizeを読めません").len(), u64::try_from(bytes.len()).expect("旧archiveのsizeが大きすぎます")); }
            let backing_up = std::time::Instant::now(); backup(&paths); assert!(backing_up.elapsed() >= Duration::from_secs(3));
            let backup_wait_ms = backing_up.elapsed().as_millis(); let observed_archive = fs::read(&archive).ok(); assert!(!temporary.exists());
            drop(guard); let final_size = worker.join().expect("専用logの書込workerが失敗しました");
            (observed_archive, stable_wait_ms, backup_wait_ms, final_size)
        });
        let mut expected = contents.to_vec(); expected.resize(usize::try_from(final_size).expect("専用logのsizeが大きすぎます"), 0);
        assert!(final_size > initial); assert_eq!(fs::read(&source).expect("書込停止後の元logを読めません"), expected);
        // 保持違反があっても、同じpathの次回backupによる復旧を確認してから元の保持契約をassertする。
        backup(&paths);
        assert_eq!(fs::read(&archive).expect("書込停止後に同じarchiveへ再試行できません"), expected);
        assert_eq!(fs::read(&source).expect("再試行後の元logを読めません"), expected); assert!(!temporary.exists());
        if let Some(bytes) = previous {
            write_native_receipt("POLARIS_GROWING_ARCHIVE_TEST_RECEIPT_PATH", "polaris-growing-existing-archive-evidence.json",
                &format!("{{\"stable_wait_ms\":{stable_wait_ms},\"backup_wait_ms\":{backup_wait_ms},\"expected_archive_bytes\":{},\"actual_archive_bytes\":{},\"archive_present\":{},\"old_archive_preserved\":{},\"source_prefix_preserved\":true,\"source_size_grew\":true,\"temporary_published\":false,\"worker_joined\":true,\"source_bytes_verified_from_writer_length\":true,\"same_paths_retry_succeeded\":true,\"recovered_archive_bytes\":{final_size}}}", bytes.len(), observed_archive.as_ref().map_or(0, Vec::len), observed_archive.is_some(), observed_archive.as_deref() == previous));
        }
        assert_eq!(observed_archive.as_deref(), previous, "継続書込によるbackup中止で保存済みarchiveを変更してはいけません。");
    }

    #[test]
    fn safe_copy_publishes_complete_contents_and_removes_temporary_file() {
        let directory = TestDir::new();
        let source = directory.path("source.txt");
        let destination = directory.path("archive.txt");
        fs::write(&source, "日本語のログ\n次の行").expect("元ファイルを書き込めません");
        safe_copy(&source, &destination).expect("コピーに失敗しました");
        let expected = "日本語のログ\n次の行".as_bytes();
        assert_eq!(fs::read(&destination).expect("archiveを読み取れません"), expected);
        assert_eq!(fs::read(&source).expect("元ファイルを読み取れません"), expected);
        assert!(!destination.with_extension("polaris_tmp").exists());
    }

    #[test]
    fn empty_source_is_not_published_and_existing_destination_is_preserved() {
        let directory = TestDir::new();
        let source = directory.path("empty.txt");
        let destination = directory.path("archive.txt");
        fs::write(&source, []).expect("空ファイルを作成できません");
        fs::write(&destination, b"previous").expect("archiveを書き込めません");
        assert!(safe_copy(&source, &destination).is_err());
        assert_eq!(fs::read(&destination).expect("archiveを読み取れません"), b"previous");
        assert!(!destination.with_extension("polaris_tmp").exists());
    }

    #[test]
    fn missing_source_does_not_publish_destination() {
        let directory = TestDir::new();
        let destination = directory.path("archive.txt");
        assert!(safe_copy(&directory.path("missing.txt"), &destination).is_err());
        assert!(!destination.exists());
        assert!(!destination.with_extension("polaris_tmp").exists());
    }

    #[test]
    fn temporary_cleanup_is_limited_to_matching_files() {
        let directory = TestDir::new();
        let temporary = directory.path("interrupted.polaris_tmp");
        let log = directory.path("output_log_2026.txt");
        let other = directory.path("other.tmp");
        let nested = directory.path("nested.polaris_tmp");
        fs::write(&temporary, b"partial").expect("一時ファイルを作成できません");
        fs::write(&log, b"log").expect("ログを作成できません");
        fs::write(&other, b"other").expect("別ファイルを作成できません");
        fs::create_dir(&nested).expect("子directoryを作成できません");
        clean_temp_files(&directory.0);
        assert!(!temporary.exists());
        assert!(log.exists());
        assert!(other.exists());
        assert!(nested.is_dir());
        clean_temp_files(&directory.path("missing"));
    }

    #[test]
    fn backup_copies_logs_updates_changed_size_and_preserves_same_size_archive() {
        let directory = TestDir::new();
        let paths = RuntimePaths { vrchat_log_folder: directory.path("logs"), archive_folder: directory.path("archive") };
        fs::create_dir(&paths.vrchat_log_folder).expect("ログdirectoryを作成できません");
        fs::create_dir(&paths.archive_folder).expect("archive directoryを作成できません");
        for (name, contents) in [("output_log_new.txt", "new"), ("output_log_changed.txt", "changed"), ("output_log_same.txt", "abc"), ("output_log_empty.txt", ""), ("other.txt", "other")] {
            fs::write(paths.vrchat_log_folder.join(name), contents).expect("元ログを書き込めません");
        }
        fs::write(paths.archive_folder.join("output_log_changed.txt"), b"old").expect("旧archiveを書き込めません");
        fs::write(paths.archive_folder.join("output_log_same.txt"), b"xyz").expect("同サイズのarchiveを書き込めません");
        fs::write(paths.archive_folder.join("interrupted.polaris_tmp"), b"partial").expect("一時ファイルを作成できません");
        backup(&paths);
        for (name, expected) in [("output_log_new.txt", "new"), ("output_log_changed.txt", "changed"), ("output_log_same.txt", "xyz")] {
            assert_eq!(fs::read_to_string(paths.archive_folder.join(name)).expect("archiveを読み取れません"), expected);
        }
        // 同サイズのarchiveを保持する場合も、内容の異なる元ログを変更してはいけない。
        for (name, expected) in [("output_log_new.txt", "new"), ("output_log_changed.txt", "changed"), ("output_log_same.txt", "abc"), ("output_log_empty.txt", ""), ("other.txt", "other")] {
            assert_eq!(fs::read_to_string(paths.vrchat_log_folder.join(name)).expect("元ログを読み取れません"), expected);
        }
        for name in ["output_log_empty.txt", "other.txt", "interrupted.polaris_tmp"] {
            assert!(!paths.archive_folder.join(name).exists(), "{name}");
        }
    }

    #[test]
    fn backup_selects_only_eligible_direct_logs_and_revisits_empty_files() {
        let directory = TestDir::new();
        let paths = RuntimePaths { vrchat_log_folder: directory.path("source"), archive_folder: directory.path("archive") };
        fs::create_dir(&paths.vrchat_log_folder).expect("専用sourceを作成できません");
        fs::create_dir(&paths.archive_folder).expect("専用archiveを作成できません");
        fs::create_dir(paths.vrchat_log_folder.join("nested")).expect("対象外の子folderを作成できません");
        fs::create_dir(paths.vrchat_log_folder.join("output_log_directory.txt")).expect("log名のdirectoryを作成できません");
        // 日付のない名称と混在caseの拡張子も対象であり、本文は改行・Unicodeを含めて保持する。
        let sources: [(&str, &[u8]); 9] = [
            ("output_log_任意名（2）.TxT", "日本語の本文🙂\r\n次の行\n".as_bytes()),
            ("output_log_.txt", b"no date in filename\n"),
            ("output_log_empty_new.txt", b""),
            ("output_log_empty_existing.txt", b""),
            ("Output_log_case.txt", b"wrong prefix\n"),
            ("output_log_extension.json", b"wrong extension\n"),
            ("output_log_suffix.txt.bak", b"backup suffix\n"),
            ("nested/output_log_nested.txt", b"nested log\n"),
            ("output_log_directory.txt/output_log_child.txt", b"directory child\n"),
        ];
        for (name, bytes) in sources { fs::write(paths.vrchat_log_folder.join(name), bytes).expect("対象判定の専用入力を書き込めません"); }
        let previous = b"previous completed archive\n";
        fs::write(paths.archive_folder.join("output_log_empty_existing.txt"), previous).expect("0byte原本に対応する旧archiveを準備できません");
        backup(&paths);
        let expected = [
            ("output_log_任意名（2）.TxT", "日本語の本文🙂\r\n次の行\n".as_bytes()),
            ("output_log_.txt", b"no date in filename\n".as_slice()),
            ("output_log_empty_existing.txt", previous.as_slice()),
        ];
        let mut actual_names: Vec<_> = fs::read_dir(&paths.archive_folder).expect("対象判定後のarchiveを列挙できません")
            .map(|entry| entry.expect("archive entryを読めません").file_name().to_string_lossy().into_owned()).collect();
        let mut expected_names: Vec<_> = expected.iter().map(|(name, _)| (*name).to_owned()).collect();
        actual_names.sort(); expected_names.sort(); assert_eq!(actual_names, expected_names, "直下の非空logだけを公開する必要があります");
        for (name, bytes) in expected { assert_eq!(fs::read(paths.archive_folder.join(name)).expect("対象判定後の本文を読めません"), bytes); }
        for (name, bytes) in sources { assert_eq!(fs::read(paths.vrchat_log_folder.join(name)).expect("対象判定後の原本を読めません"), bytes); }

        let new = "空logから書込完了した本文\n".as_bytes();
        let replacement = "空になっていた原本の再開後の本文\n".as_bytes();
        fs::write(paths.vrchat_log_folder.join("output_log_empty_new.txt"), new).expect("空の新規logを更新できません");
        fs::write(paths.vrchat_log_folder.join("output_log_empty_existing.txt"), replacement).expect("空の既存logを更新できません");
        assert_ne!(replacement.len(), previous.len());
        backup(&paths);
        for (name, bytes) in [("output_log_empty_new.txt", new), ("output_log_empty_existing.txt", replacement)] {
            assert_eq!(fs::read(paths.archive_folder.join(name)).expect("次回の非空logが保存されません"), bytes);
            assert_eq!(fs::read(paths.vrchat_log_folder.join(name)).expect("次回の原本を読めません"), bytes);
        }
        for (name, bytes) in sources.into_iter().filter(|(name, _)| !name.starts_with("output_log_empty_")) {
            assert_eq!(fs::read(paths.vrchat_log_folder.join(name)).expect("次回処理後の対象外を含む原本を読めません"), bytes);
        }
        for (name, bytes) in expected.into_iter().filter(|(name, _)| *name != "output_log_empty_existing.txt") {
            assert_eq!(fs::read(paths.archive_folder.join(name)).expect("次回処理後の既存archiveを読めません"), bytes);
        }
        assert_eq!(fs::read_dir(&paths.archive_folder).expect("次回処理後のarchiveを列挙できません").count(), 4);
        write_native_receipt("POLARIS_BACKUP_ELIGIBILITY_TEST_RECEIPT_PATH", "polaris-backup-eligibility-evidence.json",
            "{\"backup_calls\":2,\"source_files\":9,\"direct_nonempty_logs_only\":true,\"mixed_case_extension_copied\":true,\"non_date_name_copied\":true,\"unicode_and_crlf_preserved\":true,\"nested_logs_not_copied\":true,\"log_named_directory_not_copied\":true,\"wrong_prefix_and_extension_not_copied\":true,\"empty_source_previous_archive_preserved\":true,\"empty_to_nonempty_next_backup_succeeded\":true,\"source_bytes_preserved\":true,\"published_files_after_retry\":4}");
    }

    #[test]
    fn missing_log_directory_preserves_existing_archives() {
        let directory = TestDir::new();
        let paths = RuntimePaths { vrchat_log_folder: directory.path("missing"), archive_folder: directory.path("archive") };
        fs::create_dir(&paths.archive_folder).expect("archive directoryを作成できません");
        let existing = paths.archive_folder.join("output_log_existing.txt");
        fs::write(&existing, b"existing").expect("archiveを書き込めません");
        backup(&paths);
        assert_eq!(fs::read(&existing).expect("archiveを読み取れません"), b"existing");
    }

    #[test]
    fn failed_temporary_copy_preserves_existing_destination_and_source() {
        let directory = TestDir::new();
        let source = directory.path("source.txt");
        let destination = directory.path("archive.txt");
        fs::write(&source, b"new log").expect("元ログを書き込めません");
        fs::write(&destination, b"previous").expect("旧archiveを書き込めません");
        fs::create_dir(destination.with_extension("polaris_tmp")).expect("一時pathを準備できません");
        let error = safe_copy(&source, &destination).expect_err("directoryへコピーしてはいけません");
        assert!(error.to_string().contains("一時ファイルへのコピーに失敗"));
        assert_eq!(fs::read(&destination).expect("旧archiveを読み取れません"), b"previous");
        assert_eq!(fs::read(&source).expect("元ログを読み取れません"), b"new log");
    }

    #[test]
    fn rename_failure_leaves_complete_temporary_file_for_next_cleanup() {
        let directory = TestDir::new();
        let source = directory.path("source.txt");
        let destination = directory.path("destination.txt");
        fs::write(&source, b"complete log").expect("元ログを書き込めません");
        fs::create_dir(&destination).expect("置換不可のdirectoryを作成できません");
        let error = safe_copy(&source, &destination).expect_err("directoryを置換してはいけません");
        assert!(error.to_string().contains("リネームに失敗"));
        assert!(destination.is_dir());
        assert_eq!(fs::read(destination.with_extension("polaris_tmp")).expect("一時ログを読み取れません"), b"complete log");
        clean_temp_files(&directory.0);
        assert!(!destination.with_extension("polaris_tmp").exists());
        assert!(destination.is_dir());
    }

    #[test]
    fn safe_copy_retries_same_paths_after_temporary_and_rename_failures() {
        let directory = TestDir::new(); let source = directory.path("source.txt"); let destination = directory.path("destination.txt");
        let temporary = destination.with_extension("polaris_tmp"); let expected = "再試行する完全な日本語ログ\n".as_bytes();
        fs::write(&source, expected).expect("元logを書き込めません"); fs::write(&destination, b"previous archive").expect("旧archiveを書き込めません");
        fs::create_dir(&temporary).expect("一時pathの故障を準備できません");
        let error = safe_copy(&source, &destination).expect_err("一時pathがdirectoryのときに成功しました");
        assert!(error.to_string().starts_with("一時ファイルへのコピーに失敗:"));
        assert_eq!(fs::read(&source).expect("故障後の元logを読めません"), expected);
        assert_eq!(fs::read(&destination).expect("故障後の旧archiveを読めません"), b"previous archive");
        fs::remove_dir(&temporary).expect("所有する一時pathの故障を除去できません");
        fs::remove_file(&destination).expect("所有する置換先を準備できません"); fs::create_dir(&destination).expect("renameの故障を準備できません");
        let error = safe_copy(&source, &destination).expect_err("置換先がdirectoryのときに成功しました");
        assert!(error.to_string().starts_with("リネームに失敗:")); assert!(destination.is_dir());
        assert_eq!(fs::read(&temporary).expect("rename失敗後の一時logを読めません"), expected);
        assert_eq!(fs::read(&source).expect("rename失敗後の元logを読めません"), expected);
        fs::remove_dir(&destination).expect("所有するrenameの故障を除去できません");
        safe_copy(&source, &destination).expect("同じpathの再試行が失敗しました");
        assert_eq!(fs::read(&destination).expect("再試行後のarchiveを読めません"), expected);
        assert_eq!(fs::read(&source).expect("再試行後の元logを読めません"), expected); assert!(!temporary.exists());
    }

    #[test]
    fn temporary_cleanup_preserves_locked_file_and_retries_after_release() {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = TestDir::new(); let locked_path = directory.path("locked.polaris_tmp"); let removable = directory.path("removable.polaris_tmp");
        let retained = directory.path("retained.txt"); let nested = directory.path("nested.polaris_tmp");
        let expected = "排他中の完全な一時ログ\n".as_bytes(); fs::write(&locked_path, expected).expect("lock対象を作成できません");
        fs::write(&removable, b"removable").expect("別の一時logを作成できません"); fs::write(&retained, b"retained").expect("対象外fileを作成できません");
        fs::create_dir(&nested).expect("対象外directoryを作成できません"); fs::write(nested.join("child.polaris_tmp"), b"nested retained").expect("子fileを作成できません");
        let locked = fs::OpenOptions::new().read(true).share_mode(0).open(&locked_path).expect("所有する一時logをlockできません");
        clean_temp_files(&directory.0); assert!(locked_path.is_file()); assert!(!removable.exists());
        assert_eq!(fs::read(&retained).expect("対象外fileを読めません"), b"retained");
        assert_eq!(fs::read(nested.join("child.polaris_tmp")).expect("対象外の子fileを読めません"), b"nested retained");
        drop(locked); assert_eq!(fs::read(&locked_path).expect("lock解除後の一時logを読めません"), expected);
        clean_temp_files(&directory.0); assert!(!locked_path.exists()); assert!(!removable.exists());
        assert_eq!(fs::read(&retained).expect("再試行後の対象外fileを読めません"), b"retained");
        assert_eq!(fs::read(nested.join("child.polaris_tmp")).expect("再試行後の対象外の子fileを読めません"), b"nested retained");
    }

    #[test]
    fn unavailable_archive_directory_does_not_change_source_or_existing_file() {
        let directory = TestDir::new();
        let paths = RuntimePaths { vrchat_log_folder: directory.path("logs"), archive_folder: directory.path("archive") };
        fs::create_dir(&paths.vrchat_log_folder).expect("元ログdirectoryを作成できません");
        fs::write(paths.vrchat_log_folder.join("output_log_a.txt"), b"source log").expect("元ログを書き込めません");
        fs::write(&paths.archive_folder, b"occupied").expect("占有ファイルを書き込めません");
        backup(&paths);
        assert_eq!(fs::read(&paths.archive_folder).expect("占有ファイルを読み取れません"), b"occupied");
        assert_eq!(fs::read(paths.vrchat_log_folder.join("output_log_a.txt")).expect("元ログを読み取れません"), b"source log");
    }

    #[test]
    fn backup_recovers_archive_directory_creation_failure_using_same_runtime_paths() {
        let directory = TestDir::new();
        let paths = RuntimePaths { vrchat_log_folder: directory.path("logs"), archive_folder: directory.path("archive") };
        fs::create_dir(&paths.vrchat_log_folder).expect("所有する元log directoryを作成できません");
        let expected = "保存先の修復後に保存する日本語ログ\n".as_bytes();
        let other = b"other complete source log\n";
        let source = paths.vrchat_log_folder.join("output_log_recovery.txt");
        let other_source = paths.vrchat_log_folder.join("output_log_other.TXT");
        fs::write(&source, expected).expect("所有する元logを書き込めません");
        fs::write(&other_source, other).expect("所有する別logを書き込めません");
        fs::write(&paths.archive_folder, b"occupied archive directory").expect("保存先の故障を準備できません");
        backup(&paths);
        assert_eq!(fs::read(&paths.archive_folder).expect("保存先の占有fileを読めません"), b"occupied archive directory");
        assert_eq!(fs::read(&source).expect("失敗後の元logを読めません"), expected);
        assert_eq!(fs::read(&other_source).expect("失敗後の別logを読めません"), other);
        fs::remove_file(&paths.archive_folder).expect("所有する保存先の故障を除去できません");
        backup(&paths);
        assert!(paths.archive_folder.is_dir());
        assert_eq!(fs::read(paths.archive_folder.join("output_log_recovery.txt")).expect("修復後のarchiveを読めません"), expected);
        assert_eq!(fs::read(paths.archive_folder.join("output_log_other.TXT")).expect("修復後の別archiveを読めません"), other);
        assert_eq!(fs::read(&source).expect("修復後の元logを読めません"), expected);
        assert_eq!(fs::read(&other_source).expect("修復後の別logを読めません"), other);
        assert!(!paths.archive_folder.join("output_log_recovery.polaris_tmp").exists());
        assert!(!paths.archive_folder.join("output_log_other.polaris_tmp").exists());
        // 次回も同じRuntimePathsを使い、保存済みの2fileだけを保持する。
        backup(&paths);
        assert_eq!(fs::read_dir(&paths.archive_folder).expect("再実行後のarchiveを列挙できません").count(), 2);
        assert_eq!(fs::read(paths.archive_folder.join("output_log_recovery.txt")).expect("再実行後のarchiveを読めません"), expected);
        assert_eq!(fs::read(paths.archive_folder.join("output_log_other.TXT")).expect("再実行後の別archiveを読めません"), other);
        write_native_receipt("POLARIS_BACKUP_RECOVERY_ARCHIVE_DIRECTORY_RECEIPT_PATH", "polaris-backup-recovery-archive-directory-evidence.json",
            "{\"backup_calls\":3,\"archive_creation_failure\":true,\"occupied_file_preserved\":true,\"source_bytes_preserved\":true,\"same_runtime_paths_retry_succeeded\":true,\"copied_files\":2,\"same_size_followup_preserved\":true,\"temporary_files_absent\":true}");
    }

    #[test]
    fn backup_recovers_log_directory_read_failure_without_changing_previous_archives() {
        let directory = TestDir::new();
        let paths = RuntimePaths { vrchat_log_folder: directory.path("logs"), archive_folder: directory.path("archive") };
        fs::create_dir(&paths.archive_folder).expect("所有するarchive directoryを作成できません");
        let archive = paths.archive_folder.join("output_log_recovery.txt");
        let retained = paths.archive_folder.join("output_log_retained.txt");
        let temporary = paths.archive_folder.join("interrupted.polaris_tmp");
        let previous = b"previous complete archive\n"; let kept = "保持する別archive\n".as_bytes();
        let expected = "読込先修復後の更新済み日本語ログ\n".as_bytes();
        assert_ne!(previous.len(), expected.len());
        fs::write(&archive, previous).expect("既存archiveを書き込めません");
        fs::write(&retained, kept).expect("保持対象のarchiveを書き込めません");
        fs::write(&temporary, b"interrupted").expect("残存する一時fileを書き込めません");
        fs::write(&paths.vrchat_log_folder, b"occupied source directory").expect("読込先の故障を準備できません");
        backup(&paths);
        assert_eq!(fs::read(&paths.vrchat_log_folder).expect("読込先の占有fileを読めません"), b"occupied source directory");
        assert_eq!(fs::read(&archive).expect("読込失敗後の旧archiveを読めません"), previous);
        assert_eq!(fs::read(&retained).expect("読込失敗後の別archiveを読めません"), kept);
        // 一時file整理はログdirectory列挙より先に実行する現行の順序である。
        assert!(!temporary.exists());
        fs::remove_file(&paths.vrchat_log_folder).expect("所有する読込先の故障を除去できません");
        fs::create_dir(&paths.vrchat_log_folder).expect("同じ読込先を修復できません");
        let source = paths.vrchat_log_folder.join("output_log_recovery.txt");
        fs::write(&source, expected).expect("再試行対象の元logを書き込めません");
        backup(&paths);
        assert_eq!(fs::read(&archive).expect("再試行後のarchiveを読めません"), expected);
        assert_eq!(fs::read(&source).expect("再試行後の元logを読めません"), expected);
        assert_eq!(fs::read(&retained).expect("再試行後の別archiveを読めません"), kept);
        assert!(!archive.with_extension("polaris_tmp").exists()); assert!(!temporary.exists());
        assert_eq!(fs::read_dir(&paths.archive_folder).expect("再試行後のarchiveを列挙できません").count(), 2);
        write_native_receipt("POLARIS_BACKUP_RECOVERY_LOG_DIRECTORY_RECEIPT_PATH", "polaris-backup-recovery-log-directory-evidence.json",
            "{\"backup_calls\":2,\"log_read_failure\":true,\"occupied_file_preserved\":true,\"previous_archive_preserved_after_failure\":true,\"temporary_cleanup_before_read\":true,\"same_runtime_paths_retry_succeeded\":true,\"updated_archive_verified\":true,\"unrelated_archive_preserved\":true,\"source_bytes_preserved\":true,\"temporary_files_absent\":true}");
    }

    #[test]
    fn backup_recovers_locked_temporary_copy_and_continues_other_logs() {
        use std::io::Read;
        use std::os::windows::fs::OpenOptionsExt;
        let directory = TestDir::new();
        let paths = RuntimePaths { vrchat_log_folder: directory.path("source"), archive_folder: directory.path("archive") };
        fs::create_dir(&paths.vrchat_log_folder).expect("所有する元log directoryを作成できません");
        fs::create_dir(&paths.archive_folder).expect("所有するarchive directoryを作成できません");
        let source = paths.vrchat_log_folder.join("output_log_locked_temporary.txt");
        let archive = paths.archive_folder.join("output_log_locked_temporary.txt");
        let temporary = archive.with_extension("polaris_tmp");
        let other_source = paths.vrchat_log_folder.join("output_log_other.txt");
        let other_archive = paths.archive_folder.join("output_log_other.txt");
        let retained = paths.archive_folder.join("output_log_retained.txt");
        let expected = "一時fileのlock解除後に保存する完全なログ\n".as_bytes();
        let interrupted = "占有する保存途中のbyte列\n".as_bytes();
        let other = b"other complete source log\n"; let kept = b"unrelated complete archive\n";
        fs::write(&source, expected).expect("再試行対象の元logを書き込めません");
        fs::write(&other_source, other).expect("別の元logを書き込めません");
        fs::write(&retained, kept).expect("保持対象のarchiveを書き込めません");
        fs::write(&temporary, interrupted).expect("排他対象の一時fileを書き込めません");
        let mut locked = fs::OpenOptions::new().read(true).share_mode(0).open(&temporary).expect("所有する一時fileを排他lockできません");
        backup(&paths);
        assert!(!archive.exists(), "失敗したコピーを完成済みarchiveへ公開してはいけません");
        let mut locked_bytes = Vec::new(); locked.read_to_end(&mut locked_bytes).expect("所有する一時fileのbyte列を読めません");
        assert_eq!(locked_bytes, interrupted);
        assert_eq!(fs::read(&other_archive).expect("正常な別logのarchiveを読めません"), other);
        assert_eq!(fs::read(&retained).expect("失敗後の別archiveを読めません"), kept);
        assert_eq!(fs::read(&source).expect("失敗後の元logを読めません"), expected);
        assert_eq!(fs::read(&other_source).expect("失敗後の別元logを読めません"), other);
        drop(locked); assert_eq!(fs::read(&temporary).expect("lock解除後の一時fileを読めません"), interrupted);
        backup(&paths);
        assert_eq!(fs::read(&archive).expect("同じpathの再試行後のarchiveを読めません"), expected);
        assert_eq!(fs::read(&other_archive).expect("再試行後の別archiveを読めません"), other);
        assert_eq!(fs::read(&retained).expect("再試行後の保持対象archiveを読めません"), kept);
        assert_eq!(fs::read(&source).expect("再試行後の元logを読めません"), expected);
        assert_eq!(fs::read(&other_source).expect("再試行後の別元logを読めません"), other);
        assert!(!temporary.exists()); assert!(!other_archive.with_extension("polaris_tmp").exists());
        assert_eq!(fs::read_dir(&paths.archive_folder).expect("再試行後のarchiveを列挙できません").count(), 3);
        write_native_receipt("POLARIS_BACKUP_RECOVERY_TEMPORARY_RECEIPT_PATH", "polaris-backup-recovery-temporary-evidence.json",
            "{\"backup_calls\":2,\"real_temporary_sharing_violation\":true,\"locked_temporary_bytes_preserved\":true,\"failed_copy_not_published\":true,\"other_log_copied_after_failure\":true,\"same_runtime_paths_retry_succeeded\":true,\"unrelated_archive_preserved\":true,\"source_bytes_preserved\":true,\"temporary_files_absent\":true}");
    }

    #[test]
    fn locked_existing_archive_is_preserved_when_replacement_cannot_start() {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = TestDir::new();
        let paths = RuntimePaths { vrchat_log_folder: directory.path("logs"), archive_folder: directory.path("archive") };
        fs::create_dir(&paths.vrchat_log_folder).expect("元ログdirectoryを作成できません");
        fs::create_dir(&paths.archive_folder).expect("archive directoryを作成できません");
        let name = "output_log_a.txt";
        fs::write(paths.vrchat_log_folder.join(name), b"changed log").expect("元ログを書き込めません");
        fs::write(paths.archive_folder.join(name), b"previous").expect("旧archiveを書き込めません");
        let locked = fs::OpenOptions::new().read(true).share_mode(0).open(paths.archive_folder.join(name)).expect("archiveをlockできません");
        backup(&paths);
        drop(locked);
        assert_eq!(fs::read(paths.archive_folder.join(name)).expect("旧archiveを読み取れません"), b"previous");
        assert_eq!(fs::read(paths.vrchat_log_folder.join(name)).expect("元ログを読み取れません"), b"changed log");
        backup(&paths);
        assert_eq!(fs::read(paths.archive_folder.join(name)).expect("同じarchiveへの再試行が保存されません"), b"changed log");
        assert_eq!(fs::read(paths.vrchat_log_folder.join(name)).expect("再試行後の元ログを読み取れません"), b"changed log");
        assert!(!paths.archive_folder.join(name).with_extension("polaris_tmp").exists());
        write_native_receipt("POLARIS_BACKUP_RECOVERY_ARCHIVE_LOCK_RECEIPT_PATH", "polaris-backup-recovery-archive-lock-evidence.json",
            "{\"backup_calls\":2,\"real_archive_sharing_violation\":true,\"previous_archive_preserved_after_failure\":true,\"same_runtime_paths_retry_succeeded\":true,\"updated_archive_verified\":true,\"source_bytes_preserved\":true,\"temporary_files_absent\":true}");
    }

    #[test]
    fn backup_preserves_previous_archive_when_source_copy_is_locked() {
        use std::io::Read;
        use std::os::windows::fs::OpenOptionsExt;
        let directory = TestDir::new();
        let paths = RuntimePaths { vrchat_log_folder: directory.path("source"), archive_folder: directory.path("archive") };
        fs::create_dir(&paths.vrchat_log_folder).expect("専用sourceを作成できません"); fs::create_dir(&paths.archive_folder).expect("専用archiveを作成できません");
        let source = paths.vrchat_log_folder.join("output_log_owned.txt"); let archive = paths.archive_folder.join("output_log_owned.txt");
        let other_source = paths.vrchat_log_folder.join("output_log_other.txt"); let other_archive = paths.archive_folder.join("output_log_other.txt");
        let retained = paths.archive_folder.join("output_log_retained.txt");
        fs::write(&source, b"updated source log").expect("専用logを書き込めません"); fs::write(&archive, b"previous").expect("旧archiveを書き込めません");
        fs::write(&other_source, b"other complete source").expect("別の元logを書き込めません"); fs::write(&retained, b"retained archive").expect("保持対象archiveを書き込めません");
        let mut locked = fs::OpenOptions::new().read(true).share_mode(0).open(&source).expect("専用sourceをlockできません");
        assert_eq!(fs::metadata(&source).expect("lock中のsource sizeを読めません").len(), 18); assert!(!is_same_size(&source, &archive));
        backup(&paths);
        let previous_archive_preserved = fs::read(&archive).ok().as_deref() == Some(b"previous".as_slice());
        let mut locked_bytes = Vec::new(); locked.read_to_end(&mut locked_bytes).expect("所有する元logのbyte列を読めません"); assert_eq!(locked_bytes, b"updated source log");
        assert_eq!(fs::read(&other_archive).expect("故障後の別archiveを読めません"), b"other complete source");
        assert_eq!(fs::read(&retained).expect("故障後の保持対象archiveを読めません"), b"retained archive");
        drop(locked);
        assert_eq!(fs::read(&source).expect("故障後の元logを読めません"), b"updated source log");
        assert!(!archive.with_extension("polaris_tmp").exists());
        // 保持違反を次回の復旧成功で隠さず、失敗直後の状態と復旧の両方を記録する。
        backup(&paths);
        assert_eq!(fs::read(&archive).expect("同じpathへの再試行でarchiveを復旧できません"), b"updated source log");
        assert_eq!(fs::read(&source).expect("再試行後の元logを読めません"), b"updated source log");
        assert_eq!(fs::read(&other_source).expect("再試行後の別元logを読めません"), b"other complete source");
        assert_eq!(fs::read(&other_archive).expect("再試行後の別archiveを読めません"), b"other complete source");
        assert_eq!(fs::read(&retained).expect("再試行後の保持対象archiveを読めません"), b"retained archive");
        assert!(!archive.with_extension("polaris_tmp").exists()); assert!(!other_archive.with_extension("polaris_tmp").exists());
        write_native_receipt("POLARIS_BACKUP_RECOVERY_SOURCE_LOCK_RECEIPT_PATH", "polaris-backup-recovery-source-lock-evidence.json",
            &format!("{{\"backup_calls\":2,\"real_source_sharing_violation\":true,\"previous_archive_preserved_after_failure\":{previous_archive_preserved},\"same_runtime_paths_retry_succeeded\":true,\"updated_archive_verified\":true,\"other_log_copied_after_failure\":true,\"unrelated_archive_preserved\":true,\"source_bytes_preserved\":true,\"temporary_files_absent\":true}}"));
        assert!(previous_archive_preserved, "コピー失敗前の旧archiveが失われました");
    }

    #[test]
    fn cleanup_keeps_different_case_extensions_and_log_without_parent_is_invalid() {
        use std::os::windows::ffi::OsStringExt;
        let directory = TestDir::new();
        let upper = directory.path("keep.POLARIS_TMP");
        fs::write(&upper, b"unrelated").expect("別拡張子のファイルを作成できません");
        clean_temp_files(&directory.0);
        assert_eq!(fs::read(upper).expect("別拡張子のファイルを読み取れません"), b"unrelated");
        assert!(!is_vrchat_log(Path::new("C:/")));
        let invalid = PathBuf::from(std::ffi::OsString::from_wide(&[0xd800]));
        assert!(!is_vrchat_log(&invalid));
    }

    // 入力の本文と期待suffixはtest所有のliteralで定義し、本体のlevel選択や出力から期待値を生成しない。
    const LOGGER_REPAIRED_DEBUG_SUFFIXES: [&str; 4] = [
        " DEBUG 出力先修復後: DEBUG 日本語と\"引用符\"\\path🙂",
        " INFO 出力先修復後: INFO 日本語と\"引用符\"\\path🙂",
        " WARN 出力先修復後: WARN 日本語と\"引用符\"\\path🙂",
        " ERROR 出力先修復後のログ: 日本語と\"引用符\"",
    ];
    const LOGGER_UNLOCKED_DEBUG_SUFFIXES: [&str; 4] = [
        " DEBUG lock解除後: DEBUG 日本語と\"引用符\"\\path🙂",
        " INFO lock解除後: INFO 日本語と\"引用符\"\\path🙂",
        " WARN lock解除後: WARN 日本語と\"引用符\"\\path🙂",
        " ERROR lock解除後のログ",
    ];
    const LOGGER_REPAIRED_AGAIN_DEBUG_SUFFIXES: [&str; 4] = [
        " DEBUG 再故障修復後: DEBUG 日本語と\"引用符\"\\path🙂",
        " INFO 再故障修復後: INFO 日本語と\"引用符\"\\path🙂",
        " WARN 再故障修復後: WARN 日本語と\"引用符\"\\path🙂",
        " ERROR 再故障の修復後のログ",
    ];

    fn emit_logger_lower_level_probes(stage: &str) {
        tracing::trace!("{stage}: TRACE 日本語と\"引用符\"\\path🙂");
        tracing::debug!("{stage}: DEBUG 日本語と\"引用符\"\\path🙂");
        tracing::info!("{stage}: INFO 日本語と\"引用符\"\\path🙂");
        tracing::warn!("{stage}: WARN 日本語と\"引用符\"\\path🙂");
    }

    fn assert_logger_event_suffixes(bytes: &[u8], previous: &[u8], expected: &[&str]) {
        let added = bytes.strip_prefix(previous).expect("後続のlogger出力で既存byte列が変わりました");
        let text = std::str::from_utf8(added).expect("追加ログはUTF-8で保存される契約です");
        let lines: Vec<_> = text.lines().collect(); assert_eq!(lines.len(), expected.len(), "level抑止後の保存件数が一致しません");
        for (line, suffix) in lines.iter().zip(expected) { assert!(line.ends_with(*suffix), "実loggerのlevelと本文が一致しません: {line}"); }
        assert!(!text.contains('\u{1b}'), "ANSI escapeを保存してはいけません");
    }

    const LOGGER_LEVEL_ARTIFACTS: [&str; 6] = [
        "logger-initial-fault.bin", "logger-after-initial-repair.log", "logger-after-lock.log",
        "logger-runtime-fault.bin", "logger-after-runtime-failure.log", "logger-final.log",
    ];

    // write_native_receiptで検証済みの保存先へ、childが照合した実byte列をreceiptと共に保存する。
    fn copy_logger_level_artifacts(directory: &TestDir) {
        if let Some(path) = std::env::var_os("POLARIS_LOGGER_RECOVERY_TEST_RECEIPT_PATH") {
            let receipt = PathBuf::from(path); let destination = receipt.parent().expect("logger receiptの親がありません").join("polaris-logger-levels-artifacts");
            fs::create_dir(&destination).expect("logger観測の専用保存先を作成できません");
            for name in LOGGER_LEVEL_ARTIFACTS { fs::copy(directory.path(name), destination.join(name)).expect("childが照合したloggerのbyte列を保存できません"); }
        }
    }

    // 出力先故障と復旧を実filesystemで通し、subscriberの再登録やprocess再起動に依存しないことを確認する。
    #[test]
    fn logger_recovers_after_output_lock_and_path_failure_in_same_process() {
        use std::os::windows::fs::OpenOptionsExt;
        let test = "tests::logger_recovers_after_output_lock_and_path_failure_in_same_process";
        if std::env::var("POLARIS_NATIVE_TEST_MODE").as_deref() == Ok("logger-recovery") {
            let directory = TestDir::new(); let logs = directory.path("logs"); let log_path = directory.path("logs/info.log");
            fs::write(&logs, b"occupied log directory").expect("所有する出力先の故障を準備できません");
            init_logger(&directory.0).expect("出力先故障中の初回loggerを開始できません");
            emit_logger_lower_level_probes("出力先故障中");
            tracing::error!("出力先故障中には保存しないログ");
            let initial_fault = fs::read(&logs).expect("故障中の占有fileを読めません"); assert_eq!(initial_fault, b"occupied log directory"); assert!(!log_path.exists());
            fs::remove_file(&logs).expect("所有する出力先の故障を除去できません"); fs::create_dir(&logs).expect("同じ出力先を復旧できません");
            fs::write(&log_path, "既存ログ\n").expect("保持対象のログを準備できません");
            emit_logger_lower_level_probes("出力先修復後");
            tracing::error!("出力先修復後のログ: {}", "日本語と\"引用符\"");
            let first = fs::read(&log_path).expect("出力先修復後のログを読めません"); let first_text = std::str::from_utf8(&first).expect("ログがUTF-8ではありません");
            let debug_build = cfg!(debug_assertions);
            assert_logger_event_suffixes(&first, "既存ログ\n".as_bytes(), if debug_build { &LOGGER_REPAIRED_DEBUG_SUFFIXES } else { &[" ERROR 出力先修復後のログ: 日本語と\"引用符\""] });
            assert!(!first_text.contains("出力先故障中には保存しないログ")); assert!(!first_text.contains('\u{1b}'));

            let locked = fs::OpenOptions::new().read(true).share_mode(0).open(&log_path).expect("所有するinfo.logを排他lockできません");
            emit_logger_lower_level_probes("排他中");
            tracing::error!("排他中には保存しないログ");
            assert_eq!(locked.metadata().expect("排他中のログsizeを読めません").len(), first.len() as u64);
            drop(locked); let after_lock = fs::read(&log_path).expect("lock解除直後のログを読めません"); assert_eq!(after_lock, first);
            emit_logger_lower_level_probes("lock解除後");
            tracing::error!("lock解除後のログ");
            let second = fs::read(&log_path).expect("lock解除後の出力を読めません");
            assert_logger_event_suffixes(&second, &first, if debug_build { &LOGGER_UNLOCKED_DEBUG_SUFFIXES } else { &[" ERROR lock解除後のログ"] });
            assert!(!std::str::from_utf8(&second).expect("追加ログがUTF-8ではありません").contains("排他中"));

            let displaced = directory.path("retained-logs"); fs::rename(&logs, &displaced).expect("所有する出力先を退避できません");
            fs::write(&logs, b"occupied again").expect("実行中の出力先故障を準備できません"); emit_logger_lower_level_probes("再故障中"); tracing::error!("再故障中には保存しないログ");
            let runtime_fault = fs::read(&logs).expect("再故障中の占有fileを読めません"); assert_eq!(runtime_fault, b"occupied again");
            let displaced_bytes = fs::read(displaced.join("info.log")).expect("退避した既存ログを読めません"); assert_eq!(displaced_bytes, second);
            fs::remove_file(&logs).expect("所有する再故障のfileを除去できません"); fs::rename(&displaced, &logs).expect("同じ出力先を戻せません");
            emit_logger_lower_level_probes("再故障修復後");
            tracing::error!("再故障の修復後のログ");
            let final_bytes = fs::read(&log_path).expect("再故障からの復旧後の出力を読めません");
            assert_logger_event_suffixes(&final_bytes, &second, if debug_build { &LOGGER_REPAIRED_AGAIN_DEBUG_SUFFIXES } else { &[" ERROR 再故障の修復後のログ"] });
            let final_text = std::str::from_utf8(&final_bytes).expect("最終ログがUTF-8ではありません");
            let expected_line_count = if debug_build { 13 } else { 4 };
            assert_eq!(final_text.lines().count(), expected_line_count); assert!(!final_text.contains('\u{1b}')); assert!(!displaced.exists());
            for suppressed in [": TRACE ", "出力先故障中", "排他中", "再故障中"] { assert!(!final_text.contains(suppressed), "抑止・故障中のmessageを後で再送してはいけません: {suppressed}"); }
            let receipt_root = PathBuf::from(std::env::var_os("POLARIS_NATIVE_TEST_ROOT").expect("親の専用記録先がありません")).canonicalize().expect("親の専用記録先を解決できません");
            assert_eq!(receipt_root.parent(), Some(std::env::temp_dir().canonicalize().expect("Tempの親を解決できません").as_path()));
            assert!(receipt_root.file_name().is_some_and(|name| name.to_string_lossy().starts_with("polaris-test-")));
            for (name, bytes) in LOGGER_LEVEL_ARTIFACTS.iter().zip([&initial_fault, &first, &after_lock, &runtime_fault, &displaced_bytes, &final_bytes]) { fs::write(receipt_root.join(name), bytes).expect("loggerの照合済みbyte列を保存できません"); }
            let profile = if debug_build { "Debug" } else { "Release" }; let logged_levels = if debug_build { "[\"DEBUG\",\"INFO\",\"WARN\",\"ERROR\"]" } else { "[\"ERROR\"]" };
            fs::write(receipt_root.join("logger.completed.json"), format!("{{\"child_pid\":{},\"logger_initializations\":1,\"same_process_recovery\":true,\"initial_sink_recovered\":true,\"exclusive_file_lock_recovered\":true,\"runtime_path_collision_recovered\":true,\"prior_log_bytes_preserved\":true,\"failure_messages_not_replayed\":true,\"utf8_message_suffixes_verified\":true,\"ansi_escape_absent\":true,\"final_line_count\":{expected_line_count},\"build_profile\":\"{profile}\",\"emitted_levels\":[\"TRACE\",\"DEBUG\",\"INFO\",\"WARN\",\"ERROR\"],\"logged_levels_per_recovery_stage\":{logged_levels},\"recovery_stages\":3,\"failure_stages\":3,\"level_policy_verified\":true,\"debug_level_verified\":{debug_build},\"release_level_verified\":{}}}", std::process::id(), !debug_build)).expect("logger復旧の観測を保存できません");
            return;
        }
        let directory = TestDir::new(); let mut child = OwnedTestChild::start(test, "logger-recovery", &directory.0); let child_pid = child.0.id();
        let status = child.finish(); let diagnostic = child.diagnostic(); assert!(status.success() && diagnostic.contains("test result: ok. 1 passed; 0 failed;"), "logger復旧のchildが成功していません: {diagnostic}");
        let receipt = fs::read_to_string(directory.path("logger.completed.json")).expect("logger復旧の完了記録がありません");
        assert!(receipt.starts_with(&format!("{{\"child_pid\":{child_pid},"))); assert!(receipt.contains("\"logger_initializations\":1"));
        write_native_receipt("POLARIS_LOGGER_RECOVERY_TEST_RECEIPT_PATH", "polaris-logger-recovery-evidence.json", &receipt);
        copy_logger_level_artifacts(&directory);
    }

    #[test]
    #[allow(clippy::panic, reason = "panic hookの記録を隔離した子processで検証するため")]
    fn registry_startup_and_logging_are_checked_in_an_isolated_process() {
        use std::os::windows::process::CommandExt;
        use std::process::Command;

        if let Ok(mode) = std::env::var("POLARIS_TEST_CHILD_MODE") {
            let directory = TestDir::new();
            if mode == "registry" {
                let registry = TestRegistry::new();
                assert!(ready_env().err().expect("未登録で起動できてはいけません").to_string().contains("インストール情報"));
                let (settings, _) = registry.isolated.create_subkey("Software\\CosmoArtsStore\\Polaris").expect("試験用設定を作成できません");
                assert!(ready_env().err().expect("保存値なしで起動できてはいけません").to_string().contains("インストール情報"));
                settings.set_value("InstallLocation", &directory.path("missing").to_string_lossy().as_ref()).expect("設定値を保存できません");
                assert!(ready_env().err().expect("存在しないinstall先で起動できてはいけません").to_string().contains("インストール先フォルダ"));
                settings.set_value("InstallLocation", &directory.0.to_string_lossy().as_ref()).expect("設定値を保存できません");
                let log_folder = dirs::home_dir().expect("Windows profileを取得できません").join("AppData/LocalLow/VRChat/VRChat");
                if log_folder.is_dir() {
                    let paths = ready_env().expect("有効なinstall設定で起動できません");
                    assert_eq!(paths.vrchat_log_folder, log_folder);
                    assert_eq!(paths.archive_folder, directory.path("Data/archive"));
                    assert!(directory.path("Data/logs").is_dir());
                    assert!(!paths.archive_folder.exists());
                } else {
                    assert!(ready_env().err().expect("未作成のVRChatログ先で起動できてはいけません").to_string().contains("VRChatのログ排出先"));
                    assert!(!directory.path("Data").exists());
                }
            } else {
                if mode == "sink" { fs::write(directory.path("logs"), b"occupied").expect("失敗用fileを作成できません"); }
                init_logger(&directory.0).expect("初回loggerを開始できません");
                tracing::error!("日本語の検証ログ");
                let panic = std::panic::catch_unwind(|| std::panic::panic_any("検証用panic"));
                assert!(panic.is_err());
                if mode == "sink" {
                    assert!(directory.path("logs").is_file());
                    assert!(!directory.path("logs/info.log").exists());
                } else {
                    let logged = fs::read_to_string(directory.path("logs/info.log")).expect("ログを読めません");
                    assert!(logged.contains("日本語の検証ログ"));
                    assert!(logged.contains("検証用panic"));
                    assert!(!logged.contains('\u{1b}'));
                }
                assert!(init_logger(&directory.0).is_err());
            }
            return;
        }
        for mode in ["registry", "logger", "sink"] {
            let output = Command::new(std::env::current_exe().expect("test exeの場所を取得できません"))
                .args(["--exact", "tests::registry_startup_and_logging_are_checked_in_an_isolated_process", "--nocapture"])
                .env("POLARIS_TEST_CHILD_MODE", mode).creation_flags(0x0800_0000).output().expect("子processを起動できません");
            assert!(output.status.success(), "{mode}: {} {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        }
    }

    // 当波だけの保存物を作る。既存snapshotを再利用・上書きせず、assert前の観測byteを残す。
    fn save_copy_sharing_log_observation(stage: &str, files: &[(&str, &[u8])]) {
        let Some(root) = std::env::var_os("POLARIS_COPY_SHARING_LOG_ARTIFACT_DIR") else { return };
        let root = PathBuf::from(root).canonicalize().expect("当波のartifact保存先を解決できません");
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("検証workspaceがありません");
        let boundary = workspace.join(".claude/audit-work/checks").canonicalize().expect("検証成果物の親を取得できません");
        assert!(root != boundary && root.starts_with(&boundary), "artifact保存先が内部checksの専用directory外です");
        let destination = root.join(stage);
        fs::create_dir(&destination).expect("当波の新しい観測stageを作成できません");
        for &(name, bytes) in files { fs::write(destination.join(name), bytes).expect("当波の観測byteを保存できません"); }
    }

    #[test]
    fn safe_copy_preserves_existing_archive_on_rename_sharing_failure_and_retries() {
        use std::io::Read;
        use std::os::windows::fs::OpenOptionsExt;
        let directory = TestDir::new();
        let source = directory.path("source.txt"); let destination = directory.path("archive.txt");
        let temporary = destination.with_extension("polaris_tmp");
        let expected_source = "変更後の完全な日本語ログ🙂\n".as_bytes();
        let expected_previous = "置換前の保存済みログ\n".as_bytes();
        fs::write(&source, expected_source).expect("元logを書き込めません");
        fs::write(&destination, expected_previous).expect("旧archiveを書き込めません");
        let before_source = fs::read(&source).expect("故障前の元logを読めません");
        let before_archive = fs::read(&destination).expect("故障前の旧archiveを読めません");
        let fixture = directory.0.to_string_lossy().into_owned();
        save_copy_sharing_log_observation("copy-sharing-before", &[
            ("source.bin", &before_source), ("archive.bin", &before_archive), ("fixture-root.txt", fixture.as_bytes()),
        ]);

        let mut locked = fs::OpenOptions::new().read(true).share_mode(0).open(&destination).expect("所有する旧archiveをlockできません");
        let outcome = safe_copy(&source, &destination);
        let source_after_failure = fs::read(&source);
        let temporary_after_failure = fs::read(&temporary);
        let mut archive_from_handle = Vec::new();
        let handle_read = locked.read_to_end(&mut archive_from_handle);
        let outcome_text = match &outcome { Ok(()) => "Ok".to_string(), Err(error) => format!("Err: {error}") };
        let reads = format!("source_read_ok={}\ntemporary_read_ok={}\narchive_handle_read_ok={}\n", source_after_failure.is_ok(), temporary_after_failure.is_ok(), handle_read.is_ok());
        let mut failed_files = vec![("outcome.txt", outcome_text.as_bytes()), ("read-status.txt", reads.as_bytes()), ("archive-held.bin", archive_from_handle.as_slice())];
        if let Ok(bytes) = &source_after_failure { failed_files.push(("source.bin", bytes.as_slice())); }
        if let Ok(bytes) = &temporary_after_failure { failed_files.push(("temporary.bin", bytes.as_slice())); }
        save_copy_sharing_log_observation("copy-sharing-failed", &failed_files);
        drop(failed_files);
        let error = outcome.expect_err("所有する旧archiveの共有違反中にrenameが成功しました");
        assert!(error.to_string().starts_with("リネームに失敗:"), "コピー完了後のrename故障へ到達していません");
        assert!(handle_read.is_ok(), "所有するread handleで旧archiveを読めません");
        assert_eq!(archive_from_handle, expected_previous);
        assert_eq!(source_after_failure.expect("故障後の元logを読めません"), expected_source);
        assert_eq!(temporary_after_failure.expect("rename故障後のtmpを読めません"), expected_source);
        drop(locked);
        assert_eq!(fs::read(&destination).expect("lock解除後の旧archiveを読めません"), expected_previous);

        // 旧archiveやtmpを手動削除せず、同じsource/destinationへの実copy/renameを再試行する。
        let retry = safe_copy(&source, &destination);
        let source_after_retry = fs::read(&source); let archive_after_retry = fs::read(&destination);
        let temporary_exists = temporary.exists();
        let retry_text = match &retry { Ok(()) => "Ok".to_string(), Err(error) => format!("Err: {error}") };
        let retry_state = format!("source_read_ok={}\narchive_read_ok={}\ntemporary_exists={}\n", source_after_retry.is_ok(), archive_after_retry.is_ok(), temporary_exists);
        let mut retried_files = vec![("outcome.txt", retry_text.as_bytes()), ("read-status.txt", retry_state.as_bytes())];
        if let Ok(bytes) = &source_after_retry { retried_files.push(("source.bin", bytes.as_slice())); }
        if let Ok(bytes) = &archive_after_retry { retried_files.push(("archive.bin", bytes.as_slice())); }
        save_copy_sharing_log_observation("copy-sharing-retried", &retried_files);
        drop(retried_files);
        retry.expect("lock解除後に同じpathへの再試行が失敗しました");
        assert_eq!(source_after_retry.expect("再試行後の元logを読めません"), expected_source);
        assert_eq!(archive_after_retry.expect("再試行後のarchiveを読めません"), expected_source);
        assert!(!temporary_exists, "成功後にtmpが残っています");
    }

    #[test]
    fn logger_appends_complete_long_utf8_error_between_ordered_events() {
        // timestampを期待本文へ含めず、ERROR本文と終端LFの全byteを入力literalと照合する。
        fn assert_sequence(bytes: &[u8], previous: &[u8], expected: &[&str]) {
            let added = bytes.strip_prefix(previous).expect("loggerが既存prefixのbyteを変更しました");
            let text = std::str::from_utf8(added).expect("追加logger本文がUTF-8ではありません");
            assert!(!text.contains('\u{1b}'), "loggerがANSI escapeを保存しました");
            let lines: Vec<_> = text.split_inclusive('\n').collect();
            assert_eq!(lines.len(), expected.len(), "loggerの保存event数が一致しません");
            for (index, (line, message)) in lines.iter().zip(expected).enumerate() {
                let line = line.strip_suffix('\n').expect("logger eventの終端LFがありません");
                let (timestamp, actual_message) = line.split_once(" ERROR ").expect("ERROR levelと本文の区切りがありません");
                assert!(!timestamp.is_empty(), "logger eventのtimestamp fieldがありません");
                assert!(actual_message.as_bytes() == message.as_bytes(), "logger本文の全byteが一致しません: event={index}");
            }
        }

        let test = "tests::logger_appends_complete_long_utf8_error_between_ordered_events";
        let previous = "記録前から保持するログ\r\n".as_bytes();
        let long_message = format!("長文記録: {}", "日本語🙂".repeat(1024));
        let messages = ["長文前の開始記録", long_message.as_str(), "長文後の終了記録"];
        let profile = if cfg!(debug_assertions) { "Debug" } else { "Release" };
        if std::env::var("POLARIS_NATIVE_TEST_MODE").as_deref() == Ok("logger-long-sequence") {
            let receipt_root = PathBuf::from(std::env::var_os("POLARIS_NATIVE_TEST_ROOT").expect("親の専用記録先がありません")).canonicalize().expect("親の専用記録先を解決できません");
            assert_eq!(receipt_root.parent(), Some(std::env::temp_dir().canonicalize().expect("Tempの親を解決できません").as_path()));
            assert!(receipt_root.file_name().is_some_and(|name| name.to_string_lossy().starts_with("polaris-test-")));
            let directory = TestDir::new(); let log_path = directory.path("logs/info.log");
            fs::create_dir(directory.path("logs")).expect("所有するlogger directoryを作成できません");
            fs::write(&log_path, previous).expect("保持対象のlogger prefixを保存できません");
            let before = fs::read(&log_path).expect("初期logger prefixを読めません");
            init_logger(&directory.0).expect("所有するloggerを1回初期化できません");
            tracing::error!("{}", messages[0]);
            tracing::error!("{}", messages[1]);
            tracing::error!("{}", messages[2]);
            let after = fs::read(&log_path).expect("3event後のloggerを読めません");
            let child_pid = std::process::id().to_string();
            let fixture = directory.0.to_string_lossy().into_owned();
            let observation = format!("{{\"case\":\"{test}\",\"child_pid\":{},\"build_profile\":\"{profile}\",\"logger_initializations\":1,\"emitted_error_events\":3,\"long_message_bytes\":{}}}\n", std::process::id(), long_message.len());
            save_copy_sharing_log_observation("logger-long-observed", &[
                ("initial-info.log", &before), ("observed-info.log", &after), ("observation.json", observation.as_bytes()), ("fixture-root.txt", fixture.as_bytes()),
            ]);
            fs::write(receipt_root.join("logger-initial.bin"), &before).expect("親へ初期logger byteを保存できません");
            fs::write(receipt_root.join("logger-observed.bin"), &after).expect("親へ観測logger byteを保存できません");
            fs::write(receipt_root.join("logger-child-pid.txt"), &child_pid).expect("親へ所有child PIDを保存できません");
            fs::write(receipt_root.join("logger-profile.txt"), profile).expect("親へ実profileを保存できません");
            assert_eq!(before, previous);
            assert_sequence(&after, previous, &messages);
            fs::write(receipt_root.join("logger-completed.txt"), b"verified").expect("親へ本文照合完了を保存できません");
            save_copy_sharing_log_observation("logger-long-verified", &[("completed.txt", b"verified")]);
            return;
        }
        let directory = TestDir::new();
        let mut child = OwnedTestChild::start(test, "logger-long-sequence", &directory.0); let child_pid = child.0.id();
        let status = child.finish(); let diagnostic = child.diagnostic();
        assert!(status.success() && diagnostic.contains("test result: ok. 1 passed; 0 failed;"), "長文loggerの所有childが成功していません: {diagnostic}");
        assert!(diagnostic.contains(&format!("test {test} ... ok")), "所有childのexact case結果がありません");
        assert_eq!(fs::read_to_string(directory.path("logger-child-pid.txt")).expect("所有child PIDを読めません"), child_pid.to_string());
        assert_eq!(fs::read_to_string(directory.path("logger-profile.txt")).expect("実profileを読めません"), profile);
        let before = fs::read(directory.path("logger-initial.bin")).expect("初期logger byteを読めません");
        let after = fs::read(directory.path("logger-observed.bin")).expect("観測logger byteを読めません");
        assert_eq!(before, previous);
        assert_sequence(&after, previous, &messages);
        assert_eq!(fs::read(directory.path("logger-completed.txt")).expect("本文照合完了を読めません"), b"verified");
    }
}
