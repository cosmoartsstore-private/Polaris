use std::collections::HashSet;
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::PathBuf;

/// 文言resourceをliteralへ展開し、キーとformat引数の不整合をコンパイル時に検出する。
pub fn compile(catalog_path: &str) -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed={catalog_path}");
    println!("cargo:rerun-if-changed=build/messages.rs");
    let source = fs::read_to_string(catalog_path)?;
    let mut keys = HashSet::new();
    let mut generated = String::from("macro_rules! message {\n");
    for (index, raw_line) in source.lines().enumerate() {
        let line = raw_line.trim_start();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(|| invalid_line(catalog_path, index, "区切りがありません"))?;
        let key = key.trim();
        if key.is_empty() || !key.bytes().all(|character| character.is_ascii_alphanumeric() || matches!(character, b'.' | b'_')) {
            return Err(invalid_line(catalog_path, index, "キーが不正です").into());
        }
        if !keys.insert(key) {
            return Err(invalid_line(catalog_path, index, "キーが重複しています").into());
        }
        let value = unescape(value);
        writeln!(generated, "    ({key:?}) => {{ {value:?} }};")?;
    }
    generated.push_str("}\n");
    let output = PathBuf::from(std::env::var("OUT_DIR")?).join("messages.rs");
    fs::write(output, generated)?;
    Ok(())
}

fn invalid_line(path: &str, index: usize, reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{path}:{} {reason}", index + 1))
}

fn unescape(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '\\' {
            result.push(character);
            continue;
        }
        match characters.peek() {
            Some('n') => {
                characters.next();
                result.push('\n');
            }
            Some('r') => {
                characters.next();
                result.push('\r');
            }
            Some('t') => {
                characters.next();
                result.push('\t');
            }
            Some('\\') => {
                characters.next();
                result.push('\\');
            }
            Some('=' | ':') => {
                if let Some(escaped) = characters.next() {
                    result.push(escaped);
                }
            }
            _ => result.push('\\'),
        }
    }
    result
}
