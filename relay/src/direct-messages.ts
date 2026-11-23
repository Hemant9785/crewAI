import { approvedDevice } from './access';
import { decodeBase64, stableJson } from './protocol';

export const DIRECT_SCHEMA = `
CREATE TABLE IF NOT EXISTS relay_direct_messages (
  sequence INTEGER PRIMARY KEY AUTOINCREMENT,
  message_id TEXT NOT NULL UNIQUE,
  sender_member_id TEXT NOT NULL,
  recipient_member_id TEXT NOT NULL,
  envelope_json TEXT NOT NULL,
  message_bytes INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS relay_direct_sender ON relay_direct_messages(sender_member_id, sequence);
CREATE INDEX IF NOT EXISTS relay_direct_recipient ON relay_direct_messages(recipient_member_id, sequence);`;

interface DirectMessage {
  message_id:string; team_id:string; sender_member_id:string; sender_device_id:string;
  recipient_member_id:string; reply_to:string|null; body:string; created_at:number; signature:string;
}
interface MessageRow { [key:string]:string|number; sequence:number; envelope_json:string }
interface ExistsRow { [key:string]:string|number; found:number }
const encoder = new TextEncoder();
const bytes = (v:Uint8Array):ArrayBuffer => Uint8Array.from(v).buffer;
const keys = ['body','created_at','message_id','recipient_member_id','reply_to','sender_device_id','sender_member_id','signature','team_id'];
const validId = (v:unknown):v is string => typeof v==='string' && v.length>0 && v.length<=512 && !/[\x00-\x1f\x7f]/.test(v);
function validMessage(value:unknown,team:string):value is DirectMessage {
  if (!value || typeof value!=='object' || Array.isArray(value)) return false;
  const m=value as Record<string,unknown>;
  if (Object.keys(m).sort().join(',')!==keys.join(',')) return false;
  return validId(m.message_id) && validId(m.team_id) && m.team_id===team &&
    validId(m.sender_member_id) && validId(m.sender_device_id) && validId(m.recipient_member_id) &&
    m.sender_member_id!==m.recipient_member_id && (m.reply_to===null || validId(m.reply_to)) &&
    typeof m.body==='string' && !!m.body.trim() && encoder.encode(m.body).length<=16*1024 &&
    !/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/.test(m.body) &&
    Number.isSafeInteger(m.created_at) && (m.created_at as number)>=0 &&
    typeof m.signature==='string' && decodeBase64(m.signature,64)!==null;
}

export async function putDirect(sql:SqlStorage, team:string, authenticatedDevice:string, input:unknown):Promise<{status:number;body:object}> {
  if (!validMessage(input,team)) return {status:400,body:{error:'invalid_direct_message'}};
  const m=input;
  const device=approvedDevice(sql,authenticatedDevice);
  if (!device || m.sender_device_id!==authenticatedDevice || m.sender_member_id!==device.member_id)
    return {status:403,body:{error:'invalid_direct_sender'}};
  const recipient=sql.exec<ExistsRow>('SELECT 1 AS found FROM relay_members WHERE member_id=? AND active=1',m.recipient_member_id).toArray()[0];
  if (!recipient) return {status:403,body:{error:'invalid_direct_recipient'}};
  const signature=decodeBase64(m.signature,64), publicKey=decodeBase64(device.public_key,32);
  if (!signature || !publicKey) return {status:400,body:{error:'invalid_direct_signature'}};
  const signed=encoder.encode(stableJson(['team-assistant/direct-message/v1',m.message_id,m.team_id,m.sender_member_id,m.sender_device_id,m.recipient_member_id,m.reply_to,m.body,m.created_at]));
  try {
    const key=await crypto.subtle.importKey('raw',bytes(publicKey),{name:'Ed25519'},false,['verify']);
    if (!await crypto.subtle.verify('Ed25519',key,bytes(signature),bytes(signed))) return {status:401,body:{error:'invalid_direct_signature'}};
  } catch {return {status:401,body:{error:'invalid_direct_signature'}};}
  const encoded=stableJson(m);
  const existing=sql.exec<{envelope_json:string}>('SELECT envelope_json FROM relay_direct_messages WHERE message_id=?',m.message_id).toArray()[0];
  if (existing) return existing.envelope_json===encoded ? {status:200,body:{duplicate:true}} : {status:409,body:{error:'conflicting_direct_message'}};
  if (m.reply_to) {
    const parent=sql.exec<ExistsRow>('SELECT 1 AS found FROM relay_direct_messages WHERE message_id=? AND ((sender_member_id=? AND recipient_member_id=?) OR (sender_member_id=? AND recipient_member_id=?))',m.reply_to,m.sender_member_id,m.recipient_member_id,m.recipient_member_id,m.sender_member_id).toArray()[0];
    if (!parent) return {status:409,body:{error:'invalid_direct_reply'}};
  }
  const highest=sql.exec<{count:number}>('SELECT COALESCE(MAX(sequence),0) AS count FROM relay_direct_messages').one().count;
  if (highest>=3000) return {status:413,body:{error:'direct_mailbox_full'}};
  sql.exec('INSERT INTO relay_direct_messages(message_id,sender_member_id,recipient_member_id,envelope_json,message_bytes) VALUES(?,?,?,?,?)',m.message_id,m.sender_member_id,m.recipient_member_id,encoded,encoder.encode(encoded).length);
  return {status:201,body:{accepted:true}};
}

export function getDirect(sql:SqlStorage,authenticatedDevice:string,after:number,limit:number):{status:number;body:object} {
  const device=approvedDevice(sql,authenticatedDevice);
  if (!device) return {status:403,body:{error:'device_not_approved'}};
  const capped=Math.min(limit,16);
  const rows=sql.exec<MessageRow>('SELECT sequence,envelope_json FROM relay_direct_messages WHERE sequence>? AND (sender_member_id=? OR recipient_member_id=?) ORDER BY sequence LIMIT ?',after,device.member_id,device.member_id,capped+1).toArray();
  const hasMore=rows.length>capped;
  const page=rows.slice(0,capped);
  return {status:200,body:{messages:page.map(row=>({sequence:row.sequence,message:JSON.parse(row.envelope_json)})),cursor:page.length?page[page.length-1].sequence:after,has_more:hasMore}};
}
