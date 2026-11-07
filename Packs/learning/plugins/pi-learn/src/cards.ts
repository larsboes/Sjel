/**
 * Inline cards for the `ask` and `quiz` widgets of `Packs/knowledge/extensions/questions.ts`.
 *
 * Each card takes the tool's parameters and resolves with the result object the TUI widget
 * returns, so the tool's own report reaches the model unchanged. The field meanings are defined
 * on the interfaces in `questions.ts`.
 */

export type RenderMarkdown = (el: HTMLElement, markdown: string) => void;

interface QuizQuestion { id: string; prompt: string; options: string[]; correctIndex: number; explanation?: string }
interface QuizAnswer {
  id: string; selectedIndex: number | null; correct: boolean; idk: boolean;
  thinking: string | null; confidence: number | null; note: string | null; contested: boolean;
}
interface AskOption { label: string; consequence?: string; preview?: string }
interface AskQuestion {
  id: string; title?: string; prompt: string; context?: string;
  options: AskOption[]; recommendedIndex?: number; allowNote?: boolean;
}
interface AskAnswer {
  id: string; title?: string; selectedIndex: number | null; label: string | null; freeText: string | null;
  note: string | null; followedRecommendation: boolean | null; skipped: boolean;
}
interface QuizReview { id: string; prompt: string; chosen: string; correct: string; accepted: boolean; reason: string }
interface ReviewOutcome { id: string; accepted: boolean; note: string | null }

const text = (s: string) => (s.trim() ? s.trim() : null);

function shell(parent: HTMLElement, kind: string, topic: string, total: number) {
  const card = parent.createDiv({ cls: `pi-learn-card pi-learn-card-${kind}` });
  const head = card.createDiv({ cls: 'pi-learn-card-label' });
  head.createSpan({ cls: 'pi-learn-card-kind', text: kind });
  head.createSpan({ cls: 'pi-learn-card-topic', text: topic });
  const counter = head.createSpan({ cls: 'pi-learn-card-count', attr: { 'aria-live': 'polite' } });
  const body = card.createDiv({ cls: 'pi-learn-card-body' });
  const setCount = (i: number) => counter.setText(`${Math.min(i + 1, total)} / ${total}`);
  return { card, body, setCount };
}

function textarea(parent: HTMLElement, label: string, placeholder = label): HTMLTextAreaElement {
  const el = parent.createEl('textarea', {
    cls: 'pi-learn-field', attr: { rows: '2', placeholder, 'aria-label': label },
  });
  el.addEventListener('input', () => {
    el.style.height = 'auto';
    el.style.height = `${el.scrollHeight}px`;
  });
  return el;
}

function button(parent: HTMLElement, label: string, cls = ''): HTMLButtonElement {
  return parent.createEl('button', { cls: `pi-learn-btn ${cls}`.trim(), text: label });
}

function disableControls(root: HTMLElement) {
  root.querySelectorAll<HTMLInputElement | HTMLTextAreaElement | HTMLButtonElement | HTMLSelectElement>(
    'input, textarea, button, select',
  ).forEach((control) => { control.disabled = true; });
}

function archive(item: HTMLElement, line: string) {
  disableControls(item);
  item.querySelectorAll('.pi-learn-actions, .pi-learn-think-row').forEach((row) => row.remove());
  const details = item.parentElement!.createEl('details', { cls: 'pi-learn-card-archive' });
  details.createEl('summary', { text: line });
  details.appendChild(item);
}

function summarize(card: HTMLElement, line: string) {
  card.addClass('is-done');
  card.createDiv({ cls: 'pi-learn-card-summary', text: line });
}

