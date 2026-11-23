import type {
  Broadcast,
  Invite,
  Member,
  PresenceInfo,
  ReadRequest,
  Task,
  TaskStatus,
  Team,
} from './types.js';

export type Unsubscribe = () => void;

export class BackendError extends Error {
  constructor(
    public readonly code:
      | 'permission-denied'
      | 'not-found'
      | 'invalid'
      | 'expired'
      | 'conflict'
      | 'unavailable',
    message: string,
  ) {
    super(message);
  }
}

/**
 * Everything the app needs from a team service. Firebase is one implementation;
 * the in-memory one is used by tests. The UI never talks to Firebase directly.
 */
export interface Backend {
  readonly uid: string;
  /** Called when a live subscription fails (for example permission lost or offline). */
  onError?: (e: BackendError) => void;
  /**
   * Best estimate of the service's clock. Event times are server times, so "what is new" must be
   * judged against this, never against the laptop's own clock.
   */
  serverNow(): number;

  createTeam(teamName: string, displayName: string): Promise<Team>;
  /** Manager only. Six digits, valid ten minutes, one successful claim. */
  createInvite(teamId: string): Promise<Invite>;
  joinTeam(code: string, displayName: string): Promise<Team>;
  getTeam(teamId: string): Promise<Team | null>;
  /** Manager only. Removes someone (not the team owner) and frees their name. */
  removeMember(teamId: string, uid: string): Promise<void>;
  /** Leave the team yourself. The owner cannot leave. */
  leaveTeam(teamId: string): Promise<void>;
  /** Manager only. Makes a member a manager. */
  promoteMember(teamId: string, uid: string): Promise<void>;

  heartbeat(teamId: string, info: PresenceInfo): Promise<void>;
  subscribeMembers(teamId: string, cb: (members: Member[]) => void): Unsubscribe;

  sendBroadcast(teamId: string, text: string): Promise<Broadcast>;
  subscribeBroadcasts(teamId: string, cb: (items: Broadcast[]) => void): Unsubscribe;

  createTasks(
    teamId: string,
    items: { assigneeUid: string; assigneeName: string; text: string }[],
  ): Promise<Task[]>;
  /** Assignee, assigner or a manager may update. */
  updateTask(teamId: string, taskId: string, patch: { status: TaskStatus; note?: string }): Promise<void>;
  subscribeTasks(teamId: string, cb: (items: Task[]) => void): Unsubscribe;

  createRequest(
    teamId: string,
    input: { targetUid: string; targetName: string; question: string },
  ): Promise<ReadRequest>;
  /** Asker only, only while pending. Appends the previous target to movedFrom. */
  rerouteRequest(
    teamId: string,
    requestId: string,
    target: { uid: string; name: string },
  ): Promise<void>;
  /** Asker only, while pending or running. */
  cancelRequest(teamId: string, requestId: string): Promise<void>;
  /** Current target only, while pending or running. */
  respondRequest(
    teamId: string,
    requestId: string,
    patch: {
      status: 'running' | 'answered' | 'declined' | 'failed';
      answer?: string;
      agent?: string;
      error?: string;
    },
  ): Promise<void>;
  /** Requests where I am the asker, the current target, or a previous target. */
  subscribeRequests(teamId: string, cb: (items: ReadRequest[]) => void): Unsubscribe;

  close(): Promise<void>;
}
