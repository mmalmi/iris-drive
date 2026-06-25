use std::ffi::{CStr, CString, c_char};
use std::path::Path;
use std::ptr;
use std::sync::Arc;

#[cfg(target_os = "android")]
use jni::JNIEnv;
#[cfg(target_os = "android")]
use jni::objects::{JClass, JObject, JString};
#[cfg(target_os = "android")]
use jni::sys::{jlong, jstring};
use qrcode::QrCode;
use serde::Serialize;

use crate::{
    FfiApp, NativeAppAction, NativeAppState,
    ffi::native_calendar_export_json,
    ffi::native_provider_compose_path_json,
    ffi::native_provider_delete_json,
    ffi::native_provider_import_shared_file_json,
    ffi::native_provider_is_child_document_json,
    ffi::native_provider_list_json,
    ffi::native_provider_mkdir_json,
    ffi::native_provider_normalize_path_json,
    ffi::native_provider_read_json,
    ffi::native_provider_rename_json,
    ffi::native_provider_resolve_path_json,
    ffi::native_provider_write_json,
    ffi::{
        classify_link_input, drive_link_for_cid, export_recovery_secret, generate_recovery_key,
        recovery_pubkey_for_phrase, validate_link_input,
    },
    native_provider::install_rustls_crypto_provider,
};
use iris_drive_core::updater::{
    ProductUpdateMode, ProductUpdateResult, check_product_update_blocking,
    download_product_update_blocking, product_update_config_for_dir,
};

