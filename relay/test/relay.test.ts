import { exports } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import {
  MAX_EVENT_BYTES,
  stableJson,
  type EventEnvelope,
  type TeamEvent,
} from "../src/protocol";

const TEAM_ID = "team-relay-test-01";
const TOKEN = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const AUTHORIZATION = `Bearer ${TOKEN}`;

interface RelayPage {
  events: Array<EventEnvelope & { sequence: number }>;
  cursor: number;
  has_more: boolean;
}

describe("team relay", () => {
  it("stores only valid signed events and pages them with the team token", async () => {
    const first = await signedEnvelope(TEAM_ID, "team.created", "evt-bootstrap");
    const second = await signedEnvelope(TEAM_ID, "broadcast.created", "evt-broadcast");

    const created = await upload(TEAM_ID, [first]);
    expect(created.status).toBe(200);
    expect(await created.json()).toMatchObject({ accepted: 1, duplicates: 0, cursor: 1 });

    const appended = await upload(TEAM_ID, [second]);
    expect(appended.status).toBe(200);
    expect(await appended.json()).toMatchObject({ accepted: 1, duplicates: 0, cursor: 2 });

    const duplicate = await upload(TEAM_ID, [second]);
    expect(duplicate.status).toBe(200);
    expect(await duplicate.json()).toMatchObject({ accepted: 0, duplicates: 1, cursor: 2 });

    const pageResponse = await fetchPage(TEAM_ID, 0, 1);
    expect(pageResponse.status).toBe(200);
    const page = await pageResponse.json<RelayPage>();
    expect(page.events.map((row) => row.event.event_id)).toEqual(["evt-bootstrap"]);
    expect(page.cursor).toBe(1);
    expect(page.has_more).toBe(true);

    const nextPage = await fetchPage(TEAM_ID, page.cursor, 1);
    expect((await nextPage.json<RelayPage>()).events.map((row) => row.event.event_id)).toEqual(["evt-broadcast"]);
  });

  it("rejects unsigned, invalid-signature, wrong-team, and repository-path events", async () => {
    const bootstrap = await signedEnvelope("team-relay-negative", "team.created", "evt-bootstrap-negative");
    expect((await upload("team-relay-negative", [bootstrap])).status).toBe(200);

    const unsigned = structuredClone(await signedEnvelope("team-relay-negative", "broadcast.created", "evt-unsigned"));
    unsigned.event.signature = null;
    expect((await upload("team-relay-negative", [unsigned])).status).toBe(400);

    const invalidSignature = structuredClone(await signedEnvelope("team-relay-negative", "broadcast.created", "evt-invalid-signature"));
    invalidSignature.event.signature = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";
    expect((await upload("team-relay-negative", [invalidSignature])).status).toBe(401);

    const wrongTeam = await signedEnvelope("different-team", "broadcast.created", "evt-wrong-team");
    expect((await upload("team-relay-negative", [wrongTeam])).status).toBe(400);

    const repoPath = await signedEnvelope("team-relay-negative", "broadcast.created", "evt-repo-path", {
      message_id: "message-repo-path",
      sender_member_id: "member-test",
      scope: "team",
      local_path: "C:/private/repository",
      message: "hello",
      created_at: 1,
    });
    expect((await upload("team-relay-negative", [repoPath])).status).toBe(400);
  });

  it("rejects duplicate IDs with changed contents and enforces team token isolation", async () => {
    const bootstrap = await signedEnvelope("team-relay-conflict", "team.created", "evt-bootstrap-conflict");
    expect((await upload("team-relay-conflict", [bootstrap])).status).toBe(200);

    const event = await signedEnvelope("team-relay-conflict", "broadcast.created", "evt-conflict");
    expect((await upload("team-relay-conflict", [event])).status).toBe(200);
    const conflict = await signedEnvelope("team-relay-conflict", "broadcast.created", "evt-conflict", {
      message_id: "other-message",
      sender_member_id: "member-test",
      scope: "team",
      message: "changed",
      created_at: 2,
    });
    expect((await upload("team-relay-conflict", [conflict])).status).toBe(409);

    const wrongToken = await fetchPage("team-relay-conflict", 0, 10, "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB");
    expect(wrongToken.status).toBe(401);
    expect((await fetchPage("team-relay-conflict", 0, 129)).status).toBe(400);
  });

  it("rejects payloads beyond the event and batch limits before storage", async () => {
    const bootstrap = await signedEnvelope("team-relay-large", "team.created", "evt-bootstrap-large");
    expect((await upload("team-relay-large", [bootstrap])).status).toBe(200);

    const oversized = await signedEnvelope("team-relay-large", "broadcast.created", "evt-oversized", {
      message: "x".repeat(MAX_EVENT_BYTES),
    });
    expect((await upload("team-relay-large", [oversized])).status).toBe(413);

    const body = JSON.stringify({ events: [] });
    const malformed = await exports.default.fetch!(new Request("https://relay.test/v1/teams/team-relay-large/events", {
      method: "POST",
      headers: { authorization: AUTHORIZATION, "content-type": "application/json" },
      body,
    }));
    expect(malformed.status).toBe(400);
  });

  it("lets only the team authority rotate the relay token and revokes the old token", async () => {
    const teamId = "team-relay-rotate";
    const owner = await testIdentity();
    const bootstrap = await signedEnvelope(teamId, "team.created", "evt-bootstrap-rotate", undefined, owner);
    expect((await upload(teamId, [bootstrap])).status).toBe(200);

    const stranger = await testIdentity();
    const unauthorized = await rotateToken(teamId, TOKEN, stranger, toBase64Url(new Uint8Array(32).fill(2)));
    expect(unauthorized.status).toBe(403);

    const nextToken = toBase64Url(new Uint8Array(32).fill(1));
    const rotated = await rotateToken(teamId, TOKEN, owner, nextToken);
    expect(rotated.status).toBe(200);
    expect(await rotated.json()).toMatchObject({ rotated: true });
    expect((await fetchPage(teamId, 0, 10, TOKEN)).status).toBe(401);
    expect((await fetchPage(teamId, 0, 10, nextToken)).status).toBe(200);
  });
});

