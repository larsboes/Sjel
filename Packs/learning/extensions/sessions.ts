import { SessionManager, type ExtensionAPI, type SessionInfo } from '@earendil-works/pi-coding-agent';

type Kind = 'teach' | 'practice' | 'curate';
type SessionSource = Pick<SessionInfo, 'path' | 'name' | 'modified' | 'messageCount' | 'firstMessage'>;
export interface LearnSession {
  path: string;
  name: string;
  modified: string;
  messageCount: number;
  kind: Kind;
  target: string;
}

function targetText(text: string): string {
  const link = /^\[\[([^\]]+)\]\]$/.exec(text.trim());
  if (!link) return text.trim();
  const path = link[1].split('|')[0].split('#')[0].trim();
  return path && !path.endsWith('.md') ? `${path}.md` : path;
}

function learningTarget(name: string | undefined, firstMessage: string): { kind: Kind; target: string } | null {
  const named = /^Learn · (teach|practice|curate) · (.+)$/s.exec(name ?? '');
  if (named) {
    const target = targetText(named[2]);
    return target ? { kind: named[1] as Kind, target } : null;
  }

  let prompt = firstMessage.trim();
  let kind: Kind | undefined;
  const expanded = /^<skill name="(teach|practice|curate)" location="[^"]+">/.exec(prompt);
  if (expanded) {
    const end = prompt.lastIndexOf('</skill>');
    if (end < 0) return null;
    kind = expanded[1] as Kind;
    prompt = prompt.slice(end + '</skill>'.length).trim();
  }
  const invocation = /^\/skill:(teach|practice|curate)\s+/.exec(prompt);
  if (invocation) {
    kind = invocation[1] as Kind;
    prompt = prompt.slice(invocation[0].length);
  }
  if (!kind || !prompt) return null;

  // Match only Learn's opening phrases, not links mentioned later in a request.
  const anchor = '(\\[\\[[^\\]\\n]+\\]\\]|[^\\n]+?\\.md)';
  const path = new RegExp(`^Continue the learning path ${anchor}(?= at |\\.|$)`).exec(prompt)
    ?? new RegExp(`^Practice the held nodes of the learning path ${anchor}(?=\\.|$)`).exec(prompt)
    ?? new RegExp(`^Practice the claims of ${anchor}, on the learning path ${anchor}(?=\\.|$)`).exec(prompt);
  const focus = new RegExp(`^(?:Teach |Practice the claims of |Curate )${anchor}(?=[:.]|$)`).exec(prompt);
  const topic = kind === 'teach' ? /^Teach (.+?): probe what I hold of it, then deepen the note\./.exec(prompt)
    : kind === 'practice' ? /^Practice the claims of (.+?)\.(?:\n|$)/.exec(prompt)
    : /^Curate (.+?)\. Start with this note(?: |$)/.exec(prompt);
  const target = targetText(path ? path[path.length - 1] : focus?.[1] ?? topic?.[1] ?? prompt.split('\n')[0]);
  return target ? { kind, target } : null;
}

export function learningSessions(sessions: readonly SessionSource[]): LearnSession[] {
  return sessions.flatMap((session) => {
    if (session.messageCount === 0) return [];
    const learning = learningTarget(session.name, session.firstMessage);
    if (!learning) return [];
    return [{
      path: session.path,
      name: `Learn · ${learning.kind} · ${learning.target}`,
      modified: session.modified.toISOString(),
      messageCount: session.messageCount,
      ...learning,
    }];
  }).sort((a, b) => b.modified.localeCompare(a.modified));
}

export default function (pi: ExtensionAPI) {
  pi.registerCommand('learn-sessions', {
    description: 'List saved learning sessions in Obsidian Learn',
    handler: async (_args, ctx) => {
      if (ctx.mode !== 'rpc' || process.env.PI_QUESTIONS_CLIENT !== 'pi-questions/v1') {
        ctx.ui.notify('Open Obsidian Learn to browse learning history. Use /resume for Pi’s native session picker.', 'info');
        return;
      }
      const sessions = learningSessions(await SessionManager.list(ctx.cwd));
      ctx.ui.setWidget('learn-sessions', [JSON.stringify({ sessions })]);
    },
  });
}
