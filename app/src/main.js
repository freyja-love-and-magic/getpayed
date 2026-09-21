const { core, dialog, fs } = window.__TAURI__;
const Channel = window.__TAURI__.core.Channel;

// Keep in sync with MAX_CANONICAL_FIELDS in src-tauri/src/lib.rs.
const MAX_PROFILE_FIELDS = 20;

const listView = document.getElementById('list-view');
const createView = document.getElementById('create-view');
const detailView = document.getElementById('detail-view');
const profileView = document.getElementById('profile-view');
const backNavBtn = document.getElementById('back-nav-btn');
const profileNavBtn = document.getElementById('profile-nav-btn');
const payoutsView = document.getElementById('payouts-view');
const payoutsNavBtn = document.getElementById('payouts-nav-btn');
const statusMsg = document.getElementById('status-msg');

const newInvoiceBtn = document.getElementById('new-invoice-btn');
const invoiceListEl = document.getElementById('invoice-list');
const emptyHint = document.getElementById('empty-hint');
const stripeRequiredBanner = document.getElementById('stripe-required-banner');
const stripeRequiredConnectBtn = document.getElementById('stripe-required-connect-btn');

const invoiceForm = document.getElementById('invoice-form');
const fromNote = document.getElementById('from-note');
const fieldToName = document.getElementById('field-to-name');
const fieldDescription = document.getElementById('field-description');
const fieldAmount = document.getElementById('field-amount');
const fieldWorkDate = document.getElementById('field-work-date');
const fieldDueDate = document.getElementById('field-due-date');
const dueDateLabel = document.getElementById('due-date-label');
const createTitle = document.getElementById('create-title');
const createSubtitle = document.getElementById('create-subtitle');
const createSubmitBtn = document.getElementById('create-submit-btn');
const cancelCreateBtn = document.getElementById('cancel-create-btn');
const kindInvoiceBtn = document.getElementById('kind-invoice-btn');
const kindEstimateBtn = document.getElementById('kind-estimate-btn');

const detailTitle = document.getElementById('detail-title');
const detailAmount = document.getElementById('detail-amount');
const detailDescription = document.getElementById('detail-description');
const detailFrom = document.getElementById('detail-from');
const detailTo = document.getElementById('detail-to');
const detailWorkDate = document.getElementById('detail-work-date');
const detailDueDate = document.getElementById('detail-due-date');
const detailCreatedAt = document.getElementById('detail-created-at');
const detailStatusBadge = document.getElementById('detail-status-badge');
const detailStatusNote = document.getElementById('detail-status-note');
const detailConvertedLink = document.getElementById('detail-converted-link');
const detailActions = document.getElementById('detail-actions');
const payLinkRow = document.getElementById('pay-link-row');
const payLinkText = document.getElementById('pay-link-text');
const detailPayout = document.getElementById('detail-payout');
const detailPayoutHeadline = document.getElementById('detail-payout-headline');
const detailPayoutDetail = document.getElementById('detail-payout-detail');
const copyPayLinkBtn = document.getElementById('copy-pay-link-btn');
const shareAgainBtn = document.getElementById('share-again-btn');

const newEstimateBtn = document.getElementById('new-estimate-btn');
const estimatesList = document.getElementById('estimates-list');
const estimatesEmptyHint = document.getElementById('estimates-empty-hint');
const payoutsInvoicesList = document.getElementById('payouts-invoices-list');
const payoutsInvoicesEmptyHint = document.getElementById('payouts-invoices-empty-hint');

const profileForm = document.getElementById('canonical-profile-form');
const profilePhotoPreview = document.getElementById('profile-photo-preview');
const profileChoosePhotoBtn = document.getElementById('profile-choose-photo-btn');
const profileFieldsEl = document.getElementById('profile-fields');
const profileNewFieldName = document.getElementById('profile-new-field-name');
const profileNewFieldValue = document.getElementById('profile-new-field-value');
const profileAddFieldBtn = document.getElementById('profile-add-field-btn');
const profileFieldLimitHint = document.getElementById('profile-field-limit-hint');
const profileCloseBtn = document.getElementById('profile-close-btn');

const payoutsConnected = document.getElementById('payouts-connected');
const payoutsVerifying = document.getElementById('payouts-verifying');
const payoutsIncomplete = document.getElementById('payouts-incomplete');
const payoutsIncompleteHint = document.getElementById('payouts-incomplete-hint');
const payoutsManageBtn = document.getElementById('payouts-manage-btn');
const payoutsRefreshBtn = document.getElementById('payouts-refresh-btn');
const payoutsResumeBtn = document.getElementById('payouts-resume-btn');
const payoutsForm = document.getElementById('payouts-form');
const payoutsCountry = document.getElementById('payouts-country');
const payoutsEmail = document.getElementById('payouts-email');
const payoutsConnectBtn = document.getElementById('payouts-connect-btn');
const payoutsCloseBtn = document.getElementById('payouts-close-btn');

const PHOTO_SIZE = 480;
const PHOTO_QUALITY = 0.85;

let invoices = [];
let selectedInvoiceId = null;
let cachedProfileFromName = undefined; // pulled from canonical profile's "name" field
let stripeConnected = false;
// Whether the open invoice names a payout identity THIS install no longer
// holds — a fact about this device, not about the invoice, so it isn't
// stored on the record. Set by check/retry, cleared when a different
// invoice is opened.
let payoutIdentityStale = false;
let pendingKind = 'invoice'; // 'invoice' | 'estimate' — controlled by the kind toggle in the create form

