# DBmind

**[简体中文](README.zh-CN.md) | English**

🌐 **Official website**: <https://rick-works.github.io/dbmind/>

**Give your database a brain** — DBmind (Database + Mind) is an AI-native workbench for unified database management & development: one client for every database in your stack, with AI that explains, optimizes and fixes your SQL.

Runs locally, your data never leaves your network. Rust core + Vue 3 frontend + Java agent bridge (JDBC drivers), available as Web / Desktop / CLI / MCP.

## Features

| Module | Description |
|---|---|
| **🤖 Deep AI integration** | Explain / optimize / rewrite / fix / diagnose / chat — six one-click actions. AI-generated SQL can be verified with EXPLAIN and a dry run. An MCP server exposes the whole workbench to any AI client (see [AI, more than a chat box](#ai-more-than-a-chat-box)) |
| **🧠 AI teammate** | "Autonomous check" works the database itself: inspects table structures, suggests missing indexes, reads sync / backup status and dry-runs read-only queries — every change becomes a **change request** you approve (high-risk ones ask twice, and an applied one can be undone). Long-term memory keeps your conventions, conversations are saved and resumable, and each answer footnotes its token cost |
| Multi-datasource | **17 engines, one client** — relational (MySQL, MariaDB, PostgreSQL, KingbaseES, Oracle, SQL Server, DB2, Derby, H2, DM), analytical (ClickHouse, Doris), embedded / file (SQLite, DuckDB) and NoSQL (Redis, MongoDB, Elasticsearch). JDBC drivers download on demand |
| SQL editor | Monaco-based: completion, formatting, multi-statement batch runs with per-statement timing, history, snippets, template variables (`:name` prompts on run) |
| Transaction mode | Writes are not auto-committed: edit → verify → commit or roll back. Transactions are bound to the editor's session, on every JDBC datasource |
| In-grid editing | Double-click a cell → buffered changes → batched UPDATE (inside the transaction if enabled); snapshot guards prevent stale writes after re-paging |
| Huge result sets | Virtual scrolling + server-side paging + response compression (4.2MB → 233KB measured); total counts computed async without blocking rows |
| Table & object designer | Change columns, types, defaults, comments and indexes in a grid — the exact `ALTER` script is previewed before anything runs; views, procedures, functions, triggers and events open their DDL and can be edited as `ALTER` |
| Users & privileges | Users and roles per engine, with a privilege matrix by scope (global / server / database / schema / table / routine). Every tick previews the precise `GRANT` / `REVOKE` diff; only the changed privileges are submitted |
| Monitoring | Sessions, locks and per-database metrics, with in-place session kill. Engines without a monitoring API say so instead of showing empty charts |
| ER diagram | Relations inferred from naming conventions plus real foreign keys; drag to rearrange, wheel to zoom, export to PNG / SVG |
| Compare & transfer | Schema and data diff between two connections, then a Navicat-style wizard that transfers the chosen objects to the target |
| Data transfer | Export to CSV / Excel / JSON / INSERT; long tasks run in background with notifications |
| Backup & test data | Native dump / restore per engine, plus generated test data for scratch tables |
| Object search | Find tables, views and procedures across the whole connection without expanding the tree |
| Production guard | Mark a connection as PROD: badge everywhere, write confirmation, full audit trail |
| Data pivot | Frame-select to aggregate (count / sum / avg / min / max), group and drill down |
| Driver manager | Download, update and repair drivers from Maven with live progress; point it at a private mirror for air-gapped networks |
| Self-maintenance | A background pass at startup reclaims holes in the metadata database and clears expired snapshots and leftover temp files (805 MB → 54 MB measured); clearing logs / cache reclaims space on the spot |

## Screenshots

Every screen ships in both themes. Captured at 2880 px wide (Retina) and downscaled — click any image to open it full size.

| Screen | Light | Dark |
|:--|:-:|:-:|
| **Home**<br>all connections | <img src="docs/images/home-light.png" width="330" alt="DBmind home screen (light)"> | <img src="docs/images/home-dark.png" width="330" alt="DBmind home screen (dark)"> |
| **SQL editor**<br>completion, timings | <img src="docs/images/sql-editor-light.png" width="330" alt="SQL editor and results grid (light)"> | <img src="docs/images/sql-editor-dark.png" width="330" alt="SQL editor and results grid (dark)"> |
| **Table designer**<br>preview ALTER | <img src="docs/images/table-designer-light.png" width="330" alt="Table structure editor (light)"> | <img src="docs/images/table-designer-dark.png" width="330" alt="Table structure editor (dark)"> |
| **AI assistant**<br>plan, dry run | <img src="docs/images/ai-assistant-light.png" width="330" alt="AI assistant panel (light)"> | <img src="docs/images/ai-assistant-dark.png" width="330" alt="AI assistant panel (dark)"> |
| **Governance**<br>inferred relations | <img src="docs/images/governance-light.png" width="330" alt="Data governance view (light)"> | <img src="docs/images/governance-dark.png" width="330" alt="Data governance view (dark)"> |
| **Data transfer**<br>CSV / XLSX / JSON | <img src="docs/images/data-transfer-light.png" width="330" alt="Data transfer (light)"> | <img src="docs/images/data-transfer-dark.png" width="330" alt="Data transfer (dark)"> |
| **Scripts**<br>snippets, variables | <img src="docs/images/scripts-light.png" width="330" alt="Script library (light)"> | <img src="docs/images/scripts-dark.png" width="330" alt="Script library (dark)"> |
| **Tasks**<br>progress, logs | <img src="docs/images/tasks-light.png" width="330" alt="Task center (light)"> | <img src="docs/images/tasks-dark.png" width="330" alt="Task center (dark)"> |
| **Users**<br>roles, privileges | <img src="docs/images/users-light.png" width="330" alt="User management (light)"> | <img src="docs/images/users-dark.png" width="330" alt="User management (dark)"> |
| **Monitoring**<br>sessions, locks | <img src="docs/images/monitor-light.png" width="330" alt="Monitoring (light)"> | <img src="docs/images/monitor-dark.png" width="330" alt="Monitoring (dark)"> |
| **ER diagram**<br>drag, export | <img src="docs/images/er-diagram-light.png" width="330" alt="ER diagram (light)"> | <img src="docs/images/er-diagram-dark.png" width="330" alt="ER diagram (dark)"> |
| **Settings**<br>drivers, themes | <img src="docs/images/settings-light.png" width="330" alt="Settings (light)"> | <img src="docs/images/settings-dark.png" width="330" alt="Settings (dark)"> |

## AI, more than a chat box

Most tools bolt on a chat window; DBmind bakes AI into **every step of database work**:

- **Six ready-to-use actions** — select SQL and hit explain / optimize / rewrite; on execution errors hit "AI fix" (full error context attached); "diagnose" for performance issues; free-form chat for everything else
- **A teammate, not a chat box** — "Autonomous check" calls tools on its own: inspects table structures, suggests the indexes you are missing, reads sync / backup status, dry-runs read-only queries. Anything that touches data becomes a **change request** — approved by you, confirmed twice when high-risk, and undoable after it runs. Multi-turn picks up where the last answer ended, and naming two tables expands two tables
- **It remembers your conventions** — say "remember: amounts in this database are stored in cents" once and it comes along next time; memories are tagged "you said" / "AI inferred" and can be deleted. Conversations are saved, so "continue" continues
- **Context is budgeted, not the whole schema dumped in** — large databases no longer ship hundreds of table names; a library-level question ("what is this database for") now gets the full table list with comments (about 2.7k tokens for 395 tables, versus hundreds of thousands for every column). Measured on one fix: 2345 → 956 prompt tokens, chat 2404 → 1030. Every answer footnotes its token cost, and a zero-cost breakdown shows where the request went
- **Verify AI answers before they land** — one click to view the **execution plan** or **dry-run** AI-generated SQL (costs no AI calls); hallucinations never reach your database
- **AI channel is read-only by default** — writes via AI / MCP are blocked until explicitly allowed: the smartest model cannot bypass safety policy
- **MCP server** — built-in `dbmind-mcp` (stdio): exposes queries, metadata and schema browsing as **tools** to any MCP-capable AI client (Claude Desktop, etc.)
- **Bring your own model** — endpoint and key are yours to configure; conversations only pass through services you control

## What the AI actually sends

Worry about data usually means "I don't know what leaves the machine". Instead of a privacy slogan, here is one request, laid out. Judge for yourself whether an external model is acceptable.

**There is no relay server.** Requests go from your machine straight to the endpoint you configured under Settings → AI (OpenAI-compatible). AI is off by default: with no model configured, not a single byte is sent.

**What goes out (the complete content of one question)**

| Content | Exactly what | When |
|---|---|---|
| Your question | The prompt, the current SQL, the error text, plus the last 6 messages (6000 characters combined, long ones trimmed in the middle) | Every request |
| Table structure | Table name + column name / type / nullable / default / primary key (**column comments are not sent**) | Tables named or detected in the question, at most 40; otherwise the first 8 |
| One sample row | **1 real row** per expanded table, first 20 columns, every value cut to 24 characters | Together with the structure |
| Table catalogue | Table names + table comments (no column structure) | Library-level questions like "what is this database for" |
| Team terms / examples | The glossary and accepted SQL examples you entered under AI Knowledge | If configured |
| Long-term memory | Conventions you stated ("amounts are in cents") | If that connection / database has memories |
| Knowledge base | Matching document snippets, ≤900 characters each, labelled with the source | If a knowledge base is configured and hits |
| Built-in prompts | Persona and dialect rules (plain text, none of your data) | Every request |

**What never goes out**

- **Whole tables or result sets** — only that one sample row. The single exception: when you switch on Autonomous check, the read-only queries it runs return at most 100 rows, truncated to 3200 characters, into the conversation. It cannot "look at your data" for free; this is stated plainly rather than buried.
- **Connection passwords / SSH secrets / API keys** — stored only in your local `~/.dbmind/dbmind.db`; the API returns "saved", never the value.
- **Telemetry** — no usage statistics, no crash reporting, no calls to any DBmind server (the update check reads a version number and has nothing to do with AI).
- **Other connections or databases** — context comes only from the connection and database used for this question.

**If even the sample row is too much, tighten it**

- **Local models only**: once enabled, any endpoint that is not internal/local is refused outright, naming the blocked host. Allowed: `localhost` / loopback, IPv4 private ranges (10 / 172.16–31 / 192.168 / 169.254), IPv6 ULA (fc00::/7), the `.local/.internal/.lan/.corp/.intra` suffixes, and whatever you add to `DBMIND_AI_LOCAL_HOSTS`. Both chat and knowledge-base embedding obey this — a self-hosted Ollama / vLLM works, a cloud model simply cannot be reached.
- **Narrow the tables**: name tables in the question and only those are included; library-level questions get names + comments with no rows at all.
- **Look before you send**: `POST /api/ai/preview` calls no model and reports the token cost and per-part breakdown — it is what "where did this request go" in the UI uses.
- **Not configured, not used**: no knowledge base means no snippets, no memories means no memory block, no Autonomous check means no query is ever executed.

**A real request, redacted**

```
【目标数据库】类型 MYSQL ｜ 版本 5.7.42 ｜ 标识符引用风格 反引号 ｜ 分页写法 `limit {offset}, {limit}`
【表清单】共 42 张表，仅展开用到的那几张（需要其它表的字段请先说明表名）
【表 orders】
  id BIGINT not null 主键
  status VARCHAR(16) not null default 'NEW'
  amount BIGINT not null default 0
  created_at DATETIME not null
  样例：id=10231, status=PAID, amount=19900, created_at=2026-10-30 12:00:00
```

That sample row is **real data**, truncated to 24 characters: it is how the model knows the date format and that `status` holds `PAID` rather than `1`. If the first 20 columns include phone numbers or ID numbers, they go with it — in that case keep the endpoint inside your network (Local models only) and nothing leaves at all.

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
├── scripts/             dev entries (start/stop) + build/ release/ test/ tools/
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
mvn -B -q -DskipTests package -f agents\dbmind-agent-jdbc\pom.xml  # Java hosts (4 modules)
```

## Packaging & release

**Release artifacts are built by CI.** Pushing a `v*` tag runs `.github/workflows/release.yml`, which produces every platform's packages (portable zip, MSI, NSIS, dmg, deb, rpm, AppImage) and attaches them to one Release.

The local one-shot scripts (`release.ps1` / `release.bat`) have been removed. To assemble a bundle on your own machine, run the same steps CI runs:

```powershell
cargo build --release
cd frontend; npm install; npm run build; cd ..
foreach ($m in 'jdbc','mongodb','redis','elasticsearch') { mvn -B -q -DskipTests package -f "agents\dbmind-agent-$m\pom.xml" }
powershell -File scripts\build\package.ps1 -SkipCargo -SkipAgents -SkipFrontend
```

`scripts\build\package.ps1` assembles the distribution directory (CI uses the same script); `scripts\build\dmg-settings.py` is the macOS DMG appearance config.

> The complete release checklist — version level, what `release.mjs` rewrites, why highlights must be committed *before* tagging, backfilling the Release body and website after CI, syncing the Homebrew cask, and what to verify — lives in [`docs/build.md`](docs/build.md) (Chinese).

### CI: all platforms, every variant

`.github/workflows/release.yml` runs on a `v*` tag push (or manually with a `tag` input) and attaches every platform's packages to the same Release:

| Runner | Packages |
|---|---|
| `windows-latest` | `DBmind_<version>_x64_en-US.msi`, `DBmind_<version>_x64-setup.exe` (NSIS wizard), **`dbmind-<version>-portable.zip`** (no-install: unzip and run — ships `agents/` + `jre/`, so no Java needed on the target machine) |
| `macos-latest` | `DBmind_<version>_aarch64.dmg` (+ the `.app`, which is itself install-free: drag to Applications) |
| `macos-15-intel` | `DBmind_<version>_x64.dmg` (+ `.app`) — `macos-13` is retired; Intel now needs the `-intel` label |
| `ubuntu-22.04` | `dbmind_<version>_amd64.deb` (apt) and `*.AppImage` (**no-install**: `chmod +x` and run) |

### macOS: the first launch is blocked once (expected — the build is fine)

macOS always blocks an app that was *downloaded from the internet* and is **not notarised by Apple** —
that is a macOS rule, not something the app can bypass. To open it: click OK, then go to
**System Settings → Privacy & Security**, scroll to *Security*, click **Open Anyway**, confirm once.
After that, double-clicking works normally. **No sudo, no terminal commands needed**
(the DMG ships this note as `首次打开请看这里.txt` too).

To never see that dialog at all: the switch is the *downloaded from the internet* flag
(`com.apple.quarantine`), and it is set **by whatever program downloaded the file** — fetch it another way
and Gatekeeper has nothing to check:

```bash
# (1) curl never sets that flag. Replace <version> with the latest from the releases page, e.g. 1.2.7
curl -LO https://github.com/rick-works/dbmind/releases/download/v<version>/DBmind_<version>_aarch64.dmg
open DBmind_<version>_aarch64.dmg     # mount, drag to Applications, double-click — no prompt
#     (on Intel, swap aarch64 for x64)

# (2) Homebrew cask: --no-quarantine strips that flag for you
brew install --cask --no-quarantine rick-works/tap/dbmind
```

Two things worth knowing so you do not chase them twice: the **signature cannot be dropped** —
on Apple silicon code needs at least an ad-hoc signature, and removing it does not remove a dialog,
it makes the app refuse to launch (macOS then says it *is damaged*); and **even with signing plus
notarisation the first launch still shows one dialog**, just the friendlier one with an **Open** button.
Notarisation needs a paid Apple Developer account ($99/year — see the table below).

Code signing is opt-in via repository secrets; the workflow detects them and signs only when present, so unsigned builds keep working:

| Secret | Platform | Purpose |
|---|---|---|
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD` | macOS | sign with a **Developer ID Application** certificate (base64 `.p12`) |
| `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | macOS | notarization (requires a paid Apple Developer account; app-specific password) |
| `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD` | Windows | sign exe / msi / NSIS with a certificate in the local store (`signtool`, base64 `.pfx`) |
| `AZURE_SIGN_ENDPOINT`, `AZURE_SIGN_ACCOUNT`, `AZURE_SIGN_PROFILE`, `AZURE_SIGN_DESCRIPTION` + `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_CLIENT_SECRET` | Windows | cloud signing via **Azure Artifact Signing** (formerly Trusted Signing) — for certificates whose private key lives in a hardware module, i.e. no `.pfx` to export |

Certificate lifetimes are set by policy, not by accident — plan for rotation:

- **Windows code signing: max 460 days** (CA/B Forum CSC-31, mandatory for certificates issued from 2026-03-01; it used to be 39 months). Since 2023-06-01 the private key must sit in a hardware crypto module, which is exactly why new certificates usually come without a `.pfx` and why the Azure cloud-signing path exists.
- **Apple**: the **$99/year membership must stay active** to sign new builds (Developer ID certificates themselves last 5 years, provisioning profiles 18 years, max 5 certificates per type).
- **Nothing to "permanently enable":** a certificate that leaks cannot be un-leaked, so expiry is the backstop. Artifacts signed *before* expiry keep working forever — signatures carry a timestamp, so verification checks that the certificate was valid at signing time, not now. Renewal = drop the new certificate into the secret and re-run.

<details>
<summary><b>How to produce each secret</b></summary>

**macOS** — you do *not* need a Mac; OpenSSL on Windows is enough.

1. Join the Apple Developer Program ($99/year), then *Certificates, Identifiers & Profiles → Certificates → + → **Developer ID Application*** and upload a CSR:
   ```bash
   openssl req -new -newkey rsa:2048 -nodes -keyout apple.key -out apple.csr \
     -subj "/CN=Your Name/emailAddress=your@apple.id"
   ```
2. Download `developerid_application.cer` and build a `.p12` (chaining Apple's intermediate CA):
   ```bash
   curl -O https://www.apple.com/certificateauthority/DeveloperIDG2CA.cer
   openssl x509 -inform DER -in DeveloperIDG2CA.cer -out ca.pem
   openssl x509 -inform DER -in developerid_application.cer -out cert.pem
   openssl pkcs12 -export -inkey apple.key -in cert.pem -certfile ca.pem -out apple.p12
   ```
   (On a Mac instead: double-click the `.cer`, then export the entry from *My Certificates* as `.p12`.)
3. Base64 it: macOS `openssl base64 -A -in apple.p12 -out apple.b64`; Windows `certutil -encode apple.p12 apple.b64`. Paste the **whole file** into `APPLE_CERTIFICATE` — boundary lines are fine, the workflow strips them.
4. `KEYCHAIN_PASSWORD` is just a throwaway password for the temporary keychain CI creates.
5. Notarization: `APPLE_PASSWORD` is an **app-specific password** (appleid.apple.com → Sign-In and Security), `APPLE_TEAM_ID` is on the Membership page.

**Windows, local certificate** — `certutil -encode code.pfx code.b64` → `WINDOWS_CERTIFICATE` (whole file), password → `WINDOWS_CERTIFICATE_PASSWORD`.

**Windows, cloud signing** — register Azure Artifact Signing (~$9.99/month), note the endpoint / account / certificate-profile names, and create an Entra ID app registration holding the *Artifact Signing Certificate Profile Signer* role; feed its client id / tenant id / client secret into the `AZURE_*` secrets. The workflow installs `artifact-signing-cli` and wires it into Tauri's `signCommand` on its own.

```powershell
$repo = 'rick-works/dbmind'
gh secret set APPLE_CERTIFICATE           --repo $repo --body (Get-Content apple.b64 -Raw -Encoding ASCII)
gh secret set APPLE_CERTIFICATE_PASSWORD  --repo $repo   # prompts for the value
gh secret set KEYCHAIN_PASSWORD           --repo $repo
gh secret set APPLE_ID                    --repo $repo
gh secret set APPLE_PASSWORD              --repo $repo
gh secret set APPLE_TEAM_ID               --repo $repo
gh secret set WINDOWS_CERTIFICATE           --repo $repo --body (Get-Content code.b64 -Raw -Encoding ASCII)
gh secret set WINDOWS_CERTIFICATE_PASSWORD  --repo $repo
gh secret list --repo $repo
```

Then re-run the workflow — a manual dispatch with an empty `tag` keeps the signed packages in Actions artifacts and leaves the published Release untouched.

</details>

Each signed platform runs an explicit verification step (`codesign --verify` + `stapler validate`; `Get-AuthenticodeSignature`), so an unsigned package can never ship silently. `*.p12 / *.pfx / *.csr / *.cer / *.key / *.b64` are gitignored: private keys live in secrets only.

**Without certificates, macOS builds still get an ad-hoc signature** (`signingIdentity: "-"`, injected only when no real identity is present). On Apple Silicon every piece of executable code must carry at least an ad-hoc signature or the app cannot launch at all, so this turns "the app is damaged and won't open" into "right-click → Open once". Windows builds then simply ship unsigned and SmartScreen asks for confirmation. Any user-facing wording for that is in the release notes ("First install and launch").

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
powershell -File scripts\test\smoke-api.ps1   # end-to-end smoke (68 checks)
powershell -File scripts\test\host-tests.ps1     # real-JVM agent regression
```

> In-depth docs (architecture / storage / build / troubleshooting / db-lab) are in [docs/](docs/) — written in Chinese.

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
