# Linux / Omarchy

This port preserves the React interface and runs natively with Tauri 2 / GTK / WebKitGTK.

## Build and install on Arch / Omarchy

System prerequisites: Rust/Cargo, Node.js/npm, WebKitGTK 4.1, GTK 3, libsecret
(`secret-tool`), sqlite3, and a running, unlocked Secret Service implementation
(such as GNOME Keyring).

```bash
npm ci
npm run build
npm run test:rust
npm run tauri -- build --no-bundle
bash scripts/install-linux.sh
swidex
```

The app is installed under `~/.local/lib/swidex`, with a launcher in
`~/.local/bin` and a desktop menu entry. No root installation is required.
To update, rebuild and run the installation script again.

## Platform differences

- Provider secrets use Linux Secret Service via `secret-tool`. Secrets are sent
  on stdin, not command-line arguments. Missing or locked keyrings produce an
  actionable error; there is no plaintext fallback for saved provider secrets.
- Optional API environment persistence writes to `~/.bashrc` on Linux and
  restricts the file to owner read/write. This option explicitly stores a
  plaintext copy, separate from the keyring, just as on macOS.
- GUI environment inspection and updates use `systemctl --user`. They apply to
  newly launched user services in the current desktop session. After logging in
  again, start Codex from a new Bash terminal or enable the provider again.
- Existing processes retain their old environment. Restart Codex after switching.
- Only API/provider environment variables are modified. FlClash's `HTTP_PROXY`,
  `HTTPS_PROXY` and `NO_PROXY` settings are preserved.
- Other custom environment sources, such as hand-written Hyprland Lua or
  environment.d provider overrides, are not automatically rewritten.
- The inherited history migration logic targets `state_5.sqlite`; test migrations
  with backups before using them with a different Codex database schema.

## Verification

Unit tests use temporary files, including a Bash persistence/cleanup regression
test that verifies local HTTP proxy settings remain unchanged.
An optional live keyring test uses and deletes only its own synthetic entry:

```bash
cargo test --manifest-path src-tauri/Cargo.toml keyring_round_trip -- --ignored
```

Launching Swidex only diagnoses the current configuration. Saving a provider,
switching providers, resetting authentication and migrating history still require
the corresponding user actions in the interface.
