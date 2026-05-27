<p align="center">
  <img src="src-tauri/icons/app-icon.png" width="160" alt="YGPTCreditBar app icon" />
</p>

<h1 align="center">YGPTCreditBar</h1>

<p align="center">
  A macOS menu-bar app that surfaces your YourGPT credit and quota usage at a glance.<br/>
  Inspired by CodexBar.
</p>

The bar-chart icon in your menu bar turns yellow as you approach plan limits, then red when you're about to exhaust them. Click it for the full breakdown by bucket. Native macOS banners fire automatically when a limit is crossed, so you can keep working in another app and still hear about it.

## Download

Apple Silicon Macs (macOS 12+):
**[Download the latest `.dmg`](https://github.com/Gourav-InfoTech/YGPT-Credit-Bar/releases/latest)**

The build is currently unsigned. After dragging into Applications, run this once to bypass Gatekeeper:

```bash
xattr -dr com.apple.quarantine /Applications/YGPTCreditBar.app
```

Then double-click to open normally. macOS will ask once for notification permission, click Allow.

## Features

**Hero quota card** showing the bucket closest to its limit, with a severity-tinted glow (green / yellow / red) and a contextual label that scales with usage: HIGHEST USAGE → APPROACHING LIMIT → NEAR LIMIT → AT LIMIT.

**Eight quota buckets** tracked: Credits, Voice Credits, Voice Lite Credits, Campaign Credits, Chatbots, Documents, Webpages, Team Members. Numbers are abbreviated as K / M / B for readability.

**Subscription summary** with plan name, status (Active / Trial / Past due / Canceled / etc.), and the next payment or trial-expiry date.

**Multi-organization support.** If you belong to multiple YourGPT orgs, the app fetches usage for all of them on every poll cycle. Native banners include the org name in the title, so when "Acme Inc Credits hit 100%" you know exactly which account is affected.

**Per-bucket consequence copy** in notifications. Instead of a generic "Credits at 100%", the banner explains what stops working: "Credits at 100%. Your chatbot can't answer messages." Count-cap buckets (Chatbots / Documents / etc.) say "Upgrade to add more."

**Native macOS banners** at 80%, 95%, and 100% per bucket, with per-(org, bucket, threshold) dedup so you don't get noise. A bucket already at 100% on app launch fires one banner, not three. Past-due subscription transitions trigger a separate synthesised banner.

**Native macOS glass** via NSVisualEffectView for the popover background.

**Hidden from the Dock**, pure menu-bar app.

**Token stored in the macOS Keychain**, never on disk.

## Setup

1. Sign in at [chatbot.yourgpt.ai](https://chatbot.yourgpt.ai).
2. Open DevTools (⌥⌘I) → **Application** → **Cookies** → find the auth cookie and copy its value.
3. Click the YGPTCreditBar icon in the menu bar → **Settings…**
4. Paste the token, pick your organization, optionally adjust the refresh interval (15–300s, default 30).
5. Click **Save**.

The app starts polling immediately. Usage refreshes every interval, native banners fire when a threshold is newly crossed.

## Develop

Prerequisites:

- macOS 12+ (Apple Silicon)
- Rust toolchain (`brew install rust` or via [rustup](https://rustup.rs))
- Node.js 18+ and pnpm
- `librsvg` for icon rasterisation (`brew install librsvg`) — only needed if you regenerate the app icon

Clone and run:

```bash
git clone https://github.com/Gourav-InfoTech/YGPT-Credit-Bar.git
cd YGPT-Credit-Bar
pnpm install
pnpm tauri dev
```

> **Note on `tauri dev`:** macOS Notification Center won't deliver banners to the unbundled dev binary. To test notifications, build the `.app`:
> ```bash
> pnpm tauri build
> open src-tauri/target/release/bundle/macos/YGPTCreditBar.app
> ```

Build a release DMG:

```bash
pnpm tauri build
```

Output lands at `src-tauri/target/release/bundle/dmg/YGPTCreditBar_<version>_aarch64.dmg`.

## API endpoints used

All requests go to `https://api.yourgpt.ai` with `Authorization: Bearer <jwt>` and `app_id: "1"` auto-injected into the body.

| Endpoint | Used for |
| --- | --- |
| `POST /api/v1/getMyOrganizations` | Listing orgs on setup and on every poll cycle |
| `POST /api/v1/getOrgPlanDetail` | Per-org plan, subscription, usage, trial expiry, past-due flag |
| `POST /chatbot/v1/getMyNotification` | Filtering server-side billing notifications (CREDIT_LIMIT_*, PAST_DUE_SUBSCRIPTION, VOICE_CREDIT_*) for native banner mirroring |

Authentication: JWT taken from the dashboard's auth cookie. PATs (`api-v1-…`) are recognised by the YourGPT platform but the endpoints we need are gated to JWT-only.

## Architecture

```
src-tauri/src/
├── lib.rs         Tauri builder, tray icon, popover positioning, window event handlers
├── main.rs        Thin entry point
├── api.rs         YourGPT HTTP client: list_orgs, get_plan_detail, get_notifications
├── models.rs      Bucket, Usage, Severity, Snapshot, Notification serde structs
├── state.rs       AppState (snapshot, settings, fired_thresholds dedup set)
├── keychain.rs    Wraps the `keyring` crate for macOS Keychain storage
├── poller.rs      Background poll loop, threshold checks, per-org native banners
├── tray.rs        Rasterises the SVG logo at runtime and tints it by severity
└── commands.rs    Tauri commands invoked from the popover JS

src/
├── index.html     Popover layout (hero card + "Other quotas" + subscription + actions)
├── settings.html  Settings window (JWT input, org picker, refresh interval)
├── popover.js     Render loop, org switcher, action handlers, event listeners
├── settings.js    Token validation, org-list loading, save flow
├── notifications  Removed in v0.1.3 (native banners only, no in-app inbox)
└── styles.css     Dark theme, hero gradients, severity colors
```

## Versioning + release flow

Versions follow [semver](https://semver.org). Bump in three places before each release: `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, and the on-screen badge in `src/settings.html`. See [recent releases](https://github.com/Gourav-InfoTech/YGPT-Credit-Bar/releases) for what's shipped.

Standard release flow:

```bash
# After bumping the three version files:
pnpm tauri build
git add -u && git commit -m "release: vX.Y.Z - summary"
git push origin main
git tag -a vX.Y.Z -m "Release vX.Y.Z"
git push origin vX.Y.Z
gh release create vX.Y.Z \
  --title "vX.Y.Z — short description" \
  --notes "Release notes here." \
  src-tauri/target/release/bundle/dmg/YGPTCreditBar_X.Y.Z_aarch64.dmg
```

## License

Internal Delta4 Infotech project. Not open source yet.
