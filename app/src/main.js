const { core, dialog, fs } = window.__TAURI__;

// Keep in sync with MAX_CANONICAL_FIELDS in src-tauri/src/lib.rs.
const MAX_PROFILE_FIELDS = 20;

const listView = document.getElementById('list-view');
const createView = document.getElementById('create-view');
const detailView = document.getElementById('detail-view');
const profileView = document.getElementById('profile-view');
const profileNavBtn = document.getElementById('profile-nav-btn');
const payoutsView = document.getElementById('payouts-view');
const payoutsNavBtn = document.getElementById('payouts-nav-btn');
const stripeCtaBtn = document.getElementById('stripe-cta-btn');
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
const createSubmitBtn = document.getElementById('create-submit-btn');
const cancelCreateBtn = document.getElementById('cancel-create-btn');

const detailAmount = document.getElementById('detail-amount');
const detailDescription = document.getElementById('detail-description');
const detailFrom = document.getElementById('detail-from');
const detailTo = document.getElementById('detail-to');
const detailWorkDate = document.getElementById('detail-work-date');
const detailCreatedAt = document.getElementById('detail-created-at');
const paidBadge = document.getElementById('paid-badge');
const payLinkRow = document.getElementById('pay-link-row');
const payLinkText = document.getElementById('pay-link-text');
const copyPayLinkBtn = document.getElementById('copy-pay-link-btn');
const shareAgainBtn = document.getElementById('share-again-btn');
const checkPaymentBtn = document.getElementById('check-payment-btn');
const markPaidBtn = document.getElementById('mark-paid-btn');
const detailBackBtn = document.getElementById('detail-back-btn');

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
const payoutsPending = document.getElementById('payouts-pending');
const payoutsReopenBtn = document.getElementById('payouts-reopen-btn');
const payoutsDoneBtn = document.getElementById('payouts-done-btn');
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

// ── View / status helpers ────────────────────────────────────────────────────

function updateStripeCtaVisibility() {
    const hideFooterNav = !profileView.hidden || !payoutsView.hidden;
    stripeCtaBtn.hidden = hideFooterNav || stripeConnected;
}

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
    profileNavBtn.hidden = name === 'profile' || name === 'payouts';
    payoutsNavBtn.hidden = name === 'profile' || name === 'payouts';
    updateStripeCtaVisibility();
}

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

    // Most recently worked-on first — what a freelancer actually cares
    // about ordering by, not whichever moment the record happened to be
    // typed into the app (createdAt).
    const sorted = [...invoices].sort((a, b) => Number(b.workPerformedAt) - Number(a.workPerformedAt));
    for (const inv of sorted) {
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
        const badge = document.createElement('div');
        badge.className = inv.paid ? 'invoice-list-badge paid' : 'invoice-list-badge';
        badge.textContent = inv.paid ? 'Paid' : 'Unpaid';
        right.append(amountEl, badge);

        li.append(text, right);
        li.addEventListener('click', () => openDetail(inv.id));
        invoiceListEl.appendChild(li);
    }
}

async function loadInvoices() {
    invoices = await core.invoke('load_invoices');
    renderInvoiceList();
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

async function openCreateForm() {
    if (!stripeConnected) {
        setStatus('Connect a Stripe account before creating invoices.');
        await openPayoutsView();
        return;
    }

    fieldToName.value = '';
    fieldDescription.value = '';
    fieldAmount.value = '';
    fieldWorkDate.value = toDatetimeLocalValue(new Date());

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
        fromNote.textContent = 'No shared profile name set yet — set one in Profile so invoices show who they’re from.';
    }

    showView('create');
}

newInvoiceBtn.addEventListener('click', openCreateForm);
cancelCreateBtn.addEventListener('click', () => showView('list'));

invoiceForm.addEventListener('submit', async (e) => {
    e.preventDefault();

    const description = fieldDescription.value.trim();
    const amount = parseFloat(fieldAmount.value);
    if (!description || !Number.isFinite(amount) || amount <= 0 || !fieldWorkDate.value) return;
    const amountCents = Math.round(amount * 100);
    const toName = fieldToName.value.trim() || undefined;
    const workPerformedAt = String(new Date(fieldWorkDate.value).getTime());

    createSubmitBtn.disabled = true;
    setStatus('Publishing invoice…');
    try {
        const invoice = await core.invoke('create_invoice', {
            description,
            amountCents,
            toName,
            fromName: cachedProfileFromName,
            workPerformedAt,
        });
        invoices.push(invoice);
        renderInvoiceList();

        if (invoice.shareUrl) {
            try {
                await core.invoke('plugin:share-sheet|share_text', { text: invoice.shareUrl });
            } catch {
                // Sharing is optional at creation time — the invoice is already saved.
            }
        }

        showView('list');
        setStatus('Invoice created!');
    } catch (err) {
        setStatus(`Couldn't create invoice: ${err}`);
    } finally {
        createSubmitBtn.disabled = false;
    }
});

