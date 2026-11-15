import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import { askCard, quizCard, reviewCard } from '../src/cards';
import { button, document, key, markdown, root, window } from './dom';

const question = { id: 'q1', prompt: 'Which mechanism explains it?', options: ['Right mechanism', 'Wrong mechanism'], correctIndex: 0, explanation: 'The mechanism, not its label, determines the result.' };
const quiz = { topic: 'Mechanisms', purpose: 'check' as const, questions: [question] };
const ask = { topic: 'Scope', questions: [{ id: 'scope', prompt: 'How far?', options: [{ label: 'One step' }, { label: 'Everything' }], recommendedIndex: 0 }] };
const review = { topic: 'Grading', reviews: [{ id: 'q1', prompt: question.prompt, chosen: 'Wrong mechanism', correct: 'Right mechanism', accepted: false, reason: 'The observation rules out the chosen mechanism.' }] };

beforeEach(() => document.body.replaceChildren());

function reveal(el: HTMLElement, choice = 0) {
  const current = el.querySelector('.pi-learn-card-body > .pi-learn-item') ?? el;
  (current.querySelectorAll('.pi-learn-option')[choice] as HTMLButtonElement).click();
  button(el, 'Submit answer').click();
}

describe('completed learning cards', () => {
  test('each quiz answer remains expandable with thinking, confidence, feedback and contest note', async () => {
    const el = root();
    const pending = quizCard(el, { ...quiz, questions: [question, { ...question, id: 'q2' }] }, markdown);
    const thinking = el.querySelector('textarea')!;
    thinking.value = 'I think the second mechanism is responsible.';
    const slider = el.querySelector('input')!;
    slider.value = '85';
    slider.dispatchEvent(new Event('input'));
    reveal(el, 1);
    button(el, 'Contest grading').click();
    el.querySelectorAll('textarea')[1].value = 'There is another valid interpretation.';
    button(el, 'Next question').click();
    reveal(el);
    button(el, 'Done').click();
    const result = await pending;
    expect(result.cancelled).toBe(false);
    expect(result.answers[0]).toMatchObject({ thinking: thinking.value, confidence: 85, contested: true, note: 'There is another valid interpretation.' });
    expect(el.querySelectorAll('details')).toHaveLength(2);
    expect(el.querySelector('details')!.textContent).toContain(question.explanation);
    expect(el.querySelector('details textarea')!.value).toBe(thinking.value);
    expect(el.querySelector('details .pi-learn-actions')).toBeNull();
    expect([...el.querySelectorAll('input,textarea,button')].every((control) => (control as HTMLInputElement).disabled)).toBe(true);
  });

  test('completing the last card keeps keyboard focus inside the card', async () => {
    const el = root();
    const pending = quizCard(el, quiz, markdown);
    const proto = window.HTMLElement.prototype as unknown as { focus: () => void };
    const original = proto.focus;
    const calls: HTMLElement[] = [];
    proto.focus = function (this: HTMLElement) { calls.push(this); };
    try {
      reveal(el);
      button(el, 'Done').click();
      await pending;
    } finally {
      proto.focus = original;
    }
    expect(calls.at(-1)?.tagName).toBe('SUMMARY');
    expect(el.querySelector('details')!.contains(calls.at(-1)!)).toBe(true);
  });

  test('predictions are explored, not presented as a score', async () => {
    const el = root();
    const pending = quizCard(el, { ...quiz, purpose: 'predict' }, markdown);
    reveal(el, 1);
    button(el, 'Done').click();
    expect((await pending).answers[0].correct).toBe(false);
    expect(el.querySelector('.pi-learn-card-summary')!.textContent).toContain('1 prediction explored');
    expect(el.querySelector('.pi-learn-card-summary')!.textContent).not.toContain('correct');
  });

  test('Ask retains the decision and free-text alternative; Review retains its explanation', async () => {
    const el = root();
    const pending = askCard(el, ask, markdown);
    el.querySelector('textarea')!.value = 'Only the prerequisite.';
    button(el, 'Done').click();
    expect((await pending).answers[0]).toMatchObject({ freeText: 'Only the prerequisite.', skipped: false });
    expect(el.querySelector('details textarea')!.value).toBe('Only the prerequisite.');
    const reviewed = reviewCard(el, review, markdown);
    button(el, 'Accept').click();
    expect((await reviewed).outcomes).toEqual([{ id: 'q1', accepted: true, note: null }]);
    expect(el.querySelectorAll('details')).toHaveLength(2);
    expect(el.textContent).toContain(review.reviews[0].reason);
  });
});

