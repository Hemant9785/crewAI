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
const owners = new Map<string, TestIdentity>();

interface RelayPage {
  events: Array<EventEnvelope & { sequence: number }>;
  cursor: number;
  has_more: boolean;
}

describe("team relay", () => {
  it("delivers signed join requests to the authority and records approval without a return code", async () => {
    const teamId = "team-auto-join";
    const owner = await testIdentity(), applicant = await testIdentity();
    expect((await upload(teamId, [await signedEnvelope(teamId, "team.created", "evt-auto-bootstrap", undefined, owner)])).status).toBe(200);
    const request = await signedJoin(teamId, applicant);
    expect((await joinCommand(teamId, { action: "submit", request })).status).toBe(201);
    expect((await joinCommand(teamId, { action: "submit", request })).status).toBe(200);
    const inbox = await controlJoin(teamId, owner, "list", "");
    expect(await inbox.json()).toMatchObject({ requests: [{ request, status: "pending" }] });
    expect((await controlJoin(teamId, applicant, "list", "")).status).toBe(403);
    expect((await controlJoin(teamId, applicant, "approved", applicant.deviceId)).status).toBe(403);
    expect((await controlJoin(teamId, owner, "approved", applicant.deviceId, undefined, request.signature)).status).toBe(200);
    expect(await (await controlJoin(teamId, applicant, "status", applicant.deviceId)).json()).toEqual({ status: "approved" });
    expect(await (await controlJoin(teamId, owner, "list", "")).json()).toEqual({ requests: [] });
  });

  it("rejects forged joins, stale or unauthorized decisions, and conflicting replay", async () => {
    const teamId = "team-auto-join-negative";
    const owner = await testIdentity(), applicant = await testIdentity(), stranger = await testIdentity();
    expect((await upload(teamId, [await signedEnvelope(teamId, "team.created", "evt-auto-negative", undefined, owner)])).status).toBe(200);
    const request = await signedJoin(teamId, applicant);
    expect((await joinCommand(teamId, { action: "submit", request: { ...request, member: { ...request.member, display_name: "Forged" } } })).status).toBe(401);
    expect((await joinCommand(teamId, { action: "submit", request: { ...request, team_id: "wrong-team" } })).status).toBe(400);
    expect((await joinCommand(teamId, { action: "submit", request })).status).toBe(201);
    expect((await controlJoin(teamId, stranger, "status", applicant.deviceId)).status).toBe(403);
    expect((await controlJoin(teamId, owner, "rejected", applicant.deviceId, 1)).status).toBe(400);
    expect((await controlJoin(teamId, owner, "rejected", applicant.deviceId, undefined, request.signature)).status).toBe(200);
    expect((await controlJoin(teamId, owner, "rejected", applicant.deviceId, undefined, request.signature)).status).toBe(200);
    expect((await controlJoin(teamId, owner, "approved", applicant.deviceId, undefined, request.signature)).status).toBe(409);
    expect(await (await controlJoin(teamId, applicant, "status", applicant.deviceId)).json()).toEqual({ status: "rejected" });
    expect(await (await joinCommand(teamId, { action: "submit", request })).json()).toEqual({ status: "rejected" });
    const newer = await signedJoin(teamId, applicant, "new-invitation", request.created_at + 1);
    expect((await joinCommand(teamId, { action: "submit", request: newer })).status).toBe(201);
    expect((await controlJoin(teamId, owner, "rejected", applicant.deviceId, undefined, request.signature)).status).toBe(409);
    expect(await (await controlJoin(teamId, applicant, "status", applicant.deviceId)).json()).toEqual({ status: "pending" });
  });
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

  it("issues a short invite code for a team token and allows one claim", async () => {
    const teamId = "team-short-invite";
    const bootstrap = await signedEnvelope(teamId, "team.created", "evt-bootstrap-short-invite");
    expect((await upload(teamId, [bootstrap])).status).toBe(200);

    const inviteCode = `HERDR-INVITE-1:${"A".repeat(180)}`;
    const issued = await createShortInvite(teamId, inviteCode);
    expect(issued.status).toBe(201);
    const { code, expires_at: expiresAt } = await issued.json<{ code: string; expires_at: number }>();
    expect(code).toMatch(/^\d{6}$/);
    expect(expiresAt - Date.now()).toBeGreaterThan(9 * 60 * 1000);
    expect(expiresAt - Date.now()).toBeLessThanOrEqual(10 * 60 * 1000);

    const claimed = await claimShortInvite(code, "192.0.2.10");
    expect(claimed.status).toBe(200);
    expect(await claimed.json()).toEqual({ invite_code: inviteCode });
    expect((await claimShortInvite(code, "192.0.2.10")).status).toBe(404);
  });

  it("requires a team token to publish short invites and limits failed claims", async () => {
    const teamId = "team-short-invite-auth";
    const bootstrap = await signedEnvelope(teamId, "team.created", "evt-bootstrap-short-invite-auth");
    expect((await upload(teamId, [bootstrap])).status).toBe(200);

    const denied = await exports.default.fetch!(new Request(`https://relay.test/v1/teams/${teamId}/invite-codes`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ invite_code: "HERDR-INVITE-1:payload" }),
    }));
    expect(denied.status).toBe(401);

    for (let attempt = 0; attempt < 10; attempt += 1) {
      expect((await claimShortInvite(String(attempt).padStart(6, "0"), "192.0.2.11")).status).toBe(404);
    }
    expect((await claimShortInvite("999999", "192.0.2.11")).status).toBe(429);
  });

  it("rejects malformed short invite codes and invite payloads", async () => {
    expect((await claimShortInvite("abc123", "192.0.2.12")).status).toBe(400);

    const teamId = "team-short-invite-malformed";
    const bootstrap = await signedEnvelope(teamId, "team.created", "evt-bootstrap-short-invite-malformed");
    expect((await upload(teamId, [bootstrap])).status).toBe(200);
    const invalidPayload = await createShortInvite(teamId, "not-a-signed-invite");
    expect(invalidPayload.status).toBe(400);
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
  const signer = identity ?? owners.get(teamId) ?? await testIdentity();
  if(eventType === "team.created" || !owners.has(teamId)) owners.set(teamId,signer);
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
  const body=JSON.stringify({events}), target=`/v1/teams/${teamId}/events`;
  return exports.default.fetch!(new Request(`https://relay.test${target}`, {
    method: "POST",
    headers: { authorization: AUTHORIZATION, "content-type": "application/json", ...await proof(teamId,owners.get(teamId)!,"POST",target,body) },
    body,
  }));
}

async function fetchPage(teamId: string, after: number, limit: number, token = TOKEN): Promise<Response> {
  const target=`/v1/teams/${teamId}/events?after=${after}&limit=${limit}`;
  return exports.default.fetch!(new Request(`https://relay.test${target}`, {
    headers: { authorization: `Bearer ${token}`, ...await proof(teamId,owners.get(teamId)!,"GET",target,"") },
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

async function signedJoin(teamId: string, identity: TestIdentity, invitation_id = "invitation-join-test", created_at = Math.floor(Date.now() / 1000)) {
  const member = { member_id: "applicant", display_name: "Developer", role: "Implementer", active: true };
  const device = { device_id: identity.deviceId, member_id: member.member_id, public_key: identity.publicKeyBase64, active: true };
  const signature = toBase64(new Uint8Array(await crypto.subtle.sign("Ed25519", identity.privateKey,
    asArrayBuffer(encoder.encode(JSON.stringify(["team-assistant/join-request/v1", invitation_id, teamId, member, device, created_at]))))));
  return { invitation_id, team_id: teamId, member, device, created_at, signature };
}

async function joinCommand(teamId: string, body: unknown): Promise<Response> {
  return exports.default.fetch!(new Request(`https://relay.test/v1/teams/${teamId}/join-requests`, {
    method: "POST", headers: { authorization: AUTHORIZATION, "content-type": "application/json" }, body: JSON.stringify(body),
  }));
}

async function controlJoin(teamId: string, signer: TestIdentity, action: string, request_id: string, created_at = Math.floor(Date.now() / 1000), request_signature = "") {
  const signature = toBase64(new Uint8Array(await crypto.subtle.sign("Ed25519", signer.privateKey,
    asArrayBuffer(encoder.encode(JSON.stringify(["team-assistant/join-inbox/v1", teamId, action, request_id, created_at, request_signature]))))));
  return joinCommand(teamId, { action, request_id, created_at, signature, request_signature });
}

async function createShortInvite(teamId: string, inviteCode: string, token = TOKEN): Promise<Response> {
  return exports.default.fetch!(new Request(`https://relay.test/v1/teams/${teamId}/invite-codes`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${token}`,
      "content-type": "application/json",
    },
    body: JSON.stringify({ invite_code: inviteCode }),
  }));
}

async function claimShortInvite(code: string, ip: string): Promise<Response> {
  return exports.default.fetch!(new Request(`https://relay.test/v1/invites/${code}`, {
    headers: { "CF-Connecting-IP": ip },
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

async function proof(teamId:string,identity:TestIdentity,method:string,target:string,body:string,timestamp=Math.floor(Date.now()/1000)) {
  const nonce=crypto.randomUUID(), digest=await sha256Hex(encoder.encode(body));
  const message=encoder.encode(stableJson(["team-assistant/relay-access/v1",teamId,identity.deviceId,method,target,digest,timestamp,nonce]));
  const signature=toBase64(new Uint8Array(await crypto.subtle.sign("Ed25519",identity.privateKey,asArrayBuffer(message))));
  return {"x-relay-device":identity.deviceId,"x-relay-time":String(timestamp),"x-relay-nonce":nonce,"x-relay-signature":signature};
}

describe("approved device access",()=>{
  it("denies pairing-only and rejected devices, verifies bound proofs, and preserves revocation",async()=>{
    const team="team-access-control",owner=await testIdentity(),applicant=await testIdentity();
    expect((await upload(team,[await signedEnvelope(team,"team.created","access-created",undefined,owner)])).status).toBe(200);
    const target=`/v1/teams/${team}/events?after=0&limit=10`;
    const read=async(headers:Record<string,string>)=>exports.default.fetch!(new Request(`https://relay.test${target}`,{headers:{authorization:AUTHORIZATION,...headers}}));
    expect((await read({})).status).toBe(403);
    expect((await read(await proof(team,applicant,"GET",target,""))).status).toBe(403);
    const request=await signedJoin(team,applicant);
    await joinCommand(team,{action:"submit",request});
    expect((await read(await proof(team,applicant,"GET",target,""))).status).toBe(403);
    await controlJoin(team,owner,"rejected",applicant.deviceId,undefined,request.signature);
    expect((await read(await proof(team,applicant,"GET",target,""))).status).toBe(403);
    const replay=await proof(team,owner,"GET",target,"");
    expect((await read(replay)).status).toBe(200);
    expect((await read(replay)).status).toBe(403);
    expect((await read(await proof(team,owner,"GET",target,"",1))).status).toBe(403);
    expect((await read(await proof(team,owner,"GET",target+"0",""))).status).toBe(403);
    expect((await upload(team,[await signedEnvelope(team,"member.added","access-member",{member_id:"applicant",active:true},owner)])).status).toBe(200);
    expect((await upload(team,[await signedEnvelope(team,"device.added","access-device",{member_id:"applicant",device_id:applicant.deviceId,public_key:applicant.publicKeyBase64,active:true},owner)])).status).toBe(200);
    expect((await read(await proof(team,applicant,"GET",target,""))).status).toBe(200);
    expect((await upload(team,[await signedEnvelope(team,"member.added","access-revoke",{member_id:"applicant",active:false},owner)])).status).toBe(200);
    expect((await read(await proof(team,applicant,"GET",target,""))).status).toBe(403);
  });
});
