import { beforeEach, describe, expect, test } from 'bun:test';
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
