# AGENTS.md — EFTRegionWatcher

## 0. Project Identity

**Project name:** EFTRegionWatcher  
**Repository name:** `eft-region-watcher`  
**Target OS:** Windows 11 x64  
**Primary language:** Rust  
**Application type:** Lightweight native Windows desktop/tray application  
**Distribution:** Public GitHub repository + GitHub Releases

EFTRegionWatcher is an **unofficial** utility for Escape from Tarkov players. It detects the remote server endpoint currently used by the game by reading Escape from Tarkov's own log files, then displays the inferred region/location of that IP address.

The project is not affiliated with, endorsed by, or sponsored by Battlestate Games.

---

# 1. Primary Goal

Build a Windows 11 desktop application that can run continuously while Escape from Tarkov is running with **minimal impact on game performance**.

The application must:

1. Detect the server IP/port used by Escape from Tarkov.
2. Infer a useful geographic/network location from the detected IP.
3. Present the result in a very small native UI and Windows system tray.
4. Avoid invasive interaction with the game.
5. Check GitHub Releases for updates.
6. Prompt the user when an update exists.
7. Never force an update.
8. Be simple enough for users with little PC knowledge.

Performance and low interference with the game have higher priority than visual sophistication.

---

# 2. Hard Constraints

The following are mandatory.

## 2.1 Do not interact invasively with Escape from Tarkov

The application MUST NOT:

- inject DLLs;
- hook the EFT process;
- read or write EFT process memory;
- modify EFT files;
- modify BattlEye files;
- install a kernel driver;
- capture packets with WinPcap/Npcap;
- use packet injection;
- use screen scraping to determine the server;
- automate gameplay;
- simulate player input;
- circumvent anti-cheat systems.

The intended data source is **Escape from Tarkov's own log files only**.

The application should require **no administrator privileges**.

---

# 3. Technology Stack

Use the following stack unless there is a strong technical reason not to.

## Core

- Rust stable
- Windows x64
- Native Win32 API
- No WebView
- No Electron
- No Chromium
- No Node.js runtime
- No Python runtime
- No Java runtime

Prefer direct Windows APIs or thin Rust wrappers.

Useful Windows APIs may include:

- `ReadDirectoryChangesW`
- `Shell_NotifyIconW`
- `RegisterHotKey`
- `CreateFileW`
- `ReadFile`
- `WinHTTP`
- Windows Credential Manager / DPAPI if credentials are ever needed
- Windows startup registration APIs

Avoid heavyweight GUI frameworks when a small Win32 window is sufficient.

---

# 4. Performance Budget

The application is meant to run during gameplay.

Target budgets:

| Resource | Target |
|---|---|
| Idle CPU | effectively 0%, ideally <= 0.1% |
| Active parsing CPU | short bursts only |
| RAM | target 10–25 MB |
| GPU | effectively 0 |
| Disk I/O | incremental log reads only |
| Network | only when needed |
| Background polling | avoid wherever possible |

Do not repeatedly rescan entire log files.

Do not poll the EFT log directory every second.

Use event-driven filesystem notifications.

---

# 5. EFT Log Detection

The program must automatically locate the Escape from Tarkov log directory when possible.

Expected common location:

`C:\Battlestate Games\EFT\Logs`

However, EFT may be installed elsewhere.

Detection strategy:

1. Check configured path.
2. Check known/default EFT locations.
3. Attempt reasonable discovery if inexpensive.
4. If not found, show a simple folder picker.
5. Store the selected path.

Do not perform expensive full-disk scans.

---

# 6. Log Monitoring

Use `ReadDirectoryChangesW` or an equivalent low-overhead Windows filesystem notification mechanism.

Desired behavior:

```text
No log change
    |
    +--> thread remains blocked / idle

Log changes
    |
    +--> receive filesystem event
           |
           +--> read only appended content
                  |
                  +--> run parsers
```

The watcher must tolerate:

