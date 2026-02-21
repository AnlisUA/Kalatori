'use strict';

function getHtml() {
  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Kalatori Webhook Simulator</title>
<style>
*, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; line-height: 1.5; color: #1a1a1a; background: #f5f5f5; padding: 24px; max-width: 900px; margin: 0 auto; }
h1 { font-size: 1.5rem; margin-bottom: 4px; }
.subtitle { color: #666; font-size: 0.9rem; margin-bottom: 20px; }
.section { background: #fff; border: 1px solid #ddd; border-radius: 8px; padding: 20px; margin-bottom: 16px; }
.section h2 { font-size: 1.1rem; margin-bottom: 12px; border-bottom: 1px solid #eee; padding-bottom: 8px; }
label { display: block; font-weight: 600; font-size: 0.85rem; margin-bottom: 4px; color: #333; }
input[type="text"], input[type="password"], select { width: 100%; padding: 8px 12px; border: 1px solid #ccc; border-radius: 4px; font-size: 0.9rem; font-family: inherit; }
.secret-row { display: flex; gap: 8px; }
.secret-row input { flex: 1; }
.btn-toggle { padding: 8px 12px; border: 1px solid #ccc; border-radius: 4px; background: #f9fafb; cursor: pointer; font-size: 0.8rem; white-space: nowrap; color: #555; }
.btn-toggle:hover { background: #e5e7eb; }
input:focus, select:focus, textarea:focus { outline: none; border-color: #4a90d9; box-shadow: 0 0 0 2px rgba(74,144,217,0.2); }
.field { margin-bottom: 12px; }
.row { display: flex; gap: 12px; }
.row > .field { flex: 1; }
textarea#payload-editor { width: 100%; min-height: 300px; font-family: "SF Mono", "Fira Code", "Fira Mono", "Roboto Mono", monospace; font-size: 0.82rem; padding: 12px; border: 1px solid #ccc; border-radius: 4px; resize: vertical; line-height: 1.4; tab-size: 2; }
textarea#payload-editor.invalid { border-color: #d32f2f; background: #fff5f5; }
.btn { display: inline-block; padding: 10px 20px; border: none; border-radius: 4px; font-size: 0.9rem; font-weight: 600; cursor: pointer; transition: background 0.15s; }
.btn-primary { background: #2563eb; color: #fff; }
.btn-primary:hover { background: #1d4ed8; }
.btn-primary:disabled { background: #93c5fd; cursor: not-allowed; }
.btn-secondary { background: #e5e7eb; color: #333; }
.btn-secondary:hover { background: #d1d5db; }
.btn-small { padding: 6px 12px; font-size: 0.8rem; }
.btn-row { display: flex; gap: 8px; align-items: center; margin-bottom: 12px; }
#result-panel { display: none; }
.result-status { font-size: 1.1rem; font-weight: 700; padding: 8px 0; }
.result-status.success { color: #16a34a; }
.result-status.failure { color: #dc2626; }
.result-detail { margin-bottom: 8px; }
.result-detail summary { cursor: pointer; font-weight: 600; font-size: 0.85rem; color: #555; }
.result-detail pre { background: #f8f8f8; border: 1px solid #e5e5e5; border-radius: 4px; padding: 10px; font-size: 0.8rem; overflow-x: auto; margin-top: 4px; white-space: pre-wrap; word-break: break-all; }
.production-note { background: #fffbeb; border: 1px solid #fbbf24; border-radius: 4px; padding: 12px; margin-top: 12px; font-size: 0.85rem; line-height: 1.5; }
.production-note strong { color: #92400e; }
.info-box { background: #eff6ff; border: 1px solid #bfdbfe; border-radius: 4px; padding: 12px; font-size: 0.85rem; margin-bottom: 16px; }
.error-text { color: #dc2626; font-size: 0.8rem; margin-top: 4px; }
.self-test-results { margin-top: 8px; font-size: 0.85rem; }
.test-pass { color: #16a34a; }
.test-fail { color: #dc2626; }
.spinner { display: inline-block; width: 16px; height: 16px; border: 2px solid #93c5fd; border-top-color: #2563eb; border-radius: 50%; animation: spin 0.6s linear infinite; vertical-align: middle; margin-right: 6px; }
@keyframes spin { to { transform: rotate(360deg); } }
</style>
</head>
<body>

<h1>Kalatori Webhook Simulator</h1>
<p class="subtitle">Test your webhook endpoint against Kalatori's exact signing and delivery behavior.</p>

<div class="info-box">
  This tool sends a single webhook request with a properly signed HMAC-SHA256 payload, matching
  Kalatori's production behavior. Requests are sent server-side (via local proxy), so there are no
  CORS restrictions &mdash; just like production. It does <strong>not</strong> retry on failure &mdash;
  instead it reports what would happen in production.
</div>

<!-- Configuration -->
<div class="section">
  <h2>Configuration</h2>
  <div class="field">
    <label for="webhook-url">Webhook URL</label>
    <input type="text" id="webhook-url" placeholder="https://your-server.com/webhooks/invoices" value="http://localhost:8000/webhooks/invoices">
  </div>
  <div class="field">
    <label for="secret-key">HMAC Secret Key</label>
    <div class="secret-row">
      <input type="password" id="secret-key" placeholder="your-shared-secret" value="secret">
      <button type="button" class="btn-toggle" id="toggle-secret" onclick="toggleSecret()">Show</button>
    </div>
  </div>
</div>

<!-- Event Selection -->
<div class="section">
  <h2>Event</h2>
  <div class="row">
    <div class="field">
      <label for="event-type">Event Type</label>
      <select id="event-type">
        <option value="created">created</option>
        <option value="updated">updated</option>
        <option value="paid">paid</option>
        <option value="partially_paid">partially_paid</option>
        <option value="expired">expired</option>
        <option value="admin_canceled">admin_canceled</option>
        <option value="customer_canceled">customer_canceled</option>
      </select>
    </div>
    <div class="field">
      <label for="invoice-status">Invoice Status</label>
      <select id="invoice-status">
        <option value="Waiting">Waiting</option>
      </select>
    </div>
  </div>
</div>

<!-- Payload Editor -->
<div class="section">
  <h2>Payload</h2>
  <div class="btn-row">
    <button class="btn btn-secondary btn-small" onclick="regeneratePayload()">Regenerate Payload</button>
    <button class="btn btn-secondary btn-small" onclick="formatPayload()">Format JSON</button>
    <span id="json-error" class="error-text"></span>
  </div>
  <textarea id="payload-editor" spellcheck="false"></textarea>
</div>

<!-- Send -->
<div style="margin-bottom: 16px;">
  <button class="btn btn-primary" id="send-btn" onclick="sendWebhook()">Send Webhook</button>
</div>

<!-- Result -->
<div class="section" id="result-panel">
  <h2>Result</h2>
  <div id="result-status" class="result-status"></div>

  <details class="result-detail" open>
    <summary>Request Details</summary>
    <pre id="request-details"></pre>
  </details>

  <details class="result-detail" open>
    <summary>Response</summary>
    <pre id="response-details"></pre>
  </details>

  <div id="production-note" class="production-note"></div>
</div>

<!-- Self-Test -->
<div class="section">
  <h2>Self-Test (HMAC Verification)</h2>
  <p style="font-size: 0.85rem; margin-bottom: 8px;">
    Verify that this tool's HMAC-SHA256 implementation matches Kalatori's Rust implementation
    using pre-computed test vectors.
  </p>
  <button class="btn btn-secondary btn-small" onclick="runSelfTest()">Run Self-Test</button>
  <div id="self-test-results" class="self-test-results"></div>
</div>

<script>
// ============================================================================
// Test Vectors (generated from Rust code via generate_hmac_test_vectors.rs)
// ============================================================================
const TEST_VECTORS = [
  {"body":"{\\"id\\":\\"test\\"}","expected_signature":"4c6738b4a89261548d5b7a031a4c61c092a6a4adaa6487a708cdc59e95d7d391","method":"POST","path":"/webhooks/invoices","secret":"secret","timestamp":"1706745600"},
  {"body":"","expected_signature":"79f3f1c0508a9936b7679445c534f9d8809f81405f9f938db7cc7ffd274254e5","method":"POST","path":"/webhooks/invoices","secret":"secret","timestamp":"1706745600"},
  {"body":"{\\"event_type\\":\\"created\\",\\"payload\\":{\\"amount\\":\\"100.00\\"}}","expected_signature":"ed859d57b84464396bcafdae8e1f826cc8f978f65a68121d4f4a055c9266290f","method":"POST","path":"/api/v3/webhooks","secret":"my-webhook-secret-key-2024","timestamp":"1700000000"},
  {"body":"","expected_signature":"edca57bf46268d48a7997fdee7a90786fa724e255f6bd57992dffb17875ace0a","method":"GET","path":"/webhooks/invoices","secret":"secret","timestamp":"1706745600"},
  {"body":"{\\"test\\":true}","expected_signature":"62186c90aad3845be9c5ef87e45d60259efc87650e0d71962edfd136cef550f0","method":"POST","path":"/webhooks/invoices","secret":"a-very-long-secret-key-that-is-longer-than-sixty-four-bytes-to-test-hmac-key-hashing-behavior","timestamp":"1706745600"},
  {"body":"{\\"name\\":\\"caf\\u00e9\\",\\"price\\":\\"\\u20ac100.00\\",\\"note\\":\\"line1\\\\nline2\\"}","expected_signature":"5e95ff1d2f1d58b81be6d29bfce1f0cde0fc7c5009bbddc891727c4cb8e080de","method":"POST","path":"/webhooks/invoices","secret":"secret","timestamp":"1706745600"},
  {"body":"{\\"event_entity\\":\\"invoice\\",\\"event_type\\":\\"created\\",\\"id\\":\\"550e8400-e29b-41d4-a716-446655440000\\",\\"payload\\":{\\"amount\\":\\"100.00\\",\\"asset_id\\":\\"1984\\",\\"asset_name\\":\\"USDT\\",\\"cart\\":{\\"items\\":[{\\"name\\":\\"Widget Pro\\",\\"price\\":\\"50.00\\",\\"quantity\\":2}]},\\"chain\\":\\"PolkadotAssetHub\\",\\"created_at\\":\\"2024-01-31T12:00:00Z\\",\\"id\\":\\"7c9e6679-7425-40de-944b-e07fc1f90ae7\\",\\"order_id\\":\\"order-12345\\",\\"payment_address\\":\\"5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY\\",\\"payment_url\\":\\"https://app.kalatori.com/invoice/7c9e6679-7425-40de-944b-e07fc1f90ae7\\",\\"redirect_url\\":\\"https://example.com/thank-you\\",\\"status\\":\\"Waiting\\",\\"total_received_amount\\":\\"0\\",\\"transactions\\":[],\\"updated_at\\":\\"2024-01-31T12:00:00Z\\",\\"valid_till\\":\\"2024-02-01T12:00:00Z\\"},\\"timestamp\\":\\"2024-01-31T12:00:00Z\\"}","expected_signature":"6c5e53f66e85f8f5b5fe5aa4bb971c889e168cff0a17f049d903e0c389aa4592","method":"POST","path":"/webhooks/invoices","secret":"production-secret-abc123","timestamp":"1706702400"},
  {"body":"{\\"event_entity\\":\\"invoice\\",\\"event_type\\":\\"paid\\",\\"id\\":\\"a1b2c3d4-e5f6-7890-abcd-ef1234567890\\",\\"payload\\":{\\"amount\\":\\"250.50\\",\\"asset_id\\":\\"1984\\",\\"asset_name\\":\\"USDT\\",\\"cart\\":{\\"items\\":[]},\\"chain\\":\\"PolkadotAssetHub\\",\\"created_at\\":\\"2024-01-31T12:00:00Z\\",\\"id\\":\\"deadbeef-1234-5678-9abc-def012345678\\",\\"order_id\\":\\"ORD-2024-0042\\",\\"payment_address\\":\\"5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty\\",\\"payment_url\\":\\"https://pay.example.com/inv/deadbeef\\",\\"redirect_url\\":\\"https://shop.example.com/order/42/complete\\",\\"status\\":\\"Paid\\",\\"total_received_amount\\":\\"250.50\\",\\"transactions\\":[{\\"amount\\":\\"250.50\\",\\"asset_id\\":\\"1984\\",\\"asset_name\\":\\"USDT\\",\\"block_number\\":12345678,\\"chain\\":\\"PolkadotAssetHub\\",\\"created_at\\":\\"2024-01-31T14:30:00Z\\",\\"destination_address\\":\\"5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty\\",\\"id\\":\\"11111111-2222-3333-4444-555555555555\\",\\"invoice_id\\":\\"deadbeef-1234-5678-9abc-def012345678\\",\\"position_in_block\\":2,\\"source_address\\":\\"5DAAnrj7VHTznn2AWBemMuyBwZWs6FNFjdyVXUeYum3PTXFy\\",\\"status\\":\\"Confirmed\\",\\"transaction_type\\":\\"Incoming\\",\\"tx_hash\\":\\"0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890\\",\\"updated_at\\":\\"2024-01-31T14:30:00Z\\"}],\\"updated_at\\":\\"2024-01-31T14:30:00Z\\",\\"valid_till\\":\\"2024-02-01T12:00:00Z\\"},\\"timestamp\\":\\"2024-01-31T14:30:00Z\\"}","expected_signature":"e8599841e652b41bbee8f4d74b7d8183935359351dde1b5cd8e3d368a1d7ece9","method":"POST","path":"/hooks/kalatori","secret":"merchant-secret-xyz","timestamp":"1706711400"},
  {"body":"{\\"minimal\\":true}","expected_signature":"cf8ac5477c3f9d1b5fc9f6a551854fa3e27421b0d6a0cf418197dbd7d0b973fc","method":"POST","path":"/webhooks/invoices","secret":"secret","timestamp":"0"},
  {"body":"{\\"id\\":\\"test\\"}","expected_signature":"780cee1a41d32cfb041d816b8496ce547708e62f98fec8b9bb8aac6d135b4e48","method":"POST","path":"/webhooks/invoices/","secret":"secret","timestamp":"1706745600"}
];

// ============================================================================
// Event Type -> Status Mapping
// ============================================================================
const EVENT_STATUS_MAP = {
  created:            [{ value: 'Waiting', label: 'Waiting' }],
  updated:            [{ value: 'Waiting', label: 'Waiting' }],
  admin_canceled:     [{ value: 'AdminCanceled', label: 'AdminCanceled' }],
  customer_canceled:  [{ value: 'CustomerCanceled', label: 'CustomerCanceled' }],
  paid:               [{ value: 'Paid', label: 'Paid' }, { value: 'OverPaid', label: 'OverPaid' }],
  partially_paid:     [{ value: 'PartiallyPaid', label: 'PartiallyPaid' }],
  expired:            [{ value: 'UnpaidExpired', label: 'UnpaidExpired' }, { value: 'PartiallyPaidExpired', label: 'PartiallyPaidExpired' }],
};

// ============================================================================
// UUID v4 Generator
// ============================================================================
function uuidv4() {
  return crypto.randomUUID();
}

// ============================================================================
// HMAC-SHA256 Signing (matches client/src/utils.rs calculate_hmac exactly)
//
// Message format: METHOD\\nPATH\\nBODY\\nTIMESTAMP
// Output: hex-encoded HMAC-SHA256
// ============================================================================
async function computeSignature(secret, method, path, body, timestamp) {
  const enc = new TextEncoder();
  const key = await crypto.subtle.importKey(
    'raw',
    enc.encode(secret),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign']
  );
  const message = method + '\\n' + path + '\\n' + body + '\\n' + timestamp;
  const sig = await crypto.subtle.sign('HMAC', key, enc.encode(message));
  return Array.from(new Uint8Array(sig))
    .map(b => b.toString(16).padStart(2, '0'))
    .join('');
}

// ============================================================================
// Payload Generation
// ============================================================================
function generatePayload(eventType, status) {
  const now = new Date();
  const createdAt = new Date(now.getTime() - 3600000);
  const validTill = new Date(createdAt.getTime() + 86400000);
  const invoiceId = uuidv4();
  const eventId = uuidv4();
  const amount = '100.00';

  let totalReceived = '0';
  let transactions = [];

  if (status === 'Paid') {
    totalReceived = amount;
    transactions = [makeSampleTransaction(invoiceId, amount, now)];
  } else if (status === 'OverPaid') {
    totalReceived = '150.00';
    transactions = [makeSampleTransaction(invoiceId, '150.00', now)];
  } else if (status === 'PartiallyPaid') {
    totalReceived = '50.00';
    transactions = [makeSampleTransaction(invoiceId, '50.00', now)];
  } else if (status === 'PartiallyPaidExpired') {
    totalReceived = '50.00';
    transactions = [makeSampleTransaction(invoiceId, '50.00', createdAt)];
  }

  const event = {
    id: eventId,
    event_entity: 'invoice',
    event_type: eventType,
    payload: {
      id: invoiceId,
      order_id: 'order-' + Math.floor(Math.random() * 100000),
      asset_name: 'USDT',
      asset_id: '1984',
      chain: 'PolkadotAssetHub',
      amount: amount,
      payment_address: '5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY',
      status: status,
      payment_url: 'https://app.kalatori.com/invoice/' + invoiceId,
      redirect_url: 'https://example.com/thank-you',
      cart: {
        items: [{
          name: 'Widget Pro',
          quantity: 2,
          price: '50.00'
        }]
      },
      total_received_amount: totalReceived,
      transactions: transactions,
      valid_till: validTill.toISOString().replace(/\\.\\d{3}Z$/, 'Z'),
      created_at: createdAt.toISOString().replace(/\\.\\d{3}Z$/, 'Z'),
      updated_at: now.toISOString().replace(/\\.\\d{3}Z$/, 'Z'),
    },
    timestamp: now.toISOString().replace(/\\.\\d{3}Z$/, 'Z'),
  };

  return event;
}

function makeSampleTransaction(invoiceId, amount, date) {
  return {
    id: uuidv4(),
    invoice_id: invoiceId,
    block_number: 12345678 + Math.floor(Math.random() * 1000),
    position_in_block: Math.floor(Math.random() * 10),
    tx_hash: '0x' + Array.from(crypto.getRandomValues(new Uint8Array(32)))
      .map(b => b.toString(16).padStart(2, '0')).join(''),
    transaction_type: 'Incoming',
    asset_name: 'USDT',
    asset_id: '1984',
    chain: 'PolkadotAssetHub',
    amount: amount,
    source_address: '5DAAnrj7VHTznn2AWBemMuyBwZWs6FNFjdyVXUeYum3PTXFy',
    destination_address: '5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY',
    created_at: date.toISOString().replace(/\\.\\d{3}Z$/, 'Z'),
    updated_at: date.toISOString().replace(/\\.\\d{3}Z$/, 'Z'),
    status: 'Confirmed'
  };
}

// ============================================================================
// UI Logic
// ============================================================================
const eventTypeSelect = document.getElementById('event-type');
const statusSelect = document.getElementById('invoice-status');
const payloadEditor = document.getElementById('payload-editor');
const jsonError = document.getElementById('json-error');

function updateStatusOptions() {
  const eventType = eventTypeSelect.value;
  const statuses = EVENT_STATUS_MAP[eventType] || [];
  statusSelect.innerHTML = '';
  for (const s of statuses) {
    const opt = document.createElement('option');
    opt.value = s.value;
    opt.textContent = s.label;
    statusSelect.appendChild(opt);
  }
}

function regeneratePayload() {
  const eventType = eventTypeSelect.value;
  const status = statusSelect.value;
  const payload = generatePayload(eventType, status);
  payloadEditor.value = JSON.stringify(payload, null, 2);
  payloadEditor.classList.remove('invalid');
  jsonError.textContent = '';
}

function formatPayload() {
  try {
    const parsed = JSON.parse(payloadEditor.value);
    payloadEditor.value = JSON.stringify(parsed, null, 2);
    payloadEditor.classList.remove('invalid');
    jsonError.textContent = '';
  } catch (e) {
    payloadEditor.classList.add('invalid');
    jsonError.textContent = 'Invalid JSON: ' + e.message;
  }
}

eventTypeSelect.addEventListener('change', () => {
  updateStatusOptions();
  regeneratePayload();
});

statusSelect.addEventListener('change', () => {
  regeneratePayload();
});

// Initialize
updateStatusOptions();
regeneratePayload();

// ============================================================================
// Toggle Secret Visibility
// ============================================================================
function toggleSecret() {
  const input = document.getElementById('secret-key');
  const btn = document.getElementById('toggle-secret');
  if (input.type === 'password') {
    input.type = 'text';
    btn.textContent = 'Hide';
  } else {
    input.type = 'password';
    btn.textContent = 'Show';
  }
}

// ============================================================================
// Send Webhook (via local proxy — no CORS issues)
// ============================================================================
let attemptCounter = 0;

function showResult(statusText, statusClass, requestText, responseText, productionHtml) {
  const resultPanel = document.getElementById('result-panel');
  const resultStatus = document.getElementById('result-status');
  const requestDetails = document.getElementById('request-details');
  const responseDetails = document.getElementById('response-details');
  const productionNote = document.getElementById('production-note');

  resultPanel.style.display = '';
  resultPanel.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
  resultStatus.textContent = statusText;
  resultStatus.className = 'result-status ' + (statusClass || '');
  requestDetails.textContent = requestText || '';
  responseDetails.textContent = responseText || '';
  productionNote.innerHTML = productionHtml || '';
}

async function sendWebhook() {
  attemptCounter++;
  const attempt = attemptCounter;
  const attemptLabel = 'Attempt #' + attempt;

  const resultPanel = document.getElementById('result-panel');
  const sendBtn = document.getElementById('send-btn');

  resultPanel.style.display = '';
  resultPanel.scrollIntoView({ behavior: 'smooth', block: 'nearest' });

  const url = document.getElementById('webhook-url').value.trim();
  const secret = document.getElementById('secret-key').value;
  const body = payloadEditor.value;

  // Validate inputs
  if (!url) {
    showResult(
      attemptLabel + ' — Validation Error', 'failure',
      '', 'Webhook URL is empty. Please enter a URL above.', ''
    );
    return;
  }

  let parsedUrl;
  try {
    parsedUrl = new URL(url);
  } catch {
    showResult(
      attemptLabel + ' — Validation Error', 'failure',
      '', 'Invalid URL format: "' + url + '"', ''
    );
    return;
  }

  try {
    JSON.parse(body);
  } catch (e) {
    payloadEditor.classList.add('invalid');
    jsonError.textContent = 'Invalid JSON: ' + e.message;
    showResult(
      attemptLabel + ' — Validation Error', 'failure',
      '', 'Payload is not valid JSON:\\n' + e.message, ''
    );
    return;
  }

  payloadEditor.classList.remove('invalid');
  jsonError.textContent = '';

  sendBtn.disabled = true;
  sendBtn.innerHTML = '<span class="spinner"></span>Sending...';

  const path = parsedUrl.pathname;
  const timestamp = Math.floor(Date.now() / 1000).toString();

  let signature;
  try {
    signature = await computeSignature(secret, 'POST', path, body, timestamp);
  } catch (e) {
    showResult(
      attemptLabel + ' — Signing Error', 'failure',
      '', 'Failed to compute HMAC signature: ' + e.message, ''
    );
    sendBtn.disabled = false;
    sendBtn.textContent = 'Send Webhook';
    return;
  }

  const requestText =
    attemptLabel + ' at ' + new Date().toLocaleTimeString() + '\\n\\n' +
    'POST ' + url + '\\n' +
    'Content-Type: application/json\\n' +
    'X-KALATORI-SIGNATURE: ' + signature + '\\n' +
    'X-KALATORI-TIMESTAMP: ' + timestamp + '\\n' +
    '\\nHMAC signed message (for debugging):\\n' +
    'POST\\\\n' + path + '\\\\n<body>\\\\n' + timestamp + '\\n' +
    '\\nPath extracted from URL: ' + path;

  showResult(
    attemptLabel + ' — Sending...', '',
    requestText, '(waiting for response...)', ''
  );

  const startTime = performance.now();

  try {
    const proxyResponse = await fetch('/api/proxy', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        url: url,
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'X-KALATORI-SIGNATURE': signature,
          'X-KALATORI-TIMESTAMP': timestamp,
        },
        body: body,
      }),
    });

    const result = await proxyResponse.json();

    if (result.error === 'timeout') {
      showResult(
        attemptLabel + ' — Timeout', 'failure',
        requestText,
        'Request timed out after 60 seconds.\\nTime: ' + result.elapsed + 'ms',
        '<strong>Production behavior:</strong> ' +
        'Request timeout (60 seconds). ' +
        'In production, Kalatori would retry this event indefinitely (no backoff, polling every ~100ms). ' +
        'All subsequent webhook events for this same invoice would be held in queue ' +
        'until this one succeeds.'
      );
    } else if (result.error === 'connection_error') {
      showResult(
        attemptLabel + ' — Connection Error', 'failure',
        requestText,
        'Error: ' + result.message + '\\nTime: ' + result.elapsed + 'ms',
        '<strong>Production behavior:</strong> ' +
        'Connection failed. In production, Kalatori would retry this event indefinitely ' +
        '(no backoff, polling every ~100ms). All subsequent webhook events for this same invoice ' +
        'would be held in queue until this one succeeds.'
      );
    } else if (result.status !== undefined) {
      const isSuccess = result.status >= 200 && result.status < 300;
      const responseBody = result.responseBody || '';

      const responseText =
        'Status: ' + result.status + ' ' + result.statusText + '\\n' +
        'Time: ' + result.elapsed + 'ms\\n\\n' +
        'Body:\\n' + (responseBody.length > 2000 ? responseBody.slice(0, 2000) + '\\n... (truncated)' : responseBody);

      let productionHtml;
      if (isSuccess) {
        productionHtml =
          '<strong>Production behavior:</strong> ' +
          'This event would be marked as delivered and removed from the queue. ' +
          'No retries.';
      } else {
        productionHtml =
          '<strong>Production behavior:</strong> ' +
          'Server responded with <strong>' + result.status + '</strong>. ' +
          'In production, Kalatori would retry this event indefinitely (no backoff, polling every ~100ms). ' +
          'All subsequent webhook events for this same invoice would be held in queue ' +
          'until this one succeeds (FIFO ordering per entity). ' +
          'Up to 10 concurrent webhook deliveries are allowed across different invoices.';
      }

      showResult(
        attemptLabel + ' — ' + result.status + ' ' + result.statusText,
        isSuccess ? 'success' : 'failure',
        requestText, responseText, productionHtml
      );
    } else {
      showResult(
        attemptLabel + ' — Unexpected Error', 'failure',
        requestText,
        'Unexpected proxy response: ' + JSON.stringify(result), ''
      );
    }
  } catch (e) {
    const elapsed = Math.round(performance.now() - startTime);
    showResult(
      attemptLabel + ' — Proxy Error', 'failure',
      requestText,
      'Error communicating with local proxy: ' + e.message + '\\nTime: ' + elapsed + 'ms\\n\\n' +
      'Make sure the webhook simulator server is still running.',
      ''
    );
  }

  sendBtn.disabled = false;
  sendBtn.textContent = 'Send Webhook';
}

// ============================================================================
// Self-Test
// ============================================================================
async function runSelfTest() {
  const resultsDiv = document.getElementById('self-test-results');
  resultsDiv.innerHTML = '<span class="spinner"></span> Running ' + TEST_VECTORS.length + ' test vectors...';

  let passed = 0;
  let failed = 0;
  let details = '';

  for (let i = 0; i < TEST_VECTORS.length; i++) {
    const tv = TEST_VECTORS[i];
    try {
      const computed = await computeSignature(
        tv.secret, tv.method, tv.path, tv.body, tv.timestamp
      );
      if (computed === tv.expected_signature) {
        passed++;
        details += '<div class="test-pass">  #' + (i + 1) + ' PASS</div>';
      } else {
        failed++;
        details += '<div class="test-fail">  #' + (i + 1) + ' FAIL — expected ' +
          tv.expected_signature.slice(0, 16) + '..., got ' + computed.slice(0, 16) + '...</div>';
      }
    } catch (e) {
      failed++;
      details += '<div class="test-fail">  #' + (i + 1) + ' ERROR — ' + e.message + '</div>';
    }
  }

  const summary = failed === 0
    ? '<strong class="test-pass">All ' + passed + ' tests passed.</strong> HMAC implementation matches Kalatori.'
    : '<strong class="test-fail">' + failed + ' of ' + (passed + failed) + ' tests failed.</strong> HMAC implementation may not match Kalatori.';

  resultsDiv.innerHTML = summary + '<br>' + details;
}
</script>

</body>
</html>`;
}

module.exports = { getHtml };
