// src-tauri/src/main.rs
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use enigo::{Enigo, KeyboardControllable};
use tauri::{Manager, SystemTray, SystemTrayEvent, CustomMenuItem, SystemTrayMenu};
use global_hotkey::{GlobalHotKeyManager, hotkey::{HotKey, Modifiers, Code}};
use std::sync::Mutex;

struct AppState {
    is_recording: Mutex<bool>,
}

#[tauri::command]
fn inject_text(text: String) {
    let mut enigo = Enigo::new();
    enigo.key_sequence(&text);
}

// Simple Toggle Command (in case Frontend wants to trigger)
#[tauri::command]
fn toggle_recording(app_handle: tauri::AppHandle) {
    // Determine state and emit event to Frontend to start/stop WebSocket
    app_handle.emit_all("toggle-recording", ()).unwrap();
}

fn main() {
    let tray_menu = SystemTrayMenu::new()
        .add_item(CustomMenuItem::new("quit".to_string(), "Quit"));
    
    let tray = SystemTray::new().with_menu(tray_menu);

    tauri::Builder::default()
        .system_tray(tray)
        .on_system_tray_event(|app, event| match event {
            SystemTrayEvent::MenuItemClick { id, .. } => {
                match id.as_str() {
                    "quit" => {
                        std::process::exit(0);
                    }
                    _ => {}
                }
            }
            _ => {}
        })
        .manage(AppState { is_recording: Mutex::new(false) })
        .invoke_handler(tauri::generate_handler![inject_text, toggle_recording])
        .setup(|app| {
             // Register Global Hotkey (F2 or Cmd+Shift+Space)
             // NOTE: For MVP we just use the Frontend listener via Enigo/Shortcuts, 
             // but Rust GlobalHotKeyManager would be cleaner here.
             
             let main_window = app.get_window("main").unwrap();
             // Set window to be transparent and always on top
             main_window.set_always_on_top(true).unwrap();
             
             Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
