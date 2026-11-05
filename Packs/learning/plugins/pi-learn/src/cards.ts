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
  const counter = head.createSpan({ cls: 'pi-learn-card-count' });
  const body = card.createDiv({ cls: 'pi-learn-card-body' });
  const setCount = (i: number) => counter.setText(`${Math.min(i + 1, total)} / ${total}`);
  return { card, body, setCount };
}

function textarea(parent: HTMLElement, placeholder: string): HTMLTextAreaElement {
  const el = parent.createEl('textarea', { cls: 'pi-learn-field', attr: { rows: '2', placeholder } });
  el.addEventListener('input', () => {
    el.style.height = 'auto';
    el.style.height = `${el.scrollHeight}px`;
  });
  return el;
}

function button(parent: HTMLElement, label: string, cls = ''): HTMLButtonElement {
  return parent.createEl('button', { cls: `pi-learn-btn ${cls}`.trim(), text: label });
}

function summarize(card: HTMLElement, line: string) {
  card.addClass('is-done');
  card.querySelector('.pi-learn-card-body')?.empty();
  card.createDiv({ cls: 'pi-learn-card-summary', text: line });
}

/* ───────────────────────────── quiz ───────────────────────────── */

/** A radio-style option row. `key` is shown dim on the right as the keyboard shortcut. */
function optionRow(list: HTMLElement, key: string, label: string, md: RenderMarkdown, extra = ''): HTMLButtonElement {
  const b = list.createEl('button', { cls: `pi-learn-option ${extra}`.trim() });
  b.createSpan({ cls: 'pi-learn-radio' });
  md(b.createSpan({ cls: 'pi-learn-option-label' }), label);
  b.createSpan({ cls: 'pi-learn-option-key', text: key });
  return b;
}

