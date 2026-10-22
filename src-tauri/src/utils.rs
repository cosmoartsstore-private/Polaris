use std::path::Path;

/// Win32の文字列引数に必要な終端NULを1つ付ける。
pub fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

/// 読取り可能な2ファイルが同じ非ゼロのサイズを持つか判定する。
pub fn is_same_size(first: &Path, second: &Path) -> bool {
    let first_size = std::fs::metadata(first).map(|metadata| metadata.len()).unwrap_or(0);
    let second_size = std::fs::metadata(second).map(|metadata| metadata.len()).unwrap_or(0);
    first_size == second_size && first_size > 0
}