- log rotation;
- new session directories;
- EFT restart;
- file replacement;
- temporary file locks;
- incomplete lines;
- UTF-8 / encoding irregularities where reasonable.

Keep track of file offset so existing log contents are not continually reread.

---

# 7. Server Endpoint Parsing

Parser logic must be isolated from the filesystem watcher.

Create a parser interface/trait so log formats can be changed without rewriting the rest of the application.

## 7.1 Primary parser

Current expected pattern:

```text
Connect (address: IP:PORT)
```

Example:

```text
Connect (address: 79.127.145.28:17007)
```

Extract:

```text
ip   = 79.127.145.28
port = 17007
```

The parser must support IPv4.

Architect the endpoint model so IPv6 can be added later if necessary.

## 7.2 Legacy fallback parser

Support older known EFT-style entries such as:

```text
RaidMode: Online, Ip: x.x.x.x
```

Use the current parser first and legacy parser second.

Do not hard-code parsing logic directly into UI code.

---

# 8. Connection State Model

Do not simply display the most recent IP forever as "connected."

Maintain explicit state.

Recommended model:

```rust
enum ConnectionState {
    Waiting,
    Connected(ServerEndpoint),
    LastSeen(ServerEndpoint),
}
```

The actual model may be richer.

The UI must clearly distinguish:

- currently detected connection;
- waiting / not currently connected;
- last known connection.

Store at least:

```text
IP
port
detected timestamp
resolved region
country
city, if useful
ASN/provider, if available
```

Do not claim geographic information is exact.

Use wording such as:

- "推定地域"
- "Estimated region"
- "IP geolocation"

rather than representing GeoIP as an official EFT server designation.

---

# 9. GeoIP Resolution

When a new IP is detected:

1. Look for it in local cache.
2. If cached and still valid, use cache.
3. Otherwise perform a GeoIP lookup.
4. Cache the result.

Avoid repeated lookups for the same IP.

Suggested cached information:

```json
{
  "ip": "203.0.113.10",
  "country": "Japan",
  "region": "Tokyo",
  "city": "Tokyo",
  "asn": "ASxxxxx",
  "organization": "Example Hosting",
  "updated_at": "..."
}
```

The GeoIP provider must be wrapped behind an interface so it can be replaced without affecting core application logic.

Do not make any network request continuously while waiting for EFT activity.

---

# 10. Main UI

The main application should primarily live in the Windows system tray.

A full-size desktop window is unnecessary.

Recommended compact window:

```text
+--------------------------------+
| EFTRegionWatcher               |
|                                |
| ● CONNECTED                    |
|                                |
| Estimated Region               |
| Japan / Tokyo                  |
|                                |
| 203.xxx.xxx.xxx:17007          |
|                                |
| Detected 12:42:18              |
|                                |
| [Copy IP]                      |
+--------------------------------+
```

When not connected:

```text
○ WAITING

Last server
Japan / Tokyo
203.xxx.xxx.xxx
```

Use native Windows controls where practical.

Avoid animation.

Avoid GPU-heavy rendering.

---

# 11. System Tray

The tray icon is a core part of the application.

Required tray actions:

```text
Open
Current server
Copy IP
Check for updates
Settings
Exit
```

Optional:

- Start with Windows
- Connection notification toggle

The app should remain usable without keeping its main window open.

---

# 12. Notifications

Connection notifications must be optional.

Default may be OFF if gameplay interruption is a concern.

Example:

```text
EFTRegionWatcher

Server detected
Japan / Tokyo
203.xxx.xxx.xxx
```

Do not spam notifications for repeated log lines containing the same endpoint.

Deduplicate identical endpoint events.

---

# 13. Global Hotkey

Optional feature for V1 if implementation remains lightweight.

Use Windows `RegisterHotKey`.

Do NOT install a keyboard hook.

Example configurable hotkey:

```text
Ctrl + Alt + S
```