function interaction(card: HTMLElement, signal: AbortSignal | undefined, complete: (cancelled: boolean) => void) {
  let done = false;
  let current: HTMLElement | null = null;
  let onKey: ((e: KeyboardEvent) => void) | null = null;
  const keydown = (e: KeyboardEvent) => {
    if (done || e.defaultPrevented || e.isComposing || e.repeat || e.ctrlKey || e.metaKey || e.altKey) return;
    const target = e.target as HTMLElement;
    if (target !== card && !current?.contains(target)) return;
    onKey?.(e);
  };
  const finish = (cancelled: boolean) => {
    if (done) return;
    done = true;
    onKey = null;
    signal?.removeEventListener('abort', abort);
    card.removeEventListener('keydown', keydown);
    disableControls(card);
    complete(cancelled);
    const next = card.querySelector<HTMLElement>('summary') ?? card;
    next.focus({ preventScroll: true });
  };
  const abort = () => finish(true);
  card.tabIndex = -1;
  card.addEventListener('keydown', keydown);
  signal?.addEventListener('abort', abort);
  return {
    finish,
    get done() { return done; },
    keys(item: HTMLElement, handler: (e: KeyboardEvent) => void) {
      current = item;
      onKey = handler;
    },
  };
}

function typing(e: KeyboardEvent): boolean {
  return Boolean((e.target as HTMLElement).closest('textarea, input, select, [contenteditable]:not([contenteditable="false"])'));
}

function submitKey(e: KeyboardEvent): boolean {
  return e.key === 'Enter' && !e.shiftKey && !(e.target as HTMLElement).closest(
    'button, a, summary, input, select, [contenteditable]:not([contenteditable="false"])',
  );
}

/** Enter outside every text field: a note field keeps Enter for its own newline. */
function advanceKey(e: KeyboardEvent): boolean {
  return submitKey(e) && !typing(e);
}

/* ───────────────────────────── quiz ───────────────────────────── */

/** A radio-style option row. `key` is shown dim on the right as the keyboard shortcut. */
function optionRow(list: HTMLElement, key: string, label: string, md: RenderMarkdown, extra = ''): HTMLButtonElement {
  const b = list.createEl('button', {
    cls: `pi-learn-option ${extra}`.trim(), attr: { 'aria-pressed': 'false' },
  });
  b.createSpan({ cls: 'pi-learn-radio' });
  md(b.createSpan({ cls: 'pi-learn-option-label' }), label);
  b.createSpan({ cls: 'pi-learn-option-key', text: key });
  return b;
}

