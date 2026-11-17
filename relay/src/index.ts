import { DurableObject } from "cloudflare:workers";
import {
  MAX_BATCH_BYTES,
  MAX_BATCH_EVENTS,
  MAX_PAGE_EVENTS,
  MAX_PAGE_BYTES,
  MAX_REQUEST_BYTES,
  MAX_TEAM_BYTES,
  MAX_TEAM_EVENTS,
  sha256Hex,
  stableJson,
  validateEnvelope,
  validateEventBatch,
  validatePaging,
  validateTeamId,
  decodeBase64,
  type EventEnvelope,
  type RelayEvent,
} from "./protocol";

interface Env {
  TEAM_RELAY: DurableObjectNamespace<TeamRelay>;
}

interface StoredEventRow {
  [key: string]: string | number;
  sequence: number;
  event_json: string;
}

interface CountRow {
  [key: string]: string | number;
  event_count: number;
  total_bytes: number;
}

interface StoredTeamRow {
  [key: string]: string | number;
  token_hash: string;
  authority_member_id: string;
  authority_device_id: string;
  authority_public_key: string;
}

interface AppendResult {
  status: number;
  accepted: number;
  duplicates: number;
  cursor: number;
  error?: string;
}

const encoder = new TextEncoder();

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const parts = url.pathname.split("/").filter(Boolean);
    if (parts.length === 1 && parts[0] === "healthz" && request.method === "GET") {
      return json({ status: "ok", protocol_version: 1 });
    }
    if (parts.length !== 4 || parts[0] !== "v1" || parts[1] !== "teams") {
      return json({ error: "not_found" }, 404);
    }

    const teamId = parts[2];
    if (!validateTeamId(teamId)) return json({ error: "invalid_team_id" }, 400);
    const token = bearerToken(request.headers.get("authorization"));
    if (token === null) return json({ error: "authorization_required" }, 401);
    const tokenHash = await sha256Hex(encoder.encode(token));

    let forwarded: Request;
    if (parts[3] === "rotate-token") {
      if (request.method !== "POST") return json({ error: "method_not_allowed" }, 405, { allow: "POST" });
      const body = await readBoundedBody(request, 4096);
      if (body === null) return json({ error: "request_too_large" }, 413);
      let decoded: unknown;
      try {
        decoded = JSON.parse(new TextDecoder().decode(body));
      } catch {
        return json({ error: "malformed_json" }, 400);
      }
      if (decoded === null || typeof decoded !== "object" || Array.isArray(decoded)) {
        return json({ error: "malformed_rotation" }, 400);
      }
      forwarded = new Request("https://relay.internal/v1/rotate-token", {
        method: "POST",
        headers: {
          "content-type": "application/json",
          "x-team-id": teamId,
          "x-team-token-hash": tokenHash,
        },
        body: JSON.stringify(decoded),
      });
    } else if (parts[3] === "events" && request.method === "POST") {
      const body = await readBoundedBody(request, MAX_REQUEST_BYTES);
      if (body === null) return json({ error: "request_too_large" }, 413);
      let decoded: unknown;
      try {
        decoded = JSON.parse(new TextDecoder().decode(body));
      } catch {
        return json({ error: "malformed_json" }, 400);
      }
      if (decoded === null || typeof decoded !== "object" || Array.isArray(decoded)) {
        return json({ error: "malformed_batch" }, 400);
      }
      const events = (decoded as { events?: unknown }).events;
      if (!validateEventBatch(events)) return json({ error: "invalid_batch_size" }, 400);
      if (encoder.encode(JSON.stringify(decoded)).byteLength > MAX_BATCH_BYTES) {
        return json({ error: "batch_too_large" }, 413);
      }
      for (const envelope of events) {
        const validation = await validateEnvelope(envelope, teamId);
        if (validation) return json({ error: validation.code, message: validation.message }, validation.status);
      }
      forwarded = new Request("https://relay.internal/v1/events", {
        method: "POST",
        headers: {
          "content-type": "application/json",
          "x-team-id": teamId,
          "x-team-token-hash": tokenHash,
        },
        body: JSON.stringify({ events }),
      });
    } else if (parts[3] === "events" && request.method === "GET") {
      const paging = validatePaging(url.searchParams.get("after"), url.searchParams.get("limit"));
      if (paging === null) return json({ error: "invalid_cursor" }, 400);
      forwarded = new Request(`https://relay.internal/v1/events?after=${paging.after}&limit=${paging.limit}`, {
        method: "GET",
        headers: { "x-team-id": teamId, "x-team-token-hash": tokenHash },
      });
    } else if (parts[3] === "events") {
      return json({ error: "method_not_allowed" }, 405, { allow: "GET, POST" });
    } else {
      return json({ error: "not_found" }, 404);
    }

    const relayId = env.TEAM_RELAY.idFromName(teamId);
    return env.TEAM_RELAY.get(relayId).fetch(forwarded);
  },
} satisfies ExportedHandler<Env>;

