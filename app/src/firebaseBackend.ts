import { randomInt } from 'node:crypto';
import { deleteApp, initializeApp, type FirebaseApp } from 'firebase/app';
import {
  connectAuthEmulator,
  createUserWithEmailAndPassword,
  getAuth,
  signInWithEmailAndPassword,
  type Auth,
} from 'firebase/auth';
import {
  collection,
  connectFirestoreEmulator,
  doc,
  getDoc,
  getFirestore,
  limit,
  onSnapshot,
  orderBy,
  query,
  runTransaction,
  serverTimestamp,
  setDoc,
  terminate,
  updateDoc,
  where,
  writeBatch,
  type DocumentData,
  type DocumentSnapshot,
  type Firestore,
  type Query,
  type QueryDocumentSnapshot,
  type Timestamp,
} from 'firebase/firestore';
import { BackendError, type Backend, type Unsubscribe } from './backend.js';
import type { Identity } from './config.js';
import type { FirebaseSettings } from './firebaseConfig.js';
import {
  INVITE_TTL_MS,
  LIMITS,
  validName,
  type Broadcast,
  type Invite,
  type Member,
  type PresenceInfo,
  type ReadRequest,
  type Task,
  type TaskStatus,
  type Team,
} from './types.js';

// Pending server times read as null and are filled with the estimated server clock, never the
// laptop's own clock (which may be off).
const SNAP = { serverTimestamps: 'none' } as const;
const REQUEST_LIMIT = 200;

function toMs(v: unknown, fallback: number): number {
  if (typeof v === 'number') return v;
  if (v && typeof (v as Timestamp).toMillis === 'function') return (v as Timestamp).toMillis();
  return fallback;
}

function mapError(e: unknown): BackendError {
  if (e instanceof BackendError) return e;
  const code = String((e as { code?: string }).code ?? '');
  if (code.includes('permission-denied')) return new BackendError('permission-denied', "You don't have access to that.");
  if (code.includes('configuration-not-found')) {
    return new BackendError('unavailable', 'Sign-in is not set up in the Firebase project. Enable Authentication > Email/Password in the console.');
  }
  if (code.includes('not-found')) return new BackendError('not-found', 'Not found.');
  if (code.includes('unavailable') || code.includes('network-request-failed') || code.includes('deadline')) {
    return new BackendError('unavailable', "Can't reach the team service. Check your connection.");
  }
  if (code.includes('operation-not-allowed')) {
    return new BackendError('unavailable', 'Email/Password sign-in is not enabled in the Firebase project.');
  }
  if (code.includes('aborted') || code.includes('failed-precondition') || code.includes('already-exists')) {
    return new BackendError('conflict', 'That changed while you were working. Try again.');
  }
  return new BackendError('unavailable', 'Something went wrong talking to the team service.');
}

async function guard<T>(fn: () => Promise<T>): Promise<T> {
  try {
    return await fn();
  } catch (e) {
    throw mapError(e);
  }
}

const toMember = (d: QueryDocumentSnapshot | DocumentSnapshot, now: number): Member => {
  const x = d.data(SNAP) as DocumentData;
  return {
    uid: d.id, name: x.name, role: x.role, lastSeen: toMs(x.lastSeen, now),
    agent: x.agent ?? null, project: x.project ?? '', joinedAt: toMs(x.joinedAt, now),
  };
};
const toBroadcast = (d: QueryDocumentSnapshot, now: number): Broadcast => {
  const x = d.data(SNAP);
  return { id: d.id, fromUid: x.fromUid, fromName: x.fromName, text: x.text, createdAt: toMs(x.createdAt, now) };
};
const toTask = (d: QueryDocumentSnapshot, now: number): Task => {
  const x = d.data(SNAP);
  return {
    id: d.id, num: x.num, assigneeUid: x.assigneeUid, assigneeName: x.assigneeName,
    assignerUid: x.assignerUid, assignerName: x.assignerName, text: x.text, status: x.status,
    note: x.note ?? '', createdAt: toMs(x.createdAt, now), updatedAt: toMs(x.updatedAt, now),
  };
};
const toRequest = (d: QueryDocumentSnapshot | DocumentSnapshot, now: number): ReadRequest => {
  const x = d.data(SNAP) as DocumentData;
  return {
    id: d.id, askerUid: x.askerUid, askerName: x.askerName, targetUid: x.targetUid, targetName: x.targetName,
    question: x.question, status: x.status, answer: x.answer ?? '', agent: x.agent ?? '', error: x.error ?? '',
    movedFrom: x.movedFrom ?? [], createdAt: toMs(x.createdAt, now), updatedAt: toMs(x.updatedAt, now),
  };
};

