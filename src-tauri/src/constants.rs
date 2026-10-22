//! アプリ共通の識別子と、既存の保存先・外部ログ形式に関する契約値。

pub const APP_NAME: &str = "Polaris";
pub const REGISTRY_KEY_PATH: &str = "Software\\CosmoArtsStore\\Polaris";
pub const INSTALL_LOCATION_VALUE: &str = "InstallLocation";
pub const DATA_DIRECTORY: &str = "Data";
pub const ARCHIVE_DIRECTORY: &str = "archive";
pub const LOG_DIRECTORY: &str = "logs";
pub const LOG_FILE: &str = "info.log";
pub const VRCHAT_LOG_RELATIVE_PATH: [&str; 4] = ["AppData", "LocalLow", "VRChat", "VRChat"];
pub const VRCHAT_PROCESS_NAME: &str = "vrchat.exe";
pub const VRCHAT_LOG_PREFIX: &str = "output_log_";
pub const VRCHAT_LOG_EXTENSION: &str = "txt";
