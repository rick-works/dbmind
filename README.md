# DBmind

**[简体中文](README.zh-CN.md) | English**

🌐 **Official website**: <https://rick-works.github.io/dbmind/>

**Give your database a brain** — DBmind (Database + Mind) is an AI-native workbench for unified database management & development: one client for every database in your stack, with AI that explains, optimizes and fixes your SQL.

Runs locally, your data never leaves your network. Rust core + Vue 3 frontend + Java agent bridge (JDBC drivers), available as Web / Desktop / CLI / MCP.

## Features

| Module | Description |
|---|---|
| **🤖 Deep AI integration** | Explain / optimize / rewrite / fix / diagnose / chat — six one-click actions. AI-generated SQL can be verified with EXPLAIN and a dry run. An MCP server exposes the whole workbench to any AI client (see [AI, more than a chat box](#ai-more-than-a-chat-box)) |
| Multi-datasource | MySQL / PostgreSQL / Oracle / SQL Server / ClickHouse / Doris / MariaDB / DM / Kingbase / SQLite / H2 / Derby / DuckDB / Redis / MongoDB / Elasticsearch |
| SQL editor | Monaco-based: completion, formatting, multi-statement batch runs with per-statement timing, history, snippets, template variables (`:name` prompts on run) |
| Transaction mode | Writes are not auto-committed: edit → verify → commit or roll back. Transactions are bound to the editor's session, on every JDBC datasource |
| In-grid editing | Double-click a cell → buffered changes → batched UPDATE (inside the transaction if enabled); snapshot guards prevent stale writes after re-paging |
| Huge result sets | Virtual scrolling + server-side paging + response compression (4.2MB → 233KB measured); total counts computed async without blocking rows |
| Data transfer | Export to CSV / Excel / JSON / INSERT; long tasks run in background with notifications |
| Production guard | Mark a connection as PROD: badge everywhere, write confirmation, full audit trail |
| Data pivot | Frame-select to aggregate (count / sum / avg / min / max), group and drill down |

## AI, more than a chat box

Most tools bolt on a chat window; DBmind bakes AI into **every step of database work**:

- **Six ready-to-use actions** — select SQL and hit explain / optimize / rewrite; on execution errors hit "AI fix" (full error context attached); "diagnose" for performance issues; free-form chat for everything else
- **Verify AI answers before they land** — one click to view the **execution plan** or **dry-run** AI-generated SQL (costs no AI calls); hallucinations never reach your database
- **AI channel is read-only by default** — writes via AI / MCP are blocked until explicitly allowed: the smartest model cannot bypass safety policy
- **MCP server** — built-in `dbmind-mcp` (stdio): exposes queries, metadata and schema browsing as **tools** to any MCP-capable AI client (Claude Desktop, etc.)
- **Bring your own model** — endpoint and key are yours to configure; conversations only pass through services you control

## Architecture

```
┌──────────────┐  ┌──────────────┐  ┌──────────┐  ┌──────────┐
│  Desktop     │  │  Web         │  │ CLI      │  │ MCP      │
│ dbmind-      │  │ dbmind-web   │  │ dbmind   │  │ dbmind-  │
│ desktop.exe  │  │ .exe + browser│  │          │  │ mcp      │
│ (Tauri shell)│  │ (axum static) │  │          │  │ (stdio)  │
└──────┬───────┘  └──────┬───────┘  └────┬─────┘  └────┬─────┘
       └────────────┬────┴───────────────┴─────────────┘
                    ▼
        ┌───────────────────────────┐
        │   dbmind-core (Rust)      │   pools / lane sessions / safety /
        │   SQL split · metadata     │   audit · counts · export
        └──────┬─────────────┬──────┘
               ▼             ▼
     ┌──────────────┐  ┌──────────────────────┐
     │ Native driver │  │ agent hosts (Java 17+)│
     │ SQLite        │  │ JDBC → MySQL/PG/      │
     │ (rusqlite)    │  │ Oracle/MSSQL/CH/Doris │
     │               │  │ Redis/Mongo/ES clients│
     └──────────────┘  └──────────────────────┘
```

- **Core** (`crates/dbmind-core`): every shell shares one kernel — connection pools with lane sessions, safety policy, dialect-aware SQL splitting, metadata cache, meta storage.
- **Agent hosts** (`agents/`): four protocol bridges (JDBC / MongoDB / Redis / Elasticsearch). The kernel spawns them on demand over a stdin/stdout JSON protocol; driver jars auto-download from Maven.
- **Frontend** (`frontend/`): Vue 3 + Monaco + Element Plus SPA, served by the web shell.

## Directory layout

```
├── crates/
│   ├── dbmind-core      core kernel
│   ├── dbmind-web       web shell (axum API + static hosting, port 20361)
│   ├── dbmind-desktop   desktop shell (Tauri, embeds the kernel)
│   ├── dbmind-cli       command line (conn / driver / setting)
│   └── dbmind-mcp       MCP server (stdio)
├── agents/              Java hosts (jdbc / mongodb / redis / elasticsearch)
├── frontend/            Vue 3 frontend
├── plugins/             database type declarations (JDBC URL templates, driver manifest)
├── resources/           native deps (SQL Server auth dll)
├── scripts/             start / package / release / smoke tests
├── deploy/              real servers for integration tests (redis / es / mongo)
└── website/             product website (single-file static page)
```

## Requirements

| Tool | Version | For |
|---|---|---|
| Rust (rustup) | stable | kernel & shells |
| Node.js + npm | 18+ | frontend build |
| JDK + Maven | 17+ / 3.6.3+ | building Java hosts (needed for all datasources except SQLite) |

> Database driver jars auto-download from Maven into `~/.dbmind/` on demand.

## Quick start (development)

```powershell
# One shot: builds if needed → starts the server → opens the browser
powershell -ExecutionPolicy Bypass -File scripts\start.ps1

# Or step by step:
cargo build -p dbmind-web
cd frontend; npm install; npm run build
cargo run -p dbmind-web -- --port 20361 --dist frontend\dist
```

- Web UI: `http://127.0.0.1:20361`
- Hot-reload dev: `cd frontend; npm run dev` (vite proxies to 20361)

## Building each shell

```powershell
cargo build --release -p dbmind-desktop   # desktop
cargo build --release -p dbmind-web       # web shell
cargo build --release -p dbmind-cli       # CLI
cargo build --release -p dbmind-mcp       # MCP
powershell -File scripts\build-agents.ps1 # Java hosts (4 bridges)
```

## Packaging & release

```powershell
# Full pipeline (portable + zip + MSI/NSIS installers); same as double-clicking scripts\release.bat
powershell -File scripts\release.ps1 -WithJre -Jdk D:\develop\tools\jdk-25.0.4.1+1

# Common switches:
release.ps1 -SkipBundle        # portable only
release.ps1 -SkipPortable      # installers only
release.ps1 -SkipBuild         # reuse compiled artifacts
```

Artifacts land in `dist\`: `dbmind-<version>\` (portable dir), `*-portable.zip`, `installer\*.msi / *-setup.exe`, `SHA256SUMS.txt`.

### CI: all platforms, every variant

`.github/workflows/release.yml` runs on a `v*` tag push (or manually with a `tag` input) and attaches every platform's packages to the same Release:

| Runner | Packages |
|---|---|
| `windows-latest` | `DBmind_<version>_x64_en-US.msi`, `DBmind_<version>_x64-setup.exe` (NSIS wizard), **`dbmind-<version>-portable.zip`** (no-install: unzip and run — ships `agents/` + `jre/`, so no Java needed on the target machine) |
| `macos-latest` | `DBmind_<version>_aarch64.dmg` (+ the `.app`, which is itself install-free: drag to Applications) |
| `macos-15-intel` | `DBmind_<version>_x64.dmg` (+ `.app`) — `macos-13` is retired; Intel now needs the `-intel` label |
| `ubuntu-22.04` | `dbmind_<version>_amd64.deb` (apt) and `*.AppImage` (**no-install**: `chmod +x` and run) |

Code signing is opt-in via repository secrets; the workflow detects them and signs only when present, so unsigned builds keep working:

| Secret | Platform | Purpose |
|---|---|---|
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD` | macOS | sign with a **Developer ID Application** certificate (base64 `.p12`) |
| `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | macOS | notarization (requires a paid Apple Developer account; app-specific password) |
| `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD` | Windows | sign exe / msi / NSIS (base64 `.pfx`) |

Secrets are consumed as `base64 -A` of a keychain-exported `.p12` (macOS) and `certutil -encode` of a `.pfx` (Windows) — see Tauri's Code Signing docs. Each signed platform gets an explicit verification step, so an unsigned package can never ship silently.

## Using it

| Shell | How to run |
|---|---|
| Desktop | double-click `dbmind-desktop.exe` |
| Web | `dbmind-web.exe --port 8787 --dist web`, open `http://127.0.0.1:8787` |
| CLI | `dbmind conn list` / `dbmind driver list` / `dbmind setting get <key>` |
| MCP | run `dbmind-mcp` and register it in any MCP-capable AI client |

- **Data directory**: `~/.dbmind` by default (meta database, driver cache, logs). Override with `DBMIND_HOME` or migrate in Settings.
- **Common env vars**: `DBMIND_HOME`, `DBMIND_AGENT_JAR`, `DBMIND_JAVA`, `DBMIND_TOOL_DIR`.

## Testing

```powershell
cargo test --workspace                      # unit tests (no external deps)
powershell -File scripts\smoke-dbmind.ps1   # end-to-end smoke (69 checks)
powershell -File scripts\host-tests.ps1     # real-JVM agent regression
```

> In-depth docs (architecture / storage / build / troubleshooting) are in [docs/](docs/) — written in Chinese.

## Tech stack

Rust (axum / rusqlite / serde / tracing) · Vue 3 + Monaco + Element Plus (rolldown-vite) · Java 17 (plain stdin/stdout JSON protocol) · Tauri 2

## Sponsor

If DBmind saves you time, buying the author a coffee is always appreciated ☕ (WeChat / Alipay):

| WeChat Pay | Alipay |
|:---:|:---:|
| <img src="docs/assets/sponsor-wechat.jpg" width="240" /> | <img src="docs/assets/sponsor-alipay.jpg" width="240" /> |

> Sponsoring is purely voluntary — the software itself is and stays free.

## License

Free for individuals and teams: use, copy, modify and redistribute freely (keep this notice). **You may NOT sell this software or modified versions on their own, or bundle them as the paid core of a commercial product, without written permission from the author.** Full terms in [LICENSE](LICENSE).