The hotkey should show the current connection window or a small status popup.

---

# 14. Configuration

Suggested configuration directory:

```text
%LOCALAPPDATA%\EFTRegionWatcher\
```

Suggested files:

```text
config.toml
geo_cache.json
logs\
```

Do not store unnecessary user data.

Example configuration:

```toml
eft_log_path = "C:\\Battlestate Games\\EFT\\Logs"
start_with_windows = false
connection_notifications = false

[update]
enabled = true
check_interval_hours = 24
skipped_version = ""
```

---

# 15. Update System

The repository will be PUBLIC.

Therefore normal stable releases should be downloadable from GitHub Releases without requiring users to authenticate to GitHub.

The updater must be designed for users with minimal PC knowledge.

## 15.1 Update behavior

Check for an update:

- at startup only if the previous check is old enough;
- when the user manually clicks "Check for updates";
- optionally once every 24 hours while the program remains open, but avoid frequent polling.

Preferred default:

```text
check interval = 24 hours
```

If no update exists:

```text
do nothing
```

Do not show unnecessary dialogs during normal startup.

---

# 16. Versioning

Use Semantic Versioning.

Examples:

```text
v1.0.0
v1.0.1
v1.1.0
v2.0.0
```

The application version should come from Cargo package metadata.

Compare installed version against the latest stable GitHub Release.

Ignore by default:

- draft releases;
- prereleases;
- beta releases;
- alpha releases.

A future developer setting may allow prerelease channels.

---

# 17. Update Prompt UX

When an update exists, show a simple dialog.

Example:

```text
EFTRegionWatcher の新しいバージョンがあります

現在: v1.2.0
最新: v1.3.0

変更内容:
- EFTログ解析を改善
- サーバー判定を修正
- 不具合を修正

[更新する]

[あとで]

[このバージョンをスキップ]
```

Required behavior:

## Update now

Downloads and installs the new release.

## Later

Do nothing now.

The application may remind the user on the next appropriate update check.

## Skip this version

Store the release version in configuration.

Example:

```toml
skipped_version = "1.3.0"
```

Do not prompt for that exact version again.

When `1.3.1` or another newer version appears, prompting resumes.

Updates are NEVER mandatory.

---

# 18. Updating While EFT Is Running

Do not perform disruptive update operations automatically while the game is running.

If the user selects "Update" while Escape from Tarkov is running, show:

```text
Escape from Tarkov is currently running.

Recommended:
[Update after EFT closes]

[Update now]

[Cancel]
```

Default / highlighted action:

```text
Update after EFT closes
```

Do not silently terminate EFT.

Do not interfere with EFT shutdown.

The application may wait for EFT to exit and then run the updater.

This waiting must be lightweight.

---

# 19. Updater Architecture

Prefer a separate small updater executable.

Expected binaries:

```text
EFTRegionWatcher.exe
EFTRegionWatcher.Updater.exe
```

The updater executable must not remain running during normal use.

Normal state:

```text
EFTRegionWatcher.exe         running
EFTRegionWatcher.Updater.exe not running
```

Update flow:

```text
1. User accepts update.
2. Download release asset to a temporary path.
3. Validate integrity.
4. Launch updater.
5. Main app exits.
6. Updater replaces the old executable.
7. Updater starts the new main executable.
8. Updater exits.
```

The updater should be minimal and dependency-light.

---

# 20. Release Integrity

At minimum, verify a cryptographic digest before replacing the executable.

Preferred:

```text
SHA-256
```

Release automation should publish:

```text
EFTRegionWatcher.exe
EFTRegionWatcher.Updater.exe
SHA256SUMS.txt
```

Alternatively publish a ZIP/package plus its SHA-256 digest.

If validation fails:

- do not replace the current executable;
- delete or quarantine the invalid download;
- show a clear error;
- continue using the installed version.

Never execute a partially downloaded update.

---

# 21. GitHub Release Asset Naming

