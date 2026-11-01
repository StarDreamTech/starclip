// No analytics SDK, storage, cookies, request logging, or automatic retries.
export const EVENT_NAMES = Object.freeze([
  'app_started', 'studio_opened', 'card_exported', 'backup_exported',
  'backup_imported', 'update_check_completed', 'font_downloaded',
  'update_download_completed', 'update_install_started',
  // Process heartbeat: "the app was running and tried to report", nothing more.
  'app_running',
]);
const MAX_BODY_BYTES = 4096;
const WINDOWS_MODELS = Object.freeze([
  'Surface', 'ThinkPad', 'ThinkCentre', 'Latitude', 'OptiPlex', 'Precision', 'XPS',
  'Inspiron', 'Pavilion', 'EliteBook', 'ProBook', 'ZBook', 'Vivobook', 'Zenbook',
  'ROG', 'Swift', 'Aspire', 'WindowsPC',
]);

function exactKeys(value, keys) {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    && Object.keys(value).length === keys.length
    && keys.every(key => Object.hasOwn(value, key));
}

function validDevice(device, platform) {
  if (!exactKeys(device, ['architecture', 'osMajor', 'model'])
      || !['aarch64', 'x86_64', 'x86', 'arm', 'unknown'].includes(device.architecture)
      || typeof device.osMajor !== 'string' || device.osMajor !== device.osMajor.trim()
      || !(device.osMajor === 'unknown' || /^[1-9][0-9]?$/.test(device.osMajor))
      || typeof device.model !== 'string' || device.model !== device.model.trim()) return false;
  // Match the desktop's coarse normalization, never forward firmware text or identifiers.
  if (platform === 'macos') return device.model === 'Mac'
    || /^(?:MacBookPro|MacBookAir|MacBook|Macmini|MacPro|iMacPro|iMac|Mac)[0-9]{1,2},[0-9]{1,2}$/.test(device.model);
  if (platform === 'windows') return ['10', '11', 'unknown'].includes(device.osMajor)
    && WINDOWS_MODELS.includes(device.model);
  return platform === 'linux' && device.model === 'LinuxPC';
}

const SESSION_FIELDS = ['version', 'sessionId', 'appVersion', 'platform', 'events'];

// Each protocol version has one exact field set. v3 adds the installation
// identity, and only in per-installation mode.
function shapeOf(value) {
  if (value?.version === 1 && exactKeys(value, SESSION_FIELDS)) return 'v1';
  if (value?.version === 2 && exactKeys(value, [...SESSION_FIELDS, 'device'])) return 'v2';
  if (value?.version !== 3) return null;
  if (exactKeys(value, [...SESSION_FIELDS, 'device'])) return 'v3-session';
  if (exactKeys(value, [...SESSION_FIELDS, 'device', 'installationId'])) return 'v3-installation';
  return null;
}

export function validPayload(value) {
  const shape = shapeOf(value);
  if (!shape) return false;
  const deviceValid = shape === 'v1' || validDevice(value.device, value.platform);
  return deviceValid
    && typeof value.sessionId === 'string' && /^[a-f0-9]{32}$/.test(value.sessionId)
    && (shape !== 'v3-installation'
      || (typeof value.installationId === 'string' && /^[a-f0-9]{32}$/.test(value.installationId)))
    && typeof value.appVersion === 'string' && value.appVersion.length <= 64
    && /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.test(value.appVersion)
    && ['macos', 'windows', 'linux'].includes(value.platform)
    && Array.isArray(value.events) && value.events.length >= 1 && value.events.length <= EVENT_NAMES.length
    && new Set(value.events.map(event => event?.name)).size === value.events.length
    && value.events.every(event => exactKeys(event, ['name', 'count'])
      && EVENT_NAMES.includes(event.name) && Number.isInteger(event.count) && event.count >= 1 && event.count <= 100);
}

function deviceProperties(payload) {
  // v2 and v3 both carry the normalized device block; v1 never does.
  return payload.version === 2 || payload.version === 3 ? {
    app_arch: payload.device.architecture,
    os_major: payload.device.osMajor,
    device_model: payload.device.model,
  } : {};
}

function configuration(env) {
  if (!/^G-[A-Z0-9]{5,32}$/.test(env.GA4_MEASUREMENT_ID ?? '')
      || !/^[A-Za-z0-9_-]{8,256}$/.test(env.GA4_API_SECRET ?? '')
      || ![undefined, 'false', 'true'].includes(env.ENABLE_APPLICATION_INSIGHTS)) return null;
  const googleUrl = new URL('https://www.google-analytics.com/mp/collect');
  googleUrl.searchParams.set('measurement_id', env.GA4_MEASUREMENT_ID);
  googleUrl.searchParams.set('api_secret', env.GA4_API_SECRET);
  // A leftover connection string must never activate a potentially billable service.
  if (env.ENABLE_APPLICATION_INSIGHTS !== 'true') return { googleUrl: googleUrl.href };
  if (typeof env.APPLICATIONINSIGHTS_CONNECTION_STRING !== 'string'
      || env.APPLICATIONINSIGHTS_CONNECTION_STRING.length > 4096) return null;
  const values = new Map();
  for (const part of env.APPLICATIONINSIGHTS_CONNECTION_STRING.split(';').filter(Boolean)) {
    const separator = part.indexOf('=');
    if (separator < 1) return null;
    const key = part.slice(0, separator).trim().toLowerCase();
    if (values.has(key)) return null;
    values.set(key, part.slice(separator + 1).trim());
  }
  const key = values.get('instrumentationkey');
  if (!/^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/i.test(key ?? '')
      || (values.has('authorization') && values.get('authorization')?.toLowerCase() !== 'ikey')) return null;
  let endpoint;
  try { endpoint = new URL(values.get('ingestionendpoint')); } catch { return null; }
  // Only public Azure ingestion hosts, never an arbitrary proxy or a caller-supplied URL.
  if (endpoint.protocol !== 'https:' || endpoint.username || endpoint.password || endpoint.port
      || endpoint.search || endpoint.hash || endpoint.pathname !== '/'
      || !(endpoint.hostname === 'dc.applicationinsights.azure.com'
        || /^[a-z0-9-]+\.in\.applicationinsights\.azure\.com$/.test(endpoint.hostname))) return null;
  return { googleUrl: googleUrl.href, microsoftUrl: new URL('v2/track', endpoint).href, key };
}

