//! BusLink Studio Tauri shell: IPC over the core session service.
//! Monitoring only in this release of the shell (safety rules 1–2).

mod commands;
mod serial;
mod state;

use std::sync::atomic::Ordering;
use tauri::Manager;

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let st = state::AppState::new(app.handle())?;
            state::spawn_emitter(app.handle().clone(), st.session.clone(), st.stop.clone());
            app.manage(st);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::list_ports,
            commands::list_profiles,
            commands::connect_link,
            commands::reconnect_link,
            commands::disconnect_link,
            commands::remove_link,
            commands::start_polling,
            commands::stop_polling,
            commands::add_device,
            commands::remove_device,
            commands::live_snapshot,
            commands::trend_query,
            commands::historian_sessions,
            commands::export_session,
            commands::inspect_device,
            commands::alarm_list,
            commands::alarm_add,
            commands::alarm_remove,
            commands::alarm_ack,
            commands::alarm_shelve,
            commands::alarm_unshelve,
            commands::alarm_set_oos,
            commands::alarm_events,
            commands::alarm_defaults,
            commands::training_status,
            commands::training_start,
            commands::training_fault,
            commands::training_clear,
        ])
        .build(tauri::generate_context!());
    let app = match app {
        Ok(a) => a,
        Err(e) => {
            eprintln!("BusLink Studio failed to start: {e}");
            std::process::exit(1);
        }
    };
    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event
            && let Some(st) = handle.try_state::<state::AppState>()
        {
            st.stop.store(true, Ordering::SeqCst);
            st.session.shutdown();
        }
    });
}