export function quizCard(
  parent: HTMLElement,
  params: { topic: string; purpose?: 'probe' | 'predict' | 'check'; questions: QuizQuestion[] },
  md: RenderMarkdown,
  signal?: AbortSignal,
): Promise<{ topic: string; answers: QuizAnswer[]; cancelled: boolean }> {
  const { topic, questions } = params;
  const predict = params.purpose === 'predict';
  const answers: QuizAnswer[] = [];
  const label = predict ? 'prediction' : params.purpose === 'check' ? 'check' : params.purpose === 'probe' ? 'probe' : 'question';
  const { card, body, setCount } = shell(parent, label, topic, questions.length);

  return new Promise((resolve) => {
    const session = interaction(card, signal, (cancelled) => {
      const score = answers.filter((a) => a.correct).length;
      const marks = answers.map((a) => (a.correct ? '✓' : a.idk ? '?' : '✗') + (a.contested ? '⚑' : '')).join(' ');
      const line = predict ? `${answers.length} prediction${answers.length === 1 ? '' : 's'} explored` : `${marks}  ${score}/${questions.length} correct`;
      summarize(card, `${line}${cancelled ? ' · stopped early' : ''}`);
      resolve({ topic, answers, cancelled });
    });

    const show = (i: number) => {
      if (session.done) return;
      if (i >= questions.length) return session.finish(false);
      setCount(i);
      const item = body.createDiv({ cls: 'pi-learn-item' });
      const q = questions[i];
      md(item.createDiv({ cls: 'pi-learn-prompt' }), q.prompt);

      let active = true;
      let revealed = false;
      let selected: number | null | undefined; // null = I don't know
      const list = item.createDiv({ cls: 'pi-learn-options' });
      const choices = q.options.map((label, idx) => optionRow(list, String(idx + 1), label, md));
      choices.push(optionRow(list, '0', "I don't know", md, 'is-idk'));
      const choose = (idx: number) => {
        if (session.done || !active || revealed) return;
        selected = idx < q.options.length ? idx : null;
        choices.forEach((c, j) => {
          c.toggleClass('is-selected', j === idx);
          c.setAttribute('aria-pressed', String(j === idx));
        });
        submit.disabled = false;
      };
      choices.forEach((c, idx) => (c.onclick = () => choose(idx)));

      const conf = item.createDiv({ cls: 'pi-learn-confidence' });
      conf.createSpan({ cls: 'pi-learn-label', text: 'Confidence' });
      const slider = conf.createEl('input', {
        cls: 'pi-learn-slider is-unset',
        attr: { type: 'range', min: '0', max: '100', step: '5', value: '50', 'aria-label': 'Confidence' },
      });
      const confValue = conf.createSpan({ cls: 'pi-learn-confidence-value', text: '—' });
      let confidence: number | null = null;
      slider.oninput = () => {
        if (session.done || !active || revealed) return;
        confidence = Number(slider.value);
        slider.removeClass('is-unset');
        confValue.setText(`${confidence}%`);
      };

      const think = item.createDiv({ cls: 'pi-learn-think' });
      think.createDiv({ cls: 'pi-learn-label', text: 'Your thinking' });
      const thinking = think.createEl('textarea', {
        cls: 'pi-learn-think-field',
        attr: {
          rows: '2', 'aria-label': 'Your thinking',
          placeholder: predict ? 'What would have to happen? Guess out loud.' : 'Why this answer? Whatever comes to mind.',
        },
      });
      const row = think.createDiv({ cls: 'pi-learn-think-row' });
      row.createSpan({ cls: 'pi-learn-hint', text: '1–9 pick · 0 don\'t know · ↵ submit · ⇧↵ newline' });
      const stop = button(row, 'Stop', 'mod-muted');
      const submit = button(row, 'Submit answer', 'mod-cta');
      submit.disabled = true;
      const cancel = () => { if (active) session.finish(true); };
      stop.onclick = cancel;
      thinking.addEventListener('input', () => {
        thinking.style.height = 'auto';
        thinking.style.height = `${thinking.scrollHeight}px`;
      });

      session.keys(item, (e) => {
        if (submitKey(e) && selected !== undefined) {
          e.preventDefault();
          submit.click();
        } else if (!typing(e) && /^[0-9]$/.test(e.key)) {
          const n = Number(e.key);
          if (n === 0 || n <= q.options.length) {
            e.preventDefault();
            choose(n === 0 ? q.options.length : n - 1);
          }
        }
      });

      submit.onclick = () => {
        if (session.done || !active || revealed || selected === undefined) return;
        revealed = true;
        const idk = selected === null;
        const correct = !idk && selected === q.correctIndex;
        choices.forEach((c, j) => {
          c.disabled = true;
          c.toggleClass('is-correct', j === q.correctIndex);
          c.toggleClass('is-wrong', j === selected && !correct && !idk);
        });
        thinking.disabled = true;
        slider.disabled = true;
        row.remove();

        const reveal = item.createDiv({ cls: `pi-learn-reveal ${correct ? 'is-correct' : 'is-wrong'}` });
        const verdict = predict
          ? correct ? 'That is what happens' : idk ? 'Open for now: the next step answers it' : 'Not what happens: the next step shows why'
          : correct ? 'Correct' : idk ? 'Not known yet' : 'Not quite';
        reveal.createDiv({ cls: 'pi-learn-verdict', text: verdict });
        if (q.explanation) md(reveal.createDiv(), q.explanation);
        const note = textarea(item, 'After-answer note', correct ? 'Anything to add? (optional)' : 'Why did you choose it? (optional)');
        const answer: QuizAnswer = {
          id: q.id,
          selectedIndex: idk ? null : selected as number,
          correct, idk,
          thinking: text(thinking.value), confidence,
          note: null, contested: false,
        };
        answers.push(answer);
        note.addEventListener('input', () => { if (!session.done && active) answer.note = text(note.value); });
        const after = item.createDiv({ cls: 'pi-learn-actions' });
        button(after, 'Stop', 'mod-muted').onclick = cancel;
        let contested = false;
        if (!correct && !idk) {
          const contest = button(after, 'Contest grading', 'mod-muted');
          contest.setAttribute('aria-pressed', 'false');
          contest.onclick = () => {
            if (session.done || !active) return;
            contested = !contested;
            answer.contested = contested;
            contest.toggleClass('is-active', contested);
            contest.setAttribute('aria-pressed', String(contested));
            note.placeholder = 'Your reason the grading is wrong';
            note.focus({ preventScroll: true });
          };
        }
        const next = button(after, i + 1 < questions.length ? 'Next question' : 'Done', 'mod-cta');
        next.onclick = () => {
          if (session.done || !active) return;
          active = false;
          answer.note = text(note.value);
          archive(item, `Question ${i + 1} · ${verdict}${contested ? ' · grading contested' : ''}`);
          show(i + 1);
        };
        session.keys(item, (e) => {
          if (advanceKey(e)) {
            e.preventDefault();
            next.click();
          }
        });
        next.focus({ preventScroll: true });
      };
      card.focus({ preventScroll: true });
    };
    if (signal?.aborted) session.finish(true);
    else show(0);
  });
}