export class TeamRelay extends DurableObject<Env> {
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    ctx.blockConcurrencyWhile(async () => {
      ctx.storage.sql.exec(`
        CREATE TABLE IF NOT EXISTS relay_team (
          team_id TEXT PRIMARY KEY,
          token_hash TEXT NOT NULL,
          authority_member_id TEXT NOT NULL,
          authority_device_id TEXT NOT NULL,
          authority_public_key TEXT NOT NULL,
          created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS relay_events (
          sequence INTEGER PRIMARY KEY AUTOINCREMENT,
          event_id TEXT NOT NULL UNIQUE,
          event_json TEXT NOT NULL,
          event_bytes INTEGER NOT NULL,
          created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS relay_events_created
          ON relay_events(created_at, sequence);
      `);
    });
  }

  async fetch(request: Request): Promise<Response> {
    const teamId = request.headers.get("x-team-id");
    const tokenHash = request.headers.get("x-team-token-hash");
    if (!teamId || !validateTeamId(teamId) || !tokenHash || !/^[a-f0-9]{64}$/.test(tokenHash)) {
      return json({ error: "invalid_relay_request" }, 400);
    }
    const url = new URL(request.url);
    if (request.method === "POST" && url.pathname === "/v1/events") {
      const body = await readBoundedBody(request, MAX_REQUEST_BYTES);
      if (body === null) return json({ error: "request_too_large" }, 413);
      let decoded: unknown;
      try {
        decoded = JSON.parse(new TextDecoder().decode(body));
      } catch {
        return json({ error: "malformed_json" }, 400);
      }
      const events = (decoded as { events?: unknown } | null)?.events;
      if (!validateEventBatch(events)) return json({ error: "invalid_batch_size" }, 400);
      const result = this.appendEvents(teamId, tokenHash, events);
      return json(result, result.status);
    }
    if (request.method === "POST" && url.pathname === "/v1/rotate-token") {
      const body = await readBoundedBody(request, 4096);
      if (body === null) return json({ error: "request_too_large" }, 413);
      let decoded: unknown;
      try {
        decoded = JSON.parse(new TextDecoder().decode(body));
      } catch {
        return json({ error: "malformed_json" }, 400);
      }
      const result = await this.rotateToken(teamId, tokenHash, decoded);
      return json(result.body, result.status);
    }
    if (request.method === "GET" && url.pathname === "/v1/events") {
      const paging = validatePaging(url.searchParams.get("after"), url.searchParams.get("limit"));
      if (paging === null) return json({ error: "invalid_cursor" }, 400);
      const result = this.readEvents(teamId, tokenHash, paging.after, paging.limit);
      return json(result, result.status);
    }
    return json({ error: "not_found" }, 404);
  }

  private appendEvents(teamId: string, tokenHash: string, events: EventEnvelope[]): AppendResult {
    const team = this.ctx.storage.sql
      .exec<StoredTeamRow>("SELECT token_hash FROM relay_team WHERE team_id = ?", teamId)
      .toArray()[0];
    if (team && !timingSafeEqualText(team.token_hash, tokenHash)) {
      return { status: 401, accepted: 0, duplicates: 0, cursor: 0, error: "invalid_team_token" };
    }
    const bootstrap = events.find(({ event }) => event.event_type === "team.created");
    if (!team && !bootstrap) {
      return { status: 403, accepted: 0, duplicates: 0, cursor: 0, error: "team_bootstrap_required" };
    }
    if (!team) {
      const payload = bootstrap?.event.payload;
      if (
        !payload ||
        payload.authority_member_id !== bootstrap?.event.actor_member_id ||
        payload.team_id !== teamId ||
        typeof bootstrap?.public_key !== "string"
      ) {
        return { status: 400, accepted: 0, duplicates: 0, cursor: 0, error: "invalid_team_bootstrap" };
      }
    }

    const unique = new Map<string, { envelope: EventEnvelope; encoded: string; bytes: number }>();
    let duplicatesInBatch = 0;
    for (const envelope of events) {
      const eventId = envelope.event.event_id;
      const encoded = stableJson(envelope);
      const bytes = encoder.encode(encoded).byteLength;
      const batchDuplicate = unique.get(eventId);
      if (batchDuplicate) {
        if (batchDuplicate.encoded !== encoded) {
          return { status: 409, accepted: 0, duplicates: 0, cursor: 0, error: "conflicting_event_id" };
        }
        duplicatesInBatch += 1;
        continue;
      }
      unique.set(eventId, { envelope, encoded, bytes });
    }

    const existingCount = this.ctx.storage.sql
      .exec<CountRow>("SELECT COUNT(*) AS event_count, COALESCE(SUM(event_bytes), 0) AS total_bytes FROM relay_events")
      .one();
    let additionalCount = 0;
    let additionalBytes = 0;
    const pending = [] as { eventId: string; encoded: string; bytes: number }[];
    for (const [eventId, entry] of unique) {
      const existing = this.ctx.storage.sql
        .exec<{ event_json: string }>("SELECT event_json FROM relay_events WHERE event_id = ?", eventId)
        .toArray()[0];
      if (existing) {
        if (existing.event_json !== entry.encoded) {
          return { status: 409, accepted: 0, duplicates: 0, cursor: 0, error: "conflicting_event_id" };
        }
        duplicatesInBatch += 1;
        continue;
      }
      pending.push({ eventId, encoded: entry.encoded, bytes: entry.bytes });
      additionalCount += 1;
      additionalBytes += entry.bytes;
    }
    if (existingCount.event_count + additionalCount > MAX_TEAM_EVENTS || existingCount.total_bytes + additionalBytes > MAX_TEAM_BYTES) {
      return { status: 413, accepted: 0, duplicates: duplicatesInBatch, cursor: 0, error: "team_relay_capacity_reached" };
    }

    const now = Date.now();
    this.ctx.storage.transactionSync(() => {
      if (!team) {
        const owner = bootstrap!;
        this.ctx.storage.sql.exec(
          `INSERT INTO relay_team
            (team_id, token_hash, authority_member_id, authority_device_id, authority_public_key, created_at)
           VALUES (?, ?, ?, ?, ?, ?)`,
          teamId,
          tokenHash,
          owner.event.actor_member_id,
          owner.event.device_id,
          owner.public_key,
          now,
        );
      }
      for (const entry of pending) {
        this.ctx.storage.sql.exec(
          "INSERT INTO relay_events (event_id, event_json, event_bytes, created_at) VALUES (?, ?, ?, ?)",
          entry.eventId,
          entry.encoded,
          entry.bytes,
          now,
        );
      }
    });
    const cursor = this.ctx.storage.sql
      .exec<{ sequence: number }>("SELECT COALESCE(MAX(sequence), 0) AS sequence FROM relay_events")
      .one().sequence;
    return { status: 200, accepted: pending.length, duplicates: duplicatesInBatch, cursor };
  }

  private async rotateToken(
    teamId: string,
    currentTokenHash: string,
    value: unknown,
  ): Promise<{ status: number; body: Record<string, unknown> }> {
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      return { status: 400, body: { error: "malformed_rotation" } };
    }
    const rotation = value as Record<string, unknown>;
    if (
      typeof rotation.new_token !== "string" || !isBearerToken(rotation.new_token) ||
      typeof rotation.actor_member_id !== "string" ||
      typeof rotation.device_id !== "string" ||
      typeof rotation.public_key !== "string" ||
      typeof rotation.signature !== "string" ||
      !Number.isSafeInteger(rotation.created_at)
    ) {
      return { status: 400, body: { error: "malformed_rotation" } };
    }
    const team = this.ctx.storage.sql
      .exec<StoredTeamRow>(
        `SELECT token_hash, authority_member_id, authority_device_id, authority_public_key
         FROM relay_team WHERE team_id = ?`,
        teamId,
      )
      .toArray()[0];
    if (!team) return { status: 404, body: { error: "team_not_found" } };
    if (!timingSafeEqualText(team.token_hash, currentTokenHash)) {
      return { status: 401, body: { error: "invalid_team_token" } };
    }
    if (
      rotation.actor_member_id !== team.authority_member_id ||
      rotation.device_id !== team.authority_device_id ||
      rotation.public_key !== team.authority_public_key
    ) {
      return { status: 403, body: { error: "team_authority_required" } };
    }
    const publicKey = decodeBase64(rotation.public_key, 32);
    const signature = decodeBase64(rotation.signature, 64);
    if (!publicKey || !signature) return { status: 400, body: { error: "invalid_rotation_signature" } };
    const bytes = encoder.encode(stableJson([
      "team-assistant/relay-token-rotation/v1",
      teamId,
      rotation.actor_member_id,
      rotation.device_id,
      rotation.new_token,
      rotation.created_at,
    ]));
    try {
      const key = await crypto.subtle.importKey("raw", asArrayBuffer(publicKey), { name: "Ed25519" }, false, ["verify"]);
      const verified = await crypto.subtle.verify(
        { name: "Ed25519" }, key, asArrayBuffer(signature), asArrayBuffer(bytes),
      );
      if (!verified) return { status: 401, body: { error: "invalid_rotation_signature" } };
    } catch {
      return { status: 400, body: { error: "invalid_rotation_signature" } };
    }
    const nextHash = await sha256Hex(encoder.encode(rotation.new_token));
    this.ctx.storage.sql.exec("UPDATE relay_team SET token_hash = ? WHERE team_id = ?", nextHash, teamId);
    return { status: 200, body: { rotated: true } };
  }

  private readEvents(teamId: string, tokenHash: string, after: number, limit: number): {
    status: number;
    events?: RelayEvent[];
    cursor?: number;
    has_more?: boolean;
    error?: string;
  } {
    const team = this.ctx.storage.sql
      .exec<StoredTeamRow>("SELECT token_hash FROM relay_team WHERE team_id = ?", teamId)
      .toArray()[0];
    if (!team) return { status: 404, error: "team_not_found" };
    if (!timingSafeEqualText(team.token_hash, tokenHash)) return { status: 401, error: "invalid_team_token" };

    const rows = this.ctx.storage.sql
      .exec<StoredEventRow>(
        "SELECT sequence, event_json FROM relay_events WHERE sequence > ? ORDER BY sequence LIMIT ?",
        after,
        Math.min(limit, MAX_PAGE_EVENTS),
      )
      .toArray();
    const events: RelayEvent[] = [];
    let pageBytes = 2;
    for (const row of rows) {
      const eventBytes = encoder.encode(row.event_json).byteLength + 32;
      if (events.length > 0 && pageBytes + eventBytes > MAX_PAGE_BYTES) break;
      if (eventBytes > MAX_PAGE_BYTES) return { status: 500, error: "stored_event_exceeds_page_limit" };
      events.push({ ...(JSON.parse(row.event_json) as EventEnvelope), sequence: row.sequence });
      pageBytes += eventBytes;
    }
    const cursor = events.at(-1)?.sequence ?? after;
    const hasMore = this.ctx.storage.sql
      .exec<{ present: number }>("SELECT EXISTS(SELECT 1 FROM relay_events WHERE sequence > ?) AS present", cursor)
      .one().present === 1;
    return { status: 200, events, cursor, has_more: hasMore };
  }
}