async function readPayload(request) {
  if (!request.body) throw 400;
  const length = request.headers.get('content-length');
  if (length && (!/^\d+$/.test(length) || Number(length) > MAX_BODY_BYTES)) throw 413;
  const reader = request.body.getReader();
  const chunks = [];
  let size = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > MAX_BODY_BYTES) {
        await reader.cancel();
        throw 413;
      }
      chunks.push(value);
    }
  } finally { reader.releaseLock(); }
  const body = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) { body.set(chunk, offset); offset += chunk.byteLength; }
  try { return JSON.parse(new TextDecoder('utf-8', { fatal: true, ignoreBOM: false }).decode(body)); } catch { throw 400; }
}

// Low-cardinality label derived from the protocol shape, never from data.
function identityMode(payload) {
  return payload.version === 3 && payload.installationId ? 'installation' : 'session';
}

function clientId(payload) {
  // GA4 accepts two positive decimal numbers. Per-installation mode reuses the
  // persistent 128-bit value; per-session mode keeps the ephemeral one.
  const hex = payload.version === 3 && payload.installationId ? payload.installationId : payload.sessionId;
  return [hex.slice(0, 16), hex.slice(16)]
    .map(part => (BigInt(`0x${part}`) + 1n).toString()).join('.');
}

function googleBody(payload) {
  return {
    client_id: clientId(payload),
    // No user_id and no custom dimensions: only event parameters.
    non_personalized_ads: true,
    consent: { ad_user_data: 'DENIED', ad_personalization: 'DENIED' },
    events: payload.events.map(event => ({
      name: event.name,
      params: {
        count: event.count, app_version: payload.appVersion, platform: payload.platform,
        identity_mode: identityMode(payload), ...deviceProperties(payload),
      },
    })),
  };
}

function microsoftBody(payload, key) {
  const time = new Date().toISOString();
  return payload.events.map(event => ({
    name: `Microsoft.ApplicationInsights.${key.replaceAll('-', '')}.Event`,
    time,
    iKey: key,
    data: {
      baseType: 'EventData',
      baseData: {
        ver: 2, name: event.name,
        properties: {
          app_version: payload.appVersion, platform: payload.platform,
          identity_mode: identityMode(payload), ...deviceProperties(payload),
        },
        measurements: { count: event.count },
      },
    },
  }));
}

async function send(fetchImpl, url, body) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 5000);
  try {
    const result = await fetchImpl(url, {
      method: 'POST', headers: {
        'Content-Type': 'application/json',
        // Do not let Worker subrequests inherit visitor IP forwarding headers.
        'CF-Connecting-IP': '0.0.0.0', 'X-Real-IP': '0.0.0.0', 'X-Forwarded-For': '0.0.0.0',
      },
      // Workers supports manual/follow only. Reject 3xx below without forwarding secrets.
      body: JSON.stringify(body), signal: controller.signal, redirect: 'manual',
    });
    // Microsoft 206 means partial ingestion. GA4 2xx only acknowledges receipt.
    const accepted = result.ok && result.status !== 206;
    await result.body?.cancel();
    return accepted;
  } catch { return false; } finally { clearTimeout(timeout); }
}

function response(status) {
  return new Response(null, { status, headers: { 'Cache-Control': 'no-store' } });
}

export async function handleRequest(request, env, fetchImpl = fetch) {
  const url = new URL(request.url);
  if (url.pathname !== '/collect' || url.search) return response(404);
  if (request.method !== 'POST') return response(405);
  if (!/^application\/json(?:\s*;|$)/i.test(request.headers.get('content-type') ?? '')
      || !['', 'identity'].includes(request.headers.get('content-encoding') ?? '')) return response(415);
  let payload;
  try { payload = await readPayload(request); } catch (status) { return response(status === 413 ? 413 : 400); }
  if (!validPayload(payload)) return response(400);
  const config = configuration(env);
  if (!config) return response(503);
  const requests = [send(fetchImpl, config.googleUrl, googleBody(payload))];
  if (config.microsoftUrl) requests.push(send(fetchImpl, config.microsoftUrl, microsoftBody(payload, config.key)));
  const results = await Promise.all(requests);
  return response(results.every(Boolean) ? 204 : 503);
}

export default { fetch: (request, env) => handleRequest(request, env) };