/* ──────────────────────────── quiz review ──────────────────────────── */

export function reviewCard(
  parent: HTMLElement,
  params: { topic: string; reviews: QuizReview[] },
  md: RenderMarkdown,
  signal?: AbortSignal,
): Promise<{ topic: string; outcomes: ReviewOutcome[]; cancelled: boolean }> {
  const { topic, reviews } = params;
  const outcomes: ReviewOutcome[] = [];
  const { card, body, setCount } = shell(parent, 'review', topic, reviews.length);

  return new Promise((resolve) => {
    const session = interaction(card, signal, (cancelled) => {
      const pushed = outcomes.filter((o) => !o.accepted).length;
      const line = pushed ? `${pushed} pushed back` : `${outcomes.length} accepted`;
      summarize(card, `${line}${cancelled ? ' · stopped early' : ''}`);
      resolve({ topic, outcomes, cancelled });
    });
    const show = (i: number) => {
      if (session.done) return;
      if (i >= reviews.length) return session.finish(false);
      setCount(i);
      let active = true;
      const item = body.createDiv({ cls: 'pi-learn-item' });
      const r = reviews[i];
      md(item.createDiv({ cls: 'pi-learn-prompt' }), r.prompt);
      const facts = item.createDiv({ cls: 'pi-learn-review-facts' });
      facts.createDiv({ text: `You chose: ${r.chosen}` });
      facts.createDiv({ text: `Marked correct: ${r.correct}` });
      const verdict = item.createDiv({ cls: `pi-learn-reveal ${r.accepted ? 'is-correct' : 'is-wrong'}` });
      verdict.createDiv({ cls: 'pi-learn-verdict', text: r.accepted ? 'You were right' : 'The verdict stands' });
      md(verdict.createDiv(), r.reason);
      const note = textarea(item, 'Pushback note', 'Push back: what does this answer miss?');
      const actions = item.createDiv({ cls: 'pi-learn-actions' });
      button(actions, 'Stop', 'mod-muted').onclick = () => { if (active) session.finish(true); };
      const respond = (accepted: boolean) => {
        if (session.done || !active) return;
        if (!accepted && !text(note.value)) return note.focus({ preventScroll: true });
        active = false;
        outcomes.push({ id: r.id, accepted, note: accepted ? null : text(note.value) });
        archive(item, `Review ${i + 1} · ${accepted ? 'Accepted' : 'Pushed back'}`);
        show(i + 1);
      };
      const push = button(actions, 'Push back', 'mod-muted');
      push.onclick = () => respond(false);
      const accept = button(actions, 'Accept', 'mod-cta');
      accept.onclick = () => respond(true);
      session.keys(item, (e) => {
        if (advanceKey(e)) {
          e.preventDefault();
          (text(note.value) ? push : accept).click();
        }
      });
      card.focus({ preventScroll: true });
    };
    if (signal?.aborted) session.finish(true);
    else show(0);
  });
}

/* ───────────────────────────── ask ───────────────────────────── */