// ── View / status helpers ────────────────────────────────────────────────────

// Invoices can't be created until Stripe is connected — an invoice with no
// creatorAddiePubKey never gets a payout split (see payments.js/payOutCreator
// on the eumachia side), so a payer's money would have nowhere to go. Swap
// "+ New Invoice" for an explanatory banner + CTA instead of just disabling
// it, since the reason (Stripe is how disbursements actually happen) isn't
// obvious from the button alone.
function updateInvoiceCreationGate() {
    stripeRequiredBanner.hidden = stripeConnected;
    newInvoiceBtn.hidden = !stripeConnected;
}

function showView(name) {
    listView.hidden = name !== 'list';
    createView.hidden = name !== 'create';
    detailView.hidden = name !== 'detail';
    profileView.hidden = name !== 'profile';
    payoutsView.hidden = name !== 'payouts';
    // Header: Back shows on subviews of the list, Profile/Payouts hide only
    // when already on their own view (their overlay covers the header anyway).
    backNavBtn.hidden = !(name === 'create' || name === 'detail');
    profileNavBtn.hidden = name === 'profile';
    payoutsNavBtn.hidden = name === 'payouts';
}

backNavBtn.addEventListener('click', () => showView('list'));

let statusTimeout = null;
function setStatus(message) {
    statusMsg.textContent = message;
    statusMsg.classList.add('visible');
    clearTimeout(statusTimeout);
    statusTimeout = setTimeout(() => statusMsg.classList.remove('visible'), 2500);
}

function formatAmount(amountCents) {
    return `$${(amountCents / 100).toFixed(2)}`;
}

function formatEpochMsDate(epochMsStr) {
    const ms = Number(epochMsStr);
    if (!Number.isFinite(ms) || ms <= 0) return null;
    return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });
}

// ── Invoice list ──────────────────────────────────────────────────────────────

function renderInvoiceList() {
    invoiceListEl.innerHTML = '';
    emptyHint.hidden = invoices.length > 0;

    // Home list is invoices only; estimates live in Payouts view. Most
    // recently worked-on first — what a freelancer actually cares about
    // ordering by, not whichever moment the record happened to be typed
    // into the app (createdAt).
    const onlyInvoices = invoices.filter((inv) => (inv.kind || 'invoice') === 'invoice');
    const sorted = [...onlyInvoices].sort((a, b) => Number(b.workPerformedAt) - Number(a.workPerformedAt));
    for (const inv of sorted) {
        invoiceListEl.appendChild(renderInvoiceRow(inv));
    }
    if (emptyHint) emptyHint.hidden = sorted.length > 0;
}

// Derives the human display state from the stored status + due date, so
// "past due" surfaces automatically as time passes without us needing to
// mutate the record on a timer.
function displayStatus(doc) {
    const status = doc.status || 'pending';
    if (status === 'pending' && doc.dueDate) {
        const dueMs = Number(doc.dueDate);
        if (Number.isFinite(dueMs) && dueMs > 0 && Date.now() > dueMs) return 'past_due';
    }
    return status;
}

function displayStatusLabel(status) {
    switch (status) {
        case 'pending':      return 'Pending';
        case 'past_due':     return 'Past due';
        case 'paid_stripe':  return 'Paid';
        case 'paid_manual':  return 'Paid (manual)';
        case 'waived':       return 'Waived';
        case 'canceled':     return 'Canceled';
        case 'active':       return 'Estimate';
        case 'converted':    return 'Converted';
        default:             return status;
    }
}

function displayStatusClass(status) {
    switch (status) {
        case 'paid_stripe':
        case 'paid_manual':  return 'paid';
        case 'past_due':     return 'past-due';
        default:             return status.replace(/_/g, '-');
    }
}

function renderInvoiceRow(inv) {
    const li = document.createElement('li');
    li.className = 'invoice-list-item';

    const text = document.createElement('div');
    text.className = 'invoice-list-text';
    text.innerHTML = '<div class="invoice-list-desc"></div><div class="invoice-list-sub"></div>';
    text.querySelector('.invoice-list-desc').textContent = inv.description || 'Untitled';
    const workDate = formatEpochMsDate(inv.workPerformedAt);
    text.querySelector('.invoice-list-sub').textContent = [
        inv.toName ? `To ${inv.toName}` : null,
        workDate,
    ].filter(Boolean).join(' · ') || 'No recipient set';

    const right = document.createElement('div');
    right.className = 'invoice-list-right';
    const amountEl = document.createElement('div');
    amountEl.className = 'invoice-list-amount';
    amountEl.textContent = formatAmount(inv.amountCents);
    const status = displayStatus(inv);
    const badge = document.createElement('div');
    badge.className = `invoice-list-badge ${displayStatusClass(status)}`;
    badge.textContent = displayStatusLabel(status);
    right.append(amountEl, badge);

    li.append(text, right);
    li.addEventListener('click', () => openDetail(inv.id));
    return li;
}