function bearerToken(header: string | null): string | null {
  const match = header?.match(/^Bearer ([A-Za-z0-9_-]{43})$/);
  return match && isBearerToken(match[1]) ? match[1] : null;
}

function isBearerToken(value: string): boolean {
  if (!/^[A-Za-z0-9_-]{43}$/.test(value)) return false;
  try {
    const bytes = atob(value.replaceAll("-", "+").replaceAll("_", "/") + "=");
    return bytes.length === 32 && btoa(bytes).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "") === value;
  } catch {
    return false;
  }
}

function asArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return copy.buffer;
}

async function readBoundedBody(request: Request, maximumBytes: number): Promise<Uint8Array | null> {
  if (!request.body) return new Uint8Array();
  const reader = request.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    while (true) {
      const part = await reader.read();
      if (part.done) break;
      total += part.value.byteLength;
      if (total > maximumBytes) {
        await reader.cancel();
        return null;
      }
      chunks.push(part.value);
    }
  } finally {
    reader.releaseLock();
  }
  const result = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    result.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return result;
}

function timingSafeEqualText(left: string, right: string): boolean {
  let difference = left.length ^ right.length;
  const count = Math.max(left.length, right.length);
  for (let index = 0; index < count; index += 1) {
    difference |= (left.charCodeAt(index) || 0) ^ (right.charCodeAt(index) || 0);
  }
  return difference === 0;
}

function json(value: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return Response.json(value, { status, headers: { "cache-control": "no-store", ...headers } });
}
