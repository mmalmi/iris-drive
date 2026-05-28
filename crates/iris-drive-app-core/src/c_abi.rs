use std::ffi::{CStr, CString, c_char};
use std::ptr;
use std::sync::Arc;

#[cfg(target_os = "android")]
use jni::JNIEnv;
#[cfg(target_os = "android")]
use jni::objects::{JClass, JObject, JString};
#[cfg(target_os = "android")]
use jni::sys::{jlong, jstring};
use serde::Serialize;

use crate::{FfiApp, NativeAppAction, NativeAppState};

pub struct IrisDriveAppHandle {
    app: Arc<FfiApp>,
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_app_new(
    data_dir: *const c_char,
    app_version: *const c_char,
) -> *mut IrisDriveAppHandle {
    let data_dir = c_string_lossy(data_dir);
    let app_version = c_string_lossy(app_version);
    Box::into_raw(Box::new(IrisDriveAppHandle {
        app: FfiApp::new(data_dir, app_version),
    }))
}

/// # Safety
///
/// `handle` must be null or a pointer returned by `iris_drive_app_new` that has not already been
/// freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iris_drive_app_free(handle: *mut IrisDriveAppHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_app_state_json(handle: *const IrisDriveAppHandle) -> *mut c_char {
    let state = app_from_handle(handle).map_or_else(
        |error| error_state(error.to_string()),
        |handle| handle.app.state(),
    );
    json_string(&state)
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_app_refresh_json(handle: *const IrisDriveAppHandle) -> *mut c_char {
    let state = app_from_handle(handle).map_or_else(
        |error| error_state(error.to_string()),
        |handle| handle.app.refresh(),
    );
    json_string(&state)
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_app_dispatch_json(
    handle: *const IrisDriveAppHandle,
    action_json: *const c_char,
) -> *mut c_char {
    let state = app_from_handle(handle).map_or_else(
        |error| error_state(error.to_string()),
        |handle| {
            let action_json = c_string_lossy(action_json);
            match serde_json::from_str::<NativeAppAction>(&action_json) {
                Ok(action) => handle.app.dispatch(action),
                Err(error) => {
                    let mut state = handle.app.state();
                    state.error = format!("invalid native action JSON: {error}");
                    state
                }
            }
        },
    );
    json_string(&state)
}

/// # Safety
///
/// `value` must be null or a pointer returned by an Iris Drive C ABI function that has not
/// already been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iris_drive_string_free(value: *mut c_char) {
    if value.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(value));
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_initializeAndroidContext(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    _context: JObject<'_>,
) {
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_appNew(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    data_dir: JString<'_>,
    app_version: JString<'_>,
) -> jlong {
    let data_dir = jni_string_lossy(&mut env, &data_dir);
    let app_version = jni_string_lossy(&mut env, &app_version);
    Box::into_raw(Box::new(IrisDriveAppHandle {
        app: FfiApp::new(data_dir, app_version),
    })) as jlong
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_appFree(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle == 0 {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle as *mut IrisDriveAppHandle));
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_stateJson(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jstring {
    jni_state_json(env, handle, |handle| handle.app.state())
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_refreshJson(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jstring {
    jni_state_json(env, handle, |handle| handle.app.refresh())
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_dispatchJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
    action_json: JString<'_>,
) -> jstring {
    let state = app_from_jlong(handle).map_or_else(
        |error| error_state(error.to_string()),
        |handle| {
            let action_json = jni_string_lossy(&mut env, &action_json);
            match serde_json::from_str::<NativeAppAction>(&action_json) {
                Ok(action) => handle.app.dispatch(action),
                Err(error) => {
                    let mut state = handle.app.state();
                    state.error = format!("invalid native action JSON: {error}");
                    state
                }
            }
        },
    );
    jni_json_string(env, &state)
}

fn app_from_handle(
    handle: *const IrisDriveAppHandle,
) -> Result<&'static IrisDriveAppHandle, &'static str> {
    if handle.is_null() {
        Err("native app handle is null")
    } else {
        Ok(unsafe { &*handle })
    }
}

#[cfg(target_os = "android")]
fn app_from_jlong(handle: jlong) -> Result<&'static IrisDriveAppHandle, &'static str> {
    if handle == 0 {
        Err("native app handle is null")
    } else {
        Ok(unsafe { &*(handle as *const IrisDriveAppHandle) })
    }
}

fn error_state(message: String) -> NativeAppState {
    NativeAppState {
        error: message,
        ..NativeAppState::default()
    }
}

fn c_string_lossy(value: *const c_char) -> String {
    if value.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(target_os = "android")]
fn jni_state_json(
    env: JNIEnv<'_>,
    handle: jlong,
    state: impl FnOnce(&IrisDriveAppHandle) -> NativeAppState,
) -> jstring {
    let state = app_from_jlong(handle).map_or_else(
        |error| error_state(error.to_string()),
        |handle| state(handle),
    );
    jni_json_string(env, &state)
}

#[cfg(target_os = "android")]
fn jni_string_lossy(env: &mut JNIEnv<'_>, value: &JString<'_>) -> String {
    env.get_string(value).map_or_else(
        |_| String::new(),
        |value| value.to_string_lossy().into_owned(),
    )
}

#[cfg(target_os = "android")]
fn jni_json_string(env: JNIEnv<'_>, value: &impl Serialize) -> jstring {
    let json =
        serde_json::to_string(value).unwrap_or_else(|error| format!(r#"{{"error":"{error}"}}"#));
    jni_raw_string(env, json)
}

#[cfg(target_os = "android")]
fn jni_raw_string(env: JNIEnv<'_>, value: String) -> jstring {
    env.new_string(value)
        .map_or(ptr::null_mut(), |value| value.into_raw())
}

fn json_string(value: &impl Serialize) -> *mut c_char {
    match serde_json::to_string(value) {
        Ok(json) => into_c_string(&json),
        Err(error) => into_c_string(&format!(r#"{{"error":"{error}"}}"#)),
    }
}

fn into_c_string(value: &str) -> *mut c_char {
    match CString::new(value.replace('\0', "")) {
        Ok(value) => value.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::{CStr, CString};

    use serde_json::Value;

    use super::{
        iris_drive_app_dispatch_json, iris_drive_app_free, iris_drive_app_new,
        iris_drive_app_state_json, iris_drive_string_free,
    };

    #[test]
    fn c_abi_returns_json_state_and_action_errors() {
        let data_dir = CString::new("/tmp/iris-drive").expect("data dir CString");
        let app_version = CString::new("test").expect("app version CString");
        let handle = iris_drive_app_new(data_dir.as_ptr(), app_version.as_ptr());
        assert!(!handle.is_null());

        let state_json = take_string(iris_drive_app_state_json(handle));
        let state: Value = serde_json::from_str(&state_json).expect("state JSON");
        assert_eq!(state["ui"]["roots"].as_array().map(Vec::len), Some(0));

        let bad_action = CString::new(r#"{"type":"nope"}"#).expect("action CString");
        let state_json = take_string(iris_drive_app_dispatch_json(handle, bad_action.as_ptr()));
        let state: Value = serde_json::from_str(&state_json).expect("error state JSON");
        assert!(
            state["error"]
                .as_str()
                .is_some_and(|error| error.contains("invalid native action JSON"))
        );

        unsafe { iris_drive_app_free(handle) };
    }

    fn take_string(ptr: *mut std::ffi::c_char) -> String {
        assert!(!ptr.is_null());
        let value = unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        unsafe { iris_drive_string_free(ptr) };
        value
    }
}
