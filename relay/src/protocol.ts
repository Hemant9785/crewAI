export const MAX_EVENT_BYTES = 256 * 1024;
export const MAX_BATCH_EVENTS = 64;
export const MAX_BATCH_BYTES = 512 * 1024;
export const MAX_REQUEST_BYTES = 600 * 1024;
export const MAX_PAGE_EVENTS = 128;
export const MAX_PAGE_BYTES = 512 * 1024;
export const MAX_TEAM_EVENTS = 20_000;
export const MAX_TEAM_BYTES = 64 * 1024 * 1024;
export const CURRENT_SCHEMA_VERSION = 1;

export const EVENT_TYPES = new Set([
  "team.created",
  "member.added",
  "device.added",
  "repository.approved",
  "permission.granted",
  "permission.revoked",
  "task.created",
  "task.status_changed",
  "task.follow_up_added",
  "broadcast.created",
  "request.created",
  "request.queued",
  "request.delivered",
  "request.started",
  "request.completed",
  "request.failed",
  "request.cancelled",
]);

const encoder = new TextEncoder();

export interface TeamEvent {
  event_id: string;
  team_id: string;
  actor_member_id: string;
  device_id: string;
  event_type: string;
  payload: Record<string, unknown>;
  schema_version: number;
  logical_time: number;
  signature: string | null;
}

export interface EventEnvelope {
  event: TeamEvent;
  public_key: string;
}

export interface RelayEvent extends EventEnvelope {
  sequence: number;
}

export interface ValidationError {
  status: number;
  code: string;
  message: string;
}

export function validateTeamId(value: string): boolean {
  return value.length > 0 && value.length <= 256 && /^[A-Za-z0-9_-]+$/.test(value);
}

export function decodeBase64(value: string, expectedBytes?: number): Uint8Array | null {
  if (value.length === 0 || value.length > 1024 * 1024 || !/^[A-Za-z0-9+/]+={0,2}$/.test(value)) {
    return null;
  }
  try {
    const raw = atob(value);
    if (expectedBytes !== undefined && raw.length !== expectedBytes) return null;
    const bytes = Uint8Array.from(raw, (character) => character.charCodeAt(0));
    return btoa(String.fromCharCode(...bytes)) === value ? bytes : null;
  } catch {
    return null;
  }
}

export function stableJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(stableJson).join(",")}]`;
  if (value !== null && typeof value === "object") {
    const record = value as Record<string, unknown>;
    const fields = Object.keys(record)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${stableJson(record[key])}`);
    return `{${fields.join(",")}}`;
  }
  return JSON.stringify(value);
}

function hasUnsafeNumber(value: unknown): boolean {
  if (typeof value === "number") return !Number.isSafeInteger(value);
  if (Array.isArray(value)) return value.some(hasUnsafeNumber);
  if (value !== null && typeof value === "object") {
    return Object.values(value as Record<string, unknown>).some(hasUnsafeNumber);
  }
  return false;
}

function hasRepositoryMaterialKey(value: unknown): boolean {
  if (Array.isArray(value)) return value.some(hasRepositoryMaterialKey);
  if (value === null || typeof value !== "object") return false;
  return Object.entries(value as Record<string, unknown>).some(([key, nested]) => {
    const normalized = key.toLowerCase().replaceAll("-", "_");
    return ["repository_path", "local_path", "file_path", "file_contents", "repository_contents", "diff", "patch"]
      .includes(normalized) || hasRepositoryMaterialKey(nested);
  });
}

function signingBytes(event: TeamEvent): Uint8Array {
  return encoder.encode(stableJson([
    event.event_id,
    event.team_id,
    event.actor_member_id,
    event.device_id,
    event.event_type,
    event.payload,
    event.schema_version,
    event.logical_time,
  ]));
}

