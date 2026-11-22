import { DurableObject } from "cloudflare:workers";
import { JOIN_INBOX_SCHEMA, joinInbox } from "./join-inbox";
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
  INVITE_CODES: DurableObjectNamespace<InviteCodeRegistry>;
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
const MAX_INVITE_CODE_BODY_BYTES = 32 * 1024;
const INVITE_CODE_LIFETIME_MS = 10 * 60 * 1000;
const MAX_INVITE_CODE_CLAIMS_PER_WINDOW = 10;
const INVITE_CODE_ATTEMPT_WINDOW_MS = 10 * 60 * 1000;
const MAX_ACTIVE_INVITE_CODES = 10_000;

interface InviteCodePayload {
  team_id: string;
  invite_code: string;
}

interface StoredInviteCode {
  [key: string]: string | number;
  code: string;
  invite_code: string;
  expires_at: number;
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const parts = url.pathname.split("/").filter(Boolean);
    if (parts.length === 1 && parts[0] === "healthz" && request.method === "GET") {
      return json({ status: "ok", protocol_version: 1 });
    }
    if (parts.length === 3 && parts[0] === "v1" && parts[1] === "invites") {
      if (request.method !== "GET") return json({ error: "method_not_allowed" }, 405, { allow: "GET" });
      const code = parts[2];
      if (!/^\d{6}$/.test(code)) return json({ error: "invalid_invite_code" }, 400);
      const clientAddress = request.headers.get("CF-Connecting-IP") ?? "unknown";
      const clientHash = await sha256Hex(encoder.encode(clientAddress));
      const registry = env.INVITE_CODES.get(env.INVITE_CODES.idFromName("global-v1"));
      return registry.fetch(new Request(`https://relay.internal/claim/${code}`, {
        method: "GET",
        headers: { "x-client-hash": clientHash },
      }));
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
    if (parts[3] === "join-requests" && request.method === "POST") {
      const body = await readBoundedBody(request, MAX_INVITE_CODE_BODY_BYTES);
      if (body === null) return json({ error: "request_too_large" }, 413);
      forwarded = new Request("https://relay.internal/v1/join-requests", {
        method: "POST",
        headers: { "content-type": "application/json", "x-team-id": teamId, "x-team-token-hash": tokenHash },
        body: asArrayBuffer(body),
      });
    } else if (parts[3] === "invite-codes" && request.method === "POST") {
      const body = await readBoundedBody(request, MAX_INVITE_CODE_BODY_BYTES);
      if (body === null) return json({ error: "request_too_large" }, 413);
      let decoded: unknown;
      try {
        decoded = JSON.parse(new TextDecoder().decode(body));
      } catch {
        return json({ error: "malformed_json" }, 400);
      }
      const inviteCode = (decoded as { invite_code?: unknown } | null)?.invite_code;
      if (typeof inviteCode !== "string" || !/^HERDR-INVITE-1:[A-Za-z0-9_-]+$/.test(inviteCode)) {
        return json({ error: "invalid_invite_payload" }, 400);
      }
      if (encoder.encode(inviteCode).byteLength > MAX_INVITE_CODE_BODY_BYTES - 1024) {
        return json({ error: "invite_payload_too_large" }, 413);
      }
      forwarded = new Request("https://relay.internal/v1/invite-codes", {
        method: "POST",
        headers: {
          "content-type": "application/json",
          "x-team-id": teamId,
          "x-team-token-hash": tokenHash,
        },
        body: JSON.stringify({ invite_code: inviteCode }),
      });
    } else if (parts[3] === "invite-codes") {
      return json({ error: "method_not_allowed" }, 405, { allow: "POST" });
    } else if (parts[3] === "rotate-token") {
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
      ctx.storage.sql.exec(JOIN_INBOX_SCHEMA);
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
    if (request.method === "POST" && url.pathname === "/v1/join-requests") {
      const team = this.ctx.storage.sql.exec<StoredTeamRow>("SELECT * FROM relay_team WHERE team_id = ?", teamId).toArray()[0];
      if (!team || !timingSafeEqualText(team.token_hash, tokenHash)) return json({ error: "invalid_team_token" }, 401);
      const body = await readBoundedBody(request, MAX_INVITE_CODE_BODY_BYTES);
      if (body === null) return json({ error: "request_too_large" }, 413);
      try {
        return await joinInbox(this.ctx.storage.sql, teamId, team, JSON.parse(new TextDecoder().decode(body)));
      } catch { return json({ error: "invalid_join_command" }, 400); }
    }
    if (request.method === "POST" && url.pathname === "/v1/invite-codes") {
      const team = this.ctx.storage.sql
        .exec<StoredTeamRow>("SELECT token_hash FROM relay_team WHERE team_id = ?", teamId)
        .toArray()[0];
      if (!team) return json({ error: "team_not_found" }, 404);
      if (!timingSafeEqualText(team.token_hash, tokenHash)) {
        return json({ error: "invalid_team_token" }, 401);
      }
      const body = await readBoundedBody(request, MAX_INVITE_CODE_BODY_BYTES);
      if (body === null) return json({ error: "request_too_large" }, 413);
      let decoded: unknown;
      try {
        decoded = JSON.parse(new TextDecoder().decode(body));
      } catch {
        return json({ error: "malformed_json" }, 400);
      }
      const inviteCode = (decoded as { invite_code?: unknown } | null)?.invite_code;
      if (typeof inviteCode !== "string" || !/^HERDR-INVITE-1:[A-Za-z0-9_-]+$/.test(inviteCode)) {
        return json({ error: "invalid_invite_payload" }, 400);
      }
      const registry = this.env.INVITE_CODES.get(this.env.INVITE_CODES.idFromName("global-v1"));
      return registry.fetch(new Request("https://relay.internal/create", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ team_id: teamId, invite_code: inviteCode } satisfies InviteCodePayload),
      }));
    }
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

