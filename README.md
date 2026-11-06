# mavex-init

PID 1, `systemctl` and `journalctl` for [MavexOS](https://github.com/MavexOS/mavexos), as one multicall Rust binary (wasm32-wasip1).
A systemd-style *subset*, written from scratch (not systemd itself).

- Units: `[Unit]` (Description, After, Before, Requires, Wants), `[Service]` (Type=simple|oneshot, ExecStart, Restart, RestartSec, RemainAfterExit, WorkingDirectory, Environment), `[Install]` (WantedBy, RequiredBy), `.wants/.requires` dirs, masking via `/dev/null`.
- Search path: `/etc/systemd/system`, `/usr/lib/systemd/system`.
- Service stdout/stderr go to the journal (`/var/log/journal/system.journal`, text, rotated at 2 MiB).
- `systemctl`: list-units, list-unit-files, status, start/stop/restart/try-restart, is-active/is-enabled/is-failed, enable/disable, mask/unmask, cat, show, get-default, daemon-reload.
- `journalctl`: `-u`, `-n`, `-f`, `-o cat`.
- Not supported: ExecStartPre/ExecStop, User=, cgroups, sockets/timers, real reboot/poweroff.

IPC is file-based under `/run/mavex-init` (requests/replies/units state), because MavexOS has no sockets yet.

Build: `cargo build --release --target wasm32-wasip1`. Test: `cargo test`.
