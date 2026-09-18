use addie_rs::Addie;
use bdo_rs::BDO;
use serde::{Deserialize, Serialize};
use sessionless::hex::IntoHex;
use sessionless::secp256k1::SecretKey;
use sessionless::Sessionless;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::Manager;

// Path-based routing on the dev.8as.world droplet: nginx terminates TLS on
// 443 and proxies /<service>/ to that service's local port (bdo → 3003,
// addie → 3005). One hostname, one certificate, everything over real HTTPS
// — which is also what keeps iOS ATS happy, since the services themselves
// speak plain HTTP and are not reachable directly from outside.
const GATEWAY_BDO_URL: &str = "https://dev.8as.world/bdo/";
// NOTE: nginx has no /savage/ route yet, so publishing works but the share
// link this produces 404s until that route is added server-side.
const SAVAGE_URL: &str = "https://dev.8as.world/savage/";
// eumachia is the only piece of this stack that can render a real,
// interactive "Pay Now" page — savage strips all JavaScript from whatever
// it serves, so it can't host a Stripe checkout itself (verified this
// session while designing eumachia).
// NOTE: nginx has no /eumachia/ route yet either — pay links 404 until it does.
const EUMACHIA_PAY_URL: &str = "https://dev.8as.world/eumachia/pay/";
const BDO_HASH: &str = "gelder-invoice";

// Both BDO and Addie mint their own server-side `uuid`s, distinct from the
// local keypair used to sign requests — a uuid minted against one gateway
// 404s on another (see connect_stripe_account's and republish_invoice's
// comments for how that actually surfaced). Everything that caches one of
// those uuids (the Addie identity, an invoice's BDO record) keys its storage
// by this const instead of overwriting a single value, so switching envs
// never touches or loses whatever already existed under a previous one.
// Bump this whenever GATEWAY_BDO_URL/GATEWAY_ADDIE_URL point at a genuinely
// different deployment (not for e.g. a same-deployment code redeploy).
const GATEWAY_ENV: &str = "dev-8as-world";

fn default_currency() -> String {
    "usd".to_string()
}

// ── Data types ───────────────────────────────────────────────────────────────

/// Whether a document is a billable Invoice or a prospective Estimate.
/// Both share the same struct, differ in the status transitions we allow
/// and the actions we surface. Estimates can be converted into invoices;
/// invoices cannot become estimates.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    Invoice,
    Estimate,
}

impl Default for DocumentKind {
    fn default() -> Self { DocumentKind::Invoice }
}

/// Lifecycle state for an invoice or estimate. The variants naturally
/// partition by kind:
///
/// - Invoice: `Pending` (default) → one of `PaidStripe`, `PaidManual`,
///   `Waived`, `Canceled`. The Pending → Paid transition is either
///   automatic (eumachia poll → `PaidStripe`) or manual
///   (`set_invoice_status` → any of the others). Any terminal state
///   can be moved back to `Pending` via the same command.
/// - Estimate: `Active` (default for estimates) → `Converted` when
///   `convert_estimate_to_invoice` runs and stamps the estimate's
///   `converted_to_invoice_id`.
///
/// "Past due" is deliberately NOT a stored variant — it's derived at
/// render time from `Pending + due_date < now`, so it can never get
/// out of sync with the clock.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceStatus {
    Pending,
    PaidStripe,
    PaidManual,
    Waived,
    Canceled,
    Active,
    Converted,
}