pub struct IrisDriveAppHandle {
    app: Arc<FfiApp>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct QrMatrixResult {
    width: usize,
    cells: Vec<bool>,
    error: String,
}

#[derive(Debug, Default, Serialize)]
struct NativeUpdateResult {
    available: bool,
    current_version: String,
    latest_version: String,
    tag: String,
    asset: String,
    source: String,
    verified: bool,
    path: Option<String>,
    root_cid: Option<String>,
    release_cid: Option<String>,
    error: String,
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_install_rustls_crypto_provider() {
    install_rustls_crypto_provider();
}

impl From<ProductUpdateResult> for NativeUpdateResult {
    fn from(value: ProductUpdateResult) -> Self {
        Self {
            available: value.available,
            current_version: value.current_version,
            latest_version: value.latest_version,
            tag: value.tag,
            asset: value.asset,
            source: value.source,
            verified: value.verified,
            path: value.path,
            root_cid: value.root_cid,
            release_cid: value.release_cid,
            error: String::new(),
        }
    }
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

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_qr_matrix_json(text: *const c_char) -> *mut c_char {
    let result = qr_matrix(&c_string_lossy(text)).unwrap_or_else(|error| QrMatrixResult {
        width: 0,
        cells: Vec::new(),
        error,
    });
    json_string(&result)
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_classify_link_input_json(text: *const c_char) -> *mut c_char {
    json_string(&classify_link_input(c_string_lossy(text)))
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_validate_link_input_json(text: *const c_char) -> *mut c_char {
    json_string(&validate_link_input(c_string_lossy(text)))
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_export_recovery_secret_json(data_dir: *const c_char) -> *mut c_char {
    json_string(&export_recovery_secret(c_string_lossy(data_dir)))
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_generate_recovery_key_json() -> *mut c_char {
    json_string(&generate_recovery_key())
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_recovery_pubkey_for_phrase_json(
    recovery_phrase: *const c_char,
) -> *mut c_char {
    json_string(&recovery_pubkey_for_phrase(c_string_lossy(recovery_phrase)))
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_link_for_cid_json(root_cid: *const c_char) -> *mut c_char {
    json_string(&drive_link_for_cid(c_string_lossy(root_cid)))
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_calendar_export_json(data_dir: *const c_char) -> *mut c_char {
    json_string(&native_calendar_export_json(&c_string_lossy(data_dir)))
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_update_check_json(
    data_dir: *const c_char,
    current_version: *const c_char,
    mode: *const c_char,
) -> *mut c_char {
    let result = native_update_check(
        &c_string_lossy(data_dir),
        &c_string_lossy(current_version),
        &c_string_lossy(mode),
    )
    .unwrap_or_else(|error| NativeUpdateResult {
        error,
        ..NativeUpdateResult::default()
    });
    json_string(&result)
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_update_download_json(
    data_dir: *const c_char,
    current_version: *const c_char,
    mode: *const c_char,
    download_dir: *const c_char,
) -> *mut c_char {
    let result = native_update_download(
        &c_string_lossy(data_dir),
        &c_string_lossy(current_version),
        &c_string_lossy(mode),
        &c_string_lossy(download_dir),
    )
    .unwrap_or_else(|error| NativeUpdateResult {
        error,
        ..NativeUpdateResult::default()
    });
    json_string(&result)
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_provider_list_json(data_dir: *const c_char) -> *mut c_char {
    json_string(&native_provider_list_json(&c_string_lossy(data_dir)))
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_provider_read_json(
    data_dir: *const c_char,
    path: *const c_char,
    output_path: *const c_char,
) -> *mut c_char {
    json_string(&native_provider_read_json(
        &c_string_lossy(data_dir),
        &c_string_lossy(path),
        &c_string_lossy(output_path),
    ))
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_provider_write_json(
    data_dir: *const c_char,
    path: *const c_char,
    source_path: *const c_char,
) -> *mut c_char {
    json_string(&native_provider_write_json(
        &c_string_lossy(data_dir),
        &c_string_lossy(path),
        &c_string_lossy(source_path),
    ))
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_provider_mkdir_json(
    data_dir: *const c_char,
    path: *const c_char,
) -> *mut c_char {
    json_string(&native_provider_mkdir_json(
        &c_string_lossy(data_dir),
        &c_string_lossy(path),
    ))
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_provider_delete_json(
    data_dir: *const c_char,
    path: *const c_char,
) -> *mut c_char {
    json_string(&native_provider_delete_json(
        &c_string_lossy(data_dir),
        &c_string_lossy(path),
    ))
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_provider_rename_json(
    data_dir: *const c_char,
    old_path: *const c_char,
    new_path: *const c_char,
) -> *mut c_char {
    json_string(&native_provider_rename_json(
        &c_string_lossy(data_dir),
        &c_string_lossy(old_path),
        &c_string_lossy(new_path),
    ))
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_provider_import_shared_file_json(
    data_dir: *const c_char,
    display_name: *const c_char,
    source_path: *const c_char,
) -> *mut c_char {
    json_string(&native_provider_import_shared_file_json(
        &c_string_lossy(data_dir),
        &c_string_lossy(display_name),
        &c_string_lossy(source_path),
    ))
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_provider_resolve_path_json(
    data_dir: *const c_char,
    parent_path: *const c_char,
    display_name: *const c_char,
    excluding_path: *const c_char,
) -> *mut c_char {
    json_string(&native_provider_resolve_path_json(
        &c_string_lossy(data_dir),
        &c_string_lossy(parent_path),
        &c_string_lossy(display_name),
        &c_string_lossy(excluding_path),
    ))
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_provider_compose_path_json(
    parent_path: *const c_char,
    display_name: *const c_char,
) -> *mut c_char {
    json_string(&native_provider_compose_path_json(
        &c_string_lossy(parent_path),
        &c_string_lossy(display_name),
    ))
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_provider_normalize_path_json(path: *const c_char) -> *mut c_char {
    json_string(&native_provider_normalize_path_json(&c_string_lossy(path)))
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_provider_is_child_document_json(
    parent_path: *const c_char,
    document_path: *const c_char,
) -> *mut c_char {
    json_string(&native_provider_is_child_document_json(
        &c_string_lossy(parent_path),
        &c_string_lossy(document_path),
    ))
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

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_qrMatrixJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    text: JString<'_>,
) -> jstring {
    let text = jni_string_lossy(&mut env, &text);
    let result = qr_matrix(&text).unwrap_or_else(|error| QrMatrixResult {
        width: 0,
        cells: Vec::new(),
        error,
    });
    jni_json_string(env, &result)
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_classifyLinkInputJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    text: JString<'_>,
) -> jstring {
    let text = jni_string_lossy(&mut env, &text);
    jni_json_string(env, &classify_link_input(text))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_validateLinkInputJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    text: JString<'_>,
) -> jstring {
    let text = jni_string_lossy(&mut env, &text);
    jni_json_string(env, &validate_link_input(text))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_exportRecoverySecretJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    data_dir: JString<'_>,
) -> jstring {
    let data_dir = jni_string_lossy(&mut env, &data_dir);
    jni_json_string(env, &export_recovery_secret(data_dir))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_generateRecoveryKeyJson(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
) -> jstring {
    jni_json_string(env, &generate_recovery_key())
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_recoveryPubkeyForPhraseJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    recovery_phrase: JString<'_>,
) -> jstring {
    let recovery_phrase = jni_string_lossy(&mut env, &recovery_phrase);
    jni_json_string(env, &recovery_pubkey_for_phrase(recovery_phrase))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_exportCalendarJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    data_dir: JString<'_>,
) -> jstring {
    let data_dir = jni_string_lossy(&mut env, &data_dir);
    jni_json_string(env, &native_calendar_export_json(&data_dir))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_updateCheckJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    data_dir: JString<'_>,
    current_version: JString<'_>,
    mode: JString<'_>,
) -> jstring {
    let result = native_update_check(
        &jni_string_lossy(&mut env, &data_dir),
        &jni_string_lossy(&mut env, &current_version),
        &jni_string_lossy(&mut env, &mode),
    )
    .unwrap_or_else(|error| NativeUpdateResult {
        error,
        ..NativeUpdateResult::default()
    });
    jni_json_string(env, &result)
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_updateDownloadJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    data_dir: JString<'_>,
    current_version: JString<'_>,
    mode: JString<'_>,
    download_dir: JString<'_>,
) -> jstring {
    let result = native_update_download(
        &jni_string_lossy(&mut env, &data_dir),
        &jni_string_lossy(&mut env, &current_version),
        &jni_string_lossy(&mut env, &mode),
        &jni_string_lossy(&mut env, &download_dir),
    )
    .unwrap_or_else(|error| NativeUpdateResult {
        error,
        ..NativeUpdateResult::default()
    });
    jni_json_string(env, &result)
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_providerListJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    data_dir: JString<'_>,
) -> jstring {
    let data_dir = jni_string_lossy(&mut env, &data_dir);
    jni_json_string(env, &native_provider_list_json(&data_dir))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_providerReadJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    data_dir: JString<'_>,
    path: JString<'_>,
    output_path: JString<'_>,
) -> jstring {
    let data_dir = jni_string_lossy(&mut env, &data_dir);
    let path = jni_string_lossy(&mut env, &path);
    let output_path = jni_string_lossy(&mut env, &output_path);
    jni_json_string(
        env,
        &native_provider_read_json(&data_dir, &path, &output_path),
    )
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_providerWriteJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    data_dir: JString<'_>,
    path: JString<'_>,
    source_path: JString<'_>,
) -> jstring {
    let data_dir = jni_string_lossy(&mut env, &data_dir);
    let path = jni_string_lossy(&mut env, &path);
    let source_path = jni_string_lossy(&mut env, &source_path);
    jni_json_string(
        env,
        &native_provider_write_json(&data_dir, &path, &source_path),
    )
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_providerMkdirJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    data_dir: JString<'_>,
    path: JString<'_>,
) -> jstring {
    let data_dir = jni_string_lossy(&mut env, &data_dir);
    let path = jni_string_lossy(&mut env, &path);
    jni_json_string(env, &native_provider_mkdir_json(&data_dir, &path))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_providerDeleteJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    data_dir: JString<'_>,
    path: JString<'_>,
) -> jstring {
    let data_dir = jni_string_lossy(&mut env, &data_dir);
    let path = jni_string_lossy(&mut env, &path);
    jni_json_string(env, &native_provider_delete_json(&data_dir, &path))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_providerRenameJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    data_dir: JString<'_>,
    old_path: JString<'_>,
    new_path: JString<'_>,
) -> jstring {
    let data_dir = jni_string_lossy(&mut env, &data_dir);
    let old_path = jni_string_lossy(&mut env, &old_path);
    let new_path = jni_string_lossy(&mut env, &new_path);
    jni_json_string(
        env,
        &native_provider_rename_json(&data_dir, &old_path, &new_path),
    )
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_providerImportSharedFileJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    data_dir: JString<'_>,
    display_name: JString<'_>,
    source_path: JString<'_>,
) -> jstring {
    let data_dir = jni_string_lossy(&mut env, &data_dir);
    let display_name = jni_string_lossy(&mut env, &display_name);
    let source_path = jni_string_lossy(&mut env, &source_path);
    jni_json_string(
        env,
        &native_provider_import_shared_file_json(&data_dir, &display_name, &source_path),
    )
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_providerResolvePathJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    data_dir: JString<'_>,
    parent_path: JString<'_>,
    display_name: JString<'_>,
    excluding_path: JString<'_>,
) -> jstring {
    let data_dir = jni_string_lossy(&mut env, &data_dir);
    let parent_path = jni_string_lossy(&mut env, &parent_path);
    let display_name = jni_string_lossy(&mut env, &display_name);
    let excluding_path = jni_string_lossy(&mut env, &excluding_path);
    jni_json_string(
        env,
        &native_provider_resolve_path_json(&data_dir, &parent_path, &display_name, &excluding_path),
    )
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_providerNormalizePathJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    path: JString<'_>,
) -> jstring {
    let path = jni_string_lossy(&mut env, &path);
    jni_json_string(env, &native_provider_normalize_path_json(&path))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_providerIsChildDocumentJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    parent_path: JString<'_>,
    document_path: JString<'_>,
) -> jstring {
    let parent_path = jni_string_lossy(&mut env, &parent_path);
    let document_path = jni_string_lossy(&mut env, &document_path);
    jni_json_string(
        env,
        &native_provider_is_child_document_json(&parent_path, &document_path),
    )
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_applyOwnerSnapshotForTest(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    owner_data_dir: JString<'_>,
    linked_data_dir: JString<'_>,
) -> jstring {
    let owner_data_dir = jni_string_lossy(&mut env, &owner_data_dir);
    let linked_data_dir = jni_string_lossy(&mut env, &linked_data_dir);
    jni_json_string(
        env,
        &crate::ffi::native_apply_owner_snapshot_for_test_json(&owner_data_dir, &linked_data_dir),
    )
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

fn qr_matrix(text: &str) -> Result<QrMatrixResult, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(QrMatrixResult {
            width: 0,
            cells: Vec::new(),
            error: String::new(),
        });
    }

    let code = QrCode::new(trimmed.as_bytes()).map_err(|error| error.to_string())?;
    let width = code.width();
    let cells = code
        .to_colors()
        .into_iter()
        .map(|color| matches!(color, qrcode::Color::Dark))
        .collect();
    Ok(QrMatrixResult {
        width,
        cells,
        error: String::new(),
    })
}

fn native_update_check(
    data_dir: &str,
    current_version: &str,
    mode: &str,
) -> Result<NativeUpdateResult, String> {
    let mode = parse_update_mode(mode)?;
    let current_version = update_current_version(current_version);
    let config = product_update_config_for_dir(Path::new(data_dir));
    check_product_update_blocking(&current_version, mode, config)
        .map(NativeUpdateResult::from)
        .map_err(|error| format!("{error:#}"))
}

fn native_update_download(
    data_dir: &str,
    current_version: &str,
    mode: &str,
    download_dir: &str,
) -> Result<NativeUpdateResult, String> {
    let mode = parse_update_mode(mode)?;
    let current_version = update_current_version(current_version);
    let config = product_update_config_for_dir(Path::new(data_dir));
    let download_dir = trimmed_nonempty(download_dir).map(Path::new);
    download_product_update_blocking(&current_version, mode, config, download_dir)
        .map(NativeUpdateResult::from)
        .map_err(|error| format!("{error:#}"))
}

fn parse_update_mode(mode: &str) -> Result<ProductUpdateMode, String> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "" | "app" => Ok(ProductUpdateMode::App),
        "cli" | "idrive" => Ok(ProductUpdateMode::Cli),
        other => Err(format!("unknown update mode: {other}")),
    }
}

fn update_current_version(current_version: &str) -> String {
    trimmed_nonempty(current_version)
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .to_string()
}

fn trimmed_nonempty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

#[cfg(test)]
mod tests {
    use std::ffi::{CStr, CString};

    use serde_json::Value;

    use super::{
        iris_drive_app_dispatch_json, iris_drive_app_free, iris_drive_app_new,
        iris_drive_app_state_json, iris_drive_install_rustls_crypto_provider,
        iris_drive_qr_matrix_json, iris_drive_string_free, iris_drive_validate_link_input_json,
    };

    #[test]
    fn c_abi_installs_rustls_crypto_provider() {
        iris_drive_install_rustls_crypto_provider();

        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }

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

    #[test]
    fn c_abi_returns_qr_matrix_for_link_text() {
        let link = CString::new("https://drive.iris.to/invite/test").expect("link CString");
        let qr_json = take_string(iris_drive_qr_matrix_json(link.as_ptr()));
        let qr: Value = serde_json::from_str(&qr_json).expect("QR JSON");

        assert!(qr["width"].as_u64().unwrap_or_default() > 0);
        assert!(
            qr["cells"]
                .as_array()
                .is_some_and(|cells| !cells.is_empty())
        );
        assert_eq!(qr["error"], "");
    }

    #[test]
    fn c_abi_returns_link_input_validation() {
        let input = CString::new("npub1short").expect("link input CString");
        let validation_json = take_string(iris_drive_validate_link_input_json(input.as_ptr()));
        let validation: Value = serde_json::from_str(&validation_json).expect("validation JSON");

        assert_eq!(validation["kind"], "app_key_pubkey");
        assert_eq!(validation["is_complete"], false);
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