function renderPayoutsLists() {
    // Estimates section
    estimatesList.innerHTML = '';
    const estimates = invoices
        .filter((inv) => inv.kind === 'estimate')
        .sort((a, b) => Number(b.workPerformedAt) - Number(a.workPerformedAt));
    for (const est of estimates) estimatesList.appendChild(renderInvoiceRow(est));
    estimatesEmptyHint.hidden = estimates.length > 0;

    // Invoices section (duplicated across list + payouts by design — the
    // payouts view is the "one place with the whole financial picture").
    payoutsInvoicesList.innerHTML = '';
    const invs = invoices
        .filter((inv) => (inv.kind || 'invoice') === 'invoice')
        .sort((a, b) => Number(b.workPerformedAt) - Number(a.workPerformedAt));
    for (const inv of invs) payoutsInvoicesList.appendChild(renderInvoiceRow(inv));
    payoutsInvoicesEmptyHint.hidden = invs.length > 0;
}

async function loadInvoices() {
    invoices = await core.invoke('load_invoices');
    renderInvoiceList();
    renderPayoutsLists();
}

// ── Create flow ───────────────────────────────────────────────────────────────

// Formats a Date as the local "YYYY-MM-DDTHH:mm" string <input
// type="datetime-local"> expects, so the field starts pre-filled with "now"
// (still fully editable) instead of forcing an empty-field click for the
// common case of invoicing for work done today.
function toDatetimeLocalValue(date) {
    const pad = (n) => String(n).padStart(2, '0');
    return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function setPendingKind(kind) {
    pendingKind = kind;
    kindInvoiceBtn.classList.toggle('active', kind === 'invoice');
    kindEstimateBtn.classList.toggle('active', kind === 'estimate');
    kindInvoiceBtn.setAttribute('aria-selected', String(kind === 'invoice'));
    kindEstimateBtn.setAttribute('aria-selected', String(kind === 'estimate'));

    if (kind === 'invoice') {
        createTitle.textContent = 'New Invoice';
        createSubtitle.textContent = 'Sharing this invoice is what saves and publishes it.';
        createSubmitBtn.textContent = 'Create & Share';
        dueDateLabel.hidden = false;
    } else {
        createTitle.textContent = 'New Estimate';
        createSubtitle.textContent = 'Estimates are quotes — no payment link, and no Stripe connection needed.';
        createSubmitBtn.textContent = 'Create Estimate & Share';
        // Due date is invoice-specific; estimates use `work performed` for
        // the "when the job would happen" date, so no separate due field.
        dueDateLabel.hidden = true;
        fieldDueDate.value = '';
    }
}

kindInvoiceBtn.addEventListener('click', () => setPendingKind('invoice'));
kindEstimateBtn.addEventListener('click', () => setPendingKind('estimate'));

async function openCreateForm(kind = 'invoice') {
    // Only invoices need Stripe; estimates are quotes with no payment leg.
    if (kind === 'invoice' && !stripeConnected) {
        setStatus('Connect a Stripe account before creating invoices.');
        await openPayoutsView();
        return;
    }

    fieldToName.value = '';
    fieldDescription.value = '';
    fieldAmount.value = '';
    fieldWorkDate.value = toDatetimeLocalValue(new Date());
    fieldDueDate.value = '';
    setPendingKind(kind);

    try {
        const profile = await core.invoke('load_canonical_profile');
        const nameField = (profile?.fields || []).find((f) => f.slug === 'name');
        cachedProfileFromName = nameField?.value || undefined;
    } catch {
        cachedProfileFromName = undefined;
    }

    if (cachedProfileFromName) {
        fromNote.textContent = `From: ${cachedProfileFromName}`;
    } else {
        fromNote.textContent = 'No shared profile name set yet — set one in Profile so this shows who it’s from.';
    }

    showView('create');
}

newInvoiceBtn.addEventListener('click', () => openCreateForm('invoice'));
newEstimateBtn.addEventListener('click', () => openCreateForm('estimate'));
cancelCreateBtn.addEventListener('click', () => showView('list'));

invoiceForm.addEventListener('submit', async (e) => {
    e.preventDefault();

    const description = fieldDescription.value.trim();
    const amount = parseFloat(fieldAmount.value);
    if (!description || !Number.isFinite(amount) || amount <= 0 || !fieldWorkDate.value) return;
    const amountCents = Math.round(amount * 100);
    const toName = fieldToName.value.trim() || undefined;
    const workPerformedAt = String(new Date(fieldWorkDate.value).getTime());
    const dueDate = pendingKind === 'invoice' && fieldDueDate.value
        ? String(new Date(fieldDueDate.value).getTime())
        : undefined;

    createSubmitBtn.disabled = true;
    setStatus(pendingKind === 'estimate' ? 'Publishing estimate…' : 'Publishing invoice…');
    try {
        const doc = await core.invoke('create_invoice', {
            kind: pendingKind,
            description,
            amountCents,
            toName,
            fromName: cachedProfileFromName,
            workPerformedAt,
            dueDate,
        });
        invoices.push(doc);
        renderInvoiceList();
        renderPayoutsLists();

        if (doc.shareUrl) {
            try {
                await core.invoke('plugin:share-sheet|share_text', { text: doc.shareUrl });
            } catch {
                // Sharing is optional at creation time — the record is already saved.
            }
        }

        showView(pendingKind === 'estimate' ? 'payouts' : 'list');
        setStatus(pendingKind === 'estimate' ? 'Estimate created!' : 'Invoice created!');
    } catch (err) {
        setStatus(`Couldn't create: ${err}`);
    } finally {
        createSubmitBtn.disabled = false;
    }
});

// ── Detail view ───────────────────────────────────────────────────────────────

function findInvoice(id) {
    return invoices.find((inv) => inv.id === id) || null;
}

function renderDetail(doc) {
    const kind = doc.kind || 'invoice';
    detailTitle.textContent = kind === 'estimate' ? 'Estimate' : 'Invoice';
    detailAmount.textContent = formatAmount(doc.amountCents);
    detailDescription.textContent = doc.description;

    detailFrom.hidden = !doc.fromName;
    detailFrom.textContent = doc.fromName ? `From ${doc.fromName}` : '';
    detailTo.hidden = !doc.toName;
    detailTo.textContent = doc.toName ? `To ${doc.toName}` : '';

    const workDate = formatEpochMsDate(doc.workPerformedAt);
    detailWorkDate.hidden = !workDate;
    detailWorkDate.textContent = workDate ? `Work performed: ${workDate}` : '';

    const dueDate = doc.dueDate ? formatEpochMsDate(doc.dueDate) : null;
    detailDueDate.hidden = !dueDate;
    detailDueDate.textContent = dueDate ? `Due: ${dueDate}` : '';

    const createdDate = formatEpochMsDate(doc.createdAt);
    detailCreatedAt.hidden = !createdDate;
    detailCreatedAt.textContent = createdDate ? `Created: ${createdDate}` : '';

    // Status badge — pending stays invisible on invoices (no news is
    // implicit "waiting"), everything else shows a colored stamp.
    const status = displayStatus(doc);
    if (status === 'pending') {
        detailStatusBadge.hidden = true;
    } else {
        detailStatusBadge.hidden = false;
        detailStatusBadge.className = `status-badge status-${displayStatusClass(status)}`;
        detailStatusBadge.textContent = displayStatusLabel(status);
    }

    detailStatusNote.hidden = !doc.statusNote;
    detailStatusNote.textContent = doc.statusNote ? `Note: ${doc.statusNote}` : '';

    // "Converted to Invoice" link — only shown on an estimate that has
    // been converted. Clicking navigates to the resulting invoice.
    if (kind === 'estimate' && doc.convertedToInvoiceId) {
        detailConvertedLink.hidden = false;
        detailConvertedLink.innerHTML = 'Converted to Invoice — <a href="#" data-invoice-id="' + doc.convertedToInvoiceId + '">open</a>';
        const link = detailConvertedLink.querySelector('a');
        link.addEventListener('click', (e) => {
            e.preventDefault();
            openDetail(link.getAttribute('data-invoice-id'));
        });
    } else {
        detailConvertedLink.hidden = true;
        detailConvertedLink.innerHTML = '';
    }

    if (doc.payUrl) {
        payLinkText.textContent = doc.payUrl;
        payLinkRow.hidden = false;
    } else {
        payLinkRow.hidden = true;
    }

    const payoutView = renderPayoutPanel(doc, { identityStale: payoutIdentityStale });
    renderDetailActions(doc, payoutView);
}

// Populates the .detail-actions container with the buttons that make
// sense for this document's kind + status. All state-changing paths
// funnel through `applyStatus` / `convertEstimate` below so there's one
// call site to disable/refresh from.
// ── Payout state ────────────────────────────────────────────────────────────
//
// eumachia reports what the transfer to the creator's Stripe account did
// (see its payments.js summarizePayout). The app branches on `state` and
// `reason` — never on Stripe's error string, which is shown only as
// supporting detail.
//
// `payoutIdentityStale` overrides the reason: when the invoice names a
// payout identity this install no longer has, retrying can't help, because
// the Stripe account belongs to the previous install.

function describePayout(doc, { identityStale = false } = {}) {
    const payout = doc.payout;

    // Nothing to pay out: this invoice was created before Stripe was
    // connected, so the whole charge stayed with the platform. Retrying
    // can't invent a destination.
    if (payout?.state === 'none' || (!payout && !doc.creatorAddiePubKey)) {
        return {
            tone: 'warn',
            headline: 'Paid — but not routed to you',
            detail: 'This invoice was created before your Stripe account was connected, so it had no payout destination. New invoices will pay out to you.',
        };
    }

    if (payout?.state === 'sent') {
        return {
            tone: 'ok',
            headline: `Paid — ${formatAmount(payout.amount)} sent to your Stripe`,
            detail: 'Stripe moves it to your bank on its usual schedule.',
        };
    }

    if (payout?.state === 'failed') {
        if (identityStale) {
            return {
                tone: 'warn',
                headline: 'Paid — payout went to a previous install',
                detail: 'This invoice names the payout identity from an earlier install of GetPayed, which this device can no longer reach. The payment itself went through; recovering it means using the device that created the invoice.',
                action: 'contact',
            };
        }
        switch (payout.reason) {
            case 'onboarding_incomplete':
                return {
                    tone: 'warn',
                    headline: 'Paid — payout is waiting on your Stripe setup',
                    detail: 'Stripe still needs details from you before it will release money to your account. Finish that and retry — the payment is safe in the meantime.',
                    action: 'finish-setup',
                };
            case 'no_payout_account':
                return {
                    tone: 'warn',
                    headline: 'Paid — no payout account to send it to',
                    detail: 'Set up payouts, then retry.',
                    action: 'finish-setup',
                };
            case 'already_paid_out':
                // Stripe refused a duplicate, which means the first transfer
                // worked. Not a failure to show as one.
                return {
                    tone: 'ok',
                    headline: 'Paid — already sent to your Stripe',
                    detail: null,
                };
            case 'platform_funds':
            case 'unreachable':
                return {
                    tone: 'warn',
                    headline: 'Paid — payout could not be completed yet',
                    detail: 'A temporary problem on our side. The payment went through; retry and it should clear.',
                    action: 'retry',
                };
            case 'payment_not_succeeded':
                return {
                    tone: 'warn',
                    headline: 'Marked paid, but Stripe has no completed charge',
                    detail: 'The payer\'s browser reported success while the charge did not complete. Nothing has been paid out. Worth checking with them before treating this as settled.',
                    action: 'retry',
                };
            default:
                return {
                    tone: 'warn',
                    headline: 'Paid — payout failed',
                    detail: payout.error || 'No further detail from Stripe.',
                    action: 'retry',
                };
        }
    }

    // Paid, with no payout recorded: either paid before eumachia reported
    // payouts, or the record hasn't been fetched yet. Say so plainly
    // instead of implying either outcome.
    return {
        tone: 'unknown',
        headline: 'Paid — payout not confirmed',
        detail: 'Check again to find out whether the money reached your Stripe account.',
        action: 'check',
    };
}

function renderPayoutPanel(doc, options) {
    // Only an online payment has a payout leg. Manually-marked, waived and
    // canceled invoices were settled some other way.
    if ((doc.status || 'pending') !== 'paid_stripe') {
        detailPayout.hidden = true;
        return null;
    }

    const view = describePayout(doc, options);
    detailPayout.hidden = false;
    detailPayout.className = `payout-panel payout-${view.tone}`;
    detailPayoutHeadline.textContent = view.headline;
    detailPayoutDetail.hidden = !view.detail;
    detailPayoutDetail.textContent = view.detail || '';
    return view;
}

function renderDetailActions(doc, payoutView = null) {
    detailActions.innerHTML = '';
    const kind = doc.kind || 'invoice';
    const status = doc.status || 'pending';

    const add = (label, klass, handler) => {
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = `btn ${klass}`;
        btn.textContent = label;
        btn.addEventListener('click', handler);
        detailActions.appendChild(btn);
    };

    if (kind === 'invoice') {
        if (status === 'pending') {
            add('Check Payment', 'btn-secondary', checkPayment);
            add('Mark as Paid Manually', 'btn-secondary', () => applyStatus('paid_manual', 'How was this paid? (optional)'));
            add('Waive', 'btn-secondary', () => applyStatus('waived', 'Reason for waiving? (optional)'));
            add('Cancel Invoice', 'btn-danger', () => applyStatus('canceled', 'Reason for canceling? (optional)'));
        } else {
            // A payout that hasn't landed is the one thing still actionable
            // on an otherwise finished invoice, so it goes first.
            if (payoutView?.action === 'finish-setup') {
                add('Finish Stripe Setup', 'btn-primary', openPayoutsView);
                add('Retry Payout', 'btn-secondary', retryPayout);
            } else if (payoutView?.action === 'retry') {
                add('Retry Payout', 'btn-primary', retryPayout);
            } else if (payoutView?.action === 'check') {
                add('Check Payout', 'btn-secondary', checkPayment);
            }
            // Terminal state — offer a single "Reopen" escape hatch so a
            // mistaken Cancel/Waive isn't a permanent decision.
            add('Reopen (Pending)', 'btn-secondary', () => applyStatus('pending', null));
        }
    } else if (kind === 'estimate') {
        if (status !== 'converted') {
            add('Convert to Invoice', 'btn-primary', () => convertEstimate(doc));
        }
    }
}

async function applyStatus(status, notePrompt) {
    if (!selectedInvoiceId) return;
    let note = null;
    if (notePrompt) {
        // eslint-disable-next-line no-alert
        const entered = prompt(notePrompt);
        if (entered === null) return; // user cancelled the prompt
        note = entered.trim() || null;
    }
    setStatus('Updating…');
    try {
        const updated = await core.invoke('set_invoice_status', {
            id: selectedInvoiceId,
            status,
            note,
        });
        const index = invoices.findIndex((doc) => doc.id === updated.id);
        if (index !== -1) invoices[index] = updated;
        renderDetail(updated);
        renderInvoiceList();
        renderPayoutsLists();
        setStatus('Updated.');
    } catch (err) {
        setStatus(`Couldn't update: ${err}`);
    }
}

async function convertEstimate(estimate) {
    if (!selectedInvoiceId) return;
    if (!stripeConnected) {
        setStatus('Connect a Stripe account before converting an estimate — the resulting invoice needs a payout destination.');
        await openPayoutsView();
        return;
    }
    // eslint-disable-next-line no-alert
    const dueRaw = prompt('Optional due date for the new invoice (YYYY-MM-DD):');
    if (dueRaw === null) return;
    const dueDate = dueRaw.trim()
        ? String(new Date(dueRaw.trim()).getTime())
        : undefined;
    setStatus('Converting estimate…');
    try {
        const newInvoice = await core.invoke('convert_estimate_to_invoice', {
            id: selectedInvoiceId,
            dueDate,
        });
        // Reload the whole store so both the (now-Converted) estimate and
        // the new invoice land in the frontend's in-memory copy.
        invoices = await core.invoke('load_invoices');
        renderInvoiceList();
        renderPayoutsLists();
        openDetail(newInvoice.id);
        setStatus('Estimate converted to invoice.');
    } catch (err) {
        setStatus(`Couldn't convert: ${err}`);
    }
}

async function refreshAfterPaymentCheck(result) {
    payoutIdentityStale = !!result.payoutIdentityStale;
    if (result.changed) {
        invoices = await core.invoke('load_invoices');
        renderInvoiceList();
        renderPayoutsLists();
    }
    const doc = findInvoice(selectedInvoiceId);
    if (doc) renderDetail(doc);
}

async function checkPayment() {
    if (!selectedInvoiceId) return;
    setStatus('Checking for payment…');
    try {
        const result = await core.invoke('check_payment_status', { id: selectedInvoiceId });
        await refreshAfterPaymentCheck(result);
        if (!result.paid) {
            setStatus('Not paid yet.');
        } else if (result.payout?.state === 'sent') {
            setStatus(`Paid — ${formatAmount(result.payout.amount)} sent to your Stripe.`);
        } else if (result.payout?.state === 'failed') {
            setStatus('Paid, but the payout to you did not go through.');
        } else {
            setStatus('Payment received.');
        }
    } catch (err) {
        setStatus(`Couldn't check payment: ${err}`);
    }
}

async function retryPayout() {
    if (!selectedInvoiceId) return;
    setStatus('Retrying the payout…');
    try {
        const result = await core.invoke('retry_payout', { id: selectedInvoiceId });
        await refreshAfterPaymentCheck(result);
        if (result.payout?.state === 'sent') {
            setStatus(`Sent — ${formatAmount(result.payout.amount)} is on its way to your Stripe.`);
        } else {
            setStatus('Still not through. Nothing was charged again.');
        }
    } catch (err) {
        setStatus(`Couldn't retry: ${err}`);
    }
}

function openDetail(id) {
    // Belongs to whichever invoice was last checked, so it must not leak
    // across to this one.
    if (id !== selectedInvoiceId) payoutIdentityStale = false;
    selectedInvoiceId = id;
    const inv = findInvoice(id);
    if (!inv) return;
    renderDetail(inv);
    showView('detail');
}

copyPayLinkBtn.addEventListener('click', async () => {
    try {
        await navigator.clipboard.writeText(payLinkText.textContent);
        setStatus('Copied!');
    } catch (err) {
        setStatus(`Couldn't copy: ${err}`);
    }
});

shareAgainBtn.addEventListener('click', async () => {
    const inv = findInvoice(selectedInvoiceId);
    if (!inv?.shareUrl) return;
    shareAgainBtn.disabled = true;
    try {
        await core.invoke('plugin:share-sheet|share_text', { text: inv.shareUrl });
    } catch (err) {
        setStatus(`Couldn't share: ${err}`);
    } finally {
        shareAgainBtn.disabled = false;
    }
});

// ── Canonical profile ────────────────────────────────────────────────────────
//
// A separate, App-Group-shared record — independent of `invoices` above.
// Mirrors slugify() in src-tauri/src/lib.rs.

let preProfileView = 'list'; // the view to return to on Close/Save
let pendingProfilePhoto = null;
let pendingProfileFields = [];
let editingProfileFieldIndex = null;

function getInitials(name) {
    if (!name) return '?';
    return name
        .split(/\s+/)
        .filter(Boolean)
        .map((word) => word[0])
        .join('')
        .slice(0, 2)
        .toUpperCase();
}

function setAvatarContent(el, photo, name) {
    if (photo) {
        el.style.backgroundImage = `url(data:image/jpeg;base64,${photo})`;
        el.textContent = '';
    } else {
        el.style.backgroundImage = '';
        el.textContent = getInitials(name);
    }
}

async function resizeImageToJpegBase64(bytes) {
    const blob = new Blob([bytes]);
    const bitmap = await createImageBitmap(blob);

    const scale = Math.min(1, PHOTO_SIZE / Math.max(bitmap.width, bitmap.height));
    const width = Math.round(bitmap.width * scale);
    const height = Math.round(bitmap.height * scale);

    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext('2d');
    ctx.drawImage(bitmap, 0, 0, width, height);

    const dataUrl = canvas.toDataURL('image/jpeg', PHOTO_QUALITY);
    return dataUrl.split(',')[1];
}

function slugify(s) {
    return s
        .trim()
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '_')
        .replace(/^_+|_+$/g, '');
}

