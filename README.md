# KlyraDB

Desktop application that manages local, isolated PostgreSQL, MySQL, MariaDB,
Redis and MongoDB instances — no Docker, no config files, no root access.

[![CI](https://github.com/Glyndor/klyradb/actions/workflows/ci.yml/badge.svg)](https://github.com/Glyndor/klyradb/actions/workflows/ci.yml)

Also published on the [Snap Store](https://snapcraft.io/klyradb). License: Apache-2.0.

## Install

<details open>
<summary><strong>Linux — Snap (recommended)</strong></summary>

```bash
sudo snap install klyradb
```

Engines are bundled — no extra packages needed.

</details>

<details>
<summary><strong>Linux / macOS / Windows — Direct download</strong></summary>

<br/>

Download from [**Releases →**](https://github.com/Glyndor/klyradb/releases/latest)

| Platform | Download |
|----------|----------|
| 🐧 Linux | `klyradb-linux-amd64` |
| 🪟 Windows | `klyradb-windows-amd64-setup.exe` |
| 🍎 macOS | `klyradb-macos-arm64.zip` |

</details>

---

## Features

### Supported databases

| Engine | Default port | Versions shown |
|--------|-------------|----------------|
| **PostgreSQL** | 5432 | Latest 3 majors |
| **MySQL** | 3306 | Latest 3 majors |
| **MariaDB** | 3316 | Latest 3 majors |
| **Redis** | 6379 | Latest 3 majors |
| **MongoDB** | 27017 | Latest 3 majors |

Version lists are fetched live from [endoflife.date](https://endoflife.date) at startup so you always see the most recent releases. Falls back to a built-in list when offline.

### Instance management

- **One click** to create, start, stop or delete any instance
- **No root required** — everything runs in user space
- **No conflicts** — each instance has its own port and data directory
- **Copy connection URI** to clipboard instantly and paste into any client

### Engine updates

KlyraDB detects when a **patch update** is available for an installed engine (e.g. `18.2.1 → 18.2.2`) and shows a badge on the instance card. One click stops all affected instances, runs the system upgrade (`apt` / `brew`), and restarts them automatically.

### Engine installation

Outside the Snap, engine binaries may not be present yet. KlyraDB shows an **Install** button in that case — clicking it streams live `apt` / `brew` progress directly in the UI, no terminal needed. The instance starts automatically once the install finishes.

### Localization

Available in **30+ languages**, auto-detected from your system locale. Full RTL support for Arabic and Hebrew. Dark and light theme.

---

## How it works

```
Create instance → pick DB type + version + name
        ↓
KlyraDB allocates a free port, initializes the data directory,
writes an isolated config, and hands you a connection URI.
        ↓
Start / Stop / Delete at any time — nothing touches the rest of your system.
```

---

## Build from source

**Requirements:** Go 1.26+, Node.js, [Wails v2](https://wails.io/docs/gettingstarted/installation)

```bash
# Install Wails CLI
go install github.com/wailsapp/wails/v2/cmd/wails@latest

# Clone
git clone https://github.com/Glyndor/klyradb.git
cd KlyraDB

# Build
wails build -tags webkit2_41   # Linux
wails build                    # macOS
wails build -nsis              # Windows (requires NSIS)
```

**Tests:**
```bash
go test ./internal/...
```

---

## Project structure

```
internal/
├── engine/    shared types, port utils, PID checks
├── manager/   instance lifecycle, port allocation, persistence
├── store/     JSON state (SNAP_USER_COMMON or ~/.local/share/klyradb)
├── versions/  live version detection via endoflife.date
├── pg/        PostgreSQL engine
├── mysql/     MySQL engine
├── mariadb/   MariaDB engine
├── redis/     Redis engine
├── mongodb/   MongoDB engine
└── i18n/      locale loading and system detection
frontend/      vanilla JS + CSS (no framework, no build step)
snap/          snapcraft.yaml and desktop entry
```

---

## Contributing

Issues and pull requests are welcome. Open an [issue](https://github.com/Glyndor/klyradb/issues) to discuss a bug or feature before sending a PR.

## License

[Apache-2.0](LICENSE). Built with Go and [Wails](https://wails.io).