describe('card cancellation', () => {
  for (const [name, make] of [
    ['quiz', (el: HTMLElement, signal: AbortSignal) => quizCard(el, quiz, markdown, signal)],
    ['ask', (el: HTMLElement, signal: AbortSignal) => askCard(el, ask, markdown, signal)],
    ['review', (el: HTMLElement, signal: AbortSignal) => reviewCard(el, review, markdown, signal)],
  ] as const) {
    test(`${name} settles an already-aborted signal`, async () => {
      const controller = new AbortController();
      controller.abort();
      expect((await make(root(), controller.signal)).cancelled).toBe(true);
    });
    test(`${name} disables controls on abort and ignores a saved stale handler`, async () => {
      const el = root();
      const controller = new AbortController();
      const pending = make(el, controller.signal);
      const stale = el.querySelector('button')!.onclick;
      controller.abort();
      const result = await pending;
      stale?.call(el.querySelector('button'), new Event('click') as unknown as MouseEvent);
      controller.abort();
      expect(result.cancelled).toBe(true);
      expect([...el.querySelectorAll('input,textarea,button')].every((control) => (control as HTMLInputElement).disabled)).toBe(true);
    });
  }

  test('Stop is still available after answer reveal', async () => {
    const el = root();
    const pending = quizCard(el, quiz, markdown);
    reveal(el);
    button(el, 'Stop').click();
    const result = await pending;
    expect(result.cancelled).toBe(true);
    expect(result.answers).toHaveLength(1);
    expect(result.answers[0].correct).toBe(true);
    expect(el.textContent).toContain(question.explanation);
  });

  test('review has an explicit Stop action', async () => {
    const el = root();
    const pending = reviewCard(el, review, markdown);
    button(el, 'Stop').click();
    expect((await pending).cancelled).toBe(true);
  });
});

function fakeClock() {
  let now = 0;
  let id = 0;
  const jobs = new Map<number, { at: number; run: () => void }>();
  const schedule = spyOn(globalThis, 'setTimeout').mockImplementation(((run: () => void, ms = 0) => {
    const key = ++id;
    jobs.set(key, { at: now + ms, run });
    return key;
  }) as unknown as typeof setTimeout);
  const cancel = spyOn(globalThis, 'clearTimeout').mockImplementation(((key: number) => {
    jobs.delete(key);
  }) as typeof clearTimeout);
  return {
    get pending() { return jobs.size; },
    tick(ms: number) {
      now += ms;
      for (const [key, job] of [...jobs]) if (job.at <= now) {
        jobs.delete(key);
        job.run();
      }
    },
    restore() { schedule.mockRestore(); cancel.mockRestore(); },
  };
}

