import { decodeBase64, sha256Hex } from "./protocol";

export const JOIN_INBOX_SCHEMA = `CREATE TABLE IF NOT EXISTS join_inbox (
  device_id TEXT PRIMARY KEY, request_json TEXT NOT NULL,
  status TEXT NOT NULL, created_at INTEGER NOT NULL
);`;

interface Authority { authority_device_id: string; authority_public_key: string }
interface Row { [key: string]: string | number; device_id: string; request_json: string; status: string; created_at: number }
const encoder = new TextEncoder();
const text = (v: unknown): v is string => typeof v === "string" && v.length > 0 && v.length <= 512;
const buffer = (v: Uint8Array): ArrayBuffer => Uint8Array.from(v).buffer;
async function verify(key: unknown, signature: unknown, bytes: string): Promise<boolean> {
  if (typeof key !== "string" || typeof signature !== "string") return false;
  const pk = decodeBase64(key, 32), sig = decodeBase64(signature, 64);
  if (!pk || !sig) return false;
  try {
    const imported = await crypto.subtle.importKey("raw", buffer(pk), { name: "Ed25519" }, false, ["verify"]);
    return await crypto.subtle.verify("Ed25519", imported, buffer(sig), buffer(encoder.encode(bytes)));
  } catch { return false; }
}

export async function joinInbox(sql: SqlStorage, teamId: string, authority: Authority, input: unknown): Promise<Response> {
  const reply = (body: unknown, status = 200) => Response.json(body, { status });
  if (!input || typeof input !== "object" || Array.isArray(input)) return reply({ error: "malformed_join_command" }, 400);
  const v = input as Record<string, any>;
  const now = Math.floor(Date.now() / 1000);
  sql.exec("DELETE FROM join_inbox WHERE created_at < ?", now - 7 * 86400);
  if (v.action === "submit") {
    const r = v.request, m = r?.member, d = r?.device;
    if (!r || !m || !d || r.team_id !== teamId || !text(r.invitation_id) || !text(m.member_id) ||
        !text(m.display_name) || !["Manager", "GroupHead", "PartHead", "Implementer"].includes(m.role) ||
        m.active !== true || d.active !== true || d.member_id !== m.member_id || !text(d.device_id) ||
        !Number.isSafeInteger(r.created_at) || r.created_at > now + 300 || r.created_at < now - 7 * 86400) {
      return reply({ error: "invalid_join_request" }, 400);
    }
    const pk = typeof d.public_key === "string" ? decodeBase64(d.public_key, 32) : null;
    if (!pk) return reply({ error: "invalid_join_key" }, 400);
    const prefix = encoder.encode("team-assistant/device-id/v1");
    const combined = new Uint8Array(prefix.length + pk.length); combined.set(prefix); combined.set(pk, prefix.length);
    if (d.device_id !== `device-${await sha256Hex(combined)}`) return reply({ error: "device_key_mismatch" }, 401);
    const member = { member_id: m.member_id, display_name: m.display_name, role: m.role, active: true };
    const device = { device_id: d.device_id, member_id: d.member_id, public_key: d.public_key, active: true };
    const request = { invitation_id: r.invitation_id, team_id: teamId, member, device, created_at: r.created_at, signature: r.signature };
    if (!await verify(d.public_key, r.signature, JSON.stringify([
      "team-assistant/join-request/v1", r.invitation_id, teamId, member, device, r.created_at,
    ]))) return reply({ error: "invalid_join_signature" }, 401);
    const serialized = JSON.stringify(request);
    const existing = sql.exec<Row>("SELECT * FROM join_inbox WHERE device_id = ?", d.device_id).toArray()[0];
    if (existing) {
      if (existing.request_json === serialized) return reply({ status: existing.status });
      const previous = JSON.parse(existing.request_json);
      if (existing.status === "approved" || previous.invitation_id === r.invitation_id || existing.created_at >= r.created_at) {
        return reply({ error: "join_request_conflict" }, 409);
      }
    } else if (sql.exec<{ count: number }>("SELECT COUNT(*) AS count FROM join_inbox").one().count >= 100) {
      return reply({ error: "join_inbox_full" }, 429);
    }
    sql.exec("INSERT OR REPLACE INTO join_inbox VALUES (?, ?, 'pending', ?)", d.device_id, serialized, r.created_at);
    return reply({ status: "pending" }, 201);
  }
  if (!["list", "status", "approved", "rejected"].includes(v.action) || typeof v.request_id !== "string" || typeof v.request_signature !== "string" || v.request_signature.length > 128 ||
      !Number.isSafeInteger(v.created_at) || Math.abs(v.created_at - now) > 300) return reply({ error: "invalid_join_command" }, 400);
  const row = v.request_id ? sql.exec<Row>("SELECT * FROM join_inbox WHERE device_id = ?", v.request_id).toArray()[0] : undefined;
  const key = v.action === "status" ? (row ? JSON.parse(row.request_json).device.public_key : null) : authority.authority_public_key;
  if (!await verify(key, v.signature, JSON.stringify(["team-assistant/join-inbox/v1", teamId, v.action, v.request_id, v.created_at, v.request_signature]))) {
    return reply({ error: "join_command_unauthorized" }, 403);
  }
  if (v.action === "list") {
    const rows = sql.exec<Row>("SELECT * FROM join_inbox WHERE status = 'pending' ORDER BY created_at, device_id LIMIT 100").toArray();
    return reply({ requests: rows.map(r => ({ request: JSON.parse(r.request_json), status: r.status })) });
  }
  if (!row) return reply({ error: "join_request_missing" }, 404);
  if (v.action === "status") return reply({ status: row.status });
  const current = sql.exec<Row>("SELECT * FROM join_inbox WHERE device_id = ?", v.request_id).one();
  if (JSON.parse(current.request_json).signature !== v.request_signature || current.request_json !== row.request_json || (current.status !== "pending" && current.status !== v.action)) return reply({ error: "join_decision_conflict" }, 409);
  sql.exec("UPDATE join_inbox SET status = ? WHERE device_id = ?", v.action, v.request_id);
  return reply({ status: v.action });
}
