export type Role = 'manager' | 'member';
export type AgentName = 'codex' | 'claude';
export type AgentPref = AgentName | 'auto';

export interface Team {
  id: string;
  name: string;
  ownerUid: string;
  createdAt: number;
}

export interface Member {
  uid: string;
  name: string;
  role: Role;
  lastSeen: number;
  agent: AgentName | null;
  project: string;
  joinedAt: number;
}

export interface Invite {
  code: string;
  teamId: string;
  expiresAt: number;
}

export interface Broadcast {
  id: string;
  fromUid: string;
  fromName: string;
  text: string;
  createdAt: number;
}

export type TaskStatus = 'todo' | 'in_progress' | 'done' | 'cancelled' | 'needs_help';

export interface Task {
  id: string;
  num: number;
  assigneeUid: string;
  assigneeName: string;
  assignerUid: string;
  assignerName: string;
  text: string;
  status: TaskStatus;
  note: string;
  createdAt: number;
  updatedAt: number;
}

export type RequestStatus = 'pending' | 'running' | 'answered' | 'declined' | 'cancelled' | 'failed';

export interface ReadRequest {
  id: string;
  askerUid: string;
  askerName: string;
  targetUid: string;
  targetName: string;
  question: string;
  status: RequestStatus;
  answer: string;
  agent: string;
  error: string;
  movedFrom: string[];
  createdAt: number;
  updatedAt: number;
}

export interface PresenceInfo {
  agent: AgentName | null;
  project: string;
}

export const ONLINE_WINDOW_MS = 90_000;
export const HEARTBEAT_MS = 30_000;
export const INVITE_TTL_MS = 10 * 60_000;
export const LIMITS = { text: 2000, answer: 8000, name: 40, error: 300 } as const;

/** Names are used as @mentions, so they stay simple. Spaces are not allowed. */
export const NAME_PATTERN = /^[A-Za-z0-9_.-]{1,40}$/;
export function validName(v: string): boolean {
  return NAME_PATTERN.test(v);
}

export function isOnline(m: Pick<Member, 'lastSeen'>, now: number = Date.now()): boolean {
  return now - m.lastSeen < ONLINE_WINDOW_MS;
}

export const TERMINAL_REQUEST: ReadonlySet<RequestStatus> = new Set(['answered', 'declined', 'cancelled', 'failed']);