Use deterministic asset names.

Recommended:

```text
EFTRegionWatcher-win-x64.zip
EFTRegionWatcher-win-x64.zip.sha256
```

Release tags:

```text
v1.0.0
v1.1.0
```

GitHub release title:

```text
EFTRegionWatcher v1.1.0
```

---

# 22. Release Notes

The app should display concise release notes when an update exists.

Do not render arbitrary remote HTML.

Treat GitHub-provided release text as untrusted remote content.

Display as plain text or safely sanitized text.

Limit the amount displayed in the small update dialog.

Provide a "View details" action if needed.

---

# 23. Networking

Network requests should be minimal.

Expected remote calls:

```text
GeoIP lookup for uncached server IP
GitHub update check
GitHub release download after user approval
```

Use short timeouts.

Network failure must not prevent the main EFT server detection feature from working.

Example:

```text
GitHub unavailable
    |
    +--> update check fails silently or logs warning
    |
    +--> app remains fully usable
```

GeoIP failure:

```text
IP detected
Region unavailable
```

The raw endpoint should still be displayed.

---

# 24. Error Handling

The application must fail gracefully.

Examples:

## EFT logs unavailable

Display:

```text
EFTのログフォルダが見つかりません。

[フォルダを選択]
```

## Log permission error

Display a clear message without requesting administrator mode unless actually necessary.

## GeoIP unavailable

Show IP and port anyway.

## Update check failure

Do not block startup.

## Update installation failure

Keep the previous working executable intact.

Avoid panics in user-facing builds.

---

# 25. Logging

Application diagnostic logs should be small and rotating.

Do not log continuously at high volume.

Suggested levels:

```text
ERROR
WARN
INFO
DEBUG
```

Release builds default to INFO or WARN.

Never log secrets.

Avoid logging huge EFT log contents.

Log parsed events, not entire files.

---

# 26. Recommended Project Structure

```text
eft-region-watcher/
|
|-- Cargo.toml
|-- Cargo.lock
|-- AGENTS.md
|-- README.md
|-- LICENSE
|
|-- crates/
|   |
|   |-- app/
|   |   |-- src/
|   |       |-- main.rs
|   |       |-- tray.rs
|   |       |-- window.rs
|   |       |-- notifications.rs
|   |
|   |-- eft/
|   |   |-- src/
|   |       |-- lib.rs
|   |       |-- log_finder.rs
|   |       |-- watcher.rs
|   |       |-- parser.rs
|   |       |-- connection.rs
|   |
|   |-- geo/
|   |   |-- src/
|   |       |-- lib.rs
|   |       |-- provider.rs
|   |       |-- cache.rs
|   |
|   |-- updater/
|   |   |-- src/
|   |       |-- main.rs
|   |
|   |-- update-core/
|       |-- src/
|           |-- checker.rs
|           |-- github.rs
|           |-- version.rs
|           |-- downloader.rs
|           |-- integrity.rs
|
|-- assets/
|
|-- .github/
|   |-- workflows/
|       |-- build.yml
|       |-- release.yml
```

A simpler workspace layout is acceptable if module boundaries remain clear.

---

# 27. Architectural Boundaries

The following responsibilities must remain separated.

## EFT module

Responsible for:

- finding logs;
- watching logs;
- parsing EFT connection information.

Must not depend on GUI code.

## Geo module

Responsible for:

- IP geolocation;
- provider abstraction;
- caching.

Must not depend on EFT parsing logic.

## Update module

Responsible for:

- GitHub release lookup;
- semantic version comparison;
- release download;
- digest verification.

Must not depend on EFT parsing.

## UI module

Responsible for:

- window;
- tray;
- notifications;
- user interaction.

UI must consume state exposed by the underlying modules.

---

# 28. Threading Model

Keep thread count low.

Suggested model:

```text
Main/UI thread
    |
    +-- filesystem watcher worker
    |
    +-- short-lived network task when required
```

