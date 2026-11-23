import type { AgentName } from './types.js';
import { compareVersions, isStandalone, tagVersion } from './update.js';

export interface DoctorInput {
  version: string;
  execPath: string;
  dataDir: string;
  teamName: string;
  agentPref: string;
  detectAgents: () => Promise<AgentName[]>;
  /** Where the team service settings come from, or null when there are none. */
  service: { project: string; source: string; connect: () => Promise<void> } | null;
  latestTag: () => Promise<string>;
}

export interface DoctorReport {
  lines: string[];
  /** False when something that stops crewai from working was found. */
  ok: boolean;
}

const row = (label: string, value: string) => `${label.padEnd(9)} ${value}`;

/** A health report for support: version, where it is installed, agents, the team service and updates. */
export async function collectDoctor(d: DoctorInput): Promise<DoctorReport> {
  const lines: string[] = [];
  let ok = true;
  lines.push(`crewai ${d.version}  (${isStandalone(d.execPath) ? 'installed program' : 'running from source'})`);
  if (isStandalone(d.execPath)) lines.push(row('Program', d.execPath));
  lines.push(row('Data', d.dataDir));

  const found = await d.detectAgents().catch(() => [] as AgentName[]);
  const all: AgentName[] = ['codex', 'claude'];
  lines.push(row('Agents', all.map((a) => `${a} ${found.includes(a) ? 'found' : 'not found'}`).join(', ') + `  (setting: ${d.agentPref})`));
  if (found.length === 0) lines.push(row('', 'Install Codex or Claude Code to answer questions about your folder.'));

  lines.push(row('Team', d.teamName || 'none yet (you choose one the first time you run crewai)'));

  if (!d.service) {
    ok = false;
    lines.push(row('Service', 'not set up. Reinstall from a release, or see app/README.md.'));
  } else {
    try {
      await d.service.connect();
      lines.push(row('Service', `${d.service.project} (${d.service.source}): connected`));
    } catch (e) {
      ok = false;
      lines.push(row('Service', `${d.service.project} (${d.service.source}): cannot connect. ${e instanceof Error ? e.message : ''}`.trim()));
    }
  }

  try {
    const latest = tagVersion(await d.latestTag());
    lines.push(
      row('Update', compareVersions(latest, d.version) > 0 ? `${latest} is available. Run: crewai --update` : `${d.version} is the latest version`),
    );
  } catch {
    lines.push(row('Update', 'could not check (no connection to GitHub)'));
  }
  return { lines, ok };
}
