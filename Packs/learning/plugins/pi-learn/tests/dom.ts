import { parseHTML } from 'linkedom';

export const { window, document } = parseHTML('<!doctype html><html><body></body></html>');

Object.assign(globalThis, {
  window, document,
  HTMLElement: window.HTMLElement,
  HTMLButtonElement: window.HTMLButtonElement,
  HTMLInputElement: window.HTMLInputElement,
  HTMLTextAreaElement: window.HTMLTextAreaElement,
  Event: window.Event,
});

type Options = { cls?: string; text?: string; attr?: Record<string, string> };
const proto = window.HTMLElement.prototype as any;
proto.createEl = function (tag: string, options: Options = {}) {
  const el = document.createElement(tag);
  if (options.cls) el.className = options.cls;
  if (options.text !== undefined) el.textContent = options.text;
  for (const [name, value] of Object.entries(options.attr ?? {})) el.setAttribute(name, value);
  this.appendChild(el);
  return el;
};
proto.createDiv = function (options?: Options) { return this.createEl('div', options); };
proto.createSpan = function (options?: Options) { return this.createEl('span', options); };
proto.empty = function () { this.replaceChildren(); };
proto.addClass = function (...names: string[]) { this.classList.add(...names); };
proto.removeClass = function (...names: string[]) { this.classList.remove(...names); };
proto.toggleClass = function (name: string, enabled: boolean) { this.classList.toggle(name, enabled); };
proto.setText = function (value: string) { this.textContent = value; };
proto.setAttr = function (name: string, value: string) { this.setAttribute(name, value); };
proto.toggle = function (visible: boolean) { this.style.display = visible ? '' : 'none'; };
proto.hide = function () { this.toggle(false); };
proto.show = function () { this.toggle(true); };
window.HTMLButtonElement.prototype.click = function () {
  if (!this.disabled) this.dispatchEvent(new window.Event('click', { bubbles: true }));
};

export function key(target: HTMLElement, value: string, extra: Record<string, unknown> = {}) {
  const event = new window.Event('keydown', { bubbles: true, cancelable: true });
  Object.assign(event, { key: value, ...extra });
  target.dispatchEvent(event);
  return event;
}

export function button(root: HTMLElement, label: string): HTMLButtonElement {
  const found = [...root.querySelectorAll('button')].find((el) => el.textContent === label);
  if (!found) throw new Error(`No button named ${label}`);
  return found as HTMLButtonElement;
}

export function root(): HTMLElement {
  const el = document.createElement('div');
  document.body.appendChild(el);
  return el as unknown as HTMLElement;
}

export const markdown = (el: HTMLElement, text: string) => { el.textContent = text; };
export async function flush() { for (let i = 0; i < 20; i++) await Promise.resolve(); }