Do not create a large async runtime unless justified by measured benefit.

A lightweight async executor is acceptable only if total footprint remains within budget.

Prefer blocking system calls on dedicated lightweight worker threads where appropriate.

---

# 29. Startup Sequence

Recommended startup:

```text
1. Initialize logging.
2. Load configuration.
3. Create tray icon.
4. Locate EFT logs.
5. Start filesystem watcher.
6. Restore cached last-server state if useful.
7. If update interval elapsed:
       perform update check asynchronously.
8. Return immediately to normal idle state.
```

The UI should become responsive before network checks complete.

---

# 30. Shutdown Sequence

On application exit:

```text
1. Stop filesystem watcher.
2. Flush small configuration changes.
3. Remove tray icon.
4. Close handles.
5. Exit cleanly.
```

Do not leave background processes behind.

---

# 31. Startup with Windows

Optional setting:

```text
Start EFTRegionWatcher with Windows
```

Default:

```text
OFF
```

If enabled, use a standard per-user startup mechanism.

Do not require admin privileges.

Do not create a Windows service.

---

# 32. Security Rules

Treat all external data as untrusted:

- EFT logs;
- GeoIP responses;
- GitHub API responses;
- release notes;
- downloaded release assets.

Validate:

- IP strings;
- port ranges;
- semantic versions;
- filenames;
- update URLs;
- SHA-256 digests.

Do not allow update metadata to specify arbitrary local filesystem destinations.

Downloads must only be written to controlled application temporary paths.

Avoid command-shell execution.

Prefer direct process creation APIs with explicit executable paths and arguments.

---

# 33. Privacy

EFTRegionWatcher should collect no analytics by default.

Do not add:

- telemetry;
- tracking IDs;
- crash upload services;
- advertising;
- user identifiers.

Any future telemetry requires an explicit design change and opt-in.

---

# 34. User Experience Principles

The target user may be poor at PC operation.

Therefore:

- use plain Japanese;
- minimize configuration;
- automatically discover reasonable defaults;
- avoid technical error codes as the primary message;
- do not require users to visit GitHub for ordinary updates;
- avoid asking users to extract ZIP files manually;
- avoid requiring command-line usage;
- avoid requiring admin privileges.

The expected friend workflow is:

```text
First launch
    |
    +--> double-click EFTRegionWatcher.exe
    |
    +--> application starts

Normal use
    |
    +--> do nothing

Update available
    |
    +--> click "Update"
    |
    +--> application updates itself
```

---

# 35. V1 Feature Scope

V1 MUST include:

- Windows 11 x64 support;
- native lightweight application;
- Windows tray icon;
- EFT log directory detection;
- manual log directory selection;
- event-driven log watching;
- current `Connect (address: IP:PORT)` parser;
- legacy parser fallback;
- connection state;
- IP display;
- port display;
- estimated location display;
- GeoIP cache;
- copy IP action;
- optional connection notification;
- settings;
- update checking;
- optional update installation;
- "Later";
- "Skip this version";
- updater executable;
- SHA-256 validation;
- public GitHub Releases support.

---

# 36. Explicitly Out of Scope for V1

Do NOT add unless all V1 requirements are stable and performance targets are met:

- in-game overlay;
- DirectX hooks;
- game memory reading;
- packet capture;
- map UI;
- animated charts;
- ping history graphs;
- server performance analytics;
- account system;
- cloud sync;
- auto telemetry;
- auto-update without user approval;
- installer framework unless clearly needed.

Keep V1 small.

---

# 37. Testing Requirements

Implement automated tests for parsing and version logic.

Minimum parser tests:

```text
valid current IPv4 endpoint
different ports
multiple connections
partial line input
malformed IP
missing port
legacy entry
unrelated EFT log lines
```

Minimum update tests:

```text
installed == latest
installed < latest
installed > latest
prerelease ignored
draft ignored
skipped version ignored
newer version after skipped version is shown
malformed GitHub response
download checksum mismatch
```

