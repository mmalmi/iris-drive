# android

Native Android shell over `iris-drive-app-core` via UniFFI JNI bindings.

## Build

```bash
just android-build
```

The Gradle app cross-compiles `iris-drive-app-core` for `arm64-v8a` with
`cargo ndk`, packages the JNI library into the debug APK, and installs through
the normal Android Gradle Plugin tasks.

## Smoke

```bash
just android-smoke
```

The smoke builds the APK, installs it on the selected `adb` device or emulator,
launches `MainActivity`, and verifies the SAF `DocumentsProvider` authority is
registered. Pass the device through `IRIS_DRIVE_ANDROID_SERIAL` or
`ANDROID_SERIAL`.