export async function validateEnvelope(
  envelope: unknown,
  expectedTeamId: string,
): Promise<ValidationError | null> {
  if (envelope === null || typeof envelope !== "object" || Array.isArray(envelope)) {
    return { status: 400, code: "malformed_event", message: "event envelope must be an object" };
  }
  const candidate = envelope as Partial<EventEnvelope>;
  const event = candidate.event;
  if (event === null || typeof event !== "object" || Array.isArray(event)) {
    return { status: 400, code: "malformed_event", message: "event must be an object" };
  }
  if (
    typeof candidate.public_key !== "string" ||
    typeof event.event_id !== "string" || event.event_id.length === 0 || event.event_id.length > 512 ||
    typeof event.team_id !== "string" || event.team_id !== expectedTeamId ||
    typeof event.actor_member_id !== "string" || event.actor_member_id.length === 0 || event.actor_member_id.length > 512 ||
    typeof event.device_id !== "string" || event.device_id.length === 0 || event.device_id.length > 256 ||
    typeof event.event_type !== "string" || !EVENT_TYPES.has(event.event_type) ||
    !Number.isSafeInteger(event.schema_version) || event.schema_version !== CURRENT_SCHEMA_VERSION ||
    !Number.isSafeInteger(event.logical_time) || event.logical_time < 0 ||
    event.payload === null || typeof event.payload !== "object" || Array.isArray(event.payload) ||
    typeof event.signature !== "string" || event.signature.length === 0
  ) {
    return { status: 400, code: "malformed_event", message: "event fields or schema are invalid" };
  }
  if (hasUnsafeNumber(event.payload)) {
    return { status: 400, code: "malformed_event", message: "payload numbers must be safe integers" };
  }
  if (hasRepositoryMaterialKey(event.payload)) {
    return { status: 400, code: "repository_data_rejected", message: "repository files and paths cannot be relayed" };
  }

  const payloadBytes = encoder.encode(stableJson(event.payload));
  if (payloadBytes.byteLength > MAX_EVENT_BYTES) {
    return { status: 413, code: "event_too_large", message: "event payload exceeds the relay limit" };
  }

  const publicKey = decodeBase64(candidate.public_key, 32);
  const signature = decodeBase64(event.signature, 64);
  if (!publicKey || !signature) {
    return { status: 400, code: "unsigned_event", message: "event public key or signature is missing or malformed" };
  }
  const expectedDeviceId = `device-${await sha256Hex(concat(encoder.encode("team-assistant/device-id/v1"), publicKey))}`;
  if (event.device_id !== expectedDeviceId) {
    return { status: 401, code: "device_identity_mismatch", message: "device ID does not match its public key" };
  }

  try {
    const key = await crypto.subtle.importKey("raw", asArrayBuffer(publicKey), { name: "Ed25519" }, false, ["verify"]);
    const valid = await crypto.subtle.verify(
      { name: "Ed25519" },
      key,
      asArrayBuffer(signature),
      asArrayBuffer(signingBytes(event)),
    );
    if (!valid) return { status: 401, code: "invalid_signature", message: "event signature verification failed" };
  } catch {
    return { status: 400, code: "invalid_signature", message: "event signature could not be verified" };
  }
  return null;
}

export function validateEventBatch(value: unknown): value is EventEnvelope[] {
  if (!Array.isArray(value) || value.length === 0 || value.length > MAX_BATCH_EVENTS) return false;
  return true;
}

export function validatePaging(after: string | null, limit: string | null): { after: number; limit: number } | null {
  const parsedAfter = after === null ? 0 : Number(after);
  const parsedLimit = limit === null ? MAX_PAGE_EVENTS : Number(limit);
  if (!Number.isSafeInteger(parsedAfter) || parsedAfter < 0) return null;
  if (!Number.isSafeInteger(parsedLimit) || parsedLimit < 1 || parsedLimit > MAX_PAGE_EVENTS) return null;
  return { after: parsedAfter, limit: parsedLimit };
}

export async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", asArrayBuffer(bytes)));
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function asArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return copy.buffer;
}

export function concat(left: Uint8Array, right: Uint8Array): Uint8Array {
  const result = new Uint8Array(left.byteLength + right.byteLength);
  result.set(left);
  result.set(right, left.byteLength);
  return result;
}