const lower = (n: string) => n.toLowerCase();

export class FirebaseBackend implements Backend {
  onError?: (e: BackendError) => void;
  private constructor(
    private app: FirebaseApp,
    private db: Firestore,
    public readonly uid: string,
    /** Server clock minus local clock, from the sign-in token's issue time. */
    private clockOffset = 0,
  ) {}

  serverNow(): number {
    return Date.now() + this.clockOffset;
  }

  /** Signs in with the stable device account, creating it on first run. */
  static async connect(cfg: FirebaseSettings, identity: Identity, appName = `crewai-${Date.now()}-${Math.random()}`): Promise<FirebaseBackend> {
    const app = initializeApp(
      { apiKey: cfg.apiKey, authDomain: cfg.authDomain, projectId: cfg.projectId, appId: cfg.appId },
      appName,
    );
    const auth: Auth = getAuth(app);
    const db = getFirestore(app);
    if (cfg.emulator) {
      const [h, p] = cfg.emulator.firestore.split(':');
      connectFirestoreEmulator(db, h!, Number(p));
      connectAuthEmulator(auth, `http://${cfg.emulator.auth}`, { disableWarnings: true });
    }
    try {
      let uid: string;
      try {
        uid = (await signInWithEmailAndPassword(auth, identity.email, identity.password)).user.uid;
      } catch (e) {
        const code = String((e as { code?: string }).code ?? '');
        if (!/user-not-found|invalid-credential|invalid-login-credentials/.test(code)) throw e;
        try {
          uid = (await createUserWithEmailAndPassword(auth, identity.email, identity.password)).user.uid;
        } catch (e2) {
          if (String((e2 as { code?: string }).code ?? '').includes('email-already-in-use')) {
            throw new BackendError('permission-denied', "This device's saved sign-in no longer works.");
          }
          throw e2;
        }
      }
      // The ID token was just issued by the server; its issue time tells how far this laptop's
      // clock is off. It is truncated to whole seconds, which errs on the side of showing an event.
      let offset = 0;
      try {
        const issued = Date.parse((await auth.currentUser!.getIdTokenResult()).issuedAtTime);
        if (Number.isFinite(issued)) offset = issued - Date.now();
      } catch {
        /* keep the local clock */
      }
      return new FirebaseBackend(app, db, uid, offset);
    } catch (e) {
      await terminate(db).catch(() => {});
      await deleteApp(app).catch(() => {});
      throw mapError(e);
    }
  }

  private teamRef = (teamId: string) => doc(this.db, 'teams', teamId);
  private memberRef = (teamId: string, uid = this.uid) => doc(this.db, 'teams', teamId, 'members', uid);
  private async myName(teamId: string): Promise<string> {
    const s = await getDoc(this.memberRef(teamId));
    if (!s.exists()) throw new BackendError('permission-denied', 'You are not in this team.');
    return (s.data() as DocumentData).name as string;
  }

  async createTeam(teamName: string, displayName: string): Promise<Team> {
    const name = teamName.trim();
    const me = displayName.trim();
    if (!name || name.length > LIMITS.name) throw new BackendError('invalid', 'Give the team a short name.');
    if (!validName(me)) throw new BackendError('invalid', 'Your name can use letters, numbers, dots, dashes and underscores (no spaces).');
    return guard(async () => {
      const ref = doc(collection(this.db, 'teams'));
      const now = this.serverNow();
      const b = writeBatch(this.db);
      b.set(ref, { name, ownerUid: this.uid, createdAt: now, taskSeq: 0 });
      b.set(doc(this.db, 'teams', ref.id, 'names', lower(me)), { uid: this.uid, name: me });
      b.set(this.memberRef(ref.id), {
        name: me, role: 'manager', lastSeen: serverTimestamp(), agent: null, project: '', joinedAt: serverTimestamp(),
      });
      await b.commit();
      return { id: ref.id, name, ownerUid: this.uid, createdAt: now };
    });
  }

