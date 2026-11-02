import assert from 'node:assert/strict';
import test from 'node:test';
import { EVENT_NAMES, handleRequest } from '../telemetry-relay/worker.mjs';

// Original fixed fixtures: no real project IDs, credentials, clipboard or network.
const env = {
  GA4_MEASUREMENT_ID: 'G-TEST12345',
  GA4_API_SECRET: 'test_secret_only',
  ENABLE_APPLICATION_INSIGHTS: 'true',
  APPLICATIONINSIGHTS_CONNECTION_STRING: 'InstrumentationKey=12345678-1234-1234-1234-123456789abc;IngestionEndpoint=https://westus2.in.applicationinsights.azure.com/;ApplicationId=ignored',
};
// v1 / v2 clients only know the original nine events; `app_active` arrived with v3.
const LEGACY_EVENTS = EVENT_NAMES.slice(0, 9);
const payload = {
  version: 1, sessionId: '0123456789abcdef0123456789abcdef', appVersion: '0.2.2', platform: 'macos',
  events: LEGACY_EVENTS.map((name, index) => ({ name, count: index + 1 })),
};
function request(body: unknown = payload, extra: RequestInit = {}) {
  return new Request('https://relay.example/collect', {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), ...extra,
  });
}
function recorder(statuses = [204, 200]) {
  const calls: { url: string; init: RequestInit; body: any }[] = [];
  const fetch = async (url: string, init: RequestInit) => {
    calls.push({ url, init, body: JSON.parse(String(init.body)) });
    return new Response(null, { status: statuses[calls.length - 1] });
  };
  return { calls, fetch };
}

test('explicitly enabled dual delivery rebuilds both payloads from fixed fields only', async () => {
  assert.equal(payload.events.length, 9);
  const { calls, fetch } = recorder();
  const result = await handleRequest(request(payload, {
    headers: { 'Content-Type': 'application/json', Cookie: 'private-cookie', 'X-Forwarded-For': '192.0.2.23', 'User-Agent': 'private-agent' },
  }), env, fetch);
  assert.equal(result.status, 204);
  assert.equal(result.headers.get('Cache-Control'), 'no-store');
  assert.equal(result.headers.get('Access-Control-Allow-Origin'), null);
  assert.equal(calls.length, 2);
  const [google, microsoft] = calls;
  const gaUrl = new URL(google.url);
  assert.equal(gaUrl.origin + gaUrl.pathname, 'https://www.google-analytics.com/mp/collect');
  assert.equal(gaUrl.searchParams.get('measurement_id'), env.GA4_MEASUREMENT_ID);
  assert.equal(gaUrl.searchParams.get('api_secret'), env.GA4_API_SECRET);
  const expectedParams = (count: number) => ({ count, app_version: '0.2.2', platform: 'macos' });
  assert.deepEqual(google.body, {
    client_id: '81985529216486896.81985529216486896', non_personalized_ads: true,
    consent: { ad_user_data: 'DENIED', ad_personalization: 'DENIED' },
    events: payload.events.map(event => ({ name: event.name, params: expectedParams(event.count) })),
  });
  assert.equal(microsoft.url, 'https://westus2.in.applicationinsights.azure.com/v2/track');
  assert.equal(microsoft.body.length, LEGACY_EVENTS.length);
  for (const [index, item] of microsoft.body.entries()) {
    assert.match(item.time, /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/);
    assert.deepEqual(item, {
      name: 'Microsoft.ApplicationInsights.12345678123412341234123456789abc.Event', time: item.time,
      iKey: '12345678-1234-1234-1234-123456789abc',
      data: { baseType: 'EventData', baseData: {
        ver: 2, name: payload.events[index].name,
        properties: { app_version: '0.2.2', platform: 'macos' }, measurements: { count: index + 1 },
      } },
    });
  }
  for (const call of calls) {
    assert.equal(call.init.method, 'POST');
    assert.equal(call.init.redirect, 'manual');
    assert.deepEqual(call.init.headers, {
      'Content-Type': 'application/json', 'CF-Connecting-IP': '0.0.0.0',
      'X-Real-IP': '0.0.0.0', 'X-Forwarded-For': '0.0.0.0',
    });
    assert.equal(call.init.signal instanceof AbortSignal, true);
    assert.doesNotMatch(JSON.stringify(call.body), /private-cookie|192\.0\.2\.23|private-agent|test_secret_only/);
  }
});

