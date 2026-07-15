# Linux Native Shell

Rust GTK4/libadwaita shell for Iris Drive.

Run it from the repo root:

```bash
just run-linux
```

Useful commands:

```bash
just linux-build
just linux-install-menu
cd linux && cargo run
```

The app manages the local `idrive` CLI/daemon, opens the working folder, and
starts the loopback gateway/resolver for `*.iris.localhost` and
`nhash.iris.localhost` by default.
Installed packages register `iris-drive://` links through the desktop entry.

The crate is excluded from the root workspace so GTK toolchains do not block
core builds.
