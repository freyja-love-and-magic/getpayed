# Gelder - Development Documentation

## Overview

Gelder is a Planet Nine native invoicing app: a freelancer writes an invoice, shares it (via the system share sheet or a copyable link), and the recipient pays it online through a real Stripe-backed payment page. There is no separate "save" step — publishing the invoice (creating its BDO record) *is* what saves it.

**Location**: `/gelder/app/`
**Identifier**: `com.freyja.gelder`
**Stack**: Tauri 2 (Rust core in `src-tauri/`, vanilla JS/HTML/CSS frontend in `src/`), targeting iOS.
**Status**: Actively developed / TestFlight builds (build 15 as of September 2026).

## Architecture

### Frontend (`app/src/`)

Three files, no framework or bundler:
- `index.html` — five views as sibling `<section>`s toggled via a `hidden` attribute: `list-view`, `create-view`, `detail-view`, `payouts-view`, `profile-view`.
- `main.js` — all view logic, calling into the Rust backend via `core.invoke(...)`.
- `style.css` — dark/green/purple visual language shared across the Planet Nine app family.

### Backend (`app/src-tauri/src/lib.rs`)

Tauri commands, invoked from `main.js`:

| Command | Purpose |
|---|---|
| `load_invoices` / `create_invoice` | Local invoice store (`invoices.json` in the app data dir) + publish to BDO |
| `mark_invoice_paid` | Manual "I got paid another way" override |
| `check_payment_status` | Polls eumachia for real online-payment status |
| `get_payout_status` / `connect_stripe_account` | Stripe Express onboarding via Addie |
| `load_canonical_profile` / `save_canonical_profile` | Shared cross-app profile (App Group) |