function renderProfileFields() {
    profileFieldsEl.innerHTML = '';

    pendingProfileFields.forEach((entry, index) => {
        const li = document.createElement('li');
        li.className = 'link-entry';

        if (index === editingProfileFieldIndex) {
            const fields = document.createElement('div');
            fields.className = 'link-entry-edit-fields';

            const nameInput = document.createElement('input');
            nameInput.type = 'text';
            nameInput.placeholder = 'Field';
            nameInput.maxLength = 40;
            nameInput.value = entry.name;

            const valueInput = document.createElement('input');
            valueInput.type = 'text';
            valueInput.placeholder = 'Value';
            valueInput.value = entry.value;

            const commit = () => {
                entry.name = nameInput.value.trim();
                entry.value = valueInput.value.trim();
                entry.slug = slugify(entry.name);
                editingProfileFieldIndex = null;
                renderProfileFields();
            };
            const onEnter = (e) => { if (e.key === 'Enter') commit(); };
            nameInput.addEventListener('keydown', onEnter);
            valueInput.addEventListener('keydown', onEnter);

            fields.append(nameInput, valueInput);
            li.appendChild(fields);

            const doneBtn = document.createElement('button');
            doneBtn.type = 'button';
            doneBtn.textContent = '✓';
            doneBtn.addEventListener('click', commit);

            const actions = document.createElement('div');
            actions.className = 'link-entry-actions';
            actions.appendChild(doneBtn);
            li.appendChild(actions);

            profileFieldsEl.appendChild(li);
            nameInput.focus();
            return;
        }

        const text = document.createElement('div');
        text.className = 'link-entry-text';
        text.innerHTML = '<div class="link-entry-label"></div><div class="link-entry-url"></div>';
        text.querySelector('.link-entry-label').textContent = entry.name || entry.slug;
        text.querySelector('.link-entry-url').textContent = entry.value;
        text.addEventListener('click', () => {
            editingProfileFieldIndex = index;
            renderProfileFields();
        });
        li.appendChild(text);

        const actions = document.createElement('div');
        actions.className = 'link-entry-actions';

        const upBtn = document.createElement('button');
        upBtn.type = 'button';
        upBtn.textContent = '↑';
        upBtn.disabled = index === 0;
        upBtn.addEventListener('click', () => {
            [pendingProfileFields[index - 1], pendingProfileFields[index]] = [pendingProfileFields[index], pendingProfileFields[index - 1]];
            renderProfileFields();
        });

        const downBtn = document.createElement('button');
        downBtn.type = 'button';
        downBtn.textContent = '↓';
        downBtn.disabled = index === pendingProfileFields.length - 1;
        downBtn.addEventListener('click', () => {
            [pendingProfileFields[index], pendingProfileFields[index + 1]] = [pendingProfileFields[index + 1], pendingProfileFields[index]];
            renderProfileFields();
        });

        const removeBtn = document.createElement('button');
        removeBtn.type = 'button';
        removeBtn.textContent = '×';
        removeBtn.addEventListener('click', () => {
            pendingProfileFields.splice(index, 1);
            if (editingProfileFieldIndex === index) editingProfileFieldIndex = null;
            renderProfileFields();
        });

        actions.append(upBtn, downBtn, removeBtn);
        li.appendChild(actions);
        profileFieldsEl.appendChild(li);
    });

    const atLimit = pendingProfileFields.length >= MAX_PROFILE_FIELDS;
    profileAddFieldBtn.disabled = atLimit;
    profileFieldLimitHint.hidden = !atLimit;
}