test('v2 forwards only the three normalized device properties to both providers', async () => {
  const cases = [
    ...['Mac14,2', 'MacBookPro18,3', 'MacBookAir10,1', 'MacBook10,1', 'Macmini9,1', 'MacPro7,1', 'iMacPro1,1', 'iMac19,2', 'Mac']
      .map(model => ({ platform: 'macos', architecture: 'aarch64', osMajor: '26', model })),
    ...['Surface', 'ThinkPad', 'ThinkCentre', 'Latitude', 'OptiPlex', 'Precision', 'XPS',
      'Inspiron', 'Pavilion', 'EliteBook', 'ProBook', 'ZBook', 'Vivobook', 'Zenbook', 'ROG', 'Swift', 'Aspire', 'WindowsPC']
      .map(model => ({ platform: 'windows', architecture: 'x86_64', osMajor: '11', model })),
    { platform: 'windows', architecture: 'x86', osMajor: '10', model: 'WindowsPC' },
    { platform: 'windows', architecture: 'unknown', osMajor: 'unknown', model: 'WindowsPC' },
    { platform: 'linux', architecture: 'arm', osMajor: '6', model: 'LinuxPC' },
    { platform: 'linux', architecture: 'unknown', osMajor: 'unknown', model: 'LinuxPC' },
    { platform: 'macos', architecture: 'unknown', osMajor: 'unknown', model: 'Mac' },
  ];
  for (const { platform, ...device } of cases) {
    const { calls, fetch } = recorder();
    assert.equal((await handleRequest(request({ ...payload, version: 2, platform, device }), env, fetch)).status, 204);
    assert.equal(calls.length, 2);
    const properties = {
      app_version: payload.appVersion, platform, app_arch: device.architecture,
      os_major: device.osMajor, device_model: device.model,
    };
    assert.deepEqual(calls[0].body.events, payload.events.map(event => ({
      name: event.name, params: { count: event.count, ...properties },
    })));
    for (const item of calls[1].body) assert.deepEqual(item.data.baseData.properties, properties);
  }
});

test('v2 rejects identity fields, arbitrary text, extra fields and models from another platform', async () => {
  const device = { architecture: 'aarch64', osMajor: '26', model: 'Mac14,2' };
  const v2 = { ...payload, version: 2, device };
  const invalid = [
    { ...v2, version: 1 }, { ...v2, version: 3, firstSeenDay: '2026-10-31' }, { ...v2, hostname: 'fixed-private-host' },
    ...[null, [], {}, { ...device, serial: 'fixed-forbidden-serial' },
      { ...device, hostname: 'fixed-private-host' }, { ...device, hardwareUuid: 'fixed-uuid' },
      { ...device, architecture: 'Apple M4' }, { ...device, architecture: 'aarch64\n' },
      ...['26.1', '26\n', ' 26', '0', '01', '100', 'private-os'].map(osMajor => ({ ...device, osMajor })),
      ...['private-host', 'Mac14,2 serial123', 'Mac14,2\n', 'Mac123,4', 'Mac1,2,3', 'Mac1,x',
        'WindowsPC', 'ThinkPad', 'LinuxPC', 'unknown'].map(model => ({ ...device, model })),
    ].map(device => ({ ...v2, device })),
    ...['Mac14,2', 'LinuxPC', 'HP EliteBook 840 G9', 'private-model-SN123', 'thinkpad', 'Surface\n', 'unknown']
      .map(model => ({ ...v2, platform: 'windows', device: { ...device, osMajor: '11', model } })),
    { ...v2, platform: 'windows', device: { ...device, osMajor: '26', model: 'WindowsPC' } },
    ...['Mac14,2', 'WindowsPC', 'ThinkPad', 'private-host', 'unknown']
      .map(model => ({ ...v2, platform: 'linux', device: { ...device, osMajor: '6', model } })),
  ];
  const { calls, fetch } = recorder();
  for (const body of invalid) assert.equal((await handleRequest(request(body), env, fetch)).status, 400);
  assert.equal(calls.length, 0);
});

