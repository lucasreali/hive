//! Tauri commands. `main.rs` only wires them; everything testable lives here.

pub mod commands {
    // Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
    #[tauri::command]
    pub fn greet(name: &str) -> String {
        format!("Hello, {}! You've been greeted from Rust!", name)
    }
}

#[cfg(test)]
mod tests {
    use super::commands::*;

    #[test]
    fn greet_includes_name() {
        assert_eq!(greet("Ada"), "Hello, Ada! You've been greeted from Rust!");
    }
}
