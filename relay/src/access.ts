import { decodeBase64, sha256Hex, stableJson, validateEnvelope, type EventEnvelope } from './protocol';
export const ACCESS_SCHEMA = `
CREATE TABLE IF NOT EXISTS relay_members(member_id TEXT PRIMARY KEY, active INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS relay_devices(device_id TEXT PRIMARY KEY, member_id TEXT NOT NULL, public_key TEXT NOT NULL, active INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS relay_access_nonces(device_id TEXT NOT NULL, nonce TEXT NOT NULL, expires_at INTEGER NOT NULL, PRIMARY KEY(device_id,nonce));
CREATE TABLE IF NOT EXISTS relay_access_cursor(id INTEGER PRIMARY KEY, sequence INTEGER NOT NULL);`;
interface Authority { authority_member_id:string; authority_device_id:string; authority_public_key:string }
interface DeviceRow { [key:string]:string|number; device_id:string; member_id:string; public_key:string; active:number }
const bytes=(v:Uint8Array):ArrayBuffer=>Uint8Array.from(v).buffer;
const encoder=new TextEncoder();
export async function refreshAccess(sql:SqlStorage, teamId:string, authority:Authority):Promise<void> {
  sql.exec('INSERT OR IGNORE INTO relay_members VALUES (?,1)',authority.authority_member_id);
  sql.exec('INSERT OR IGNORE INTO relay_devices VALUES (?,?,?,1)',authority.authority_device_id,authority.authority_member_id,authority.authority_public_key);
  const cursor=sql.exec<{sequence:number}>('SELECT sequence FROM relay_access_cursor WHERE id=1').toArray()[0]?.sequence ?? 0;
  const rows=sql.exec<{sequence:number;event_json:string}>('SELECT sequence,event_json FROM relay_events WHERE sequence>? ORDER BY sequence',cursor).toArray();
  for(const row of rows) {
    const envelope=JSON.parse(row.event_json) as EventEnvelope;
    if(envelope.event.actor_member_id===authority.authority_member_id && envelope.event.device_id===authority.authority_device_id && envelope.public_key===authority.authority_public_key && !await validateEnvelope(envelope,teamId)) applyMembership(sql,envelope);
  }
  if(rows.length) sql.exec('INSERT OR REPLACE INTO relay_access_cursor VALUES (1,?)',rows.at(-1)!.sequence);
}
export function applyMembership(sql:SqlStorage,envelope:EventEnvelope):void {
  const e=envelope.event,p=e.payload;
  if(e.event_type==='member.added' && typeof p.member_id==='string' && typeof p.active==='boolean')
    sql.exec('INSERT OR REPLACE INTO relay_members VALUES (?,?)',p.member_id,p.active?1:0);
  if(e.event_type==='device.added' && typeof p.device_id==='string' && typeof p.member_id==='string' && typeof p.public_key==='string' && decodeBase64(p.public_key,32) && typeof p.active==='boolean')
    sql.exec('INSERT OR REPLACE INTO relay_devices VALUES (?,?,?,?)',p.device_id,p.member_id,p.public_key,p.active?1:0);
}
export function approvedDevice(sql:SqlStorage,deviceId:string):DeviceRow|undefined {
  return sql.exec<DeviceRow>('SELECT d.* FROM relay_devices d JOIN relay_members m ON m.member_id=d.member_id WHERE d.device_id=? AND d.active=1 AND m.active=1',deviceId).toArray()[0];
}
export async function authorizeAccess(sql:SqlStorage,request:Request,teamId:string,bootstrap?:EventEnvelope):Promise<string|null> {
  const id=request.headers.get('x-relay-device')??'', nonce=request.headers.get('x-relay-nonce')??'', timestamp=Number(request.headers.get('x-relay-time'));
  const now=Math.floor(Date.now()/1000);
  if(!id || !/^[a-zA-Z0-9_-]{16,128}$/.test(nonce) || !Number.isSafeInteger(timestamp) || Math.abs(now-timestamp)>300) return 'signed_device_access_required';
  const stored=approvedDevice(sql,id);
  const key=stored?.public_key ?? (bootstrap?.event.device_id===id ? bootstrap.public_key : undefined);
  if(!key) return 'device_not_approved';
  const pk=decodeBase64(key,32),sig=decodeBase64(request.headers.get('x-relay-signature')??'',64);
  const target=request.headers.get('x-relay-target'),hash=request.headers.get('x-relay-body-hash');
  if(!pk||!sig||!target||!hash) return 'invalid_access_signature';
  try {
    const imported=await crypto.subtle.importKey('raw',bytes(pk),{name:'Ed25519'},false,['verify']);
    const message=encoder.encode(stableJson(['team-assistant/relay-access/v1',teamId,id,request.method,target,hash,timestamp,nonce]));
    if(!await crypto.subtle.verify('Ed25519',imported,bytes(sig),bytes(message))) return 'invalid_access_signature';
  } catch { return 'invalid_access_signature'; }
  if(stored && !approvedDevice(sql,id)) return 'device_not_approved';
  sql.exec('DELETE FROM relay_access_nonces WHERE expires_at<?',now);
  if(sql.exec<{found:number}>('SELECT 1 AS found FROM relay_access_nonces WHERE device_id=? AND nonce=?',id,nonce).toArray().length) return 'access_replay';
  sql.exec('INSERT INTO relay_access_nonces VALUES (?,?,?)',id,nonce,now+600);
  return null;
}
export async function forwardAccess(request:Request,target:string,body:Uint8Array):Promise<Record<string,string>> {
  const headers:Record<string,string>={'x-relay-target':target,'x-relay-body-hash':await sha256Hex(body)};
  for(const name of ['x-relay-device','x-relay-nonce','x-relay-time','x-relay-signature']) headers[name]=request.headers.get(name)??'';
  return headers;
}