export function quizCard(
  parent: HTMLElement,
  params: { topic: string; purpose?: 'probe' | 'predict' | 'check'; questions: QuizQuestion[] },
  md: RenderMarkdown,
): Promise<{ topic: string; answers: QuizAnswer[]; cancelled: boolean }> {
  const { topic, questions } = params;
  const predict = params.purpose === 'predict';
  const answers: QuizAnswer[] = [];
  const label = predict ? 'prediction' : params.purpose === 'check' ? 'check' : params.purpose === 'probe' ? 'probe' : 'question';
  const { card, body, setCount } = shell(parent, label, topic, questions.length);
  card.tabIndex = -1;

  return new Promise((resolve) => {
    let onKey: ((e: KeyboardEvent) => void) | null = null;
    card.addEventListener('keydown', (e) => onKey?.(e));

    const finish = (cancelled: boolean) => {
      onKey = null;
      const score = answers.filter((a) => a.correct).length;
      const marks = answers.map((a) => (a.correct ? '✓' : a.idk ? '?' : '✗') + (a.contested ? '⚑' : '')).join(' ');
      summarize(card, `${marks}  ${score}/${questions.length} correct${cancelled ? ' · stopped early' : ''}`);
      resolve({ topic, answers, cancelled });
    };

    const show = (i: number) => {
      if (i >= questions.length) return finish(false);
      setCount(i);
      body.empty();
      const q = questions[i];
      md(body.createDiv({ cls: 'pi-learn-prompt' }), q.prompt);

      let selected: number | null | undefined; // null = I don't know
      const list = body.createDiv({ cls: 'pi-learn-options' });
      const choices = q.options.map((label, idx) => optionRow(list, String(idx + 1), label, md));
      choices.push(optionRow(list, '0', "I don't know", md, 'is-idk'));
      const choose = (idx: number) => {
        selected = idx < q.options.length ? idx : null;
        choices.forEach((c, j) => c.toggleClass('is-selected', j === idx));
        submit.disabled = false;
      };
      choices.forEach((c, idx) => (c.onclick = () => choose(idx)));

      const conf = body.createDiv({ cls: 'pi-learn-confidence' });
      conf.createSpan({ cls: 'pi-learn-label', text: 'Confidence' });
      const slider = conf.createEl('input', {
        cls: 'pi-learn-slider is-unset',
        attr: { type: 'range', min: '0', max: '100', step: '5', value: '50' },
      });
      const confValue = conf.createSpan({ cls: 'pi-learn-confidence-value', text: '—' });
      let confidence: number | null = null;
      slider.oninput = () => {
        confidence = Number(slider.value);
        slider.removeClass('is-unset');
        confValue.setText(`${confidence}%`);
      };

      const think = body.createDiv({ cls: 'pi-learn-think' });
      think.createDiv({ cls: 'pi-learn-label', text: 'Your thinking' });
      const thinking = think.createEl('textarea', {
        cls: 'pi-learn-think-field',
        attr: { rows: '2', placeholder: predict ? 'What would have to happen? Guess out loud.' : 'Why this answer? Whatever comes to mind.' },
      });
      const row = think.createDiv({ cls: 'pi-learn-think-row' });
      row.createSpan({ cls: 'pi-learn-hint', text: '1–9 pick · 0 don\'t know · ↵ submit · ⇧↵ newline' });
      const stop = row.createEl('button', { cls: 'pi-learn-btn mod-muted', text: 'Stop' });
      const submit = row.createEl('button', { cls: 'pi-learn-btn mod-cta', text: 'Submit answer' });
      submit.disabled = true;
      stop.onclick = () => finish(true);
      thinking.addEventListener('input', () => {
        thinking.style.height = 'auto';
        thinking.style.height = `${thinking.scrollHeight}px`;
      });

      onKey = (e) => {
        const inField = e.target instanceof HTMLTextAreaElement;
        if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && selected !== undefined) {
          e.preventDefault();
          submit.click();
        } else if (!inField && /^[0-9]$/.test(e.key)) {
          const n = Number(e.key);
          if (n === 0) choose(q.options.length);
          else if (n <= q.options.length) choose(n - 1);
        }
      };

      submit.onclick = () => {
        if (selected === undefined) return;
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

        const reveal = body.createDiv({ cls: `pi-learn-reveal ${correct ? 'is-correct' : 'is-wrong'}` });
        const verdict = predict
          ? correct ? 'That is what happens' : idk ? 'Open for now: the next step answers it' : 'Not what happens: the next step shows why'
          : correct ? 'Correct' : idk ? 'Not known yet' : 'Not quite';
        reveal.createDiv({ cls: 'pi-learn-verdict', text: verdict });
        if (q.explanation) md(reveal.createDiv(), q.explanation);
        const note = textarea(body, correct ? 'Anything to add? (optional)' : 'Why did you choose it? (optional)');
        const after = body.createDiv({ cls: 'pi-learn-actions' });
        let contested = false;
        if (!correct && !idk) {
          const contest = button(after, 'Contest grading', 'mod-muted');
          contest.onclick = () => {
            contested = !contested;
            contest.toggleClass('is-active', contested);
            note.placeholder = 'Your reason the grading is wrong';
            note.focus();
          };
        }
        const next = button(after, i + 1 < questions.length ? 'Next question' : 'Done', 'mod-cta');
        next.onclick = () => {
          answers.push({
            id: q.id,
            selectedIndex: idk ? null : (selected as number),
            correct,
            idk,
            thinking: text(thinking.value),
            confidence,
            note: text(note.value),
            contested,
          });
          show(i + 1);
        };
        onKey = (e) => {
          if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
            e.preventDefault();
            next.click();
          }
        };
        next.focus();
      };
      card.focus({ preventScroll: true });
    };
    show(0);
  });
}

/* ──────────────────────────── quiz review ──────────────────────────── */