export function askCard(
  parent: HTMLElement,
  params: { topic: string; questions: AskQuestion[] },
  md: RenderMarkdown,
  signal?: AbortSignal,
): Promise<{ topic: string; answers: AskAnswer[]; cancelled: boolean }> {
  const { topic, questions } = params;
  const answers: AskAnswer[] = [];
  const { card, body, setCount } = shell(parent, 'decide', topic, questions.length);

  return new Promise((resolve) => {
    const session = interaction(card, signal, (cancelled) => {
      const decided = answers.filter((a) => !a.skipped).length;
      summarize(card, `${decided}/${questions.length} decided${cancelled ? ' · stopped early' : ''}`);
      resolve({ topic, answers, cancelled });
    });
    const show = (i: number) => {
      if (session.done) return;
      if (i >= questions.length) return session.finish(false);
      setCount(i);
      let active = true;
      const item = body.createDiv({ cls: 'pi-learn-item' });
      const q = questions[i];
      if (q.title) item.createDiv({ cls: 'pi-learn-ask-title', text: q.title });
      md(item.createDiv({ cls: 'pi-learn-prompt' }), q.prompt);
      if (q.context) md(item.createDiv({ cls: 'pi-learn-context' }), q.context);

      let selected: number | null = q.recommendedIndex ?? null;
      const list = item.createDiv({ cls: 'pi-learn-options' });
      const preview = item.createEl('pre', { cls: 'pi-learn-preview' });
      const paint = () => {
        choices.forEach((c, j) => {
          c.toggleClass('is-selected', j === selected);
          c.setAttribute('aria-pressed', String(j === selected));
        });
        const p = selected === null ? undefined : q.options[selected]?.preview;
        preview.setText(p ?? '');
        preview.toggle(Boolean(p));
        decide.disabled = selected === null && !text(free.value);
      };
      const choose = (idx: number) => {
        if (session.done || !active) return;
        selected = idx;
        paint();
      };
      const choices = q.options.map((o, idx) => {
        const b = list.createEl('button', { cls: 'pi-learn-option' });
        b.createSpan({ cls: 'pi-learn-radio' });
        const words = b.createSpan({ cls: 'pi-learn-option-label' });
        words.createDiv({ text: o.label });
        if (o.consequence) words.createDiv({ cls: 'pi-learn-consequence', text: o.consequence });
        b.createSpan({ cls: 'pi-learn-option-key', text: `${idx + 1}${idx === q.recommendedIndex ? ' · ★ recommended' : ''}` });
        b.onclick = () => choose(idx);
        return b;
      });

      const free = textarea(item, 'Your own answer', 'None of these? Answer in your own words.');
      const note = q.allowNote === false ? null : textarea(item, 'Decision note', 'Note on this decision (optional)');
      const actions = item.createDiv({ cls: 'pi-learn-actions' });
      button(actions, 'Stop', 'mod-muted').onclick = () => { if (active) session.finish(true); };
      const skip = button(actions, 'Skip', 'mod-muted');
      skip.onclick = () => {
        if (session.done || !active) return;
        active = false;
        answers.push({
          id: q.id, title: q.title, selectedIndex: null, label: null, freeText: null,
          note: null, followedRecommendation: null, skipped: true,
        });
        archive(item, `Decision ${i + 1} · Skipped`);
        show(i + 1);
      };
      const decide = button(actions, i + 1 < questions.length ? 'Next' : 'Done', 'mod-cta');
      free.addEventListener('input', () => {
        if (!session.done && active) paint();
      });
      decide.onclick = () => {
        if (session.done || !active) return;
        const own = text(free.value);
        if (own === null && selected === null) return;
        active = false;
        answers.push({
          id: q.id,
          title: q.title,
          selectedIndex: own ? null : selected,
          label: own ? null : q.options[selected as number].label,
          freeText: own,
          note: note ? text(note.value) : null,
          followedRecommendation: own || q.recommendedIndex === undefined ? null : selected === q.recommendedIndex,
          skipped: false,
        });
        archive(item, `Decision ${i + 1} · ${own ?? q.options[selected as number].label}`);
        show(i + 1);
      };
      session.keys(item, (e) => {
        if (advanceKey(e)) {
          e.preventDefault();
          decide.click();
        } else if (!typing(e)) {
          if (/^[1-9]$/.test(e.key) && Number(e.key) <= choices.length) {
            e.preventDefault();
            choose(Number(e.key) - 1);
          } else if (e.key.toLowerCase() === 's') {
            e.preventDefault();
            skip.click();
          }
        }
      });
      paint();
      card.focus({ preventScroll: true });
    };
    if (signal?.aborted) session.finish(true);
    else show(0);
  });
}
