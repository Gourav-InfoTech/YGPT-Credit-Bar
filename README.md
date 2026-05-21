# YGPTCreditBar

A macOS menu bar app that shows YourGPT credit usage at a glance. Inspired by CodexBar.

## Features

- Live quota usage for 8 buckets: Credits, Voice Credits, Voice Lite Credits, Campaign Credits, Chatbots, Documents, Webpages, Team Members
- Tray icon color reflects worst quota state (green / yellow / red)
- Native macOS notifications when buckets cross 80%, 95%, 100%
- Resets-in countdown derived from the org's current billing cycle
- Hidden from the Dock, pure menu bar app
- Token stored in the macOS Keychain
- Multi-organization aware (pick which org to monitor in Settings)

## Tech stack

- Tauri 2 (Rust backend, vanilla HTML/CSS/JS popover)
- Rust crates: `tauri`, `reqwest`, `tokio`, `keyring`, `serde`
- Built-in plugins: `tauri-plugin-notification`, `tauri-plugin-opener`, `tauri-plugin-positioner`

## Quick start

```bash
pnpm install
pnpm tauri dev
```

To build a release dmg:

```bash
pnpm tauri build
```

## Configure

1. Sign in at https://dashboard.yourgpt.ai
2. Open Settings → API Tokens, create a new token (it starts with `api-v1-`)
3. Click the tray icon → Settings…
4. Paste the token, pick your organization, optionally adjust the refresh interval
5. Save

## Architecture

```
src-tauri/src/
├── lib.rs         # Tauri builder, tray, plugin wiring
├── main.rs        # entry point
├── models.rs      # Bucket / Usage / Snapshot / Severity
├── state.rs       # AppState (snapshot + settings)
├── keychain.rs    # macOS Keychain wrapper
├── api.rs         # YourGPT REST client
├── poller.rs      # background poll loop + threshold notifications
├── tray.rs        # generates colored circle icons at runtime
└── commands.rs    # Tauri commands exposed to JS

src/
├── index.html     # popover layout
├── settings.html  # settings window
├── popover.js     # popover frontend logic
├── settings.js    # settings frontend logic
└── styles.css     # CodexBar-inspired dark theme
```

## API endpoints used

- `POST /api/v1/getMyOrganizations` to list user's organizations during setup
- `POST /api/v1/getOrgPlanDetail` as the primary poll endpoint for plan + usage + subscription

Auth: `Authorization: Bearer api-v1-...` per the PlatformApiAuth middleware in the YourGPT core API.

## License

Internal hackathon project.
# YGPT-Credit-Bar