test('v3 uses the install ID as GA client_id with a first-seen cohort and active-user markers', async () => {
  const device = { architecture: 'aarch64', osMajor: '26', model: 'Mac' };
  const v3 = {
    ...payload, version: 3, device, installId: 'fedcba9876543210fedcba9876543210', firstSeenDay: '2026-10-31',
    events: EVENT_NAMES.filter(name => name !== 'app_running').map((name, index) => ({ name, count: index + 1 })),
  };
  const { calls, fetch } = recorder();
  assert.equal((await handleRequest(request(v3), env, fetch)).status, 204);
  const [google, microsoft] = calls;
  // Two runs of one install share client_id; the per-run session ID only feeds session_id.
  assert.equal(google.body.client_id, '18364758544493064721.18364758544493064721');
  assert.deepEqual(google.body.user_properties, { first_seen_day: { value: '2026-10-31' } });
  assert.deepEqual(google.body.events.at(-1), {
    name: 'app_active', params: {
      count: 10, app_version: '0.2.2', platform: 'macos', app_arch: 'aarch64', os_major: '26', device_model: 'Mac',
      session_id: String(0x01234567 + 1), engagement_time_msec: 1,
    },
  });
  // Microsoft keeps receiving no identifier at all.
  assert.doesNotMatch(JSON.stringify(microsoft.body), /fedcba98|first_seen|session_id|engagement/);
  assert.deepEqual(Object.keys(microsoft.body[0].data.baseData.properties),
    ['app_version', 'platform', 'app_arch', 'os_major', 'device_model']);

  const invalid = [
    { ...v3, installId: undefined }, { ...v3, firstSeenDay: undefined }, { ...v3, installId: v3.sessionId },
    { ...v3, installId: 'F'.repeat(32) }, { ...v3, installId: 'user@example.test' }, { ...v3, hostname: 'fixed-private-host' },
    ...['2026-02-30', '2025-12-31', '2999-01-01', '2026-1-01', '2026-10-31T00:00', 20261031]
      .map(firstSeenDay => ({ ...v3, firstSeenDay })),
    { ...v3, device: undefined }, { ...payload, events: [{ name: 'app_active', count: 1 }] },
    { ...v3, version: 2, events: [{ name: 'app_active', count: 1 }] },
  ];
  const rejected = recorder();
  for (const body of invalid) assert.equal((await handleRequest(request(body), env, rejected.fetch)).status, 400);
  assert.equal(rejected.calls.length, 0);
});

test('default and explicitly disabled Microsoft configurations send only to Google', async () => {
  for (const enabled of [undefined, 'false']) {
    for (const connection of [undefined, '', 'invalid-unused-value', env.APPLICATIONINSIGHTS_CONNECTION_STRING]) {
      const { calls, fetch } = recorder();
      const config = { ...env, ENABLE_APPLICATION_INSIGHTS: enabled, APPLICATIONINSIGHTS_CONNECTION_STRING: connection };
      assert.equal((await handleRequest(request(), config, fetch)).status, 204);
      assert.equal(calls.length, 1);
      assert.equal(new URL(calls[0].url).origin, 'https://www.google-analytics.com');
      assert.deepEqual(calls[0].body.events.map(event => ({ name: event.name, count: event.params.count })), payload.events);
    }
  }
  const { calls, fetch } = recorder([500]);
  assert.equal((await handleRequest(request(), { ...env, ENABLE_APPLICATION_INSIGHTS: 'false' }, fetch)).status, 503);
  assert.equal(calls.length, 1);
});

test('Google redirects are rejected without following Location or forwarding credentials to it', async () => {
  const location = 'https://redirect.example/collect';
  const calls: { url: string; init: RequestInit }[] = [];
  const fetch = async (url: string, init: RequestInit) => {
    calls.push({ url, init });
    return new Response(null, { status: 302, headers: { Location: location } });
  };
  const result = await handleRequest(request(), { ...env, ENABLE_APPLICATION_INSIGHTS: 'false' }, fetch);
  assert.equal(result.status, 503);
  assert.equal(result.headers.get('Location'), null);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].init.redirect, 'manual');
  const url = new URL(calls[0].url);
  assert.equal(url.origin + url.pathname, 'https://www.google-analytics.com/mp/collect');
  assert.equal(url.searchParams.get('api_secret'), env.GA4_API_SECRET);
  assert.equal(calls.some(call => call.url === location), false);
});

test('update downloads and install attempts preserve distinct counts in both providers', async () => {
  const { calls, fetch } = recorder();
  const events = [
    { name: 'update_download_completed', count: 100 },
    { name: 'update_install_started', count: 2 },
  ];
  assert.equal((await handleRequest(request({ ...payload, events }), env, fetch)).status, 204);
  assert.deepEqual(calls[0].body.events.map(event => ({ name: event.name, count: event.params.count })), events);
  assert.deepEqual(calls[1].body.map(item => ({
    name: item.data.baseData.name, count: item.data.baseData.measurements.count,
  })), events);
});