profileAddFieldBtn.addEventListener('click', () => {
    const name = profileNewFieldName.value.trim();
    const value = profileNewFieldValue.value.trim();
    if (!name || !value || pendingProfileFields.length >= MAX_PROFILE_FIELDS) return;

    pendingProfileFields.push({ slug: slugify(name), name, value });
    profileNewFieldName.value = '';
    profileNewFieldValue.value = '';
    renderProfileFields();
});

profileChoosePhotoBtn.addEventListener('click', async () => {
    try {
        const path = await dialog.open({
            multiple: false,
            filters: [{ name: 'Image', extensions: ['png', 'jpg', 'jpeg', 'heic'] }],
        });
        if (!path) return;

        const bytes = await fs.readFile(path);
        pendingProfilePhoto = await resizeImageToJpegBase64(bytes);
        setAvatarContent(profilePhotoPreview, pendingProfilePhoto, '');
    } catch (err) {
        setStatus(`Couldn't set photo: ${err}`);
    }
});

function fillProfileForm(profile) {
    pendingProfilePhoto = profile?.photo || null;
    pendingProfileFields = (profile?.fields || []).map((f) => ({ ...f }));
    editingProfileFieldIndex = null;
    setAvatarContent(profilePhotoPreview, profile?.photo, '');
    renderProfileFields();
}