No server of its own — Gelder is a thin client over three allyabase services, all currently pointed at `https://allyabase-gateway-12345.netlify.app/`:
- **BDO** (`GATEWAY_BDO_URL`) — publishes each invoice as its own public record (own sessionless keypair per invoice, `BDO_HASH = "gelder-invoice"`), and the shared canonical-profile-adjacent data.
- **Addie** (`GATEWAY_ADDIE_URL`) — mints a Stripe Express connected account for payouts.
- **eumachia** (`EUMACHIA_PAY_URL`, at `allyabase/deployment/eumachia`) — the *only* piece of this stack that can render a real, interactive "Pay Now" page (savage strips all JavaScript from what it serves, so it can't host a Stripe Elements checkout). Eumachia owns the payment-status record and the actual PaymentIntent/payout flow.

`GATEWAY_ENV = "test-12345"` namespaces every cached uuid (invoice's BDO uuid, Addie identity) so pointing the app at a different deployment later doesn't collide with or silently lose what's already published under this one.

### Invoice lifecycle

1. **Create** (`create_invoice`): mints a per-invoice BDO keypair, calls `create_user` to get a uuid, computes two permanent links — `share_url` (a savage-rendered SVG view) and `pay_url` (eumachia's interactive pay page, pre-signed with the same read hash/timestamp/signature scheme used for savage share links) — then republishes the full record with both links baked in. `creator_addie_pub_key` is snapshotted onto the invoice *only* if Stripe is already connected at creation time; if absent, eumachia has nowhere to route a payout split.
2. **Share**: the OS share sheet gets `share_url`.
3. **Pay**: the recipient opens `pay_url`, served by eumachia (see `allyabase/deployment/eumachia/src/server/node/eumachia.js`) — a self-contained HTML page mounting a Stripe Payment Element, with a sticky "Confirm Payment" footer, that POSTs to eumachia's own `/pay/:uuid/intent` and `/pay/:uuid/complete` routes. Handles both immediate confirmation and the redirect-return path (3D Secure etc.) via Stripe's `redirect_status` query param.
4. **Reconcile**: Gelder doesn't get pushed a "paid" event — the user taps **Check Payment** in the detail view, which polls eumachia's `/pay/:uuid/status` and, if paid, republishes the invoice's own BDO record with `paid: true` (`republish_invoice`, shared with the manual "Mark as Paid" path).

### Stripe connection gating (added September 2026)

Invoice creation is gated on having a connected Stripe account — an invoice created without one never gets a payout split (see `creator_addie_pub_key` above and `payOutCreator` in eumachia's `payments.js`), so a payer's money would have nowhere to go. The list view shows an explanatory banner + "Connect Stripe Account" CTA in place of "+ New Invoice" until `get_payout_status` reports connected; `openCreateForm()` in `main.js` also guards this server-side-truth-independent of button state, redirecting to the Payouts view if called while disconnected.

Stripe onboarding itself (`connect_stripe_account`) creates an Addie identity on first use, requests a Stripe **Express** account (not the plain/company endpoint — that's for platform revenue splits and produces no hosted onboarding page), then opens Stripe's hosted onboarding in the system browser. Because Addie marks the account "connected" the instant it's created (not once onboarding is actually finished), the frontend tracks a local `stripeOnboardingPending` flag that takes priority over that premature signal until the user confirms completion — either via the `gelder://stripe-return` deep link (Stripe's return URL) or the manual "I've Finished Onboarding" fallback button.

### Canonical Profile (shared across apps)

A separate record from invoices entirely, synced via the **`group.freyja.idothis`** iOS App Group — the same group BizBuz, Linkitylink, Gettit, and Letemcook read/write. Read/written through `tauri-plugin-app-group`'s `read_value_sync`/`write_value_sync` under the key `canonical.profile`. The Rust struct includes an `address` field Gelder has no UI for (Gettit does) — `save_canonical_profile` always carries forward whatever address is already stored rather than clobbering it with `None`, since every app that touches this record overwrites the whole thing on save. This logic is intentionally copy-pasted byte-for-byte across the sibling apps rather than shared as a library — it's thin OS-API wrapping with no business logic to diverge.

## Build & Deploy

`app/package.json` scripts:
- `npm run dev` — `tauri dev`
- `npm run build` — desktop `tauri build`
- `npm run build:ios` — `scripts/build-ios.cjs`, the real distribution path (produces a signed IPA)
- `npm run ios:dev` — `tauri ios dev`

### `scripts/build-ios.cjs`

1. Bumps `.build-number` (App Store Connect rejects re-uploading the same `CFBundleVersion`).
2. Wipes and regenerates `src-tauri/gen/apple/` via `tauri ios init` — a clean slate every time, which means...
3. ...several things get patched back in immediately after, since they don't survive that regeneration: the `ios-native/` source path (carries `PrivacyInfo.xcprivacy`), `TARGETED_DEVICE_FAMILY` restricted to iPhone-only, `ITSAppUsesNonExemptEncryption: false` (skips the App Store Connect encryption questionnaire), the real app icon (`tauri ios init` reverts to Tauri's stock icon), flattened alpha channels on all icon sizes (App Store rejects an alpha channel on the 1024×1024 marketing icon), and the App Group entitlement (`group.freyja.idothis`).
4. `tauri ios build --export-method app-store-connect`, with a manual `xcodebuild -exportArchive` fallback for a known Xcode 26 export-plist quirk.
5. Copies the resulting IPA to `builds/v{version}/{ProductName}-{buildNumber}.ipa`.

Upload to App Store Connect is a deliberately separate, manual step (Transporter.app or `xcrun altool`) — the script never uploads anything itself.

**Known gotcha**: `xcodegen generate` (invoked inside this script) resolves `${FORCE_COLOR}` in `project.yml`'s Run Script phase eagerly, using whatever `FORCE_COLOR` is set to in the *generating* shell's environment — not deferred to Xcode's build-time environment like the other `${VAR}` tokens in that same script. If `FORCE_COLOR` happens to be set (e.g. a terminal/tool that forces colored output), its value gets baked in as a stray positional argument before the real `${ARCHS}` value, which corrupts `tauri ios xcode-script`'s architecture parsing and fails the build with a misleading `Arch specified by Xcode was invalid` error — not a real toolchain or Xcode-version problem. Fix: `unset FORCE_COLOR` (and `COLORTERM`, to be safe) before running `npm run build:ios`.

There's also a top-level `~/Work/planet-nine/builds/` folder holding one "latest" IPA per sibling app (Gelder, BizBuz, Gettit, IDothis, Letemcook, Linkitylink) — this is a manually-maintained convenience copy, not written by any build script. After a real build, copy the new IPA there too if it should replace the one currently sitting in that folder.

## Known Limitations (documented, not oversights)

- **No payment webhook verification**: `/pay/:uuid/complete` (in eumachia) trusts the payer's browser telling it Stripe's `confirmPayment()` succeeded — Addie exposes no server-side webhook/verification route to check against. Acceptable for a personal invoicing tool, not a high-stakes ledger.
- **Payment status is pull, not push**: Gelder never gets notified when an invoice is paid online; the user must open the invoice and tap "Check Payment."
- **Read-modify-write race on eumachia's payment record**: concurrent payments could theoretically race on eumachia's own BDO payments map. Acceptable at this scale.

## Related Documentation

- eumachia (the pay-page backend): `/allyabase/deployment/eumachia/src/server/node/eumachia.js`, `src/payments/payments.js`, `src/invoices/invoices.js`
- Sibling apps sharing the same Tauri scaffolding and App Group: BizBuz, Linkitylink, Gettit, Letemcook, idothis (see their own `CLAUDE.md`)

## Last Updated
September 2, 2026 — Documented current architecture (this file didn't exist before); added the Stripe-connection invoice-creation gate and the eumachia payment-page fixes (duplicate click-handler, input lock during confirmation, sticky footer, redirect-return completion) from this session.
