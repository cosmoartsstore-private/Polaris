use std::ffi::{c_void, OsString};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use windows::core::PWSTR;
use windows::Win32::Foundation::{BOOL, HANDLE, HWND, LPARAM, WPARAM, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Foundation as startup_foundation;
use windows::Win32::System::Threading as startup_process;
use windows::Win32::UI::WindowsAndMessaging as startup_window;

// test専用のdesktop APIだけを直接宣言し、通常buildのWindows featureを変更しない。
#[link(name = "user32")]
extern "system" {
    fn CreateDesktopW(name: *const u16, device: *const u16, mode: *const c_void, flags: u32, access: u32, attributes: *const c_void) -> *mut c_void;
    fn CloseDesktop(desktop: *mut c_void) -> BOOL;
    fn OpenInputDesktop(flags: u32, inherit: BOOL, access: u32) -> *mut c_void;
    fn GetThreadDesktop(thread: u32) -> *mut c_void;
    fn GetUserObjectInformationW(object: *mut c_void, index: i32, information: *mut c_void, bytes: u32, required: *mut u32) -> BOOL;
    fn EnumDesktopWindows(desktop: *mut c_void, callback: Option<unsafe extern "system" fn(HWND, LPARAM) -> BOOL>, parameter: LPARAM) -> BOOL;
}

struct OwnedStartupDesktop(*mut c_void);
impl OwnedStartupDesktop {
    fn close(mut self) {
        assert!(unsafe { CloseDesktop(self.0) }.as_bool(), "専用desktopを解放できません"); self.0 = std::ptr::null_mut();
    }
}
impl Drop for OwnedStartupDesktop {
    fn drop(&mut self) { if !self.0.is_null() { let _ = unsafe { CloseDesktop(self.0) }; } }
}

fn startup_desktop_name(handle: *mut c_void) -> String {
    assert!(!handle.is_null(), "desktop handleがありません"); let mut buffer = [0_u16; 512]; let mut required = 0;
    let bytes = u32::try_from(std::mem::size_of_val(&buffer)).expect("desktop名のbufferが大きすぎます");
    assert!(unsafe { GetUserObjectInformationW(handle, 2, buffer.as_mut_ptr().cast(), bytes, &raw mut required) }.as_bool());
    let length = buffer.iter().position(|value| *value == 0).expect("desktop名の終端がありません");
    String::from_utf16(&buffer[..length]).expect("desktop名を読めません")
}

fn startup_input_desktop_name() -> String {
    let desktop = OwnedStartupDesktop(unsafe { OpenInputDesktop(0, BOOL(0), 1) });
    let name = startup_desktop_name(desktop.0); desktop.close(); name
}

struct StartupDesktop { handle: OwnedStartupDesktop, name: String }
impl StartupDesktop {
    fn new(root: &Path) -> Self {
        let current = unsafe { GetThreadDesktop(startup_process::GetCurrentThreadId()) }; let original = startup_desktop_name(current);
        let name = root.file_name().expect("専用領域の名前がありません").to_str().expect("専用領域名を読めません").to_owned();
        let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let handle = OwnedStartupDesktop(unsafe { CreateDesktopW(wide.as_ptr(), std::ptr::null(), std::ptr::null(), 0, 0x00c3, std::ptr::null()) });
        assert_eq!(startup_desktop_name(handle.0), name); assert_ne!(name, startup_input_desktop_name());
        assert_eq!(startup_desktop_name(unsafe { GetThreadDesktop(startup_process::GetCurrentThreadId()) }), original);
        Self { handle, name }
    }

    fn windows(&self) -> Vec<HWND> {
        let mut windows = Vec::new(); let parameter = LPARAM(std::ptr::from_mut(&mut windows) as isize);
        unsafe { startup_foundation::SetLastError(startup_foundation::ERROR_SUCCESS) };
        let completed = unsafe { EnumDesktopWindows(self.handle.0, Some(collect_startup_window), parameter) }; let error = unsafe { GetLastError() };
        // 空desktopのFALSE／ERROR_SUCCESSと、実際の列挙エラーを区別する。
        assert!(completed.as_bool() || (error == startup_foundation::ERROR_SUCCESS && windows.is_empty()), "専用desktopの列挙が失敗しました。error={error:?}, windows={}", windows.len()); windows
    }

    fn wait_for_message(&self, child: &StartupProcess, expected: &str) -> HWND {
        let deadline = std::time::Instant::now() + Duration::from_secs(15); let mut observed = Vec::new();
        loop {
            observed.clear();
            for window in self.windows() {
                let mut pid = 0; unsafe { startup_window::GetWindowThreadProcessId(window, Some(&raw mut pid)) }; if pid != child.pid { continue; }
                let Some(title) = startup_window_text(window) else { continue }; if title != "Polaris" || startup_window_class(window) != "#32770" { continue; }
                let mut controls = Vec::new(); let parameter = LPARAM(std::ptr::from_mut(&mut controls) as isize);
                let _ = unsafe { startup_window::EnumChildWindows(window, Some(collect_startup_window), parameter) };
                let texts: Vec<String> = controls.iter().filter_map(|control| startup_window_text(*control)).collect();
                let ids: Vec<i32> = controls.iter().map(|control| unsafe { startup_window::GetDlgCtrlID(*control) }).collect();
                // MessageBoxの戻り値IDOKは内部control IDではない。実際の可視buttonを特定する。
                let buttons: Vec<HWND> = controls.iter().copied().filter(|control| startup_window_class(*control) == "Button" && startup_window_text(*control).as_deref() == Some("OK") && unsafe { startup_window::IsWindowVisible(*control) }.as_bool()).collect();
                observed.push(format!("text={texts:?}, ids={ids:?}, buttons={buttons:?}"));
                if texts.iter().any(|text| text == expected) && buttons.len() == 1 { return buttons[0]; }
            }
            assert!(child.running(), "本体がdialog確認前に終了しました: {observed:?} {}", child.diagnostic());
            assert!(std::time::Instant::now() < deadline, "実MessageBoxの本文を確認できません: {observed:?}"); std::thread::sleep(Duration::from_millis(20));
        }
    }
}

// 同期した列挙中だけ有効なVecを渡す。callbackからpanicやWindow操作は行わない。
unsafe extern "system" fn collect_startup_window(window: HWND, parameter: LPARAM) -> BOOL {
    unsafe { &mut *(parameter.0 as *mut Vec<HWND>) }.push(window); BOOL(1)
}

fn startup_window_text(window: HWND) -> Option<String> {
    let mut buffer = [0_u16; 1024]; let mut length = 0;
    let result = unsafe { startup_window::SendMessageTimeoutW(window, startup_window::WM_GETTEXT, WPARAM(buffer.len()), LPARAM(buffer.as_mut_ptr() as isize), startup_window::SMTO_ABORTIFHUNG, 1_000, Some(&raw mut length)) };
    if result.0 == 0 || length >= buffer.len() { return None; } String::from_utf16(&buffer[..length]).ok()
}

fn startup_window_class(window: HWND) -> String {
    let mut buffer = [0_u16; 128]; let length = unsafe { startup_window::GetClassNameW(window, &mut buffer) };
    String::from_utf16(&buffer[..usize::try_from(length).expect("window classの長さが不正です")]).expect("window classを読めません")
}

fn startup_registry_path(root: &Path) -> String {
    format!("Software\\Polaris.Tests\\startup-{}", root.file_name().expect("専用領域名がありません").to_str().expect("専用領域名を読めません"))
}

/// 本体のexitはstackを破棄しないため、親processが専用registry keyを所有する。
struct StartupRegistry { original: RegKey, isolated: RegKey, path: String }
impl StartupRegistry {
    fn new(root: &Path) -> Self {
        let original = RegKey::predef(HKEY_CURRENT_USER); let path = startup_registry_path(root);
        let (isolated, _) = original.create_subkey(&path).expect("専用startup registryを作成できません");
        isolated.set_value("Owner", &root.to_str().expect("専用領域を読めません")).expect("所有markerを保存できません"); Self { original, isolated, path }
    }
}
impl Drop for StartupRegistry {
    fn drop(&mut self) { let _ = self.original.delete_subkey_all(&self.path); }
}

struct StartupChildScope { root: PathBuf, _isolated: RegKey }
impl Drop for StartupChildScope {
    fn drop(&mut self) { let _ = unsafe { RegOverridePredefKey(HKEY_CURRENT_USER, 0) }; }
}

fn startup_child_scope(mode: &str) -> Option<StartupChildScope> {
    let current = std::env::var("POLARIS_STARTUP_TEST_MODE").ok()?; assert_eq!(current, mode);
    let root = PathBuf::from(std::env::var_os("POLARIS_STARTUP_TEST_ROOT").expect("専用startup領域がありません")).canonicalize().expect("専用領域を解決できません");
    assert_eq!(root.parent(), Some(std::env::temp_dir().canonicalize().expect("Temp領域を解決できません").as_path()));
    let name = root.file_name().expect("専用領域名がありません").to_str().expect("専用領域名を読めません"); assert!(name.starts_with("polaris-test-"));
    assert_eq!(startup_desktop_name(unsafe { GetThreadDesktop(startup_process::GetCurrentThreadId()) }), name);
    assert_ne!(name, startup_input_desktop_name());
    let isolated = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(startup_registry_path(&root), winreg::enums::KEY_READ | winreg::enums::KEY_WRITE).expect("所有startup registryを開けません");
    let owner = PathBuf::from(isolated.get_value::<String, _>("Owner").expect("所有markerがありません")).canonicalize().expect("所有markerの領域を解決できません");
    assert_eq!(owner, root);
    assert_eq!(unsafe { RegOverridePredefKey(HKEY_CURRENT_USER, isolated.raw_handle()) }, 0);
    fs::write(root.join("startup.before.txt"), std::process::id().to_string()).expect("本体呼出し前を記録できません"); Some(StartupChildScope { root, _isolated: isolated })
}

/// CreateProcessが返したhandleだけを所有し、異常時も利用者のprocessを終了しない。
struct StartupProcess { handle: HANDLE, pid: u32, log: PathBuf }
impl StartupProcess {
    fn start(test: &str, mode: &str, root: &Path, desktop: &StartupDesktop) -> Self {
        let executable = std::env::current_exe().expect("test exeを取得できません"); let application: Vec<u16> = executable.as_os_str().encode_wide().chain([0]).collect();
        let mut command: Vec<u16> = format!("\"{}\" --exact {test} --test-threads=1 --nocapture", executable.to_str().expect("test exeのpathを読めません")).encode_utf16().chain([0]).collect();
        let mut environment: Vec<(OsString, OsString)> = std::env::vars_os().filter(|(key, _)| !key.to_string_lossy().eq_ignore_ascii_case("POLARIS_STARTUP_TEST_MODE") && !key.to_string_lossy().eq_ignore_ascii_case("POLARIS_STARTUP_TEST_ROOT")).collect();
        environment.extend([(OsString::from("POLARIS_STARTUP_TEST_MODE"), OsString::from(mode)), (OsString::from("POLARIS_STARTUP_TEST_ROOT"), root.as_os_str().to_owned())]);
        environment.sort_by_key(|(key, _)| key.to_string_lossy().to_uppercase()); let mut block = Vec::new();
        for (key, value) in environment { block.extend(key.encode_wide()); block.push(u16::from(b'=')); block.extend(value.encode_wide()); block.push(0); } block.push(0);
        let mut desktop_name: Vec<u16> = desktop.name.encode_utf16().chain([0]).collect();
        let log = root.join("startup.child.log"); let output = fs::OpenOptions::new().write(true).create_new(true).open(&log).expect("専用child logを作成できません");
        let input = fs::File::open("NUL").expect("child用の空入力を開けません"); let output_handle = HANDLE(output.as_raw_handle()); let input_handle = HANDLE(input.as_raw_handle());
        for handle in [input_handle, output_handle] { unsafe { startup_foundation::SetHandleInformation(handle, startup_foundation::HANDLE_FLAG_INHERIT.0, startup_foundation::HANDLE_FLAG_INHERIT) }.expect("専用standard handleの継承を設定できません"); }
        let startup = startup_process::STARTUPINFOW { cb: u32::try_from(std::mem::size_of::<startup_process::STARTUPINFOW>()).expect("startup infoが大きすぎます"),
            lpDesktop: PWSTR(desktop_name.as_mut_ptr()), dwFlags: startup_process::STARTF_FORCEOFFFEEDBACK | startup_process::STARTF_USESTDHANDLES,
            hStdInput: input_handle, hStdOutput: output_handle, hStdError: output_handle, ..Default::default() };
        let mut information = startup_process::PROCESS_INFORMATION::default();
        unsafe { startup_process::CreateProcessW(PCWSTR(application.as_ptr()), PWSTR(command.as_mut_ptr()), None, None, true,
            startup_process::CREATE_NO_WINDOW | startup_process::CREATE_UNICODE_ENVIRONMENT, Some(block.as_ptr().cast()), PCWSTR::null(), &raw const startup, &raw mut information) }.expect("専用desktopへchildを起動できません");
        let child = Self { handle: information.hProcess, pid: information.dwProcessId, log }; unsafe { CloseHandle(information.hThread) }.expect("child thread handleを解放できません");
        for handle in [input_handle, output_handle] { unsafe { startup_foundation::SetHandleInformation(handle, startup_foundation::HANDLE_FLAG_INHERIT.0, startup_foundation::HANDLE_FLAGS(0)) }.expect("専用handleの継承を解除できません"); } child
    }

    fn running(&self) -> bool { (unsafe { startup_process::WaitForSingleObject(self.handle, 0) }) == WAIT_TIMEOUT }

    fn diagnostic(&self) -> String { fs::read_to_string(&self.log).expect("専用child logを読めません") }

    fn wait_before(&self, root: &Path) {
        let deadline = std::time::Instant::now() + Duration::from_secs(15); let marker = root.join("startup.before.txt");
        while !marker.is_file() {
            let mut code = 0; unsafe { startup_process::GetExitCodeProcess(self.handle, &raw mut code) }.expect("childの状態を読めません");
            assert!(self.running(), "startup marker前にchildが終了しました。exit={code}: {}", self.diagnostic());
            assert!(std::time::Instant::now() < deadline); std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(fs::read_to_string(marker).expect("startup markerを読めません"), self.pid.to_string());
    }

    fn finish(&self) -> u32 {
        assert_eq!(unsafe { startup_process::WaitForSingleObject(self.handle, 15_000) }, WAIT_OBJECT_0, "専用startup childが終了しません"); let mut code = 0;
        unsafe { startup_process::GetExitCodeProcess(self.handle, &raw mut code) }.expect("childの終了codeを読めません"); code
    }
}
impl Drop for StartupProcess {
    fn drop(&mut self) {
        if self.running() { let _ = unsafe { startup_process::TerminateProcess(self.handle, 1) }; let _ = unsafe { startup_process::WaitForSingleObject(self.handle, 5_000) }; }
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

fn assert_startup_mutex_available() {
    let name: Vec<u16> = SINGLE_INSTANCE_MUTEX_NAME.encode_utf16().chain([0]).collect();
    let handle = OwnedNativeHandle(unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.expect("起動状態を確認できません"));
    assert_ne!(unsafe { GetLastError() }, ERROR_ALREADY_EXISTS, "実Polarisが稼働しているため本体起動testを開始できません"); drop(handle);
}

#[test]
fn native_startup_main_reports_install_errors_and_exits_after_ok() {
    if let Some(scope) = startup_child_scope("main-error") { super::main(); fs::write(scope.root.join("startup.returned.txt"), b"returned").expect("本体returnを記録できません"); return; }
    let _process_lock = lock_native_process_tests(); let input = startup_input_desktop_name();
    for case in ["unregistered", "missing-value", "wrong-type", "missing-directory"] {
        assert_startup_mutex_available(); let directory = TestDir::new(); let desktop = StartupDesktop::new(&directory.0); let registry = StartupRegistry::new(&directory.0);
        let settings = if case == "unregistered" { None } else { Some(registry.isolated.create_subkey("Software\\CosmoArtsStore\\Polaris").expect("専用install設定を作成できません").0) };
        if case == "wrong-type" { settings.as_ref().expect("設定keyがありません").set_value("InstallLocation", &23_u32).expect("型違いの値を保存できません"); }
        if case == "missing-directory" { settings.as_ref().expect("設定keyがありません").set_value("InstallLocation", &directory.path("missing-install").to_str().expect("専用pathを読めません")).expect("不存在のinstall先を保存できません"); }
        let previous = settings.as_ref().and_then(|key| key.get_raw_value("InstallLocation").ok());
        let child = StartupProcess::start("tests::native_startup_main_reports_install_errors_and_exits_after_ok", "main-error", &directory.0, &desktop); child.wait_before(&directory.0);
        let expected = if case == "missing-directory" { "インストール先フォルダが見つかりません。再インストールしてください。" } else { "インストール情報を取得できませんでした。再インストールしてください。" };
        let button = desktop.wait_for_message(&child, expected); assert!(child.running()); assert!(!directory.path("startup.returned.txt").exists());
        unsafe { startup_window::PostMessageW(button, startup_window::BM_CLICK, WPARAM(0), LPARAM(0)) }.expect("専用dialogのOKを操作できません"); assert_eq!(child.finish(), 0);
        assert!(!directory.path("startup.returned.txt").exists()); assert!(!directory.path("Data").exists()); assert!(!directory.path("missing-install").exists());
        assert_eq!(registry.isolated.get_value::<String, _>("Owner").expect("所有markerを読めません"), directory.0.to_str().expect("専用pathを読めません"));
        let current = settings.as_ref().and_then(|key| key.get_raw_value("InstallLocation").ok()); assert_eq!(current, previous);
        assert!(desktop.windows().is_empty()); let registry_path = registry.path.clone(); drop(child); drop(settings); drop(registry);
        assert!(RegKey::predef(HKEY_CURRENT_USER).open_subkey(registry_path).is_err()); desktop.handle.close(); assert_eq!(startup_input_desktop_name(), input);
    }
    write_native_receipt("POLARIS_STARTUP_ERRORS_TEST_RECEIPT_PATH", "polaris-native-startup-errors-evidence.json", "{\"install_error_cases\":4,\"title\":\"Polaris\",\"full_text_verified\":true,\"ok_then_exit_code\":0,\"data_created\":false,\"registry_values_preserved\":true,\"owned_keys_removed\":true,\"input_desktop_unchanged\":true}");
}

const STARTUP_DIRECT_MESSAGE: &str = "検証用の日本語メッセージ\n引用符「値」と path\\file.txt、🛰️";

#[test]
fn native_startup_message_preserves_text_and_returns_after_ok() {
    if let Some(scope) = startup_child_scope("message") { show_error(STARTUP_DIRECT_MESSAGE); fs::write(scope.root.join("startup.returned.txt"), b"returned").expect("dialog後のreturnを記録できません"); return; }
    let _process_lock = lock_native_process_tests(); let input = startup_input_desktop_name();
    let directory = TestDir::new(); let desktop = StartupDesktop::new(&directory.0); let registry = StartupRegistry::new(&directory.0);
    let child = StartupProcess::start("tests::native_startup_message_preserves_text_and_returns_after_ok", "message", &directory.0, &desktop); child.wait_before(&directory.0);
    let button = desktop.wait_for_message(&child, STARTUP_DIRECT_MESSAGE); assert!(!directory.path("startup.returned.txt").exists()); assert!(child.running());
    unsafe { startup_window::PostMessageW(button, startup_window::BM_CLICK, WPARAM(0), LPARAM(0)) }.expect("専用dialogのOKを操作できません"); assert_eq!(child.finish(), 0);
    assert_eq!(fs::read(directory.path("startup.returned.txt")).expect("dialog後のreturnがありません"), b"returned"); assert!(desktop.windows().is_empty());
    let registry_path = registry.path.clone(); drop(child); drop(registry); assert!(RegKey::predef(HKEY_CURRENT_USER).open_subkey(registry_path).is_err()); desktop.handle.close();
    assert_eq!(startup_input_desktop_name(), input);
    write_native_receipt("POLARIS_STARTUP_MESSAGE_TEST_RECEIPT_PATH", "polaris-native-startup-message-evidence.json", "{\"title\":\"Polaris\",\"unicode_and_newline_preserved\":true,\"blocked_before_ok\":true,\"returned_after_ok\":true,\"owned_key_removed\":true,\"input_desktop_unchanged\":true}");
}

#[test]
fn native_startup_main_duplicate_exits_before_install_error_dialog() {
    if let Some(scope) = startup_child_scope("main-duplicate") { super::main(); fs::write(scope.root.join("startup.returned.txt"), b"returned").expect("本体returnを記録できません"); return; }
    let _process_lock = lock_native_process_tests(); let input = startup_input_desktop_name(); assert_startup_mutex_available();
    let directory = TestDir::new(); let desktop = StartupDesktop::new(&directory.0); let registry = StartupRegistry::new(&directory.0);
    let name: Vec<u16> = SINGLE_INSTANCE_MUTEX_NAME.encode_utf16().chain([0]).collect();
    let mutex = OwnedNativeHandle(unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.expect("先行mutexを取得できません")); assert_ne!(unsafe { GetLastError() }, ERROR_ALREADY_EXISTS);
    let child = StartupProcess::start("tests::native_startup_main_duplicate_exits_before_install_error_dialog", "main-duplicate", &directory.0, &desktop); child.wait_before(&directory.0); assert_eq!(child.finish(), 0);
    assert!(desktop.windows().is_empty()); assert!(!directory.path("startup.returned.txt").exists()); assert!(!directory.path("Data").exists());
    let registry_path = registry.path.clone(); drop(child); drop(mutex); drop(registry); assert!(RegKey::predef(HKEY_CURRENT_USER).open_subkey(registry_path).is_err()); desktop.handle.close();
    assert_eq!(startup_input_desktop_name(), input); assert_startup_mutex_available();
    write_native_receipt("POLARIS_STARTUP_DUPLICATE_TEST_RECEIPT_PATH", "polaris-native-startup-duplicate-evidence.json", "{\"main_called\":true,\"exit_code\":0,\"no_dialog\":true,\"no_data_created\":true,\"owned_key_removed\":true,\"mutex_released\":true,\"input_desktop_unchanged\":true}");
}

#[test]
fn native_startup_mutex_type_collision_reports_error_and_releases_after_ok() {
    if let Some(scope) = startup_child_scope("mutex-error") { super::main(); fs::write(scope.root.join("startup.returned.txt"), b"returned").expect("本体returnを記録できません"); return; }
    let _process_lock = lock_native_process_tests(); let input = startup_input_desktop_name(); assert_startup_mutex_available();
    let directory = TestDir::new(); let desktop = StartupDesktop::new(&directory.0); let registry = StartupRegistry::new(&directory.0);
    let name: Vec<u16> = SINGLE_INSTANCE_MUTEX_NAME.encode_utf16().chain([0]).collect();
    let event = OwnedNativeHandle(unsafe { startup_process::CreateEventW(None, false, false, PCWSTR(name.as_ptr())) }.expect("所有する衝突eventを作成できません")); assert_ne!(unsafe { GetLastError() }, ERROR_ALREADY_EXISTS);
    let child = StartupProcess::start("tests::native_startup_mutex_type_collision_reports_error_and_releases_after_ok", "mutex-error", &directory.0, &desktop); child.wait_before(&directory.0);
    let external = windows::core::Error::from_hresult(windows::core::HRESULT::from_win32(startup_foundation::ERROR_INVALID_HANDLE.0));
    let expected = format!("ミューテックスを作成できませんでした: {external}"); let button = desktop.wait_for_message(&child, &expected); assert!(child.running());
    unsafe { startup_window::PostMessageW(button, startup_window::BM_CLICK, WPARAM(0), LPARAM(0)) }.expect("専用dialogのOKを操作できません"); assert_eq!(child.finish(), 0);
    assert!(!directory.path("startup.returned.txt").exists()); assert!(!directory.path("Data").exists()); assert!(desktop.windows().is_empty());
    let registry_path = registry.path.clone(); drop(child); drop(event); drop(registry); assert!(RegKey::predef(HKEY_CURRENT_USER).open_subkey(registry_path).is_err()); desktop.handle.close();
    assert_eq!(startup_input_desktop_name(), input); assert_startup_mutex_available();
    write_native_receipt("POLARIS_STARTUP_MUTEX_TEST_RECEIPT_PATH", "polaris-native-startup-mutex-evidence.json", "{\"main_called\":true,\"native_error_code\":6,\"full_text_verified\":true,\"ok_then_exit_code\":0,\"no_data_created\":true,\"owned_key_removed\":true,\"object_name_released\":true,\"input_desktop_unchanged\":true}");
}

const STARTUP_RUNNING_LOG: &str = "正常起動の検証ログ";

#[test]
fn native_startup_main_initializes_logger_and_monitors_owned_running_process() {
    if let Some(scope) = startup_child_scope("main-running") {
        let pid: u32 = fs::read_to_string(scope.root.join("owned-vrchat-first.started")).expect("所有processの開始記録がありません").parse().expect("所有PIDを読めません");
        assert_eq!(find_vrchat_pid(), Some(pid));
        assert!(dirs::home_dir().expect("Windows profileを取得できません").join("AppData/LocalLow/VRChat/VRChat").is_dir());
        // 実mainを呼び、所有するVRChat helperの稼働中だけ観測して利用者のlogをコピーしない。
        let main = std::thread::spawn(super::main); let log = scope.root.join("Data/logs/info.log"); let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while !fs::read_to_string(&log).is_ok_and(|text| text.contains(STARTUP_RUNNING_LOG)) {
            assert!(!main.is_finished(), "本体mainが監視前に終了しました"); assert_eq!(find_vrchat_pid(), Some(pid));
            assert!(!scope.root.join("Data/archive").exists()); assert!(std::time::Instant::now() < deadline, "本体loggerの初期化を確認できません");
            tracing::error!("正常起動の検証ログ"); std::thread::sleep(Duration::from_millis(20));
        }
        fs::write(scope.root.join("startup.ready.txt"), std::process::id().to_string()).expect("正常起動の完了を記録できません");
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while !scope.root.join("startup.stop.txt").is_file() {
            assert!(!main.is_finished(), "正常起動後に監視が終了しました"); assert_eq!(find_vrchat_pid(), Some(pid));
            assert!(!scope.root.join("Data/archive").exists()); assert!(std::time::Instant::now() < deadline, "親processの観測が終了しません"); std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(fs::read(scope.root.join("startup.stop.txt")).expect("終了markerを読めません"), b"stop");
        assert!(!main.is_finished()); assert_eq!(find_vrchat_pid(), Some(pid)); assert!(!scope.root.join("Data/archive").exists());
        fs::write(scope.root.join("startup.completed.txt"), b"completed").expect("本体観測の正常完了を記録できません"); return;
    }
    let _process_lock = lock_native_process_tests(); assert_owned_vrchat_processes(&[]); assert_startup_mutex_available();
    let input = startup_input_desktop_name(); let directory = TestDir::new(); let desktop = StartupDesktop::new(&directory.0); let registry = StartupRegistry::new(&directory.0);
    let (settings, _) = registry.isolated.create_subkey("Software\\CosmoArtsStore\\Polaris").expect("専用install設定を作成できません");
    settings.set_value("InstallLocation", &directory.0.to_str().expect("専用install先を読めません")).expect("専用install先を保存できません");
    let previous = settings.get_raw_value("InstallLocation").expect("専用設定を読めません"); let executable = directory.path("VRChat.exe");
    fs::copy(std::env::current_exe().expect("test exeを取得できません"), &executable).expect("専用process imageを作成できません");
    let mut running = OwnedTestChild::start_executable(&executable, "tests::actual_polling_loop_handles_owned_exit_and_pid_replacement", "owned-vrchat-first", &directory.0);
    running.wait_for_marker(&directory.path("owned-vrchat-first.started")); assert_owned_vrchat_processes(&[running.0.id()]); assert_eq!(find_vrchat_pid(), Some(running.0.id()));
    // 宣言順により、失敗時も本体childを先に停止してから所有VRChat helperを解放する。
    let child = StartupProcess::start("tests::native_startup_main_initializes_logger_and_monitors_owned_running_process", "main-running", &directory.0, &desktop); child.wait_before(&directory.0);
    let deadline = std::time::Instant::now() + Duration::from_secs(15); let ready = directory.path("startup.ready.txt");
    while !ready.is_file() {
        assert!(child.running(), "本体が正常起動前に終了しました: {}", child.diagnostic());
        assert!(std::time::Instant::now() < deadline, "本体の正常起動を確認できません: {}", child.diagnostic()); std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(fs::read_to_string(ready).expect("正常起動markerを読めません"), child.pid.to_string());
    let started = std::time::Instant::now(); std::thread::sleep(Duration::from_millis(2_200)); let observation_ms = started.elapsed().as_millis();
    assert!(child.running()); assert!(desktop.windows().is_empty()); assert_owned_vrchat_processes(&[running.0.id()]); assert_eq!(find_vrchat_pid(), Some(running.0.id()));
    assert!(fs::read_to_string(directory.path("Data/logs/info.log")).expect("本体loggerの出力を読めません").contains(STARTUP_RUNNING_LOG)); assert!(!directory.path("Data/archive").exists());
    assert_eq!(settings.get_raw_value("InstallLocation").expect("起動後の設定を読めません"), previous); fs::write(directory.path("startup.stop.txt"), b"stop").expect("観測終了を通知できません");
    assert_eq!(child.finish(), 0); assert!(child.diagnostic().contains("test result: ok. 1 passed; 0 failed;"));
    assert_eq!(fs::read(directory.path("startup.completed.txt")).expect("正常完了markerがありません"), b"completed"); assert!(desktop.windows().is_empty());
    let main_pid = child.pid; drop(child); stop_owned_monitor_child(&mut running); assert_owned_vrchat_processes(&[]); assert_eq!(find_vrchat_pid(), None); assert_startup_mutex_available();
    let registry_path = registry.path.clone(); drop(settings); drop(registry); assert!(RegKey::predef(HKEY_CURRENT_USER).open_subkey(registry_path).is_err()); desktop.handle.close(); assert_eq!(startup_input_desktop_name(), input);
    write_native_receipt("POLARIS_STARTUP_RUNNING_TEST_RECEIPT_PATH", "polaris-native-startup-running-evidence.json",
        &format!("{{\"main_called\":true,\"main_pid\":{main_pid},\"observation_ms\":{observation_ms},\"native_profile_metadata_only\":true,\"logger_initialized\":true,\"no_archive_created\":true,\"no_dialog\":true,\"test_child_completed\":true,\"registry_value_preserved\":true,\"owned_key_removed\":true,\"mutex_released\":true,\"input_desktop_unchanged\":true}}"));
}

const STARTUP_IDLE_NEW: &[u8] = "起動時の保存対象\n全行\n".as_bytes();
const STARTUP_IDLE_CHANGED: &[u8] = "更新後の本文\n".as_bytes();
const STARTUP_IDLE_UNRELATED: &[u8] = "他の保存済みデータ\n".as_bytes();
const STARTUP_IDLE_SOURCES: [(&str, &[u8]); 5] = [("output_log_startup.txt", STARTUP_IDLE_NEW), ("output_log_changed.txt", STARTUP_IDLE_CHANGED), ("output_log_kept.txt", b"new"), ("notes.txt", b"not a VRChat log"), ("output_log_empty.txt", b"")];
const STARTUP_IDLE_ARCHIVES: [(&str, &[u8]); 4] = [("output_log_startup.txt", STARTUP_IDLE_NEW), ("output_log_changed.txt", STARTUP_IDLE_CHANGED), ("output_log_kept.txt", b"old"), ("preserved.txt", STARTUP_IDLE_UNRELATED)];

fn prepare_startup_idle_data(root: &Path) {
    let log_folder = root.join("profile/AppData/LocalLow/VRChat/VRChat"); let archive_folder = root.join("Data/archive"); fs::create_dir_all(&log_folder).expect("専用profileのlog先を作成できません"); fs::create_dir_all(&archive_folder).expect("専用archiveを作成できません");
    for (name, bytes) in STARTUP_IDLE_SOURCES { fs::write(log_folder.join(name), bytes).expect("起動時の元logを用意できません"); }
    for (name, bytes) in [("output_log_changed.txt", b"previous".as_slice()), ("output_log_kept.txt", b"old".as_slice()), ("preserved.txt", STARTUP_IDLE_UNRELATED), ("owned.polaris_tmp", b"incomplete".as_slice())] { fs::write(archive_folder.join(name), bytes).expect("既存archiveを用意できません"); }
}

fn assert_startup_idle_data(root: &Path) {
    for (name, bytes) in STARTUP_IDLE_SOURCES { assert_eq!(fs::read(root.join("profile/AppData/LocalLow/VRChat/VRChat").join(name)).expect("起動時の元logを読めません"), bytes, "元logを変更してはいけません: {name}"); }
    for (name, bytes) in STARTUP_IDLE_ARCHIVES { assert_eq!(fs::read(root.join("Data/archive").join(name)).expect("起動時のarchiveを読めません"), bytes, "起動時の保存内容が一致しません: {name}"); }
    for name in ["notes.txt", "output_log_empty.txt", "owned.polaris_tmp"] { assert!(!root.join("Data/archive").join(name).exists(), "対象外または未完了fileを公開してはいけません: {name}"); }
    assert!(fs::read_dir(root.join("Data/archive")).expect("専用archiveを列挙できません").all(|entry| entry.expect("archive entryを読めません").path().extension().is_none_or(|extension| extension != "polaris_tmp")));
}

// profile入力だけを専用領域へ向け、実mainのlogger・起動時backup・監視継続を通す。
fn observe_startup_idle_main(scope: &StartupChildScope, message: &str) {
    let profile = scope.root.join("profile").canonicalize().expect("専用profileを解決できません"); assert_eq!(profile.parent(), Some(scope.root.as_path()));
    crate::dirs::set_test_profile(Some(profile.clone())); assert_eq!(dirs::home_dir(), Some(profile)); assert_eq!(find_vrchat_pid(), None);
    let main = std::thread::spawn(super::main); let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !STARTUP_IDLE_ARCHIVES.iter().all(|(name, bytes)| fs::read(scope.root.join("Data/archive").join(name)).is_ok_and(|actual| actual == *bytes)) || !fs::read_to_string(scope.root.join("Data/logs/info.log")).is_ok_and(|text| text.contains(message)) {
        assert!(!main.is_finished(), "起動時backupの完了前に本体mainが終了しました"); assert_eq!(find_vrchat_pid(), None);
        assert!(std::time::Instant::now() < deadline, "本体mainの起動時backupを確認できません"); tracing::error!("{message}"); std::thread::sleep(Duration::from_millis(20));
    }
    assert_startup_idle_data(&scope.root); fs::write(scope.root.join("startup.ready.txt"), std::process::id().to_string()).expect("起動時保存の完了を記録できません");
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !scope.root.join("startup.stop.txt").is_file() {
        assert!(!main.is_finished(), "起動時backup後に監視が終了しました"); assert_eq!(find_vrchat_pid(), None); assert_startup_idle_data(&scope.root);
        assert!(std::time::Instant::now() < deadline, "親processの観測が終了しません"); std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(fs::read(scope.root.join("startup.stop.txt")).expect("終了markerを読めません"), b"stop"); assert!(!main.is_finished()); assert_startup_idle_data(&scope.root);
    fs::write(scope.root.join("startup.completed.txt"), b"completed").expect("起動時保存の正常完了を記録できません");
}

#[test]
fn native_startup_main_backs_up_owned_logs_when_vrchat_is_absent() {
    if let Some(scope) = startup_child_scope("main-idle") { observe_startup_idle_main(&scope, "未起動時の保存検証ログ"); return; }
    let _process_lock = lock_native_process_tests(); assert_owned_vrchat_processes(&[]); assert_startup_mutex_available();
    let input = startup_input_desktop_name(); let directory = TestDir::new(); let desktop = StartupDesktop::new(&directory.0); let registry = StartupRegistry::new(&directory.0);
    let (settings, _) = registry.isolated.create_subkey("Software\\CosmoArtsStore\\Polaris").expect("専用install設定を作成できません");
    settings.set_value("InstallLocation", &directory.0.to_str().expect("専用install先を読めません")).expect("専用install先を保存できません"); let previous = settings.get_raw_value("InstallLocation").expect("専用設定を読めません");
    prepare_startup_idle_data(&directory.0);
    let child = StartupProcess::start("tests::native_startup_main_backs_up_owned_logs_when_vrchat_is_absent", "main-idle", &directory.0, &desktop); child.wait_before(&directory.0);
    let deadline = std::time::Instant::now() + Duration::from_secs(15); let ready = directory.path("startup.ready.txt");
    while !ready.is_file() { assert!(child.running(), "本体が起動時保存前に終了しました: {}", child.diagnostic()); assert!(std::time::Instant::now() < deadline, "起動時保存が完了しません: {}", child.diagnostic()); std::thread::sleep(Duration::from_millis(20)); }
    assert_eq!(fs::read_to_string(ready).expect("起動時保存markerを読めません"), child.pid.to_string()); let observing = std::time::Instant::now(); std::thread::sleep(Duration::from_millis(2_200)); let observation_ms = observing.elapsed().as_millis();
    assert!(child.running()); assert!(desktop.windows().is_empty()); assert_owned_vrchat_processes(&[]); assert_eq!(find_vrchat_pid(), None); assert_startup_idle_data(&directory.0);
    assert!(fs::read_to_string(directory.path("Data/logs/info.log")).expect("本体loggerの出力を読めません").contains("未起動時の保存検証ログ")); assert_eq!(settings.get_raw_value("InstallLocation").expect("起動後の設定を読めません"), previous);
    fs::write(directory.path("startup.stop.txt"), b"stop").expect("観測終了を通知できません"); assert_eq!(child.finish(), 0); assert!(child.diagnostic().contains("test result: ok. 1 passed; 0 failed;"));
    assert_eq!(fs::read(directory.path("startup.completed.txt")).expect("正常完了markerがありません"), b"completed"); assert_startup_idle_data(&directory.0); assert!(desktop.windows().is_empty());
    drop(child); assert_startup_mutex_available(); let registry_path = registry.path.clone(); drop(settings); drop(registry); assert!(RegKey::predef(HKEY_CURRENT_USER).open_subkey(registry_path).is_err()); desktop.handle.close(); assert_eq!(startup_input_desktop_name(), input);
    write_native_receipt("POLARIS_STARTUP_IDLE_TEST_RECEIPT_PATH", "polaris-native-startup-idle-evidence.json",
        &format!("{{\"main_called\":true,\"observation_ms\":{observation_ms},\"profile_provider_substituted\":true,\"actual_windows_profile_query\":false,\"vrchat_absent\":true,\"logger_initialized\":true,\"new_archive_complete\":true,\"changed_archive_complete\":true,\"same_size_archive_preserved\":true,\"unrelated_archive_preserved\":true,\"source_bytes_preserved\":true,\"ignored_files_not_archived\":true,\"temporary_removed\":true,\"no_dialog\":true,\"test_child_completed\":true,\"registry_value_preserved\":true,\"owned_key_removed\":true,\"mutex_released\":true,\"input_desktop_unchanged\":true}}"));
}

#[test]
fn native_startup_main_reports_profile_errors_and_exits_after_ok() {
    if let Some(scope) = startup_child_scope("main-profile-error") {
        let case = fs::read_to_string(scope.root.join("startup.case.txt")).expect("profile不備の条件を読めません");
        let profile = match case.as_str() { "unavailable" => None, "missing-log-directory" => Some(scope.root.join("profile").canonicalize().expect("専用profileを解決できません")), _ => unreachable!("profile不備の条件が不正です") };
        if let Some(path) = &profile { assert_eq!(path.parent(), Some(scope.root.as_path())); } crate::dirs::set_test_profile(profile);
        super::main(); fs::write(scope.root.join("startup.returned.txt"), b"returned").expect("本体returnを記録できません"); return;
    }
    let _process_lock = lock_native_process_tests(); let input = startup_input_desktop_name();
    for (case, expected) in [("unavailable", "VRChatのログ排出先を特定できないため終了します。"), ("missing-log-directory", "VRChatのログ排出先が見つからないため終了します。")] {
        assert_startup_mutex_available(); let directory = TestDir::new(); let desktop = StartupDesktop::new(&directory.0); let registry = StartupRegistry::new(&directory.0);
        let (settings, _) = registry.isolated.create_subkey("Software\\CosmoArtsStore\\Polaris").expect("専用install設定を作成できません"); settings.set_value("InstallLocation", &directory.0.to_str().expect("専用install先を読めません")).expect("専用install先を保存できません");
        let previous = settings.get_raw_value("InstallLocation").expect("専用設定を読めません"); fs::write(directory.path("startup.case.txt"), case).expect("profile不備の条件を保存できません");
        if case == "missing-log-directory" { fs::create_dir(directory.path("profile")).expect("log先のない専用profileを作成できません"); }
        let child = StartupProcess::start("tests::native_startup_main_reports_profile_errors_and_exits_after_ok", "main-profile-error", &directory.0, &desktop); child.wait_before(&directory.0);
        let button = desktop.wait_for_message(&child, expected); assert!(child.running()); assert!(!directory.path("startup.returned.txt").exists()); assert!(!directory.path("Data").exists());
        unsafe { startup_window::PostMessageW(button, startup_window::BM_CLICK, WPARAM(0), LPARAM(0)) }.expect("専用dialogのOKを操作できません"); assert_eq!(child.finish(), 0);
        assert!(!directory.path("startup.returned.txt").exists()); assert!(!directory.path("Data").exists()); assert_eq!(settings.get_raw_value("InstallLocation").expect("起動後の設定を読めません"), previous); assert!(desktop.windows().is_empty());
        drop(child); assert_startup_mutex_available(); let registry_path = registry.path.clone(); drop(settings); drop(registry); assert!(RegKey::predef(HKEY_CURRENT_USER).open_subkey(registry_path).is_err()); desktop.handle.close(); assert_eq!(startup_input_desktop_name(), input);
    }
    write_native_receipt("POLARIS_STARTUP_PROFILE_TEST_RECEIPT_PATH", "polaris-native-startup-profile-evidence.json", "{\"main_called\":true,\"profile_error_cases\":2,\"profile_provider_substituted\":true,\"actual_windows_profile_query\":false,\"full_text_verified\":true,\"ok_then_exit_code\":0,\"data_created\":false,\"registry_values_preserved\":true,\"owned_keys_removed\":true,\"mutex_released\":true,\"input_desktop_unchanged\":true}");
}

fn startup_file_snapshot(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut files = std::collections::BTreeMap::new(); let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory).expect("専用fileを列挙できません") {
            let entry = entry.expect("専用fileのentryを読めません"); let path = entry.path(); let kind = entry.file_type().expect("専用fileの種別を読めません");
            if kind.is_dir() { directories.push(path); } else { assert!(kind.is_file(), "専用fixtureに通常file以外を含めてはいけません"); files.insert(path.strip_prefix(root).expect("専用領域の外へ出てはいけません").to_path_buf(), fs::read(path).expect("専用fileの全byteを読めません")); }
        }
    }
    files
}

const STARTUP_RECOVERY_LOG: &[u8] = "保存済みの本体ログ\n".as_bytes();

#[test]
fn native_startup_main_reports_logger_failure_and_recovers_with_same_data() {
    if let Ok(mode) = std::env::var("POLARIS_STARTUP_TEST_MODE") {
        assert!(matches!(mode.as_str(), "main-logger-error" | "main-logger-retry")); let scope = startup_child_scope(&mode).expect("専用startup scopeがありません");
        let installed: String = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\CosmoArtsStore\\Polaris").expect("専用install設定を開けません").get_value("InstallLocation").expect("専用install先を読めません");
        let data = PathBuf::from(installed).join("Data").canonicalize().expect("専用Dataを解決できません"); assert_eq!(data, scope.root.join("Data").canonicalize().expect("所有Dataを解決できません"));
        fs::write(scope.root.join("startup.data-context.txt"), data.to_str().expect("Dataのpathを読めません")).expect("実install設定のDataを記録できません");
        if mode == "main-logger-error" {
            let profile = scope.root.join("profile").canonicalize().expect("専用profileを解決できません"); assert_eq!(profile.parent(), Some(scope.root.as_path())); crate::dirs::set_test_profile(Some(profile));
            // 本体のerror文をoracleにせず、実ライブラリの二重登録理由だけを独立取得する。
            tracing_subscriber::fmt().with_writer(std::io::sink).try_init().expect("専用childのloggerを先に登録できません");
            let reason = tracing_subscriber::fmt().with_writer(std::io::sink).try_init().expect_err("二重logger登録を成功としてはいけません");
            fs::write(scope.root.join("startup.expected-error.txt"), format!("ロガーを初期化できませんでした: {reason}")).expect("独立した表示期待値を保存できません");
            super::main(); fs::write(scope.root.join("startup.returned.txt"), b"returned").expect("本体returnを記録できません"); return;
        }
        observe_startup_idle_main(&scope, "復旧後の起動検証ログ"); return;
    }
    let _process_lock = lock_native_process_tests(); assert_owned_vrchat_processes(&[]); assert_startup_mutex_available();
    let input = startup_input_desktop_name(); let directory = TestDir::new(); let desktop = StartupDesktop::new(&directory.0); let registry = StartupRegistry::new(&directory.0);
    let (settings, _) = registry.isolated.create_subkey("Software\\CosmoArtsStore\\Polaris").expect("専用install設定を作成できません"); settings.set_value("InstallLocation", &directory.0.to_str().expect("専用install先を読めません")).expect("専用install先を保存できません"); let previous = settings.get_raw_value("InstallLocation").expect("専用設定を読めません");
    prepare_startup_idle_data(&directory.0); fs::create_dir(directory.path("Data/logs")).expect("保存済みlogger directoryを作成できません"); fs::write(directory.path("Data/logs/info.log"), STARTUP_RECOVERY_LOG).expect("保存済みの本体logを用意できません");
    let data_before = startup_file_snapshot(&directory.path("Data")); let source_before = startup_file_snapshot(&directory.path("profile")); assert_eq!(data_before.len(), 5); assert_eq!(source_before.len(), 5);
    let test = "tests::native_startup_main_reports_logger_failure_and_recovers_with_same_data";
    let failed = StartupProcess::start(test, "main-logger-error", &directory.0, &desktop); failed.wait_before(&directory.0); let expected_path = directory.path("startup.expected-error.txt"); let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !expected_path.is_file() { assert!(failed.running(), "logger不備の表示前に本体が終了しました: {}", failed.diagnostic()); assert!(std::time::Instant::now() < deadline, "logger不備の独立期待値を取得できません"); std::thread::sleep(Duration::from_millis(20)); }
    let expected = fs::read_to_string(expected_path).expect("logger不備の期待値を読めません"); assert!(expected.starts_with("ロガーを初期化できませんでした: ")); let button = desktop.wait_for_message(&failed, &expected);
    assert!(failed.running()); assert!(!directory.path("startup.returned.txt").exists()); assert_eq!(startup_file_snapshot(&directory.path("Data")), data_before); assert_eq!(startup_file_snapshot(&directory.path("profile")), source_before); assert_eq!(settings.get_raw_value("InstallLocation").expect("失敗中の設定を読めません"), previous);
    unsafe { startup_window::PostMessageW(button, startup_window::BM_CLICK, WPARAM(0), LPARAM(0)) }.expect("専用dialogのOKを操作できません"); assert_eq!(failed.finish(), 0);
    assert!(!directory.path("startup.returned.txt").exists()); assert!(desktop.windows().is_empty()); assert_eq!(startup_file_snapshot(&directory.path("Data")), data_before); assert_eq!(startup_file_snapshot(&directory.path("profile")), source_before); assert_eq!(settings.get_raw_value("InstallLocation").expect("失敗後の設定を読めません"), previous);
    let failed_data = fs::read_to_string(directory.path("startup.data-context.txt")).expect("失敗mainのData pathを読めません"); drop(failed); assert_startup_mutex_available();
    // 同じDataを保持し、終了済みchildの記録だけを退避して次の開始markerを区別する。
    for name in ["startup.child.log", "startup.before.txt", "startup.expected-error.txt", "startup.data-context.txt"] { let destination = directory.path(&format!("logger-error.{name}")); assert!(!destination.exists()); fs::rename(directory.path(name), destination).expect("失敗childの記録を退避できません"); }
    let recovered = StartupProcess::start(test, "main-logger-retry", &directory.0, &desktop); recovered.wait_before(&directory.0); let ready = directory.path("startup.ready.txt"); let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !ready.is_file() { assert!(recovered.running(), "再起動mainが保存完了前に終了しました: {}", recovered.diagnostic()); assert!(std::time::Instant::now() < deadline, "同じDataでの再起動を確認できません: {}", recovered.diagnostic()); std::thread::sleep(Duration::from_millis(20)); }
    assert_eq!(fs::read_to_string(ready).expect("再起動markerを読めません"), recovered.pid.to_string()); let observing = std::time::Instant::now(); std::thread::sleep(Duration::from_millis(2_200)); let observation_ms = observing.elapsed().as_millis();
    let recovered_data = fs::read_to_string(directory.path("startup.data-context.txt")).expect("再起動mainのData pathを読めません"); assert_eq!(recovered_data, failed_data); assert_eq!(PathBuf::from(recovered_data), directory.path("Data").canonicalize().expect("専用Dataを解決できません"));
    assert!(recovered.running()); assert!(desktop.windows().is_empty()); assert_owned_vrchat_processes(&[]); assert_eq!(find_vrchat_pid(), None); assert_startup_idle_data(&directory.0); assert_eq!(startup_file_snapshot(&directory.path("profile")), source_before);
    let log = fs::read(directory.path("Data/logs/info.log")).expect("復旧後の本体logを読めません"); assert!(log.starts_with(STARTUP_RECOVERY_LOG)); assert!(String::from_utf8(log).expect("本体logはUTF-8です").contains("復旧後の起動検証ログ")); assert_eq!(settings.get_raw_value("InstallLocation").expect("復旧後の設定を読めません"), previous);
    fs::write(directory.path("startup.stop.txt"), b"stop").expect("復旧観測の終了を通知できません"); assert_eq!(recovered.finish(), 0); assert!(recovered.diagnostic().contains("test result: ok. 1 passed; 0 failed;")); assert_eq!(fs::read(directory.path("startup.completed.txt")).expect("復旧の正常完了markerがありません"), b"completed"); assert_startup_idle_data(&directory.0); assert_eq!(startup_file_snapshot(&directory.path("profile")), source_before); assert!(desktop.windows().is_empty());
    drop(recovered); assert_startup_mutex_available(); let registry_path = registry.path.clone(); drop(settings); drop(registry); assert!(RegKey::predef(HKEY_CURRENT_USER).open_subkey(registry_path).is_err()); desktop.handle.close(); assert_eq!(startup_input_desktop_name(), input);
    write_native_receipt("POLARIS_STARTUP_LOGGER_TEST_RECEIPT_PATH", "polaris-native-startup-logger-evidence.json",
        &format!("{{\"main_invocations\":2,\"same_install_location\":true,\"same_data_directory\":true,\"profile_provider_substituted\":true,\"actual_windows_profile_query\":false,\"failure_full_text_verified\":true,\"failure_blocked_before_ok\":true,\"failure_exit_code\":0,\"data_files_compared\":5,\"source_files_compared\":5,\"data_bytes_preserved_after_failure\":true,\"source_bytes_preserved_after_failure\":true,\"new_process_recovered\":true,\"recovered_archive_bytes_verified\":true,\"source_bytes_preserved_after_recovery\":true,\"log_prefix_preserved\":true,\"logger_written_after_restart\":true,\"observation_ms\":{observation_ms},\"registry_value_preserved\":true,\"owned_key_removed\":true,\"mutex_released\":true,\"input_desktop_unchanged\":true}}"));
}