interface TestIdentity {
  privateKey: CryptoKey;
  publicKeyBase64: string;
  deviceId: string;
}

async function signedEnvelope(
  teamId: string,
  eventType: string,
  eventId: string,
  payload?: Record<string, unknown>,
  identity?: TestIdentity,
): Promise<EventEnvelope> {
  const signer = identity ?? await testIdentity();
  const event: TeamEvent = {
    event_id: eventId,
    team_id: teamId,
    actor_member_id: "member-test",
    device_id: signer.deviceId,
    event_type: eventType,
    payload: payload ?? (eventType === "team.created"
      ? { authority_member_id: "member-test", created_at: 1, name: "Relay test team", team_id: teamId }
      : { created_at: 1, name: "Relay test team" }),
    schema_version: 1,
    logical_time: 1,
    signature: null,
  };
  const bytes = encoder.encode(stableJson([
    event.event_id,
    event.team_id,
    event.actor_member_id,
    event.device_id,
    event.event_type,
    event.payload,
    event.schema_version,
    event.logical_time,
  ]));
  event.signature = toBase64(new Uint8Array(await crypto.subtle.sign(
    { name: "Ed25519" },
    signer.privateKey,
    asArrayBuffer(bytes),
  )));
  return { event, public_key: signer.publicKeyBase64 };
}

async function testIdentity(): Promise<TestIdentity> {
  const pair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
  const publicKey = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));
  return {
    privateKey: pair.privateKey,
    publicKeyBase64: toBase64(publicKey),
    deviceId: `device-${await sha256Hex(concat(encoder.encode("team-assistant/device-id/v1"), publicKey))}`,
  };
}

async function upload(teamId: string, events: EventEnvelope[]): Promise<Response> {
  return exports.default.fetch!(new Request(`https://relay.test/v1/teams/${teamId}/events`, {
    method: "POST",
    headers: { authorization: AUTHORIZATION, "content-type": "application/json" },
    body: JSON.stringify({ events }),
  }));
}

async function fetchPage(teamId: string, after: number, limit: number, token = TOKEN): Promise<Response> {
  return exports.default.fetch!(new Request(`https://relay.test/v1/teams/${teamId}/events?after=${after}&limit=${limit}`, {
    headers: { authorization: `Bearer ${token}` },
  }));
}

async function rotateToken(
  teamId: string,
  currentToken: string,
  identity: TestIdentity,
  newToken: string,
): Promise<Response> {
  const actorMemberId = "member-test";
  const createdAt = 5;
  const bytes = encoder.encode(stableJson([
    "team-assistant/relay-token-rotation/v1",
    teamId,
    actorMemberId,
    identity.deviceId,
    newToken,
    createdAt,
  ]));
  const signature = toBase64(new Uint8Array(await crypto.subtle.sign(
    { name: "Ed25519" },
    identity.privateKey,
    asArrayBuffer(bytes),
  )));
  return exports.default.fetch!(new Request(`https://relay.test/v1/teams/${teamId}/rotate-token`, {
    method: "POST",
    headers: { authorization: `Bearer ${currentToken}`, "content-type": "application/json" },
    body: JSON.stringify({
      actor_member_id: actorMemberId,
      created_at: createdAt,
      device_id: identity.deviceId,
      new_token: newToken,
      public_key: identity.publicKeyBase64,
      signature,
    }),
  }));
}

const encoder = new TextEncoder();

function toBase64(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes));
}

function toBase64Url(bytes: Uint8Array): string {
  return toBase64(bytes).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

function concat(left: Uint8Array, right: Uint8Array): Uint8Array {
  const bytes = new Uint8Array(left.byteLength + right.byteLength);
  bytes.set(left);
  bytes.set(right, left.byteLength);
  return bytes;
}

function asArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return copy.buffer;
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", asArrayBuffer(bytes)));
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("");
}