export class InviteCodeRegistry extends DurableObject<Env> {
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    ctx.blockConcurrencyWhile(async () => {
      ctx.storage.sql.exec(`
        CREATE TABLE IF NOT EXISTS invite_codes (
          code TEXT PRIMARY KEY,
          team_id TEXT NOT NULL,
          invite_code TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          expires_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS invite_codes_expiry ON invite_codes(expires_at);
        CREATE TABLE IF NOT EXISTS invite_code_attempts (
          client_hash TEXT PRIMARY KEY,
          window_started_at INTEGER NOT NULL,
          attempts INTEGER NOT NULL
        );
      `);
    });
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    if (request.method === "POST" && url.pathname === "/create") {
      const body = await readBoundedBody(request, MAX_INVITE_CODE_BODY_BYTES + 1024);
      if (body === null) return json({ error: "request_too_large" }, 413);
      let decoded: unknown;
      try {
        decoded = JSON.parse(new TextDecoder().decode(body));
      } catch {
        return json({ error: "malformed_json" }, 400);
      }
      if (decoded === null || typeof decoded !== "object" || Array.isArray(decoded)) {
        return json({ error: "malformed_invite" }, 400);
      }
      const { team_id: teamId, invite_code: inviteCode } = decoded as Partial<InviteCodePayload>;
      if (
        typeof teamId !== "string" || !validateTeamId(teamId) ||
        typeof inviteCode !== "string" || !/^HERDR-INVITE-1:[A-Za-z0-9_-]+$/.test(inviteCode) ||
        encoder.encode(inviteCode).byteLength > MAX_INVITE_CODE_BODY_BYTES - 1024
      ) {
        return json({ error: "invalid_invite_payload" }, 400);
      }
      const created = this.createCode(teamId, inviteCode);
      if (created.status === 201 && await this.ctx.storage.getAlarm() === null) {
        await this.ctx.storage.setAlarm(Date.now() + INVITE_CODE_LIFETIME_MS);
      }
      return json(created.body, created.status);
    }
    if (request.method === "GET" && url.pathname.startsWith("/claim/")) {
      const code = url.pathname.slice("/claim/".length);
      const clientHash = request.headers.get("x-client-hash");
      if (!/^\d{6}$/.test(code) || !clientHash || !/^[a-f0-9]{64}$/.test(clientHash)) {
        return json({ error: "invalid_invite_code" }, 400);
      }
      const rateLimit = this.recordClaimAttempt(clientHash);
      if (!rateLimit.allowed) return json({ error: "too_many_attempts" }, 429);
      const invite = this.claimCode(code);
      return invite
        ? json({ invite_code: invite.invite_code })
        : json({ error: "invite_code_not_found_or_expired" }, 404);
    }
    return json({ error: "not_found" }, 404);
  }

  async alarm(): Promise<void> {
    const now = Date.now();
    this.ctx.storage.sql.exec("DELETE FROM invite_codes WHERE expires_at <= ?", now);
    this.ctx.storage.sql.exec("DELETE FROM invite_code_attempts WHERE window_started_at + ? <= ?", INVITE_CODE_ATTEMPT_WINDOW_MS, now);
    const next = this.ctx.storage.sql.exec<{ next_expiry: number | null }>("SELECT MIN(expires_at) AS next_expiry FROM invite_codes").one().next_expiry;
    if (next !== null) await this.ctx.storage.setAlarm(next);
  }

  private createCode(teamId: string, inviteCode: string): { status: number; body: Record<string, unknown> } {
    const now = Date.now();
    this.ctx.storage.sql.exec("DELETE FROM invite_codes WHERE expires_at <= ?", now);
    const capacity = this.ctx.storage.sql
      .exec<{ code_count: number }>("SELECT COUNT(*) AS code_count FROM invite_codes")
      .one().code_count;
    if (capacity >= MAX_ACTIVE_INVITE_CODES) {
      return { status: 503, body: { error: "invite_code_capacity_reached" } };
    }
    for (let attempt = 0; attempt < 32; attempt += 1) {
      const code = generateInviteCode();
      const existing = this.ctx.storage.sql
        .exec<{ code: string }>("SELECT code FROM invite_codes WHERE code = ?", code)
        .toArray()[0];
      if (existing) continue;
      const expiresAt = now + INVITE_CODE_LIFETIME_MS;
      this.ctx.storage.sql.exec(
        "INSERT INTO invite_codes (code, team_id, invite_code, created_at, expires_at) VALUES (?, ?, ?, ?, ?)",
        code,
        teamId,
        inviteCode,
        now,
        expiresAt,
      );
      return { status: 201, body: { code, expires_at: expiresAt } };
    }
    return { status: 503, body: { error: "invite_code_generation_failed" } };
  }

  private recordClaimAttempt(clientHash: string): { allowed: boolean } {
    const now = Date.now();
    const current = this.ctx.storage.sql
      .exec<{ window_started_at: number; attempts: number }>(
        "SELECT window_started_at, attempts FROM invite_code_attempts WHERE client_hash = ?",
        clientHash,
      )
      .toArray()[0];
    this.ctx.storage.sql.exec("DELETE FROM invite_code_attempts WHERE window_started_at + ? <= ?", INVITE_CODE_ATTEMPT_WINDOW_MS, now);
    if (!current || current.window_started_at + INVITE_CODE_ATTEMPT_WINDOW_MS <= now) {
      this.ctx.storage.sql.exec(
        "INSERT OR REPLACE INTO invite_code_attempts (client_hash, window_started_at, attempts) VALUES (?, ?, 1)",
        clientHash,
        now,
      );
      return { allowed: true };
    }
    if (current.attempts >= MAX_INVITE_CODE_CLAIMS_PER_WINDOW) return { allowed: false };
    this.ctx.storage.sql.exec(
      "UPDATE invite_code_attempts SET attempts = attempts + 1 WHERE client_hash = ?",
      clientHash,
    );
    return { allowed: true };
  }

  private claimCode(code: string): StoredInviteCode | null {
    const now = Date.now();
    const invite = this.ctx.storage.sql
      .exec<StoredInviteCode>(
        "SELECT code, invite_code, expires_at FROM invite_codes WHERE code = ?",
        code,
      )
      .toArray()[0];
    if (!invite) return null;
    this.ctx.storage.sql.exec("DELETE FROM invite_codes WHERE code = ?", code);
    if (invite.expires_at <= now) return null;
    return invite;
  }
}

function generateInviteCode(): string {
  const limit = Math.floor(0x1_0000_0000 / 1_000_000) * 1_000_000;
  const value = new Uint32Array(1);
  do {
    crypto.getRandomValues(value);
  } while (value[0] >= limit);
  return String(value[0] % 1_000_000).padStart(6, "0");
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
