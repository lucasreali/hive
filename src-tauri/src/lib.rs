// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> tauri::Result<()> {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greet_includes_name() {
        assert_eq!(greet("Ada"), "Hello, Ada! You've been greeted from Rust!");
    }
}