test('invalid payloads never reach a provider', async () => {
  const invalid = [
    null, [], {}, { ...payload, content: 'must never leave device' }, { ...payload, version: 2 },
    { ...payload, sessionId: 'email@example.test' }, { ...payload, sessionId: 'F'.repeat(32) },
    { ...payload, platform: 'user-agent' }, { ...payload, appVersion: '0.2.2 /private/path' },
    { ...payload, events: [] }, { ...payload, events: [...payload.events, payload.events[0]] },
    { ...payload, events: [...payload.events, { name: 'update_install_completed', count: 1 }] },
    { ...payload, events: [payload.events[0], payload.events[0]] },
    ...[0, 101, -1, 1.2, '1', null].map(count => ({ ...payload, events: [{ name: 'app_started', count }] })),
    { ...payload, events: [{ name: 'clipboard_copied', count: 1 }] },
    { ...payload, events: [{ name: 'update_install_completed', count: 1 }] },
    { ...payload, events: [{ name: 'app_started', count: 1, params: { text: 'private text' } }] },
    { ...payload, events: [null] },
  ];
  const { calls, fetch } = recorder();
  for (const body of invalid) assert.equal((await handleRequest(request(body), env, fetch)).status, 400);
  assert.equal((await handleRequest(request(payload, { body: '{not json}' }), env, fetch)).status, 400);
  assert.equal(calls.length, 0);
});

test('Google and explicitly enabled Microsoft require valid configuration before any forwarding', async () => {
  const { calls, fetch } = recorder();
  const invalid = [
    {}, { ...env, GA4_MEASUREMENT_ID: '' }, { ...env, GA4_API_SECRET: '' },
    { ...env, ENABLE_APPLICATION_INSIGHTS: 'false', GA4_API_SECRET: '' },
    ...['', 'TRUE', 'yes', true].map(enabled => ({ ...env, ENABLE_APPLICATION_INSIGHTS: enabled })),
    { ...env, APPLICATIONINSIGHTS_CONNECTION_STRING: undefined },
    { ...env, APPLICATIONINSIGHTS_CONNECTION_STRING: '' },
    ...[
      'http://westus2.in.applicationinsights.azure.com/', 'https://untrusted.example/',
      'https://westus2.in.applicationinsights.azure.com.evil.example/',
      'https://secret@westus2.in.applicationinsights.azure.com/',
      'https://westus2.in.applicationinsights.azure.com/?api_key=test',
      'https://westus2.in.applicationinsights.azure.com:9999/',
      'https://westus2.in.applicationinsights.azure.com/not-ingestion',
    ].map(endpoint => ({ ...env, APPLICATIONINSIGHTS_CONNECTION_STRING: `InstrumentationKey=12345678-1234-1234-1234-123456789abc;IngestionEndpoint=${endpoint}` })),
    { ...env, APPLICATIONINSIGHTS_CONNECTION_STRING: `${env.APPLICATIONINSIGHTS_CONNECTION_STRING};Authorization=Entra` },
    { ...env, APPLICATIONINSIGHTS_CONNECTION_STRING: `${env.APPLICATIONINSIGHTS_CONNECTION_STRING};InstrumentationKey=12345678-1234-1234-1234-123456789abc` },
  ];
  for (const config of invalid) assert.equal((await handleRequest(request(), config, fetch)).status, 503);
  assert.equal(calls.length, 0);
});

test('streaming byte limit works even without Content-Length', async () => {
  const { calls, fetch } = recorder();
  let canceled = false;
  const body = new ReadableStream({
    pull(controller) { controller.enqueue(new Uint8Array(2049).fill(32)); },
    cancel() { canceled = true; },
  });
  const streamed = new Request('https://relay.example/collect', {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body, duplex: 'half',
  } as RequestInit);
  assert.equal((await handleRequest(streamed, env, fetch)).status, 413);
  assert.equal(canceled, true);
  assert.equal((await handleRequest(request(payload, {
    headers: { 'Content-Type': 'application/json', 'Content-Length': '4097' },
  }), env, fetch)).status, 413);
  assert.equal(calls.length, 0);
});

test('only the JSON POST collection route is available', async () => {
  const { calls, fetch } = recorder();
  for (const [url, method, status] of [
    ['https://relay.example/', 'POST', 404], ['https://relay.example/collect?content=private', 'POST', 404],
    ['https://relay.example/collect', 'OPTIONS', 405], ['https://relay.example/collect', 'GET', 405],
  ] as const) assert.equal((await handleRequest(new Request(url, { method }), env, fetch)).status, status);
  assert.equal((await handleRequest(request(payload, { headers: { 'Content-Type': 'text/plain' } }), env, fetch)).status, 415);
  assert.equal((await handleRequest(request(payload, { headers: { 'Content-Type': 'application/json', 'Content-Encoding': 'gzip' } }), env, fetch)).status, 415);
  assert.equal(calls.length, 0);
});