describe('ask idle deferral and resume', () => {
  let clock: ReturnType<typeof fakeClock>;
  beforeEach(() => { clock = fakeClock(); });
  afterEach(() => clock.restore());

  test('default idle deadline never accepts the focused recommendation', async () => {
    const el = root();
    const pending = askCard(el, ask, markdown);
    const stale = button(el, 'Done').onclick!;
    clock.tick(59999);
    expect(el.querySelector('.is-done')).toBeNull();
    clock.tick(1);
    const result = await pending;
    expect(result).toMatchObject({ cancelled: false, deferred: true, deferReason: 'idle', answers: [], draft: { id: 'scope', phase: 'options', optionIndex: 0 } });
    stale.call(button(el, 'Done'), new Event('click') as MouseEvent);
    expect(result.answers).toHaveLength(0);
    expect([...el.querySelectorAll('button, textarea')].every((c) => (c as HTMLButtonElement).disabled)).toBe(true);
    expect(clock.pending).toBe(0);
  });

  test('keydown, input and pointerdown each restart the idle deadline', async () => {
    const el = root();
    const pending = askCard(el, { ...ask, idleMs: 100 }, markdown);
    const card = el.querySelector('.pi-learn-card') as HTMLElement;
    for (const event of ['keydown', 'input', 'pointerdown']) {
      clock.tick(98);
      if (event === 'keydown') key(card, 'ArrowLeft');
      else card.dispatchEvent(new Event(event, { bubbles: true }));
      clock.tick(1);
      expect(el.querySelector('.is-done') === null).toBe(true);
    }
    clock.tick(99);
    expect((await pending).deferReason).toBe('idle');
  });

  for (const end of ['idle', 'Stop', 'abort'] as const) test(`${end} retains partial answers and both raw draft fields`, async () => {
    const el = root();
    const controller = new AbortController();
    const next = { ...ask.questions[0], id: 'next' };
    const pending = askCard(el, { ...ask, questions: [...ask.questions, next], idleMs: 100 }, markdown, controller.signal);
    button(el, 'Next').click();
    const current = el.querySelector('.pi-learn-card-body > .pi-learn-item')!;
    const [free, note] = current.querySelectorAll('textarea');
    free.value = '  another route\n';
    free.dispatchEvent(new Event('input', { bubbles: true }));
    note.value = '  unfinished reason\n';
    note.dispatchEvent(new Event('input', { bubbles: true }));
    if (end === 'idle') clock.tick(100);
    else if (end === 'Stop') button(el, 'Stop').click();
    else controller.abort();
    const result = await pending;
    expect(result.answers).toHaveLength(1);
    expect(result.answers[0].id).toBe('scope');
    expect(result).toMatchObject({ cancelled: end !== 'idle', deferred: true, deferReason: end === 'idle' ? 'idle' : 'aborted', draft: { id: 'next', phase: 'note', text: note.value, freeText: free.value, note: note.value } });
    expect(clock.pending).toBe(0);

    const resumed = root();
    const resumedPending = askCard(resumed, { ...ask, questions: [next], answers: result.answers, draft: result.draft }, markdown);
    expect([...resumed.querySelectorAll('textarea')].map((field) => field.value)).toEqual([free.value, note.value]);
    expect(resumed.querySelector('.pi-learn-card-count')!.textContent).toBe('2 / 2');
    button(resumed, 'Done').click();
    expect((await resumedPending).answers[1]).toMatchObject({ freeText: free.value.trim(), note: note.value.trim() });
    expect(resumed.querySelector('.pi-learn-card-summary')!.textContent).toBe('2/2 decided');
    expect(clock.pending).toBe(0);
  });

  test('restores selection and legacy phase text without treating skipped answers as completed', async () => {
    for (const phase of ['options', 'free', 'note'] as const) {
      const el = root();
      const pending = askCard(el, { ...ask, draft: { id: 'scope', phase, optionIndex: 1, text: '  draft  ' } }, markdown);
      expect(el.querySelectorAll('.pi-learn-option')[1].getAttribute('aria-pressed')).toBe('true');
      if (phase !== 'options') expect(el.querySelectorAll('textarea')[phase === 'free' ? 0 : 1].value).toBe('  draft  ');
      button(el, 'Done').click();
      expect((await pending).answers[0].selectedIndex).toBe(phase === 'free' ? null : 1);
    }
    const el = root();
    const pending = askCard(el, { ...ask, questions: [...ask.questions, { ...ask.questions[0], id: 'next' }] }, markdown);
    const stale = button(el, 'Next').onclick!;
    button(el, 'Skip').click();
    stale.call(button(el, 'Done'), new Event('click') as MouseEvent);
    button(el, 'Stop').click();
    const result = await pending;
    expect(result.answers).toEqual([]);
    expect(result.draft?.id).toBe('next');
  });

  test('removes activity and abort listeners and timers on completion and abort', async () => {
    for (const end of ['done', 'idle', 'abort', 'Stop', 'already aborted'] as const) {
      const el = root();
      const controller = new AbortController();
      if (end === 'already aborted') controller.abort();
      const abortCleanup = spyOn(controller.signal, 'removeEventListener');
      const pending = askCard(el, ask, markdown, controller.signal);
      const card = el.querySelector('.pi-learn-card') as HTMLElement;
      const remove = spyOn(card, 'removeEventListener');
      if (end === 'done') button(el, 'Done').click();
      else if (end === 'Stop') button(el, 'Stop').click();
      else if (end === 'abort') controller.abort();
      else if (end === 'idle') clock.tick(60000);
      await pending;
      expect(clock.pending).toBe(0);
      expect(abortCleanup.mock.calls.some(([event]) => event === 'abort')).toBe(true);
      if (end !== 'already aborted') {
        for (const event of ['keydown', 'input', 'pointerdown', 'focusin']) expect(remove.mock.calls.some(([name]) => name === event)).toBe(true);
      }
      card.dispatchEvent(new Event('input', { bubbles: true }));
      key(card, 'Enter');
      expect(clock.pending).toBe(0);
      remove.mockRestore();
      abortCleanup.mockRestore();
    }
  });

  test('already-aborted resume retains seeded answers and draft without scheduling', async () => {
    const controller = new AbortController();
    controller.abort();
    const draft = { id: 'scope', phase: 'free' as const, optionIndex: 1, text: 'raw ', freeText: 'raw ', note: 'note ' };
    const answer = { id: 'previous', selectedIndex: 0, label: 'Chosen', freeText: null, note: null, followedRecommendation: true, skipped: false };
    const result = await askCard(root(), { ...ask, answers: [answer, { ...answer, id: 'skipped', skipped: true }], draft }, markdown, controller.signal);
    expect(result).toMatchObject({ cancelled: true, deferred: true, deferReason: 'aborted', answers: [answer], draft });
    expect(clock.pending).toBe(0);
  });

  test('quiz and review never schedule idle timers', async () => {
    const el = root();
    const pendingQuiz = quizCard(el, quiz, markdown);
    expect(clock.pending).toBe(0);
    button(el, 'Stop').click();
    await pendingQuiz;
    const other = root();
    const pendingReview = reviewCard(other, review, markdown);
    expect(clock.pending).toBe(0);
    button(other, 'Accept').click();
    await pendingReview;
  });
});

