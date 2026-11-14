use super::{c_char, c_string_lossy, json_string};
use crate::native_provider::photo_backup::{import_json, publish_json};

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_photo_backup_import_json(
    dir: *const c_char,
    profile: *const c_char,
    resource: *const c_char,
    name: *const c_char,
    source: *const c_char,
) -> *mut c_char {
    json_string(&import_json(
        &c_string_lossy(dir),
        &c_string_lossy(profile),
        &c_string_lossy(resource),
        &c_string_lossy(name),
        &c_string_lossy(source),
    ))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_photoBackupImportJson(
    mut env: super::JNIEnv<'_>,
    _class: super::JClass<'_>,
    dir: super::JString<'_>,
    profile: super::JString<'_>,
    resource: super::JString<'_>,
    name: super::JString<'_>,
    source: super::JString<'_>,
) -> super::jstring {
    let [dir, profile, resource, name, source] = [dir, profile, resource, name, source]
        .map(|value| super::jni_string_lossy(&mut env, &value));
    super::jni_json_string(env, &import_json(&dir, &profile, &resource, &name, &source))
}

#[unsafe(no_mangle)]
pub extern "C" fn iris_drive_photo_backup_publish_json(
    dir: *const c_char,
    profile: *const c_char,
    allow_network: bool,
) -> *mut c_char {
    json_string(&publish_json(
        &c_string_lossy(dir),
        &c_string_lossy(profile),
        allow_network,
    ))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_to_iris_drive_app_core_NativeCore_photoBackupPublishJson(
    mut env: super::JNIEnv<'_>,
    _class: super::JClass<'_>,
    dir: super::JString<'_>,
    profile: super::JString<'_>,
    allow_network: jni::sys::jboolean,
) -> super::jstring {
    let [dir, profile] = [dir, profile].map(|value| super::jni_string_lossy(&mut env, &value));
    super::jni_json_string(env, &publish_json(&dir, &profile, allow_network != 0))
}