function canonicalProfileFromForm() {
    return {
        photo: pendingProfilePhoto || undefined,
        fields: pendingProfileFields,
    };
}

function currentViewName() {
    if (!createView.hidden) return 'create';
    if (!detailView.hidden) return 'detail';
    return 'list';
}

// ── Payouts ───────────────────────────────────────────────────────────────────
//
// Gelder-specific, unlike Profile — not shared via the App Group, since
// connecting Stripe is a real ToS-acceptance action tied to this app's own
// Addie identity, not shared profile data.

let prePayoutsView = 'list';

// Onboarding runs inside the app via the Stripe Connect iOS SDK
// (tauri-plugin-stripe-connect), so there's no browser hand-off and no
// "did you finish?" guesswork: when the sheet closes we ask Stripe what the
// account's actual state is. `connected` means Stripe activated the
// transfers capability — the thing eumachia needs to pay the creator —
// not merely that an account exists.

let payoutStatus = { connected: false, hasAccount: false };

function renderPayoutState(status) {
    payoutStatus = status || { connected: false, hasAccount: false };
    stripeConnected = !!payoutStatus.connected;

    const ready = stripeConnected;
    // Details are in and Stripe hasn't asked for anything else — it's
    // verifying. Distinguished from "unfinished" so we don't nag the user
    // to go re-enter details that are already submitted and under review.
    const verifying = !ready && payoutStatus.detailsSubmitted && !payoutStatus.requirementsDue;
    const incomplete = !ready && !verifying && payoutStatus.hasAccount;

    payoutsConnected.hidden = !ready;
    payoutsVerifying.hidden = !verifying;
    payoutsIncomplete.hidden = !incomplete;
    payoutsForm.hidden = ready || verifying || incomplete;

    if (incomplete) {
        payoutsIncompleteHint.textContent = payoutStatus.disabledReason
            ? `Stripe needs more information before you can be paid (${payoutStatus.disabledReason}).`
            : 'Stripe still needs a few details before you can be paid.';
    }

    updateInvoiceCreationGate();
}

