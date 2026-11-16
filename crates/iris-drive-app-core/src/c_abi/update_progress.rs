//! Update install steps for the native shells. The core words every step; a
//! shell reports the steps only it can take and renders the snapshot.

use std::ffi::c_char;
use std::path::Path;

use iris_drive_core::updater::progress::{UpdateStage, report_update_stage, update_progress};

use super::{c_string_lossy, json_string};

/// The current install snapshot for a profile: stage, text, fraction, retry.
#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_update_progress_json(data_dir: *const c_char) -> *mut c_char {
    let dir = c_string_lossy(data_dir);
    json_string(&update_progress(Path::new(&dir)))
}

/// Record a step the shell took (`installing`, `restarting`,
/// `opening_installer`, `installer_opened`, `failed` with a reason, `idle`)
/// and return the snapshot to render.
#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_update_report_stage_json(
    data_dir: *const c_char,
    stage: *const c_char,
    reason: *const c_char,
) -> *mut c_char {
    let dir = c_string_lossy(data_dir);
    let stage_name = c_string_lossy(stage);
    let Some(stage) = UpdateStage::parse(&stage_name) else {
        return json_string(
            &serde_json::json!({ "error": format!("unknown update stage {stage_name}") }),
        );
    };
    json_string(&report_update_stage(
        Path::new(&dir),
        stage,
        &c_string_lossy(reason),
    ))
}

#[cfg(test)]
mod tests {
    use std::ffi::{CStr, CString};

    use super::*;

    fn call(pointer: *mut c_char) -> serde_json::Value {
        let text = unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned();
        unsafe { drop(CString::from_raw(pointer)) };
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn shells_report_steps_and_read_the_worded_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let dir = CString::new(directory.path().to_str().unwrap()).unwrap();
        let empty = CString::new("").unwrap();
        let idle = call(iris_drive_update_progress_json(dir.as_ptr()));
        assert_eq!(
            (idle["stage"].as_str(), idle["active"].as_bool()),
            (Some("idle"), Some(false))
        );
        let installing = CString::new("installing").unwrap();
        let reported = call(iris_drive_update_report_stage_json(
            dir.as_ptr(),
            installing.as_ptr(),
            empty.as_ptr(),
        ));
        assert_eq!(reported["text"], "Installing the update");
        let failed = CString::new("failed").unwrap();
        let reason = CString::new("disk full").unwrap();
        let reported = call(iris_drive_update_report_stage_json(
            dir.as_ptr(),
            failed.as_ptr(),
            reason.as_ptr(),
        ));
        assert_eq!(reported["text"], "Couldn't install the update: disk full");
        assert_eq!(reported["can_retry"], true);
        assert_eq!(
            call(iris_drive_update_progress_json(dir.as_ptr())),
            reported
        );
        let bogus = CString::new("bogus").unwrap();
        let error = call(iris_drive_update_report_stage_json(
            dir.as_ptr(),
            bogus.as_ptr(),
            empty.as_ptr(),
        ));
        assert!(error["error"].as_str().unwrap().contains("bogus"));
    }
}