test('partial or failed provider delivery is never reported as full success and never retried', async () => {
  for (const statuses of [[400, 200], [204, 500], [204, 206], [302, 200]]) {
    const { calls, fetch } = recorder(statuses);
    assert.equal((await handleRequest(request(), env, fetch)).status, 503);
    assert.equal(calls.length, 2);
  }
  let count = 0;
  assert.equal((await handleRequest(request(), env, async () => { count++; throw new Error('provider unavailable'); })).status, 503);
  assert.equal(count, 2);
});

test('the two historical v3 shapes remain disjoint and compatible with their original identity policy', async () => {
  const device = { architecture: 'unknown', osMajor: 'unknown', model: 'Mac' };
  for (const installationId of [undefined, 'fedcba9876543210fedcba9876543210']) {
    const body = { ...payload, version: 3, device, ...(installationId ? { installationId } : {}), events: [{ name: 'app_running', count: 1 }] };
    const { calls, fetch } = recorder();
    assert.equal((await handleRequest(request(body), { ...env, ENABLE_APPLICATION_INSIGHTS: 'false' }, fetch)).status, 204);
    assert.equal(calls.length, 1);
    assert.equal(calls[0].body.client_id, installationId ? '18364758544493064721.18364758544493064721' : '81985529216486896.81985529216486896');
    assert.equal(calls[0].body.user_properties, undefined, 'legacy IDs have no known first-seen day');
    assert.equal(calls[0].body.events[0].params.identity_mode, installationId ? 'installation' : 'session');
    assert.equal(calls[0].body.events[0].params.engagement_time_msec, undefined);
  }
});

test('v4 separates session and installation reporting without fabricating a date or engagement', async () => {
  const base = { ...payload, version: 4, device: { architecture: 'unknown', osMajor: 'unknown', model: 'Mac' }, events: [{ name: 'app_active', count: 1 }] };
  for (const identity of [
    { identityMode: 'session' },
    { identityMode: 'installation', installId: 'fedcba9876543210fedcba9876543210' },
    { identityMode: 'installation', installId: 'fedcba9876543210fedcba9876543210', firstSeenDay: '2026-10-31' },
  ]) {
    const { calls, fetch } = recorder();
    assert.equal((await handleRequest(request({ ...base, ...identity, events: [{ name: identity.identityMode === 'session' ? 'app_started' : 'app_active', count: 1 }] }), env, fetch)).status, 204);
    const params = calls[0].body.events[0].params;
    assert.equal(params.identity_mode, identity.identityMode);
    assert.equal(params.session_id, String(0x01234567 + 1));
    assert.equal(params.engagement_time_msec, undefined);
    assert.deepEqual(calls[0].body.user_properties, 'firstSeenDay' in identity ? { first_seen_day: { value: identity.firstSeenDay } } : undefined);
    assert.equal(calls[0].body.client_id, identity.identityMode === 'installation' ? '18364758544493064721.18364758544493064721' : '81985529216486896.81985529216486896');
    assert.doesNotMatch(JSON.stringify(calls[1].body), /fedcba98|first_seen|identity_mode|session_id|engagement/);
  }
  const running = { ...base, identityMode: 'installation', installId: 'fedcba9876543210fedcba9876543210', events: [{ name: 'app_running', count: 1 }] };
  const { calls, fetch } = recorder();
  assert.equal((await handleRequest(request(running), env, fetch)).status, 204);
  assert.equal(calls[0].body.events[0].params.engagement_time_msec, undefined);
});

test('v4 rejects ambiguous identities, session heartbeats and unknown fields before forwarding', async () => {
  const base = { ...payload, version: 4, device: { architecture: 'unknown', osMajor: 'unknown', model: 'Mac' }, identityMode: 'installation', installId: 'fedcba9876543210fedcba9876543210' };
  const { calls, fetch } = recorder();
  for (const invalid of [
    { ...base, identityMode: 'off' }, { ...base, identityMode: 'session' }, { ...base, installId: base.sessionId },
    { ...base, installId: undefined }, { ...base, firstSeenDay: '2026-02-30' },
    { ...base, installationId: base.installId }, { ...base, content: 'original-forbidden-content' },
    { ...base, version: 3 },
    { ...base, identityMode: 'session', installId: undefined, firstSeenDay: '2026-10-31' },
    { ...base, identityMode: 'session', installId: undefined, events: [{ name: 'app_running', count: 1 }] },
    { ...base, identityMode: 'session', installId: undefined, events: [{ name: 'app_active', count: 1 }] },
    { ...base, events: [{ name: 'app_running', count: 2 }] },
    { ...base, version: 3, identityMode: undefined, installationId: base.installId },
  ]) assert.equal((await handleRequest(request(invalid), env, fetch)).status, 400);
  assert.equal(calls.length, 0);
});