  async createInvite(teamId: string): Promise<Invite> {
    return guard(async () => {
      for (let i = 0; i < 6; i++) {
        const code = String(randomInt(0, 1_000_000)).padStart(6, '0');
        const expiresAt = this.serverNow() + INVITE_TTL_MS;
        try {
          await setDoc(doc(this.db, 'invites', code), { teamId, createdBy: this.uid, expiresAt, used: false });
          return { code, teamId, expiresAt };
        } catch (e) {
          // A collision with a live code shows up as permission-denied (expired or used codes are
          // reissued). A non-manager fails every time.
          if (i === 5 || !String((e as { code?: string }).code).includes('permission-denied')) throw e;
        }
      }
      throw new BackendError('permission-denied', 'Only a manager can invite people.');
    }).catch((e: unknown) => {
      if (e instanceof BackendError && e.code === 'permission-denied') {
        throw new BackendError('permission-denied', 'Only a manager can invite people.');
      }
      throw e;
    });
  }

  async joinTeam(code: string, displayName: string): Promise<Team> {
    const c = code.trim();
    const me = displayName.trim();
    if (!/^\d{6}$/.test(c)) throw new BackendError('not-found', 'Codes are six digits.');
    if (!validName(me)) throw new BackendError('invalid', 'Your name can use letters, numbers, dots, dashes and underscores (no spaces).');
    return guard(async () => {
      // Say which code we are trying first; the rules allow one try every two seconds per device.
      try {
        await setDoc(doc(this.db, 'joinTickets', this.uid), { code: c, at: serverTimestamp() });
      } catch (e) {
        if (String((e as { code?: string }).code).includes('permission-denied')) {
          throw new BackendError('conflict', 'Too many tries. Wait a few seconds and try again.');
        }
        throw e;
      }
      const invRef = doc(this.db, 'invites', c);
      const inv = await getDoc(invRef);
      if (!inv.exists()) throw new BackendError('not-found', "That code isn't valid.");
      const d = inv.data() as DocumentData;
      if (d.used) throw new BackendError('conflict', 'That code was already used.');
      if (toMs(d.expiresAt, 0) <= this.serverNow()) throw new BackendError('expired', 'That code has expired. Ask for a new one.');
      const teamId = d.teamId as string;
      const b = writeBatch(this.db);
      b.update(invRef, { used: true });
      b.set(doc(this.db, 'teams', teamId, 'names', lower(me)), { uid: this.uid, name: me });
      b.set(this.memberRef(teamId), {
        name: me, role: 'member', lastSeen: serverTimestamp(), agent: null, project: '', joinedAt: serverTimestamp(), inviteCode: c,
      });
      try {
        await b.commit();
      } catch (e) {
        if (String((e as { code?: string }).code).includes('permission-denied')) {
          throw new BackendError('conflict', 'Could not join. That name may already be taken in this team, you may already be in it, or the code was just used.');
        }
        throw e;
      }
      const t = await getDoc(this.teamRef(teamId));
      const x = t.data() as DocumentData;
      return { id: teamId, name: x.name, ownerUid: x.ownerUid, createdAt: toMs(x.createdAt, this.serverNow()) };
    });
  }

  async getTeam(teamId: string): Promise<Team | null> {
    return guard(async () => {
      const t = await getDoc(this.teamRef(teamId));
      if (!t.exists()) return null;
      const x = t.data() as DocumentData;
      return { id: t.id, name: x.name, ownerUid: x.ownerUid, createdAt: toMs(x.createdAt, this.serverNow()) };
    });
  }

  private async dropMember(teamId: string, uid: string): Promise<void> {
    const m = await getDoc(this.memberRef(teamId, uid));
    if (!m.exists()) throw new BackendError('not-found', 'That person is not in this team.');
    const name = (m.data() as DocumentData).name as string;
    const b = writeBatch(this.db);
    b.delete(this.memberRef(teamId, uid));
    b.delete(doc(this.db, 'teams', teamId, 'names', lower(name)));
    await b.commit();
  }

  async removeMember(teamId: string, uid: string): Promise<void> {
    if (uid === this.uid) throw new BackendError('invalid', 'Use /team leave to leave the team yourself.');
    return guard(() => this.dropMember(teamId, uid)).catch((e: unknown) => {
      if (e instanceof BackendError && e.code === 'permission-denied') {
        throw new BackendError('permission-denied', 'Only a manager can remove people, and the team owner cannot be removed.');
      }
      throw e;
    });
  }

  async leaveTeam(teamId: string): Promise<void> {
    return guard(() => this.dropMember(teamId, this.uid)).catch((e: unknown) => {
      if (e instanceof BackendError && e.code === 'permission-denied') {
        throw new BackendError('permission-denied', 'The team owner cannot leave the team.');
      }
      throw e;
    });
  }