// Cached status: instant, works offline, correct at launch.
async function renderPayoutStatus() {
    try {
        renderPayoutState(await core.invoke('get_payout_status'));
    } catch (err) {
        setStatus(`Couldn't load payout status: ${err}`);
    }
    // Best-effort: let sibling apps (idothis) gate on the same fact.
    core.invoke('publish_stripe_connected').catch(() => {});
}

// Asks Stripe for the live answer. Separate from the above because it's a
// network round trip — used when the Payouts view opens and after onboarding.
async function refreshPayoutStatus({ quiet = false } = {}) {
    try {
        renderPayoutState(await core.invoke('refresh_payout_status'));
        core.invoke('publish_stripe_connected').catch(() => {});
    } catch (err) {
        if (!quiet) setStatus(`Couldn't check with Stripe: ${err}`);
    }
}

async function openPayoutsView() {
    prePayoutsView = currentViewName();
    renderPayoutsLists();
    await renderPayoutStatus();
    showView('payouts');
    // The cached state is on screen already; correct it in the background.
    refreshPayoutStatus({ quiet: true });
}

payoutsNavBtn.addEventListener('click', openPayoutsView);
stripeRequiredConnectBtn.addEventListener('click', openPayoutsView);

// Onboarding can also be finished on another device (or in Stripe's own
// dashboard), so re-check whenever the user comes back to this view.
document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible' && !payoutsView.hidden) {
        refreshPayoutStatus({ quiet: true });
    }
});