impl Default for InvoiceStatus {
    fn default() -> Self { InvoiceStatus::Pending }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Invoice {
    #[serde(default)]
    pub id: String,
    /// Invoice by default (so records written before the estimate feature
    /// existed keep working). Set to `Estimate` for prospective quotes.
    #[serde(default)]
    pub kind: DocumentKind,
    pub description: String,
    pub amount_cents: u64,
    #[serde(default = "default_currency")]
    pub currency: String,
    /// Snapshotted from the Canonical Profile at creation time — not
    /// live-linked, so past invoices don't retroactively change if the
    /// profile is edited later.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_name: Option<String>,
    /// BDO identity uuid this invoice is published under, per environment
    /// (see `GATEWAY_ENV`) — an invoice only ever gets an entry for the env
    /// it was actually created under; it is never re-published fresh under
    /// a later env (that would mint an unreachable record nobody has the
    /// link to — see `republish_invoice`).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub bdo_uuid_by_env: HashMap<String, String>,
    /// Permanent savage URL for VIEWING the invoice (the SVG rendering).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_url: Option<String>,
    /// Permanent eumachia URL for PAYING the invoice online — a real,
    /// interactive page, distinct from the static savage view above.
    /// Estimates leave this None; they aren't payable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pay_url: Option<String>,
    /// Snapshotted from the connected Addie payout identity at creation
    /// time, same reasoning as `from_name` — if absent, eumachia sends
    /// 100% of the charge to itself (today's behavior); if present,
    /// eumachia requests a 91%/9%-minus-fees split with this pubkey as
    /// the merchant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_addie_pub_key: Option<String>,
    /// Lifecycle state — see `InvoiceStatus`.
    #[serde(default)]
    pub status: InvoiceStatus,
    /// Optional user-set due date (unix ms as a string, matching the other
    /// timestamps). Only invoices use it; drives the derived "past due"
    /// display state in the UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    /// Free-text reason attached to Waived/Canceled/PaidManual (or empty).
    /// Kept alongside the status transition so the "why" isn't lost.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paid_at: Option<String>,
    /// Set on an Estimate when `convert_estimate_to_invoice` runs — points
    /// at the id of the invoice it spawned. Provides the bidirectional
    /// link (estimate → invoice); the invoice itself has no back-pointer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub converted_to_invoice_id: Option<String>,
    /// When the invoiced work was actually performed — user-entered at
    /// creation time (defaults to "now" in the UI, but is editable), unlike
    /// `created_at` below which is always exactly when the record was made.
    /// The invoice list sorts by this, since it's what a freelancer
    /// actually cares about ordering by.
    #[serde(default)]
    pub work_performed_at: String,
    #[serde(default)]
    pub created_at: String,
    /// Legacy pre-status field. Old records had `{paid: true}` with no
    /// `status`; `migrate_legacy_status` upgrades those to `PaidStripe`
    /// on read. `skip_serializing` means we never write it back — new
    /// records only have `status`.
    #[serde(default, rename = "paid", skip_serializing)]
    pub legacy_paid: bool,
}

impl Invoice {
    /// One-shot migration from old `{paid: true/false}` records to the new
    /// status field. Old paid==true becomes `PaidStripe` (the check-payment
    /// poll and the manual "Mark Paid" both set paid=true in the old
    /// schema, so we can't distinguish — Stripe is the more common origin,
    /// and the "Paid" display stays the same either way). Idempotent:
    /// records already carrying a non-default status are left alone.
    fn migrate_legacy_status(&mut self) {
        if self.legacy_paid && matches!(self.status, InvoiceStatus::Pending) {
            self.status = InvoiceStatus::PaidStripe;
        }
        self.legacy_paid = false;
    }

    /// Convenience — true for any status that means "no more money coming
    /// via Stripe": the four terminal invoice states plus estimate states.
    fn is_terminal(&self) -> bool {
        !matches!(self.status, InvoiceStatus::Pending)
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct InvoicesStore {
    invoices: Vec<Invoice>,
}

// ── Storage ──────────────────────────────────────────────────────────────────

fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn invoices_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("invoices.json"))
}

fn read_invoices(app: &tauri::AppHandle) -> Result<InvoicesStore, String> {
    let path = invoices_path(app)?;
    match fs::read_to_string(&path) {
        Ok(contents) => {
            let mut store: InvoicesStore =
                serde_json::from_str(&contents).map_err(|e| e.to_string())?;
            for inv in &mut store.invoices {
                inv.migrate_legacy_status();
            }
            Ok(store)
        }
        Err(_) => Ok(InvoicesStore::default()),
    }
}

fn write_invoices(app: &tauri::AppHandle, store: &InvoicesStore) -> Result<(), String> {
    let path = invoices_path(app)?;
    let json = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

fn new_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let counter = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}", nanos, counter)
}

fn unix_now_ms_string() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_default()
}

// ── BDO publishing ───────────────────────────────────────────────────────────
//
// Same pattern as BizBuz's cards: BDO's public storage is keyed by pubKey,
// not by hash, so each invoice gets its own sessionless keypair (a shared
// hash constant is fine — it's the pubKey that disambiguates records).

#[derive(Debug, Serialize, Deserialize, Clone)]
struct BdoKeypair {
    private_key_hex: String,
    pub_key_hex: String,
}

fn bdo_keys_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("bdo_keys.json"))
}