  async promoteMember(teamId: string, uid: string): Promise<void> {
    return guard(() => updateDoc(this.memberRef(teamId, uid), { role: 'manager' })).catch((e: unknown) => {
      if (e instanceof BackendError && e.code === 'permission-denied') {
        throw new BackendError('permission-denied', 'Only a manager can make someone a manager.');
      }
      throw e;
    });
  }

  async heartbeat(teamId: string, info: PresenceInfo): Promise<void> {
    return guard(() =>
      updateDoc(this.memberRef(teamId), {
        lastSeen: serverTimestamp(), agent: info.agent, project: info.project.slice(0, 120),
      }),
    );
  }

  private watch<T>(q: Query, map: (d: QueryDocumentSnapshot, now: number) => T, cb: (v: T[]) => void): Unsubscribe {
    return onSnapshot(
      q,
      (snap) => {
        const now = this.serverNow();
        cb(snap.docs.map((d) => map(d, now)));
      },
      (e) => this.onError?.(mapError(e)),
    );
  }

  subscribeMembers(teamId: string, cb: (m: Member[]) => void): Unsubscribe {
    return this.watch(collection(this.db, 'teams', teamId, 'members'), toMember, cb);
  }

  async sendBroadcast(teamId: string, text: string): Promise<Broadcast> {
    const t = text.trim();
    if (!t || t.length > LIMITS.text) throw new BackendError('invalid', 'Message is empty or too long.');
    return guard(async () => {
      const fromName = await this.myName(teamId);
      const ref = doc(collection(this.db, 'teams', teamId, 'broadcasts'));
      const createdAt = this.serverNow();
      await setDoc(ref, { fromUid: this.uid, fromName, text: t, createdAt: serverTimestamp() });
      return { id: ref.id, fromUid: this.uid, fromName, text: t, createdAt };
    });
  }
  subscribeBroadcasts(teamId: string, cb: (b: Broadcast[]) => void): Unsubscribe {
    const q = query(collection(this.db, 'teams', teamId, 'broadcasts'), orderBy('createdAt', 'desc'), limit(200));
    return this.watch(q, toBroadcast, (v) => cb(v.reverse()));
  }

  async createTasks(
    teamId: string,
    items: { assigneeUid: string; assigneeName: string; text: string }[],
  ): Promise<Task[]> {
    if (items.length === 0) return [];
    if (items.length > LIMITS.assignees) throw new BackendError('invalid', `Assign at most ${LIMITS.assignees} people at once.`);
    const clean = items.map((it) => {
      const text = it.text.trim();
      if (!text || text.length > LIMITS.text) throw new BackendError('invalid', 'Task is empty or too long.');
      return { ...it, text };
    });
    return guard(async () => {
      const assignerName = await this.myName(teamId);
      const refs = clean.map(() => doc(collection(this.db, 'teams', teamId, 'tasks')));
      // One transaction for everyone: either all tasks exist or none do, so a retry never duplicates.
      const first = await runTransaction(this.db, async (tx) => {
        const team = await tx.get(this.teamRef(teamId));
        const seq = (team.data() as DocumentData).taskSeq as number;
        tx.update(this.teamRef(teamId), { taskSeq: seq + clean.length });
        clean.forEach((it, i) => {
          tx.set(refs[i]!, {
            num: seq + 1 + i, assigneeUid: it.assigneeUid, assigneeName: it.assigneeName,
            assignerUid: this.uid, assignerName, text: it.text, status: 'todo', note: '',
            createdAt: serverTimestamp(), updatedAt: serverTimestamp(),
          });
        });
        return seq + 1;
      });
      const now = this.serverNow();
      return clean.map((it, i) => ({
        id: refs[i]!.id, num: first + i, assigneeUid: it.assigneeUid, assigneeName: it.assigneeName,
        assignerUid: this.uid, assignerName, text: it.text, status: 'todo' as const, note: '', createdAt: now, updatedAt: now,
      }));
    });
  }

  async updateTask(teamId: string, taskId: string, patch: { status: TaskStatus; note?: string }): Promise<void> {
    const data: DocumentData = { status: patch.status, updatedAt: serverTimestamp() };
    if (patch.note !== undefined) data.note = patch.note.slice(0, LIMITS.error);
    return guard(() => updateDoc(doc(this.db, 'teams', teamId, 'tasks', taskId), data));
  }
  subscribeTasks(teamId: string, cb: (t: Task[]) => void): Unsubscribe {
    const q = query(collection(this.db, 'teams', teamId, 'tasks'), orderBy('createdAt', 'desc'), limit(200));
    return this.watch(q, toTask, (v) => cb(v.reverse()));
  }