// ── Detail view ───────────────────────────────────────────────────────────────

function findInvoice(id) {
    return invoices.find((inv) => inv.id === id) || null;
}

function renderDetail(inv) {
    detailAmount.textContent = formatAmount(inv.amountCents);
    detailDescription.textContent = inv.description;

    detailFrom.hidden = !inv.fromName;
    detailFrom.textContent = inv.fromName ? `From ${inv.fromName}` : '';
    detailTo.hidden = !inv.toName;
    detailTo.textContent = inv.toName ? `To ${inv.toName}` : '';

    const workDate = formatEpochMsDate(inv.workPerformedAt);
    detailWorkDate.hidden = !workDate;
    detailWorkDate.textContent = workDate ? `Work performed: ${workDate}` : '';

    const createdDate = formatEpochMsDate(inv.createdAt);
    detailCreatedAt.hidden = !createdDate;
    detailCreatedAt.textContent = createdDate ? `Created: ${createdDate}` : '';

    paidBadge.hidden = !inv.paid;
    markPaidBtn.hidden = inv.paid;
    checkPaymentBtn.hidden = inv.paid;

    if (inv.payUrl) {
        payLinkText.textContent = inv.payUrl;
        payLinkRow.hidden = false;
    } else {
        payLinkRow.hidden = true;
    }
}

function openDetail(id) {
    selectedInvoiceId = id;
    const inv = findInvoice(id);
    if (!inv) return;
    renderDetail(inv);
    showView('detail');
}

detailBackBtn.addEventListener('click', () => showView('list'));

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

markPaidBtn.addEventListener('click', async () => {
    if (!selectedInvoiceId) return;
    markPaidBtn.disabled = true;
    setStatus('Marking as paid…');
    try {
        const updated = await core.invoke('mark_invoice_paid', { id: selectedInvoiceId });
        const index = invoices.findIndex((inv) => inv.id === updated.id);
        if (index !== -1) invoices[index] = updated;
        renderDetail(updated);
        renderInvoiceList();
        setStatus('Marked as paid.');
    } catch (err) {
        setStatus(`Couldn't update: ${err}`);
    } finally {
        markPaidBtn.disabled = false;
    }
});

checkPaymentBtn.addEventListener('click', async () => {
    if (!selectedInvoiceId) return;
    checkPaymentBtn.disabled = true;
    setStatus('Checking for payment…');
    try {
        const paidNow = await core.invoke('check_payment_status', { id: selectedInvoiceId });
        if (paidNow) {
            const updated = await core.invoke('load_invoices');
            invoices = updated;
            const inv = findInvoice(selectedInvoiceId);
            if (inv) renderDetail(inv);
            renderInvoiceList();
            setStatus('Payment received!');
        } else {
            setStatus('Not paid yet.');
        }
    } catch (err) {
        setStatus(`Couldn't check payment: ${err}`);
    } finally {
        checkPaymentBtn.disabled = false;
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

// Addie marks the Express account "connected" the instant it's created, not
// once the user actually finishes Stripe's hosted onboarding — there's no
// endpoint yet to check real charges_enabled/payouts_enabled status. So
// this local flag (persisted across app backgrounding, since onboarding
// happens in an external browser) takes priority over the backend's
// technically-premature "connected" signal until the user confirms they're
// done. It's honest about being a self-report, not a verified fact.
const STRIPE_ONBOARDING_PENDING_KEY = 'gelder.stripeOnboardingPending';
const STRIPE_ONBOARDING_URL_KEY = 'gelder.stripeOnboardingUrl';

function isOnboardingPending() {
    return localStorage.getItem(STRIPE_ONBOARDING_PENDING_KEY) === '1';
}

function setOnboardingPending(url) {
    localStorage.setItem(STRIPE_ONBOARDING_PENDING_KEY, '1');
    localStorage.setItem(STRIPE_ONBOARDING_URL_KEY, url);
}

function clearOnboardingPending() {
    localStorage.removeItem(STRIPE_ONBOARDING_PENDING_KEY);
    localStorage.removeItem(STRIPE_ONBOARDING_URL_KEY);
}

async function openInBrowser(url) {
    await core.invoke('plugin:shell|open', { path: url });
}

async function renderPayoutStatus() {
    try {
        const status = await core.invoke('get_payout_status');
        const pending = isOnboardingPending();
        stripeConnected = !!status.connected && !pending;
        payoutsConnected.hidden = !stripeConnected;
        payoutsPending.hidden = !pending;
        payoutsForm.hidden = stripeConnected || pending;
    } catch (err) {
        setStatus(`Couldn't load payout status: ${err}`);
    }
    updateStripeCtaVisibility();
    updateInvoiceCreationGate();
}

async function openPayoutsView() {
    prePayoutsView = currentViewName();
    await renderPayoutStatus();
    showView('payouts');
}

payoutsNavBtn.addEventListener('click', openPayoutsView);
stripeCtaBtn.addEventListener('click', openPayoutsView);
stripeRequiredConnectBtn.addEventListener('click', openPayoutsView);

// Re-check whenever the app regains focus — the user completes onboarding
// in the system browser, then switches back to Gelder.
document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible' && !payoutsView.hidden) {
        renderPayoutStatus();
    }
});

payoutsForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    const country = payoutsCountry.value.trim().toUpperCase();
    const email = payoutsEmail.value.trim();
    if (!country || !email) return;

    payoutsConnectBtn.disabled = true;
    setStatus('Starting Stripe onboarding…');
    try {
        const result = await core.invoke('connect_stripe_account', { country, email });
        if (result.onboardingUrl) {
            setOnboardingPending(result.onboardingUrl);
            await openInBrowser(result.onboardingUrl);
            setStatus('Finish onboarding in your browser, then come back here.');
        } else if (result.alreadyConnected) {
            clearOnboardingPending();
            setStatus('Stripe account connected!');
        }
        await renderPayoutStatus();
    } catch (err) {
        setStatus(`Couldn't connect: ${err}`);
    } finally {
        payoutsConnectBtn.disabled = false;
    }
});

payoutsReopenBtn.addEventListener('click', async () => {
    const url = localStorage.getItem(STRIPE_ONBOARDING_URL_KEY);
    if (!url) return;
    try {
        await openInBrowser(url);
    } catch (err) {
        setStatus(`Couldn't reopen: ${err}`);
    }
});

payoutsDoneBtn.addEventListener('click', async () => {
    clearOnboardingPending();
    await renderPayoutStatus();
    setStatus(stripeConnected ? 'Stripe account connected!' : 'Status updated.');
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

// ── Deep links ───────────────────────────────────────────────────────────────
//
// Stripe's hosted onboarding returns to gelder://stripe-return (see
// STRIPE_ONBOARDING_RETURN_URL in src-tauri/src/lib.rs) once the user
// finishes or abandons the flow. iOS hands that off to us here instead of
// requiring the "I've Finished Onboarding" button — that button stays as a
// fallback for cases where the OS doesn't switch back automatically.

function handleDeepLink(url) {
    let parsed;
    try {
        parsed = new URL(url);
    } catch {
        return;
    }
    if (parsed.protocol !== 'gelder:') return;
    if (parsed.hostname === 'stripe-return' || parsed.pathname.replace(/^\/+/, '') === 'stripe-return') {
        clearOnboardingPending();
        renderPayoutStatus();
        setStatus(stripeConnected ? 'Stripe account connected!' : 'Welcome back — checking payout status…');
    }
}

if (window.__TAURI__?.event) {
    // Registers the gelder:// scheme with the OS on desktop; unsupported (and
    // unnecessary) on iOS, where the scheme comes from Info.plist instead.
    core.invoke('plugin:deep-link|register', { protocols: ['gelder'] }).catch(() => {});

    window.__TAURI__.event.listen('deep-link://new-url', (event) => {
        const urls = event.payload;
        if (Array.isArray(urls)) urls.forEach(handleDeepLink);
        else if (typeof urls === 'string') handleDeepLink(urls);
    });
}

// Cold start: the new-url event fires before the listener above is
// registered, so check for a launch URL explicitly once the webview is up.
core.invoke('plugin:deep-link|get_current').then(urls => {
    if (!urls || (Array.isArray(urls) && urls.length === 0)) return;
    setTimeout(() => {
        if (Array.isArray(urls)) urls.forEach(handleDeepLink);
        else if (typeof urls === 'string') handleDeepLink(urls);
    }, 300);
}).catch(() => {});

showView('list');
loadInvoices();
renderPayoutStatus();