fn read_bdo_keys(app: &tauri::AppHandle) -> HashMap<String, BdoKeypair> {
    let path = match bdo_keys_path(app) {
        Ok(p) => p,
        Err(_) => return HashMap::new(),
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_bdo_keys(app: &tauri::AppHandle, keys: &HashMap<String, BdoKeypair>) -> Result<(), String> {
    let path = bdo_keys_path(app)?;
    let json = serde_json::to_string_pretty(keys).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

fn sessionless_from_hex(priv_key_hex: &str) -> Result<Sessionless, String> {
    let bytes = hex::decode(priv_key_hex).map_err(|e| e.to_string())?;
    let secret_key = SecretKey::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(Sessionless::from_private_key(secret_key))
}

fn load_or_create_bdo_sessionless(app: &tauri::AppHandle, invoice_id: &str) -> Result<Sessionless, String> {
    let mut keys = read_bdo_keys(app);
    if let Some(existing) = keys.get(invoice_id) {
        return sessionless_from_hex(&existing.private_key_hex);
    }

    let session = Sessionless::new();
    keys.insert(
        invoice_id.to_string(),
        BdoKeypair {
            private_key_hex: session.private_key().to_hex(),
            pub_key_hex: session.public_key().to_hex(),
        },
    );
    write_bdo_keys(app, &keys)?;
    Ok(session)
}

// ── Addie payout identity ────────────────────────────────────────────────────
//
// Unlike the per-invoice BDO keys above, this is a single, app-wide identity
// — it represents "who gets paid," not a per-record publish key. Connecting
// Stripe is an explicit, opt-in action (it accepts Stripe's ToS on the
// user's behalf via Addie's own tos_acceptance block), so this file only
// ever gains a stripe_account_id after the user deliberately taps "Connect
// Stripe Account" — it is never created implicitly.

const GATEWAY_ADDIE_URL: &str = "https://dev.8as.world/addie/";

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct AddieIdentity {
    private_key_hex: String,
    pub_key_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    addie_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stripe_account_id: Option<String>,
}

fn addie_identity_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("addie_identity.json"))
}

fn read_addie_identities(app: &tauri::AppHandle) -> HashMap<String, AddieIdentity> {
    let path = match addie_identity_path(app) {
        Ok(p) => p,
        Err(_) => return HashMap::new(),
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_addie_identities(app: &tauri::AppHandle, identities: &HashMap<String, AddieIdentity>) -> Result<(), String> {
    let path = addie_identity_path(app)?;
    let json = serde_json::to_string_pretty(identities).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

fn read_addie_identity(app: &tauri::AppHandle) -> Option<AddieIdentity> {
    read_addie_identities(app).get(GATEWAY_ENV).cloned()
}

fn write_addie_identity(app: &tauri::AppHandle, identity: &AddieIdentity) -> Result<(), String> {
    let mut identities = read_addie_identities(app);
    identities.insert(GATEWAY_ENV.to_string(), identity.clone());
    write_addie_identities(app, &identities)
}

/// Ensures a local keypair exists (creating and persisting one on first
/// call), without requiring Addie or Stripe to be involved yet — a bare
/// identity is enough to know "the user's own pubkey," even before they've
/// connected a payout method. Returns the live `Addie` client plus whatever
/// identity state is currently persisted (addie_uuid/stripe_account_id may
/// still be `None`).
fn load_or_create_addie_sessionless(app: &tauri::AppHandle) -> Result<(Addie, AddieIdentity), String> {
    if let Some(identity) = read_addie_identity(app) {
        let sessionless = sessionless_from_hex(&identity.private_key_hex)?;
        return Ok((Addie::new(Some(GATEWAY_ADDIE_URL.to_string()), Some(sessionless)), identity));
    }

    let sessionless = Sessionless::new();
    let identity = AddieIdentity {
        private_key_hex: sessionless.private_key().to_hex(),
        pub_key_hex: sessionless.public_key().to_hex(),
        addie_uuid: None,
        stripe_account_id: None,
    };
    write_addie_identity(app, &identity)?;
    Ok((Addie::new(Some(GATEWAY_ADDIE_URL.to_string()), Some(sessionless)), identity))
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct PayoutStatus {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pub_key: Option<String>,
}

#[tauri::command]
async fn get_payout_status(app: tauri::AppHandle) -> Result<PayoutStatus, String> {
    match read_addie_identity(&app) {
        Some(identity) => Ok(PayoutStatus {
            connected: identity.stripe_account_id.is_some(),
            pub_key: Some(identity.pub_key_hex),
        }),
        None => Ok(PayoutStatus::default()),
    }
}

// Stripe requires an absolute URL here; a custom scheme is accepted. Once
// the hosted onboarding flow finishes (or is abandoned without submitting),
// Stripe navigates Safari to this URL, which iOS hands off to Gelder via
// the `gelder` custom scheme registered in tauri.conf.json — the frontend's
// deep-link listener then clears the onboarding-pending flag and re-checks
// status (see the visibilitychange handler for the desktop/manual-switch
// fallback path).
const STRIPE_ONBOARDING_RETURN_URL: &str = "gelder://stripe-return";

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct StripeConnectResult {
    /// Absent only when Addie found an already-connected account for this
    /// email and skipped onboarding entirely (`already_connected: true`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onboarding_url: Option<String>,
    pub already_connected: bool,
}

/// The one deliberate, explicit action that starts connecting a real Stripe
/// payout destination — creates the Addie identity on first use, then
/// requests a Stripe **Express** account for it (not the plain/company
/// endpoint, which is for platform revenue splits and never produces a
/// hosted onboarding page). The account isn't actually payable yet at this
/// point — `stripe_account_id` gets persisted so a retry doesn't create a
/// duplicate account, but the user still has to complete Stripe's hosted
/// onboarding (opened in the system browser) before payouts can flow.
#[tauri::command]
async fn connect_stripe_account(
    app: tauri::AppHandle,
    country: String,
    email: String,
) -> Result<StripeConnectResult, String> {
    let (client, mut identity) = load_or_create_addie_sessionless(&app)?;

    if identity.addie_uuid.is_none() {
        let user = client.create_user().await.map_err(|e| e.to_string())?;
        identity.addie_uuid = Some(user.uuid);
        write_addie_identity(&app, &identity)?;
    }
    let uuid = identity.addie_uuid.clone().ok_or_else(|| "Addie identity missing uuid".to_string())?;

    let result = client
        .add_processor_express_account(
            &uuid,
            &country,
            &email,
            STRIPE_ONBOARDING_RETURN_URL,
            STRIPE_ONBOARDING_RETURN_URL,
        )
        .await
        .map_err(|e| e.to_string())?;

    identity.stripe_account_id = Some(result.stripe_account_id.clone());
    write_addie_identity(&app, &identity)?;

    Ok(StripeConnectResult {
        onboarding_url: result.stripe_onboarding_url,
        already_connected: result.already_connected,
    })
}

// ── SVG invoice rendering ────────────────────────────────────────────────────

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn format_amount(amount_cents: u64, currency: &str) -> String {
    format!("{} ${:.2}", currency.to_uppercase(), (amount_cents as f64) / 100.0)
}

const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Formats an epoch-ms timestamp string as "Mon D, YYYY" without pulling in
/// a date/time crate — Howard Hinnant's civil_from_days algorithm, the
/// standard dependency-free epoch-day-to-Gregorian-date conversion.
fn format_epoch_ms_date(epoch_ms: &str) -> Option<String> {
    let ms: i64 = epoch_ms.parse().ok()?;
    let days = ms.div_euclid(86_400_000);

    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };

    let month_name = MONTH_NAMES.get((m - 1) as usize)?;
    Some(format!("{month_name} {d}, {year}"))
}

/// Renders an `Invoice` as a self-contained SVG. Mirrors the dark/green/
/// purple visual language every other app this session uses. The "Pay
/// Online" row is a plain `<a href>` to eumachia's real payment page — the
/// SVG itself stays fully static (savage strips scripts), the interactivity
/// lives entirely on the other end of that link.
fn render_invoice_svg(invoice: &Invoice) -> String {
    const WIDTH: u32 = 400;
    // HomeVentory palette applied to the shared/public rendering. Light
    // ground for a professional invoice look on the web (savage view);
    // dark accents for the header/amount so they read as brand-owned.
    const BG: &str = "#F7F9FA";       // glacier white
    const FG: &str = "#1F2933";       // midnight slate
    const MUTED: &str = "#6B7280";     // secondary meta
    const GREEN: &str = "#2E5E4E";     // deep evergreen (CTA / paid badge)
    const RED: &str = "#ff3131";       // canceled / past-due
    const AMBER: &str = "#B87E2A";     // waived
    const BLUE: &str = "#4FA3F7";      // estimate accent

    let cx = WIDTH / 2;
    let mut y: u32 = 60;
    let mut body = String::new();

    let header_label = match invoice.kind {
        DocumentKind::Invoice => "Invoice",
        DocumentKind::Estimate => "Estimate",
    };
    let header_color = match invoice.kind {
        DocumentKind::Invoice => GREEN,
        DocumentKind::Estimate => BLUE,
    };
    body.push_str(&format!(
        r#"<text x="{cx}" y="{y}" font-family="sans-serif" font-size="24" font-weight="bold" fill="{header_color}" text-anchor="middle">{header_label}</text>
"#
    ));

    if let Some(from_name) = invoice.from_name.as_deref().filter(|s| !s.is_empty()) {
        y += 30;
        body.push_str(&format!(
            r#"<text x="40" y="{y}" font-family="sans-serif" font-size="13" fill="{MUTED}">From: {}</text>
"#,
            escape_xml(from_name),
        ));
    }
    if let Some(to_name) = invoice.to_name.as_deref().filter(|s| !s.is_empty()) {
        y += 22;
        body.push_str(&format!(
            r#"<text x="40" y="{y}" font-family="sans-serif" font-size="13" fill="{MUTED}">To: {}</text>
"#,
            escape_xml(to_name),
        ));
    }
    if let Some(work_date) = format_epoch_ms_date(&invoice.work_performed_at) {
        y += 22;
        body.push_str(&format!(
            r#"<text x="40" y="{y}" font-family="sans-serif" font-size="13" fill="{MUTED}">Work performed: {}</text>
"#,
            escape_xml(&work_date),
        ));
    }
    if let Some(due) = invoice.due_date.as_deref().and_then(format_epoch_ms_date) {
        y += 22;
        body.push_str(&format!(
            r#"<text x="40" y="{y}" font-family="sans-serif" font-size="13" fill="{MUTED}">Due: {}</text>
"#,
            escape_xml(&due),
        ));
    }

    y += 50;
    body.push_str(&format!(
        r#"<text x="{cx}" y="{y}" font-family="sans-serif" font-size="34" font-weight="bold" fill="{FG}" text-anchor="middle">{}</text>
"#,
        escape_xml(&format_amount(invoice.amount_cents, &invoice.currency)),
    ));

    y += 40;
    body.push_str(&format!(
        r#"<text x="40" y="{y}" font-family="sans-serif" font-size="15" fill="{FG}">{}</text>
"#,
        escape_xml(&invoice.description),
    ));

    y += 50;
    // Status band — one row that reflects the current lifecycle state.
    // Pending invoices get the interactive Pay Online button; everything
    // else is a badge stamp (no interactivity on savage anyway, JS is
    // stripped from published SVGs — see the module top-of-file note).
    let (badge_text, badge_bg, badge_fg): (Option<&str>, &str, &str) = match invoice.status {
        InvoiceStatus::PaidStripe => (Some("✅ Paid"), "rgba(46,94,78,0.12)", GREEN),
        InvoiceStatus::PaidManual => (Some("✅ Paid (marked manually)"), "rgba(46,94,78,0.12)", GREEN),
        InvoiceStatus::Waived => (Some("Waived"), "rgba(184,126,42,0.15)", AMBER),
        InvoiceStatus::Canceled => (Some("Canceled"), "rgba(255,49,49,0.10)", RED),
        InvoiceStatus::Converted => (Some("Converted to Invoice"), "rgba(107,114,128,0.15)", MUTED),
        InvoiceStatus::Active => (Some("Estimate"), "rgba(79,163,247,0.12)", BLUE),
        InvoiceStatus::Pending => (None, "", ""),
    };
    if let Some(text) = badge_text {
        body.push_str(&format!(
            r#"<rect x="40" y="{}" width="{}" height="48" rx="12" fill="{}" stroke="{}" stroke-width="1"/>
<text x="{cx}" y="{}" font-family="sans-serif" font-size="15" font-weight="bold" fill="{}" text-anchor="middle">{}</text>
"#,
            y,
            WIDTH - 80,
            badge_bg,
            badge_fg,
            y + 30,
            badge_fg,
            escape_xml(text),
        ));
        y += 48;
    } else if let Some(pay_url) = invoice.pay_url.as_deref() {
        // r##"..."## — the inline `fill="#FFFFFF"` contains the sequence
        // `"#` which would otherwise close a plain r#"..."# raw string.
        body.push_str(&format!(
            r##"<a href="{}"><rect x="40" y="{}" width="{}" height="48" rx="12" fill="{GREEN}"/><text x="{cx}" y="{}" font-family="sans-serif" font-size="15" font-weight="bold" fill="#FFFFFF" text-anchor="middle">Pay Online</text></a>
"##,
            escape_xml(pay_url),
            y,
            WIDTH - 80,
            y + 30,
        ));
        y += 48;
    }

    y += 30;
    body.push_str(&format!(
        r#"<text x="{cx}" y="{y}" font-family="sans-serif" font-size="11" fill="{MUTED}" text-anchor="middle">a HomeVentory offering</text>
"#
    ));

    let height = y + 24;

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{height}" viewBox="0 0 {WIDTH} {height}"><rect x="0" y="0" width="{WIDTH}" height="{height}" fill="{BG}"/>{body}</svg>"#
    )
}

// ── Commands ─────────────────────────────────────────────────────────────────

#[tauri::command]
async fn load_invoices(app: tauri::AppHandle) -> Result<Vec<Invoice>, String> {
    Ok(read_invoices(&app)?.invoices)
}

/// Creates, publishes, and locally saves an invoice in one step — "sharing
/// is what saves it" per the app's own design: there is no separate Save
/// action. Builds the SVG, publishes it to its own BDO record (pub: true),
/// computes both the savage view URL and the eumachia pay URL (a
/// pre-signed read link, same scheme as every other permanent share link
/// this session — plain ASCII query params, deliberately not BDO's
/// /emoji/:code mechanism, which has a confirmed netlify-gateway proxy bug
/// with multi-byte characters in a URL path), and returns the complete,
/// saved Invoice ready to hand to the native share sheet.
#[tauri::command]
async fn create_invoice(
    app: tauri::AppHandle,
    kind: Option<DocumentKind>,
    description: String,
    amount_cents: u64,
    to_name: Option<String>,
    from_name: Option<String>,
    work_performed_at: String,
    due_date: Option<String>,
) -> Result<Invoice, String> {
    let id = new_id();
    let kind = kind.unwrap_or(DocumentKind::Invoice);
    // Only snapshot a payout pubkey if Stripe is actually connected — a bare
    // (uninitialized) identity shouldn't make an invoice look split-eligible.
    // Estimates carry a payout pubkey too so that if/when they're converted,
    // the resulting invoice can charge — but the pubkey is only consulted
    // during actual payment, so having it on an estimate is harmless.
    let creator_addie_pub_key = read_addie_identity(&app)
        .filter(|identity| identity.stripe_account_id.is_some())
        .map(|identity| identity.pub_key_hex);
    let initial_status = match kind {
        DocumentKind::Invoice => InvoiceStatus::Pending,
        DocumentKind::Estimate => InvoiceStatus::Active,
    };
    let mut invoice = Invoice {
        id: id.clone(),
        kind,
        description,
        amount_cents,
        currency: default_currency(),
        from_name,
        to_name,
        creator_addie_pub_key,
        status: initial_status,
        due_date,
        work_performed_at,
        created_at: unix_now_ms_string(),
        ..Default::default()
    };

    let sessionless = load_or_create_bdo_sessionless(&app, &id)?;
    let client = BDO::new(Some(GATEWAY_BDO_URL.to_string()), Some(sessionless));

    // Placeholder pay_url computed AFTER we know the uuid — render once to
    // get a uuid via create_user, then re-render+update with the real link.
    let initial_json = serde_json::to_value(&invoice).map_err(|e| e.to_string())?;
    let user = client
        .create_user(BDO_HASH, &initial_json, &true)
        .await
        .map_err(|e| e.to_string())?;
    let uuid = user.uuid;

    let timestamp = unix_now_ms_string();
    let signature = client
        .sessionless
        .sign(format!("{timestamp}{uuid}{BDO_HASH}"))
        .to_hex();
    let share_url =
        format!("{SAVAGE_URL}user/{uuid}/bdo?timestamp={timestamp}&hash={BDO_HASH}&signature={signature}");
    let pay_url = format!(
        "{EUMACHIA_PAY_URL}{uuid}?hash={BDO_HASH}&timestamp={timestamp}&signature={signature}"
    );

    invoice.bdo_uuid_by_env.insert(GATEWAY_ENV.to_string(), uuid.clone());
    invoice.share_url = Some(share_url);
    // Estimates are not payable — the eumachia pay page assumes an invoice
    // with a payable amount and produces a Stripe checkout, which doesn't
    // apply to a quote. Only stamp pay_url for actual invoices.
    if matches!(invoice.kind, DocumentKind::Invoice) {
        invoice.pay_url = Some(pay_url);
    }

    let svg = render_invoice_svg(&invoice);
    let mut full_json = serde_json::to_value(&invoice).map_err(|e| e.to_string())?;
    full_json
        .as_object_mut()
        .ok_or_else(|| "invoice serialized to non-object".to_string())?
        .insert("svg".to_string(), serde_json::Value::String(svg));
    client
        .update_bdo(&uuid, BDO_HASH, &full_json, &true)
        .await
        .map_err(|e| e.to_string())?;

    let mut store = read_invoices(&app)?;
    store.invoices.push(invoice.clone());
    write_invoices(&app, &store)?;

    Ok(invoice)
}

/// Re-publishes an invoice's BDO record after its paid state changes, so
/// the shared page reflects it too if revisited. Shared by both the manual
/// "Mark as Paid" path and the automatic eumachia-status-check path — one
/// write path, two triggers.
///
/// Only ever updates the record under the env the invoice was actually
/// created in (`bdo_uuid_by_env.get(GATEWAY_ENV)`) — it deliberately does
/// NOT fall back to minting a fresh BDO record if the current env has no
/// entry. The invoice's real published page (the link a payer actually has)
/// lives on whichever gateway it was first published to; publishing a new
/// record under today's env would just be an unreachable duplicate nobody
/// has the link to, not a fix.
async fn republish_invoice(app: &tauri::AppHandle, invoice: &Invoice) -> Result<(), String> {
    let uuid = invoice.bdo_uuid_by_env.get(GATEWAY_ENV).ok_or_else(|| {
        "This invoice was published under a different environment and can't be updated from here.".to_string()
    })?;
    let sessionless = load_or_create_bdo_sessionless(app, &invoice.id)?;
    let client = BDO::new(Some(GATEWAY_BDO_URL.to_string()), Some(sessionless));

    let svg = render_invoice_svg(invoice);
    let mut json = serde_json::to_value(invoice).map_err(|e| e.to_string())?;
    json.as_object_mut()
        .ok_or_else(|| "invoice serialized to non-object".to_string())?
        .insert("svg".to_string(), serde_json::Value::String(svg));

    client
        .update_bdo(uuid, BDO_HASH, &json, &true)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Sets an invoice's lifecycle status — the single write path behind every
/// state-change action in the UI (Mark Paid Manually, Waive, Cancel,
/// Reopen). `note` is stored on the invoice for any status other than
/// Pending, so the reason for a Waive/Cancel isn't lost.
///
/// Estimate statuses (`Active`/`Converted`) are also permitted here for
/// completeness, but the normal path for going Active → Converted is
/// `convert_estimate_to_invoice` (which additionally mints the new
/// invoice and stamps the link).
#[tauri::command]
async fn set_invoice_status(
    app: tauri::AppHandle,
    id: String,
    status: InvoiceStatus,
    note: Option<String>,
) -> Result<Invoice, String> {
    let mut store = read_invoices(&app)?;
    let index = store
        .invoices
        .iter()
        .position(|i| i.id == id)
        .ok_or_else(|| "Invoice not found".to_string())?;

    store.invoices[index].status = status;
    store.invoices[index].status_note = note.filter(|s| !s.is_empty());
    // paid_at reflects "when did money actually arrive" — stamped for both
    // Stripe and manual paid transitions, cleared if the invoice is
    // reopened (Pending) so a later "paid" transition gets a fresh
    // timestamp rather than a stale one.
    match status {
        InvoiceStatus::PaidStripe | InvoiceStatus::PaidManual => {
            if store.invoices[index].paid_at.is_none() {
                store.invoices[index].paid_at = Some(unix_now_ms_string());
            }
        }
        InvoiceStatus::Pending => {
            store.invoices[index].paid_at = None;
        }
        _ => {}
    }
    let updated = store.invoices[index].clone();

    // Only republish if there's a BDO record under the current env — the
    // migration case of legacy invoices published under other envs simply
    // updates local state, same policy as republish_invoice itself.
    if updated.bdo_uuid_by_env.contains_key(GATEWAY_ENV) {
        republish_invoice(&app, &updated).await?;
    }
    write_invoices(&app, &store)?;
    Ok(updated)
}

/// Spawns a fresh Invoice from an existing Estimate — same fields
/// (description, amount, from/to, work date), fresh id + BDO record + pay
/// URL, and stamps `converted_to_invoice_id` on the estimate so the two
/// stay linked. Optional `due_date` because the estimate itself doesn't
/// carry one (its date semantics are "when the work would happen", not
/// "when payment is due").
#[tauri::command]
async fn convert_estimate_to_invoice(
    app: tauri::AppHandle,
    id: String,
    due_date: Option<String>,
) -> Result<Invoice, String> {
    // Snapshot the estimate first, without holding a mutable borrow across
    // the create_invoice await — that's the standard "clone the read side,
    // do the async work, then re-open to write the mark-converted" dance.
    let estimate = {
        let store = read_invoices(&app)?;
        store
            .invoices
            .iter()
            .find(|i| i.id == id)
            .cloned()
            .ok_or_else(|| "Estimate not found".to_string())?
    };
    if !matches!(estimate.kind, DocumentKind::Estimate) {
        return Err("Only estimates can be converted to invoices.".to_string());
    }
    if matches!(estimate.status, InvoiceStatus::Converted) {
        return Err("This estimate has already been converted.".to_string());
    }

    let new_invoice = create_invoice(
        app.clone(),
        Some(DocumentKind::Invoice),
        estimate.description.clone(),
        estimate.amount_cents,
        estimate.to_name.clone(),
        estimate.from_name.clone(),
        estimate.work_performed_at.clone(),
        due_date,
    )
    .await?;

    // Now re-open the store and stamp the estimate as converted. Doing this
    // AFTER the invoice creation succeeds means a failed conversion doesn't
    // leave a dangling "converted" estimate pointing at nothing.
    let mut store = read_invoices(&app)?;
    if let Some(est) = store.invoices.iter_mut().find(|i| i.id == id) {
        est.status = InvoiceStatus::Converted;
        est.converted_to_invoice_id = Some(new_invoice.id.clone());
        let updated_estimate = est.clone();
        if updated_estimate.bdo_uuid_by_env.contains_key(GATEWAY_ENV) {
            republish_invoice(&app, &updated_estimate).await?;
        }
    }
    write_invoices(&app, &store)?;
    Ok(new_invoice)
}

/// Polls eumachia's payment-status endpoint for a single invoice. Returns
/// `true` if this call newly discovered a real online payment (and
/// re-published the invoice as paid) — the frontend uses this to decide
/// whether to refresh its list.
#[tauri::command]
async fn check_payment_status(app: tauri::AppHandle, id: String) -> Result<bool, String> {
    let mut store = read_invoices(&app)?;
    let index = store
        .invoices
        .iter()
        .position(|i| i.id == id)
        .ok_or_else(|| "Invoice not found".to_string())?;

    // Only Pending invoices are worth polling. Anything else (already-paid,
    // waived, canceled, or an estimate) is a no-op — a paid check on a
    // converted estimate would just be wasted network.
    if !matches!(store.invoices[index].status, InvoiceStatus::Pending) {
        return Ok(false);
    }
    let uuid = store.invoices[index]
        .bdo_uuid_by_env
        .get(GATEWAY_ENV)
        .cloned()
        .ok_or_else(|| {
            "This invoice was published under a different environment and can't be checked from here.".to_string()
        })?;

    let status_url = format!("{EUMACHIA_PAY_URL}{uuid}/status");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&status_url)
        .send()
        .await
        .map_err(|e| format!("Couldn't reach eumachia: {e}"))?;
    if !resp.status().is_success() {
        return Ok(false);
    }
    let status: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let paid = status.get("paid").and_then(|v| v.as_bool()).unwrap_or(false);
    if !paid {
        return Ok(false);
    }

    store.invoices[index].status = InvoiceStatus::PaidStripe;
    store.invoices[index].paid_at = Some(unix_now_ms_string());
    let updated = store.invoices[index].clone();
    republish_invoice(&app, &updated).await?;
    write_invoices(&app, &store)?;
    Ok(true)
}

// ── Canonical profile ───────────────────────────────────────────────────────
//
// A separate, App-Group-shared record — this is what "take my shared
// profile" reads from at invoice-creation time (name snapshotted into
// from_name). Copied byte-identical from BizBuz/Linkitylink/idothis — a
// thin OS-API wrapper with no business logic to diverge, so copy-paste is
// the right call, same reasoning as the plugin itself.

const MAX_CANONICAL_FIELDS: usize = 20;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalField {
    pub slug: String,
    pub name: String,
    pub value: String,
}

/// A standard postal address on the shared Canonical Profile. This app has
/// no UI to view or edit it (Gettit does — see its lib.rs) — the field
/// exists here purely so `save_canonical_profile` below can round-trip it
/// without silently erasing whatever Gettit wrote, since every app that
/// touches Canonical Profile overwrites the whole shared record on save.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Address {
    pub street: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    pub city: String,
    pub state: String,
    pub zip: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalProfile {
    pub photo: Option<String>,
    #[serde(default)]
    pub fields: Vec<CanonicalField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<Address>,
    /// idothis-owned. Present here so getpayed round-trips it. None = don't touch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idothis_categories: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_zip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idothis_rate_cents: Option<u64>,
    /// getpayed writes this to true after a successful Stripe connect;
    /// idothis reads it to gate the "Join" action. None = "hasn't been
    /// touched yet", not the same as false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stripe_connected: Option<bool>,
    pub updated_at: Option<String>,
}

fn canonical_slugify(s: &str) -> String {
    let mut slug = String::new();
    let mut last_was_sep = true;
    for ch in s.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_sep = false;
        } else if !last_was_sep {
            slug.push('_');
            last_was_sep = true;
        }
    }
    while slug.ends_with('_') {
        slug.pop();
    }
    slug
}

#[tauri::command]
async fn load_canonical_profile(app: tauri::AppHandle) -> Result<Option<CanonicalProfile>, String> {
    let raw = tauri_plugin_app_group::read_value_sync(&app, "canonical.profile")?;
    match raw {
        Some(json) => Ok(serde_json::from_str(&json).ok()),
        None => Ok(None),
    }
}

#[tauri::command]
async fn save_canonical_profile(app: tauri::AppHandle, mut profile: CanonicalProfile) -> Result<CanonicalProfile, String> {
    // This app's own form never sends real values for address or the
    // idothis-owned fields, so always carry forward whatever's already
    // stored rather than overwriting them with the incoming None. Same
    // for stripe_connected UNLESS the caller is publishing a change (the
    // new set_stripe_connected command does that below).
    let existing = load_canonical_profile(app.clone()).await?;
    if let Some(existing) = existing {
        if profile.address.is_none() { profile.address = existing.address; }
        if profile.idothis_categories.is_none() { profile.idothis_categories = existing.idothis_categories; }
        if profile.service_zip.is_none() { profile.service_zip = existing.service_zip; }
        if profile.idothis_rate_cents.is_none() { profile.idothis_rate_cents = existing.idothis_rate_cents; }
        if profile.stripe_connected.is_none() { profile.stripe_connected = existing.stripe_connected; }
    }

    let mut deduped: Vec<CanonicalField> = Vec::new();
    for mut field in profile.fields.into_iter() {
        if field.slug.trim().is_empty() {
            field.slug = canonical_slugify(&field.name);
        }
        if field.slug.is_empty() {
            continue;
        }
        deduped.retain(|f| f.slug != field.slug);
        deduped.push(field);
    }
    deduped.truncate(MAX_CANONICAL_FIELDS);
    profile.fields = deduped;
    profile.updated_at = Some(unix_now_ms_string());
    let json = serde_json::to_string(&profile).map_err(|e| e.to_string())?;
    tauri_plugin_app_group::write_value_sync(&app, "canonical.profile", &json)?;
    Ok(profile)
}

/// Publishes the current local Stripe-connection state to the shared
/// canonical profile so sibling apps (idothis in particular) can gate on
/// it without having to run their own onboarding flow. Called on
/// getpayed startup and after a successful Stripe connect. Reads
/// `stripeAccountId` presence as the source of truth.
#[tauri::command]
async fn publish_stripe_connected(app: tauri::AppHandle) -> Result<(), String> {
    let connected = read_addie_identity(&app)
        .map(|i| i.stripe_account_id.is_some())
        .unwrap_or(false);
    let mut profile = load_canonical_profile(app.clone()).await?.unwrap_or_default();
    profile.stripe_connected = Some(connected);
    profile.updated_at = Some(unix_now_ms_string());
    let json = serde_json::to_string(&profile).map_err(|e| e.to_string())?;
    tauri_plugin_app_group::write_value_sync(&app, "canonical.profile", &json)?;
    Ok(())
}

// ── App entry ────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_share_sheet::init())
        .plugin(tauri_plugin_app_group::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_deep_link::init())
        .invoke_handler(tauri::generate_handler![
            load_invoices,
            create_invoice,
            set_invoice_status,
            convert_estimate_to_invoice,
            check_payment_status,
            get_payout_status,
            connect_stripe_account,
            load_canonical_profile,
            save_canonical_profile,
            publish_stripe_connected
        ])
        .run(tauri::generate_context!())
        .expect("error while running gelder");
}
