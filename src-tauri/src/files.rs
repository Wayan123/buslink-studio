//! Saving exports: the native dialog on Windows, the Storage Access Framework
//! (`ACTION_CREATE_DOCUMENT` through the USB plugin) on Android.
//! The webview never supplies a path (requirement 8.2).

use tauri::AppHandle;

pub struct Saved {
    pub path: Option<String>,
}

/// `Ok(None)` means the user cancelled.
#[cfg(not(target_os = "android"))]
pub fn save_bytes(app: &AppHandle, name: &str, label: &str, ext: &str, bytes: &[u8]) -> Result<Option<Saved>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .set_file_name(name)
        .add_filter(label, &[ext])
        .blocking_save_file();
    let Some(fp) = picked else {
        return Ok(None);
    };
    let path = fp.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    Ok(Some(Saved {
        path: Some(path.display().to_string()),
    }))
}

/// Allow-listed export types and their MIME types (the plugin checks the same list).
#[cfg(any(target_os = "android", test))]
fn mime_of(ext: &str) -> Option<&'static str> {
    match ext {
        "csv" => Some("text/csv"),
        "xlsx" => Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        "json" => Some("application/json"),
        "html" => Some("text/html"),
        _ => None,
    }
}

#[cfg(target_os = "android")]
pub fn save_bytes(app: &AppHandle, name: &str, _label: &str, ext: &str, bytes: &[u8]) -> Result<Option<Saved>, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use tauri_plugin_buslink_usb::{SaveRequest, UsbSerialExt};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mime = mime_of(ext).ok_or_else(|| format!(".{ext} cannot be exported"))?;
    let usb = app.usb_serial();
    let dir = usb.export_dir().map_err(|e| e.to_string())?.path;
    // Generated name: nothing from the webview reaches the file system.
    let cache_name = format!("export-{}.{ext}", COUNTER.fetch_add(1, Ordering::SeqCst));
    let cache_path = std::path::Path::new(&dir).join(&cache_name);
    std::fs::write(&cache_path, bytes).map_err(|e| format!("could not stage the export: {e}"))?;
    match usb.save_document(&SaveRequest {
        cache_file_name: cache_name,
        display_name: name.to_owned(),
        mime: mime.to_owned(),
    }) {
        Ok(o) if o.saved => Ok(Some(Saved { path: None })),
        Ok(_) => Ok(None),
        Err(e) => {
            let _ = std::fs::remove_file(&cache_path);
            Err(e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_listed_types_have_a_mime() {
        assert_eq!(mime_of("csv"), Some("text/csv"));
        assert!(mime_of("xlsx").is_some());
        assert!(mime_of("exe").is_none());
        assert!(mime_of("").is_none());
    }
}
