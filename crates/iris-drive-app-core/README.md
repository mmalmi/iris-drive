# iris-drive-app-core

Native shells use this crate as the shared app contract.

It will own:

- UI snapshot structs the native shells render
- the typed native state mirrored across platforms
- the typed action set the shells dispatch back
- platform capability projection (desktop / Android / iOS)
- a UniFFI `FfiApp` object with `state`, `refresh`, and `dispatch`

Scaffold only at present.
