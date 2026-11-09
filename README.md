# mavex-init

PID 1, `systemctl` and `journalctl` for [MavexOS](https://github.com/MavexOS/mavexos), as one multicall Rust binary (wasm32-wasip1).
A systemd-style *subset*, written from scratch (not systemd itself).

- Units: `[Unit]` (Description, After, Before, Requires, Wants), `[Service]` (Type=simple|oneshot, User, Group, ExecStart, Restart, RestartSec, RemainAfterExit, WorkingDirectory, Environment), `[Install]` (WantedBy, RequiredBy), `.wants/.requires` dirs, masking via `/dev/null`.
- Search path: `/etc/systemd/system`, `/usr/lib/systemd/system`.
- Service stdout/stderr go to the journal (`/var/log/journal/system.journal`, text, rotated at 2 MiB).
- `systemctl`: list-units, list-unit-files, status, start/stop/restart/try-restart, is-active/is-enabled/is-failed, enable/disable, mask/unmask, cat, show, get-default, daemon-reload.
- `journalctl`: `-u`, `-n`, `-f`, `-o cat`.
- Timers: `.timer` units with OnBootSec/OnStartupSec/OnActiveSec/OnUnitActiveSec/OnCalendar (minutely/hourly/daily/weekly/monthly, `*:0/15`, `Mon *-*-* 03:30`; UTC), Unit=; `systemctl list-timers`.
- Power: `reboot`, `poweroff`, `halt`, `shutdown -r|-h now`, `systemctl reboot|poweroff|halt` (root only) stop all services, then call the host `mavex.power` import.
- `User=`/`Group=` spawn the service with those credentials (host import `mavex.spawn_as`, root only).
- Not supported: ExecStartPre/ExecStop, cgroups, sockets, delayed shutdown, calendar dates other than `*-*-DD`.

IPC is file-based under `/run/mavex-init` (requests/replies/units state), because MavexOS has no sockets yet.

Build: `cargo build --release --target wasm32-wasip1`. Test: `cargo test`.