export function reviewCard(
  parent: HTMLElement,
  params: { topic: string; reviews: QuizReview[] },
  md: RenderMarkdown,
): Promise<{ topic: string; outcomes: ReviewOutcome[]; cancelled: boolean }> {
  const { topic, reviews } = params;
  const outcomes: ReviewOutcome[] = [];
  const { card, body, setCount } = shell(parent, 'review', topic, reviews.length);

  return new Promise((resolve) => {
    const finish = (cancelled: boolean) => {
      const pushed = outcomes.filter((o) => !o.accepted).length;
      summarize(card, pushed ? `${pushed} pushed back` : `${outcomes.length} accepted`);
      resolve({ topic, outcomes, cancelled });
    };
    const show = (i: number) => {
      if (i >= reviews.length) return finish(false);
      setCount(i);
      body.empty();
      const r = reviews[i];
      md(body.createDiv({ cls: 'pi-learn-prompt' }), r.prompt);
      const facts = body.createDiv({ cls: 'pi-learn-review-facts' });
      facts.createDiv({ text: `You chose: ${r.chosen}` });
      facts.createDiv({ text: `Marked correct: ${r.correct}` });
      const verdict = body.createDiv({ cls: `pi-learn-reveal ${r.accepted ? 'is-correct' : 'is-wrong'}` });
      verdict.createDiv({ cls: 'pi-learn-verdict', text: r.accepted ? 'You were right' : 'The verdict stands' });
      md(verdict.createDiv(), r.reason);
      const note = textarea(body, 'Push back: what does this answer miss?');
      const actions = body.createDiv({ cls: 'pi-learn-actions' });
      button(actions, 'Push back', 'mod-muted').onclick = () => {
        if (!text(note.value)) return note.focus();
        outcomes.push({ id: r.id, accepted: false, note: text(note.value) });
        show(i + 1);
      };
      button(actions, 'Accept', 'mod-cta').onclick = () => {
        outcomes.push({ id: r.id, accepted: true, note: null });
        show(i + 1);
      };
    };
    show(0);
  });
}

/* ───────────────────────────── ask ───────────────────────────── */

export function askCard(
  parent: HTMLElement,
  params: { topic: string; questions: AskQuestion[] },
  md: RenderMarkdown,
): Promise<{ topic: string; answers: AskAnswer[]; cancelled: boolean }> {
  const { topic, questions } = params;
  const answers: AskAnswer[] = [];
  const { card, body, setCount } = shell(parent, 'decide', topic, questions.length);

  return new Promise((resolve) => {
    const finish = (cancelled: boolean) => {
      const decided = answers.filter((a) => !a.skipped).length;
      summarize(card, `${decided}/${questions.length} decided${cancelled ? ' · stopped early' : ''}`);
      resolve({ topic, answers, cancelled });
    };
    const show = (i: number) => {
      if (i >= questions.length) return finish(false);
      setCount(i);
      body.empty();
      const q = questions[i];
      if (q.title) body.createDiv({ cls: 'pi-learn-ask-title', text: q.title });
      md(body.createDiv({ cls: 'pi-learn-prompt' }), q.prompt);
      if (q.context) md(body.createDiv({ cls: 'pi-learn-context' }), q.context);

      let selected: number | null = q.recommendedIndex ?? null;
      const list = body.createDiv({ cls: 'pi-learn-options' });
      const preview = body.createEl('pre', { cls: 'pi-learn-preview' });
      const paint = () => {
        choices.forEach((c, j) => c.toggleClass('is-selected', j === selected));
        const p = selected === null ? undefined : q.options[selected]?.preview;
        preview.setText(p ?? '');
        preview.toggle(Boolean(p));
        decide.disabled = selected === null;
      };
      const choices = q.options.map((o, idx) => {
        const b = list.createEl('button', { cls: 'pi-learn-option' });
        b.createSpan({ cls: 'pi-learn-radio' });
        const words = b.createSpan({ cls: 'pi-learn-option-label' });
        words.createDiv({ text: o.label });
        if (o.consequence) words.createDiv({ cls: 'pi-learn-consequence', text: o.consequence });
        b.createSpan({ cls: 'pi-learn-option-key', text: idx === q.recommendedIndex ? '★ recommended' : '' });
        b.onclick = () => {
          selected = idx;
          paint();
        };
        return b;
      });

      const free = textarea(body, 'None of these? Answer in your own words.');
      const note = q.allowNote === false ? null : textarea(body, 'Note on this decision (optional)');
      const actions = body.createDiv({ cls: 'pi-learn-actions' });
      button(actions, 'Stop', 'mod-muted').onclick = () => finish(true);
      button(actions, 'Skip', 'mod-muted').onclick = () => {
        answers.push({
          id: q.id, title: q.title, selectedIndex: null, label: null, freeText: null,
          note: null, followedRecommendation: null, skipped: true,
        });
        show(i + 1);
      };
      const decide = button(actions, i + 1 < questions.length ? 'Next' : 'Done', 'mod-cta');
      free.addEventListener('input', () => {
        decide.disabled = selected === null && !text(free.value);
      });
      decide.onclick = () => {
        const own = text(free.value);
        if (own === null && selected === null) return;
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
        show(i + 1);
      };
      paint();
    };
    show(0);
  });
}
