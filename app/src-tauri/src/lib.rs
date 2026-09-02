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

const GATEWAY_BDO_URL: &str = "https://allyabase-gateway-12345.netlify.app/bdo/";
const SAVAGE_URL: &str = "https://allyabase-gateway-12345.netlify.app/savage/";
// eumachia is the only piece of this stack that can render a real,
// interactive "Pay Now" page — savage strips all JavaScript from whatever
// it serves, so it can't host a Stripe checkout itself (verified this
// session while designing eumachia).
const EUMACHIA_PAY_URL: &str = "https://allyabase-gateway-12345.netlify.app/eumachia/pay/";
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
const GATEWAY_ENV: &str = "test-12345";

fn default_currency() -> String {
    "usd".to_string()
}

// ── Data types ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Invoice {
    #[serde(default)]
    pub id: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pay_url: Option<String>,
    /// Snapshotted from the connected Addie payout identity at creation
    /// time, same reasoning as `from_name` — if absent, eumachia sends
    /// 100% of the charge to itself (today's behavior); if present,
    /// eumachia requests a 91%/9%-minus-fees split with this pubkey as
    /// the merchant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_addie_pub_key: Option<String>,
    #[serde(default)]
    pub paid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paid_at: Option<String>,
    /// When the invoiced work was actually performed — user-entered at
    /// creation time (defaults to "now" in the UI, but is editable), unlike
    /// `created_at` below which is always exactly when the record was made.
    /// The invoice list sorts by this, since it's what a freelancer
    /// actually cares about ordering by.
    #[serde(default)]
    pub work_performed_at: String,
    #[serde(default)]
    pub created_at: String,
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
        Ok(contents) => serde_json::from_str(&contents).map_err(|e| e.to_string()),
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

const GATEWAY_ADDIE_URL: &str = "https://allyabase-gateway-12345.netlify.app/addie/";

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
    const BG: &str = "#0a001a";
    const GREEN: &str = "#10b981";
    const PURPLE: &str = "#a78bfa";

    let cx = WIDTH / 2;
    let mut y: u32 = 60;
    let mut body = String::new();

    body.push_str(&format!(
        r#"<text x="{cx}" y="{y}" font-family="sans-serif" font-size="24" font-weight="bold" fill="{GREEN}" text-anchor="middle">Invoice</text>
"#
    ));

    if let Some(from_name) = invoice.from_name.as_deref().filter(|s| !s.is_empty()) {
        y += 30;
        body.push_str(&format!(
            r#"<text x="40" y="{y}" font-family="sans-serif" font-size="13" fill="rgba(255,255,255,0.6)">From: {}</text>
"#,
            escape_xml(from_name),
        ));
    }
    if let Some(to_name) = invoice.to_name.as_deref().filter(|s| !s.is_empty()) {
        y += 22;
        body.push_str(&format!(
            r#"<text x="40" y="{y}" font-family="sans-serif" font-size="13" fill="rgba(255,255,255,0.6)">To: {}</text>
"#,
            escape_xml(to_name),
        ));
    }
    if let Some(work_date) = format_epoch_ms_date(&invoice.work_performed_at) {
        y += 22;
        body.push_str(&format!(
            r#"<text x="40" y="{y}" font-family="sans-serif" font-size="13" fill="rgba(255,255,255,0.6)">Work performed: {}</text>
"#,
            escape_xml(&work_date),
        ));
    }

    y += 50;
    body.push_str(&format!(
        r#"<text x="{cx}" y="{y}" font-family="sans-serif" font-size="34" font-weight="bold" fill="{PURPLE}" text-anchor="middle">{}</text>
"#,
        escape_xml(&format_amount(invoice.amount_cents, &invoice.currency)),
    ));

    y += 40;
    body.push_str(&format!(
        r#"<text x="40" y="{y}" font-family="sans-serif" font-size="15" fill="rgba(255,255,255,0.9)">{}</text>
"#,
        escape_xml(&invoice.description),
    ));

    y += 50;
    if invoice.paid {
        body.push_str(&format!(
            r#"<rect x="40" y="{}" width="{}" height="48" rx="12" fill="rgba(16,185,129,0.15)" stroke="{GREEN}" stroke-width="1"/>
<text x="{cx}" y="{}" font-family="sans-serif" font-size="15" font-weight="bold" fill="{GREEN}" text-anchor="middle">✅ Paid</text>
"#,
            y,
            WIDTH - 80,
            y + 30,
        ));
        y += 48;
    } else if let Some(pay_url) = invoice.pay_url.as_deref() {
        body.push_str(&format!(
            r#"<a href="{}"><rect x="40" y="{}" width="{}" height="48" rx="12" fill="{GREEN}"/><text x="{cx}" y="{}" font-family="sans-serif" font-size="15" font-weight="bold" fill="{BG}" text-anchor="middle">Pay Online</text></a>
"#,
            escape_xml(pay_url),
            y,
            WIDTH - 80,
            y + 30,
        ));
        y += 48;
    }

    y += 30;
    body.push_str(&format!(
        r#"<text x="{cx}" y="{y}" font-family="sans-serif" font-size="11" fill="rgba(255,255,255,0.4)" text-anchor="middle">a Freyja offering</text>
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
    description: String,
    amount_cents: u64,
    to_name: Option<String>,
    from_name: Option<String>,
    work_performed_at: String,
) -> Result<Invoice, String> {
    let id = new_id();
    // Only snapshot a payout pubkey if Stripe is actually connected — a bare
    // (uninitialized) identity shouldn't make an invoice look split-eligible.
    let creator_addie_pub_key = read_addie_identity(&app)
        .filter(|identity| identity.stripe_account_id.is_some())
        .map(|identity| identity.pub_key_hex);
    let mut invoice = Invoice {
        id: id.clone(),
        description,
        amount_cents,
        currency: default_currency(),
        from_name,
        to_name,
        creator_addie_pub_key,
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
    invoice.pay_url = Some(pay_url);

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

#[tauri::command]
async fn mark_invoice_paid(app: tauri::AppHandle, id: String) -> Result<Invoice, String> {
    let mut store = read_invoices(&app)?;
    let index = store
        .invoices
        .iter()
        .position(|i| i.id == id)
        .ok_or_else(|| "Invoice not found".to_string())?;

    store.invoices[index].paid = true;
    store.invoices[index].paid_at = Some(unix_now_ms_string());
    let updated = store.invoices[index].clone();

    republish_invoice(&app, &updated).await?;
    write_invoices(&app, &store)?;
    Ok(updated)
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

    if store.invoices[index].paid {
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

    store.invoices[index].paid = true;
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
    // This app's own form never sends a real address (no UI for it — see
    // Address's doc comment above), so always carry forward whatever's
    // already stored rather than overwriting it with the incoming None.
    profile.address = load_canonical_profile(app.clone()).await?.and_then(|p| p.address);

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
            mark_invoice_paid,
            check_payment_status,
            get_payout_status,
            connect_stripe_account,
            load_canonical_profile,
            save_canonical_profile
        ])
        .run(tauri::generate_context!())
        .expect("error while running gelder");
}
