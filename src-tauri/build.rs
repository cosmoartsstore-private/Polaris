#[path = "build/messages.rs"]
mod messages;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    messages::compile("messages/messages.ja.properties")?;
    tauri_build::build();
    Ok(())
}
