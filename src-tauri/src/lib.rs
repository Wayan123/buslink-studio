//! BusLink Studio Tauri shell: IPC over the core session service.
//! Writes exist only through `control.rs` → `buslink_core::command` (safety rules 1–2).

mod android_usb;
mod commands;
mod control;
mod files;
mod serial;
mod share;
mod state;

use std::sync::atomic::Ordering;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_buslink_usb::init());
    // Native dialogs are desktop only; Android saves through the SAF picker in the USB plugin.
    #[cfg(not(target_os = "android"))]
    let builder = builder.plugin(tauri_plugin_dialog::init());
    let app = builder
        .setup(|app| {
            let st = state::AppState::new(app.handle())?;
            state::spawn_emitter(app.handle().clone(), st.session.clone(), st.stop.clone());
            app.manage(st);
            app.manage(share::ShareState::new());
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
            control::control_status,
            control::control_arm,
            control::control_disarm,
            control::control_preview,
            control::control_confirm,
            control::control_execute,
            control::control_cancel,
            control::wizard_support,
            control::wizard_start,
            control::wizard_preview,
            control::wizard_power_cycled,
            control::wizard_reconnect,
            control::wizard_verify,
            control::wizard_revert,
            control::wizard_close,
            control::audit_list,
            control::audit_export,
            control::report_data,
            control::report_save_html,
            share::lan_interfaces,
            share::lan_start,
            share::lan_stop,
            share::lan_status,
            share::mqtt_get,
            share::mqtt_status,
            share::mqtt_save_config,
            share::mqtt_set_password,
            share::mqtt_clear_password,
            share::mqtt_test,
            share::mqtt_start,
            share::mqtt_stop,
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
            // Sharing and MQTT never outlive the window (safety rules 15 and 16).
            if let Some(sh) = handle.try_state::<share::ShareState>() {
                sh.shutdown();
            }
            st.stop.store(true, Ordering::SeqCst);
            st.control.disarm(buslink_core::command::DisarmReason::AppBackground);
            st.session.shutdown();
            #[cfg(target_os = "android")]
            android_usb::stop_service(handle);
        }
    });
}