describe('card keyboard paths', () => {
  test('numbers select, typed numbers do not, and Shift+Enter/IME do not submit', async () => {
    const el = root();
    const pending = quizCard(el, quiz, markdown);
    const card = el.querySelector('.pi-learn-card') as HTMLElement;
    const thinking = el.querySelector('textarea')!;
    key(thinking, '2');
    expect(el.querySelector('.is-selected')).toBeNull();
    key(card, '1');
    expect(el.querySelector('.is-selected')!.getAttribute('aria-pressed')).toBe('true');
    key(thinking, 'Enter', { shiftKey: true });
    key(thinking, 'Enter', { isComposing: true });
    expect(el.querySelector('.pi-learn-reveal')).toBeNull();
    key(thinking, 'Enter');
    expect(el.querySelector('.pi-learn-reveal')).not.toBeNull();
    button(el, 'Done').click();
    await pending;
  });

  test('Enter on a focused Stop or Contest button is not hijacked', async () => {
    const el = root();
    const pending = quizCard(el, quiz, markdown);
    key(el.querySelector('.pi-learn-card') as HTMLElement, '1');
    expect(key(button(el, 'Stop'), 'Enter').defaultPrevented).toBe(false);
    reveal(el, 1);
    expect(key(button(el, 'Contest grading'), 'Enter').defaultPrevented).toBe(false);
    button(el, 'Stop').click();
    await pending;
  });

  test('Ask supports numeric choice and Enter outside its fields', async () => {
    const el = root();
    const pending = askCard(el, ask, markdown);
    const card = el.querySelector('.pi-learn-card') as HTMLElement;
    key(card, '2');
    key(card, 'Enter');
    expect((await pending).answers[0].selectedIndex).toBe(1);
  });

  test('Enter in a note or free-text field writes a newline instead of submitting', async () => {
    const el = root();
    let quizSettled = false;
    const pending = quizCard(el, quiz, markdown).then((result) => { quizSettled = true; return result; });
    reveal(el);
    const note = el.querySelectorAll('textarea')[1];
    expect(key(note, 'Enter').defaultPrevented).toBe(false);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(quizSettled).toBe(false);
    button(el, 'Done').click();
    await pending;

    const asked = root();
    const askPending = askCard(asked, ask, markdown);
    const free = asked.querySelector('textarea')!;
    free.value = 'Only the prerequisite. ';
    expect(key(free, 'Enter').defaultPrevented).toBe(false);
    button(asked, 'Done').click();
    expect((await askPending).answers[0].freeText).toBe('Only the prerequisite.');

    const reviewed = root();
    const reviewPending = reviewCard(reviewed, review, markdown);
    const pushback = reviewed.querySelector('textarea')!;
    pushback.value = 'The observation was different. ';
    expect(key(pushback, 'Enter').defaultPrevented).toBe(false);
    button(reviewed, 'Push back').click();
    expect((await reviewPending).outcomes[0]).toMatchObject({ accepted: false, note: 'The observation was different.' });
  });

  test('Review accepts from the card, not from inside its note field', async () => {
    const el = root();
    const pending = reviewCard(el, review, markdown);
    key(el.querySelector('.pi-learn-card-review') as HTMLElement, 'Enter');
    expect((await pending).outcomes[0].accepted).toBe(true);
  });
});