Filesystem watcher behavior should be integration tested where practical.

---

# 38. Performance Tests

Before calling V1 complete, measure on Windows 11:

```text
idle CPU
idle memory
CPU during EFT log update
memory after several raids
file handle count
thread count
network requests per hour
```

Look for:

- memory leaks;
- duplicate watchers;
- runaway log growth;
- accidental polling loops;
- repeated GeoIP queries;
- repeated GitHub update checks.

---

# 39. Build Requirements

Release build:

```bash
cargo build --release
```

Windows target:

```text
x86_64-pc-windows-msvc
```

Prefer static/self-contained output where legally and technically appropriate.

Do not require users to install Rust.

Do not require users to install Visual Studio.

End users should receive ready-to-run release artifacts.

---

# 40. GitHub Actions

Create CI workflows.

## Pull request / push workflow

Run:

```text
cargo fmt --check
cargo clippy
cargo test
cargo build --release
```

## Release workflow

When a version tag such as:

```text
v1.2.0
```

is pushed:

1. build Windows x64 release;
2. run tests;
3. package app/updater;
4. calculate SHA-256;
5. attach artifacts to GitHub Release.

Do not publish release artifacts if tests fail.

---

# 41. README Requirements

README should include:

- what EFTRegionWatcher does;
- screenshot;
- download instructions;
- simple usage instructions;
- update behavior;
- privacy statement;
- antivirus false-positive note if needed;
- troubleshooting;
- uninstall instructions;
- unofficial-project disclaimer.

Recommended disclaimer:

```text
EFTRegionWatcher is an unofficial community tool and is not affiliated with,
endorsed by, or sponsored by Battlestate Games.
```

Do not imply official EFT support.

---

# 42. Definition of Done — V1

V1 is complete only when all of the following are true.

- The app runs on Windows 11 x64.
- No admin privileges are required.
- EFT log folder can be discovered or manually selected.
- A real EFT connection log can produce an IP and port.
- GeoIP result is displayed.
- GeoIP failure does not break endpoint detection.
- The app can remain in the tray.
- Idle CPU usage is negligible.
- No packet capture is used.
- No EFT process memory access is used.
- No DLL injection or hooks are used.
- Public GitHub Releases can be checked.
- An available stable update produces a prompt.
- User can choose Update.
- User can choose Later.
- User can Skip the specific version.
- Update is never forced.
- SHA-256 is checked before replacement.
- A failed update leaves the old installation usable.
- Tests pass.
- Release artifacts can be produced through CI.
- README explains basic usage for nontechnical users.

---

# 43. Agent Implementation Order

Agents should implement in the following order.

```text
Phase 1
Project skeleton
    |
Phase 2
EFT log discovery
    |
Phase 3
Filesystem watcher
    |
Phase 4
Connection parser
    |
Phase 5
Connection state
    |
Phase 6
Minimal Win32 UI + tray
    |
Phase 7
GeoIP + cache
    |
Phase 8
Settings
    |
Phase 9
GitHub update checker
    |
Phase 10
Updater executable
    |
Phase 11
Checksum verification
    |
Phase 12
CI / GitHub Releases
    |
Phase 13
Performance profiling
    |
Phase 14
README / release packaging
```

Do not begin optional feature expansion before the core pipeline is reliable.

---

# 44. Agent Decision Rule

Whenever there is a tradeoff between:

```text
more visual polish
```

and

```text
lower runtime overhead
```

choose lower runtime overhead.

Whenever there is a tradeoff between:

```text
clever automation
```

and

```text
lower risk of interfering with EFT
```

choose lower interference.

Whenever there is a tradeoff between:

```text
more configuration
```

and

```text
a simple safe default
```

choose the simple safe default.

The defining characteristics of EFTRegionWatcher are:

**small, native, passive, understandable, and reliable.**