// Presents Stripe's onboarding sheet. `country`/`email` are only used the
// first time, to create the account; afterwards the same call reopens
// onboarding where the user left off, which is also how "Finish Setup" and
// "Update Payout Details" work.
async function presentStripeOnboarding({ country, email } = {}) {
    const session = await core.invoke('start_stripe_onboarding', {
        country: country ?? null,
        email: email ?? null,
    });

    // Sessions expire; the SDK asks for a fresh secret through this channel
    // rather than tearing down the sheet the user is in the middle of.
    const onRefresh = new Channel();
    onRefresh.onmessage = async () => {
        let clientSecret = null;
        try {
            clientSecret = (await core.invoke('start_stripe_onboarding', { country: null, email: null })).clientSecret;
        } catch (err) {
            setStatus(`Stripe session expired: ${err}`);
        }
        await core.invoke('plugin:stripe-connect|provide_client_secret', { clientSecret });
    };

    const result = await core.invoke('plugin:stripe-connect|present_onboarding', {
        publishableKey: session.publishableKey,
        clientSecret: session.clientSecret,
        onRefresh,
    });
    if (result?.error) setStatus(`Stripe onboarding: ${result.error}`);

    // The sheet closing says nothing about whether the account is payable —
    // only Stripe can answer that.
    await refreshPayoutStatus();
    if (payoutStatus.connected) {
        setStatus('Payouts are set up — you can create invoices now.');
    } else if (payoutStatus.detailsSubmitted) {
        setStatus('Details submitted. Stripe is reviewing them.');
    } else {
        setStatus('Setup is unfinished — you can pick up where you left off.');
    }
}

payoutsForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    const country = payoutsCountry.value.trim().toUpperCase();
    const email = payoutsEmail.value.trim();
    if (!country || !email) return;

    payoutsConnectBtn.disabled = true;
    setStatus('Opening Stripe…');
    try {
        await presentStripeOnboarding({ country, email });
    } catch (err) {
        setStatus(`Couldn't start setup: ${err}`);
    } finally {
        payoutsConnectBtn.disabled = false;
    }
});

for (const btn of [payoutsResumeBtn, payoutsManageBtn]) {
    btn.addEventListener('click', async () => {
        btn.disabled = true;
        setStatus('Opening Stripe…');
        try {
            await presentStripeOnboarding();
        } catch (err) {
            setStatus(`Couldn't open Stripe: ${err}`);
        } finally {
            btn.disabled = false;
        }
    });
}

payoutsRefreshBtn.addEventListener('click', async () => {
    payoutsRefreshBtn.disabled = true;
    setStatus('Checking with Stripe…');
    try {
        await refreshPayoutStatus();
        setStatus(payoutStatus.connected ? 'Ready to get paid.' : 'Stripe is still reviewing your details.');
    } finally {
        payoutsRefreshBtn.disabled = false;
    }
});

payoutsCloseBtn.addEventListener('click', () => showView(prePayoutsView));

profileNavBtn.addEventListener('click', async () => {
    preProfileView = currentViewName();
    try {
        const profile = await core.invoke('load_canonical_profile');
        fillProfileForm(profile);
        showView('profile');
    } catch (err) {
        setStatus(`Couldn't load profile: ${err}`);
    }
});

profileForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    try {
        await core.invoke('save_canonical_profile', { profile: canonicalProfileFromForm() });
        setStatus('Profile saved — shared across your apps.');
        showView(preProfileView);
    } catch (err) {
        setStatus(`Couldn't save: ${err}`);
    }
});

profileCloseBtn.addEventListener('click', () => showView(preProfileView));

// No deep-link handling: onboarding happens inside the app now, so nothing
// leaves it and comes back. The gelder:// scheme stays registered (see
// tauri.conf.json) so old Stripe return URLs still open the app rather than
// failing — they just land on the normal launch view.

showView('list');
loadInvoices();
renderPayoutStatus();