  async createRequest(
    teamId: string,
    input: { targetUid: string; targetName: string; question: string },
  ): Promise<ReadRequest> {
    const question = input.question.trim();
    if (!question || question.length > LIMITS.text) throw new BackendError('invalid', 'Question is empty or too long.');
    return guard(async () => {
      const askerName = await this.myName(teamId);
      const ref = doc(collection(this.db, 'teams', teamId, 'requests'));
      const now = this.serverNow();
      const r = {
        askerUid: this.uid, askerName, targetUid: input.targetUid, targetName: input.targetName, question,
        status: 'pending', answer: '', agent: '', error: '', movedFrom: [] as string[],
      };
      await setDoc(ref, { ...r, createdAt: serverTimestamp(), updatedAt: serverTimestamp() });
      return { id: ref.id, ...r, createdAt: now, updatedAt: now } as ReadRequest;
    });
  }

  async rerouteRequest(teamId: string, requestId: string, target: { uid: string; name: string }): Promise<void> {
    return guard(async () => {
      const ref = doc(this.db, 'teams', teamId, 'requests', requestId);
      await runTransaction(this.db, async (tx) => {
        const s = await tx.get(ref);
        if (!s.exists()) throw new BackendError('not-found', 'Question not found.');
        const x = s.data() as DocumentData;
        if (x.status !== 'pending') throw new BackendError('conflict', 'That question is already being answered.');
        if (x.targetUid === target.uid) return;
        tx.update(ref, {
          targetUid: target.uid, targetName: target.name,
          movedFrom: [...(x.movedFrom as string[]), x.targetUid as string], updatedAt: serverTimestamp(),
        });
      });
    });
  }

  async cancelRequest(teamId: string, requestId: string): Promise<void> {
    return guard(async () => {
      const ref = doc(this.db, 'teams', teamId, 'requests', requestId);
      const s = await getDoc(ref);
      const st = (s.data() as DocumentData | undefined)?.status;
      if (st !== 'pending' && st !== 'running') throw new BackendError('conflict', 'That question is already finished.');
      await updateDoc(ref, { status: 'cancelled', updatedAt: serverTimestamp() });
    });
  }

  async respondRequest(
    teamId: string,
    requestId: string,
    patch: { status: 'running' | 'answered' | 'declined' | 'failed'; answer?: string; agent?: string; error?: string },
  ): Promise<void> {
    const data: DocumentData = { status: patch.status, updatedAt: serverTimestamp() };
    if (patch.answer !== undefined) data.answer = patch.answer.slice(0, LIMITS.answer);
    if (patch.agent !== undefined) data.agent = patch.agent.slice(0, 20);
    if (patch.error !== undefined) data.error = patch.error.slice(0, LIMITS.error);
    return guard(async () => {
      const ref = doc(this.db, 'teams', teamId, 'requests', requestId);
      const s = await getDoc(ref);
      const st = (s.data() as DocumentData | undefined)?.status;
      if (st !== 'pending' && st !== 'running') throw new BackendError('conflict', 'That question is no longer open.');
      await updateDoc(ref, data);
    });
  }

  subscribeRequests(teamId: string, cb: (r: ReadRequest[]) => void): Unsubscribe {
    const col = collection(this.db, 'teams', teamId, 'requests');
    const parts: Record<string, ReadRequest[]> = { asker: [], target: [], moved: [] };
    const push = () => {
      const byId = new Map<string, ReadRequest>();
      for (const list of Object.values(parts)) for (const r of list) byId.set(r.id, r);
      cb([...byId.values()]);
    };
    const mk = (key: string, q: Query) =>
      onSnapshot(
        q,
        (snap) => {
          const now = this.serverNow();
          parts[key] = snap.docs.map((d) => toRequest(d, now));
          push();
        },
        (e) => this.onError?.(mapError(e)),
      );
    const unsubs = [
      // Bounded: only the most recent questions matter; older ones are finished history.
      mk('asker', query(col, where('askerUid', '==', this.uid), orderBy('updatedAt', 'desc'), limit(REQUEST_LIMIT))),
      mk('target', query(col, where('targetUid', '==', this.uid), orderBy('updatedAt', 'desc'), limit(REQUEST_LIMIT))),
      mk('moved', query(col, where('movedFrom', 'array-contains', this.uid), orderBy('updatedAt', 'desc'), limit(REQUEST_LIMIT))),
    ];
    return () => unsubs.forEach((u) => u());
  }

  async close(): Promise<void> {
    await terminate(this.db).catch(() => {});
    await deleteApp(this.app).catch(() => {});
  }
}
