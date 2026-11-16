use super::{
    Path, c_char, c_string_lossy, json_string, native_provider_delete_json,
    native_provider_list_json, native_provider_mkdir_json, native_provider_read_json,
    native_provider_rename_json, native_provider_write_json,
};

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
pub extern "C" fn iris_drive_provider_reimport_json(
    data_dir: *const c_char,
    path: *const c_char,
    source_path: *const c_char,
    directory: bool,
) -> *mut c_char {
    let source = c_string_lossy(source_path);
    let result = crate::native_provider::native_provider_reimport(
        &c_string_lossy(data_dir),
        &c_string_lossy(path),
        (!source.is_empty()).then(|| Path::new(&source)),
        directory,
    )
    .unwrap_or_else(|error| serde_json::json!({"error": format!("{error:#}")}));
    json_string(&result)
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
