/**
 * Questions for the human in the loop — one widget family, three tools.
 *
 * `ask` — a decision round. Each question carries 2-4 mutually exclusive options (the schema allows
 * up to 9), a one-line consequence per option, an optional preview block rendered only for the
 * highlighted option, and your recommendation marked and focused by default. The user picks one,
 * then may attach a free-text note to that decision. Returns per-decision choices WITH the notes, so
 * the caller can tell an informed answer from an accepted default, and can write the reasoning down.
 *
 * `quiz` — a graded multiple-choice quiz. The model supplies the correct answer and the
 * explanation; the widget grades on selection and returns correct / incorrect / IDK per question.
 *
 * `narrow` — winnow a pile of candidates down to the ones worth deciding about. Space keeps and
 * advances, `s` skips, `x` drops with a reason, `u` retracts; read the narrow section below for
 * why space advances rather than toggling.
 *
 * Used by the `teach` skill (probe + understanding checks) and by question rounds in
 * `crystallize`.
 *
 * Non-interactive modes (-p, json, rpc-without-tui): both tools return a graceful error instead
 * of hanging — the model should fall back to asking in plain text.
 */

import type { ExtensionAPI, ExtensionContext, Theme } from "@earendil-works/pi-coding-agent";
import {
	Editor,
	type EditorTheme,
	Key,
	matchesKey,
	Text,
	type TUI,
	visibleWidth,
	wrapTextWithAnsi,
} from "@earendil-works/pi-tui";
import { Type } from "typebox";

/* ────────────────────────────── shared ────────────────────────────── */

/** Wraps `text` and prefixes the first line, indenting the continuation lines to match. */
function addPrefixed(lines: string[], width: number, prefix: string, text: string) {
	const pw = visibleWidth(prefix);
	if (pw >= width) {
		lines.push(...wrapTextWithAnsi(prefix + text, Math.max(1, width)));
		return;
	}
	const wrapped = wrapTextWithAnsi(text, Math.max(1, width - pw));
	const cont = " ".repeat(pw);
	for (let i = 0; i < wrapped.length; i++) lines.push(`${i === 0 ? prefix : cont}${wrapped[i]}`);
}

function editorTheme(theme: Theme): EditorTheme {
	return {
		borderColor: (s) => theme.fg("border", s),
		selectList: {
			selectedPrefix: (t) => theme.fg("accent", t),
			selectedText: (t) => theme.fg("accent", t),
			description: (t) => theme.fg("muted", t),
			scrollInfo: (t) => theme.fg("dim", t),
			noMatch: (t) => theme.fg("warning", t),
		},
	};
}

/* ────────────────────────── the scrolling window ────────────────────────── */

function terminalRows(tui: TUI): number {
	return tui.terminal?.rows ?? 24;
}

/**
 * Rows one widget's whole frame may take before it crowds out the transcript it is asking about.
 *
 * Every screen in this file is taller than a terminal once a round carries a paragraph of context,
 * a consequence per option and a preview block — and nothing in pi bounds a `ctx.ui.custom`
 * component. It takes over the editor slot and is drawn at whatever height its `render()` returns,
 * so unbounded content grows the dock until the screen is the widget. Seven tenths of the terminal
 * leaves the widget readable and the transcript above it visible.
 */
function widgetRows(tui: TUI): number {
	const rows = terminalRows(tui);
	return Math.max(4, Math.min(rows - 1, Math.floor(rows * 0.7)));
}

/**
 * Line cache for one screen, keyed by width and terminal rows.
 *
 * The TUI repaints on a resize without calling `invalidate()`, and a resize is exactly what the
 * window is measured against — a cache that only knew about content changes would keep drawing the
 * old window until the next keystroke.
 */
class ScreenCache {
	private key = "";
	private lines: string[] | undefined;

	get(width: number, rows: number): string[] | undefined {
		return this.lines !== undefined && this.key === `${width}x${rows}` ? this.lines : undefined;
	}

	set(width: number, rows: number, lines: string[]): void {
		this.key = `${width}x${rows}`;
		this.lines = lines;
	}

	clear(): void {
		this.lines = undefined;
	}
}

/**
 * The scrollable body of one screen: the lines between the two border rows.
 *
 * pi's own `ScrollView` cannot help here — it is cropped by the TUI layout engine, and a component
 * in the editor slot never passes through that. So each screen builds its full body and `cut()`
 * returns the window of it that fits, with the head (topic line) and foot (hint, bottom border)
 * left in place so the frame stays put while the body moves under it.
 *
 * `follow` keeps the focused block in view with the least movement: moving the focus sets it, and a
 * hand scroll clears it, so the window stays where the reader put it until the focus moves again.
 * A fresh question starts at the top instead — following its focus would scroll the prompt off
 * before anyone had read it, and the recommended option is often the last of nine.
 */
class BodyWindow {
	private top = 0;
	/** First and last body line of whatever holds the focus. */
	private anchor: [number, number] = [0, 0];
	/** Height of the last cut, so a hand scroll knows how far a page is. */
	private view = 0;
	/** Consumed by the next cut: start at the top however the focus moved to get here. */
	private startAtTop = true;
	follow = false;

	/** Where the focused block sits, recorded while the body is being built. */
	focus(from: number, to: number = from) {
		this.anchor = [from, Math.max(from, to)];
	}

	/** Open the next question, review or page at the top of its body. */
	reset() {
		this.startAtTop = true;
	}

	/**
	 * Move the window. True when the key was a scroll key.
	 *
	 * Ctrl+U/Ctrl+D is the pair the hint advertises, because fullscreen — which is pi's default mode
	 * — routes PageUp, PageDown, Home and End to the transcript viewport before a focused component
	 * sees them (`tui.altScreen.pageUp`, `.top`, `.bottom`), and an inline widget in the editor slot
	 * is not an overlay it defers to. The PageUp/PageDown pair is kept for regular mode, where
	 * nothing intercepts it.
	 */
	scroll(data: string): boolean {
		const page = Math.max(1, this.view - 1);
		const half = Math.max(1, Math.floor(this.view / 2));
		let next: number;
		if (matchesKey(data, Key.pageUp)) next = this.top - page;
		else if (matchesKey(data, Key.pageDown)) next = this.top + page;
		else if (matchesKey(data, Key.ctrl("u"))) next = this.top - half;
		else if (matchesKey(data, Key.ctrl("d"))) next = this.top + half;
		else return false;
		this.follow = false;
		// Only the key's own movement is applied here; `cut` clamps against the body it is given, so
		// there is one place that decides where the window may sit.
		this.top = Math.max(0, next);
		return true;
	}

	/** The body lines to draw, plus how many were hidden on each side. */
	cut(body: string[], maxRows: number): { lines: string[]; above: number; below: number } {
		const view = Math.max(1, Math.min(maxRows, body.length));
		this.view = view;
		if (this.startAtTop) {
			this.startAtTop = false;
			this.follow = false;
			this.top = 0;
		}
		if (body.length <= view) {
			this.top = 0;
			return { lines: body, above: 0, below: 0 };
		}
		if (this.follow) {
			// Show the focused block and as much as fits above it: lining the block up against the
			// bottom edge keeps the preamble in view, and a block taller than the window keeps its
			// first line rather than its last.
			const [from, to] = this.anchor;
			this.top = Math.max(0, Math.min(from, Math.max(0, to - view + 1)));
		}
		this.top = Math.max(0, Math.min(this.top, body.length - view));
		return { lines: body.slice(this.top, this.top + view), above: this.top, below: body.length - this.top - view };
	}
}

/** ` ▲ 4 · ▼ 9 · ^U/^D ` while lines are hidden, else "". */
function hiddenLabel(view: { above: number; below: number }): string {
	const parts: string[] = [];
	if (view.above > 0) parts.push(`▲ ${view.above}`);
	if (view.below > 0) parts.push(`▼ ${view.below}`);
	return parts.length > 0 ? ` ${parts.join(" · ")} · ^U/^D ` : "";
}

/**
 * The top border, carrying the scroll position when the body was cut. The border row is free
 * space, so the marker costs no room and cannot reflow the hint underneath it.
 */
function topRule(width: number, view: { above: number; below: number }, theme: Theme): string {
	const label = hiddenLabel(view);
	if (label === "" || label.length + 2 >= width) return theme.fg("accent", "─".repeat(width));
	return theme.fg("accent", `─${label}${"─".repeat(width - 1 - label.length)}`);
}

/** Both tools answer with this when there is no terminal to ask in. */
const NON_TUI_ERROR = "Error: interactive UI unavailable (non-interactive mode).";

/* ────────────────────────── ask: a decision round ────────────────────────── */

interface AskOption {
	label: string;
	consequence?: string;
	preview?: string;
}

interface AskQuestion {
	id: string;
	title?: string;
	prompt: string;
	context?: string;
	options: AskOption[];
	recommendedIndex?: number;
	allowNote?: boolean;
}

interface AskAnswer {
	id: string;
	title?: string;
	/** null when the question was skipped, or answered in free text. */
	selectedIndex: number | null;
	label: string | null;
	/**
	 * The user's own answer, recorded when none of the given options fitted. Distinct from `note`:
	 * a note reasons about a choice that was made, this is a refusal of the whole option set, and
	 * distinct from `skipped` because the question was answered rather than left open.
	 */
	freeText: string | null;
	/** Free text the user attached to this decision, or null. */
	note: string | null;
	/** null when there was no recommendation, or the question was skipped, or free text was used. */
	followedRecommendation: boolean | null;
	skipped: boolean;
}

interface AskResult {
	topic: string;
	answers: AskAnswer[];
	cancelled: boolean;
}

const AskParams = Type.Object({
	topic: Type.String({ description: "Round label, e.g. 'Round 3 — vault structure'" }),
	questions: Type.Array(
		Type.Object({
			id: Type.String({ description: "Short id, e.g. 'q1' or 'vault-root'" }),
			title: Type.Optional(
				Type.String({ description: "Short label for the fork, e.g. 'Data classes'. Shown next to the id." }),
			),
			prompt: Type.String({
				description: "The question itself, with the evidence that makes it a real fork.",
			}),
			context: Type.Optional(
				Type.String({
					description:
						"Evidence paragraph shown dim above the options — what you measured, what disagrees, where it lives.",
				}),
			),
			options: Type.Array(
				Type.Object({
					label: Type.String({ description: "The option, in a few words." }),
					consequence: Type.Optional(
						Type.String({
							description:
								"What happens if this is chosen — one line, and include the cost. Not a restatement of the option.",
						}),
					),
					preview: Type.Optional(
						Type.String({
							description:
								"Block rendered only while this option is highlighted: folder layout, config, data model. Separate lines with \\n.",
						}),
					),
				}),
				{ description: "2-4 mutually exclusive options (up to 9). Put the one you recommend first." },
			),
			recommendedIndex: Type.Optional(
				Type.Number({ description: "0-based index of the option you recommend. Marked and focused by default." }),
			),
			allowNote: Type.Optional(
				Type.Boolean({ description: "Whether the user can attach a note after choosing (default true)." }),
			),
		}),
		{ description: "The questions in this round, in order" },
	),
});

function askError(message: string, details: AskResult): { content: { type: "text"; text: string }[]; details: AskResult } {
	return { content: [{ type: "text", text: message }], details };
}

async function runAsk(
	params: { topic: string; questions: AskQuestion[] },
	ctx: ExtensionContext,
): Promise<{ content: { type: "text"; text: string }[]; details: AskResult }> {
	const { topic } = params;
	const questions = params.questions as AskQuestion[];
	const empty: AskResult = { topic, answers: [], cancelled: true };

	if (ctx.mode !== "tui") {
		return askError(
			`${NON_TUI_ERROR} Ask the round as a numbered list in text: 2-9 options each, your recommendation marked, and say a note can be added per question.`,
			empty,
		);
	}
	if (questions.length === 0) return askError("Error: no questions provided", empty);

	const ids = new Set<string>();
	for (const q of questions) {
		if (ids.has(q.id)) throw new Error(`ask: duplicate question id "${q.id}"`);
		ids.add(q.id);
		if (q.options.length < 2 || q.options.length > 9) {
			throw new Error(`ask: question "${q.id}" has ${q.options.length} options — give it 2-9`);
		}
		if (q.recommendedIndex !== undefined && (q.recommendedIndex < 0 || q.recommendedIndex >= q.options.length)) {
			throw new Error(
				`ask: question "${q.id}" recommends option ${q.recommendedIndex}, out of range for ${q.options.length} options`,
			);
		}
	}

	const result = await ctx.ui.custom<AskResult>((tui, theme, _kb, done) => {
		let qIndex = 0;
		let optionIndex = questions[0].recommendedIndex ?? 0;
		let phase: "options" | "note" | "free" = "options";
		const cache = new ScreenCache();
		const answers: AskAnswer[] = [];
		const editor = new Editor(tui, editorTheme(theme));
		const win = new BodyWindow();

		function refresh() {
			cache.clear();
			win.follow = true;
			tui.requestRender();
		}

		/** Repaint without re-following the focus, for a hand scroll. */
		function repaint() {
			cache.clear();
			tui.requestRender();
		}

		function record(note: string | null) {
			const q = questions[qIndex];
			answers.push({
				id: q.id,
				title: q.title,
				selectedIndex: optionIndex,
				label: q.options[optionIndex].label,
				freeText: null,
				note: note && note.length > 0 ? note : null,
				followedRecommendation: q.recommendedIndex === undefined ? null : q.recommendedIndex === optionIndex,
				skipped: false,
			});
		}

		/**
		 * The answer when no option fits. Kept separate from `skip()` on purpose: "none of these, it is
		 * actually X" is a decision, and a caller that cannot tell it from "I don't know yet" will
		 * re-ask a question that was already answered.
		 */
		function recordFreeText(value: string) {
			const q = questions[qIndex];
			const text = value.trim();
			answers.push({
				id: q.id,
				title: q.title,
				selectedIndex: null,
				label: null,
				freeText: text.length > 0 ? text : null,
				note: null,
				followedRecommendation: null,
				skipped: false,
			});
		}

		function advance() {
			editor.setText("");
			phase = "options";
			if (qIndex < questions.length - 1) {
				qIndex++;
				optionIndex = questions[qIndex].recommendedIndex ?? 0;
				win.reset();
				refresh();
				return;
			}
			done({ topic, answers, cancelled: false });
		}

		function skip() {
			const q = questions[qIndex];
			answers.push({
				id: q.id,
				title: q.title,
				selectedIndex: null,
				label: null,
				freeText: null,
				note: null,
				followedRecommendation: null,
				skipped: true,
			});
			advance();
		}

		editor.onChange = () => refresh();
		editor.onSubmit = (value) => {
			if (phase === "free") recordFreeText(value);
			else record(value.trim());
			advance();
		};

		function handleInput(data: string) {
			// Both editor phases route keystrokes to the editor. Missing `free` here was a real bug: the
			// typed answer was parsed as option keys instead, and `o` then Enter landed back in the note
			// phase with nothing recorded.
			if (phase === "note" || phase === "free") {
				if (matchesKey(data, Key.escape)) {
					editor.setText("");
					phase = "options";
					refresh();
					return;
				}
				editor.handleInput(data);
				refresh();
				return;
			}

			if (matchesKey(data, Key.escape)) {
				done({ topic, answers, cancelled: true });
				return;
			}
			// Scrolling is a hand action, so it sits ahead of the option keys: every option key is
			// a plain letter or digit, and none of the scroll keys is one.
			if (win.scroll(data)) {
				repaint();
				return;
			}

			const q = questions[qIndex];
			if (matchesKey(data, Key.up) || data === "k") {
				optionIndex = Math.max(0, optionIndex - 1);
				refresh();
				return;
			}
			if (matchesKey(data, Key.down) || data === "j") {
				optionIndex = Math.min(q.options.length - 1, optionIndex + 1);
				refresh();
				return;
			}
			if (data === "s") {
				skip();
				return;
			}
			// No option fits often enough to deserve a key rather than a note on the closest one.
			if (data === "o") {
				phase = "free";
				refresh();
				return;
			}
			// Number keys move focus rather than answering: every option has a consequence worth
			// reading, and the recommendation is already focused, so Enter alone is the fast path.
			const n = Number.parseInt(data, 10);
			if (!Number.isNaN(n) && data === String(n) && n >= 1 && n <= q.options.length) {
				optionIndex = n - 1;
				refresh();
				return;
			}
			if (matchesKey(data, Key.enter)) {
				if (q.allowNote === false) {
					record(null);
					advance();
					return;
				}
				phase = "note";
				refresh();
			}
		}

		function render(width: number): string[] {
			const hit = cache.get(width, terminalRows(tui));
			if (hit) return hit;
			const w = Math.max(1, width);
			const q = questions[qIndex];
			const chosen = phase === "note" ? optionIndex : null;

			// The head and the hint stay put; only the body scrolls, so the topic and the keys are
			// never scrolled off the thing they belong to.
			const head: string[] = [theme.fg("accent", "─".repeat(w))];
			addPrefixed(
				head,
				w,
				" ",
				theme.fg("muted", `${topic} · ${qIndex + 1}/${questions.length}${q.title ? ` · ${q.title}` : ""}`),
			);
			head.push("");

			const body: string[] = [];
			if (q.context) {
				addPrefixed(body, w, " ", theme.fg("dim", q.context));
				body.push("");
			}
			addPrefixed(body, w, " ", theme.fg("text", theme.bold(q.prompt)));
			body.push("");

			for (let i = 0; i < q.options.length; i++) {
				const opt = q.options[i];
				const focused = i === optionIndex;
				const from = body.length;
				let line = theme.fg(focused ? "accent" : "text", `${i + 1}. ${opt.label}`);
				if (q.recommendedIndex === i) line += "  " + theme.fg("success", "★ recommended");
				if (chosen === i) line += "  " + theme.fg("success", "◆ you chose this");
				addPrefixed(body, w, focused && phase === "options" ? theme.fg("accent", "> ") : "  ", line);
				if (opt.consequence) addPrefixed(body, w, "     ", theme.fg("muted", opt.consequence));
				// Previews are long by nature, so only the option in front of the user carries one.
				if (opt.preview && (focused || chosen === i)) {
					for (const raw of opt.preview.split("\n")) {
						addPrefixed(body, w, "     │ ", theme.fg("dim", raw));
					}
				}
				if (focused) win.focus(from, body.length - 1);
			}

			body.push("");
			let hint: string;
			if (phase === "note") {
				const from = body.length;
				addPrefixed(body, w, " ", theme.fg("text", "Note for this decision (optional):"));
				for (const line of editor.render(Math.max(1, w - 1))) body.push(` ${line}`);
				body.push("");
				// The editor holds the focus here, so the window follows it rather than the option.
				win.focus(from, body.length - 1);
				hint = "Enter to record · an empty note is fine · Esc to change answer";
			} else if (phase === "free") {
				const from = body.length;
				addPrefixed(body, w, " ", theme.fg("text", "None of these — your own answer:"));
				for (const line of editor.render(Math.max(1, w - 1))) body.push(` ${line}`);
				body.push("");
				win.focus(from, body.length - 1);
				hint = "Enter to record it as this decision · Esc back to the options";
			} else {
				hint = "↑↓ move · 1-9 focus · Enter choose · o none of these · s skip (stays open) · Esc end round";
			}

			const foot: string[] = [];
			addPrefixed(foot, w, " ", theme.fg("dim", hint));
			const view = win.cut(body, widgetRows(tui) - head.length - foot.length - 1);
			head[0] = topRule(w, view, theme);
			const lines = [...head, ...view.lines, ...foot, theme.fg("accent", "─".repeat(w))];
			cache.set(width, terminalRows(tui), lines);
			return lines;
		}

		return {
			get focused() {
				return editor.focused;
			},
			set focused(value: boolean) {
				editor.focused = value;
			},
			render,
			invalidate: () => {
				cache.clear();
			},
			handleInput,
		};
	});

	const answered = result.answers.filter((a) => !a.skipped);
	const overruled = result.answers.filter((a) => a.followedRecommendation === false).length;
	const freeText = result.answers.filter((a) => a.freeText).length;
	const header = result.cancelled
		? `Decision round stopped early: ${answered.length}/${questions.length} answered.`
		: `Decision round complete: ${answered.length}/${questions.length} answered${
				overruled > 0 ? `, ${overruled} overruled your recommendation` : ""
			}${freeText > 0 ? `, ${freeText} answered in free text (no option fitted)` : ""}.`;
	const report = [header];
	for (const a of result.answers) {
		const q = questions.find((x) => x.id === a.id);
		const tag = q?.title ? ` (${q.title})` : "";
		if (a.skipped) {
			report.push(`${a.id}${tag}: skipped — still open`);
			continue;
		}
		if (a.freeText) {
			report.push(`${a.id}${tag}: NONE OF THE OPTIONS — the answer is: ${a.freeText}`);
			report.push("    (the option set was rejected, not ignored: if you ask again, ask differently)");
			continue;
		}
		let line = `${a.id}${tag}: ${a.label}`;
		if (a.followedRecommendation === false && q?.recommendedIndex !== undefined) {
			line += ` — overruled your recommendation: ${q.options[q.recommendedIndex].label}`;
		}
		report.push(line);
		if (a.note) report.push(`    note: ${a.note.replace(/\s*\n\s*/g, " / ")}`);
	}
	if (result.cancelled && answered.length < questions.length) {
		report.push(`Not asked: ${questions.slice(result.answers.length).map((q) => q.id).join(", ")}`);
	}
	return { content: [{ type: "text" as const, text: report.join("\n") }], details: result };
}

/* ────────────────────────── quiz: graded, right or wrong ────────────────────────── */

interface QuizQuestion {
	id: string;
	prompt: string;
	options: string[];
	correctIndex: number;
	explanation?: string;
}

interface QuizAnswer {
	id: string;
	selectedIndex: number | null; // null = "I don't know"
	correct: boolean;
	idk: boolean;
	/** Why the user chose it. Null when they chose not to say. */
	note: string | null;
	/** The user disputes the grading. A widget cannot host a model turn, so this is what carries
	 * the dispute into the conversation, where the discussion actually happens. */
	contested: boolean;
}

/** One answer the user disputed, with the model's answer to their reasoning. */
interface QuizReview {
	id: string;
	prompt: string;
	chosen: string;
	correct: string;
	/** True when the user's reasoning was right and the earlier grading was wrong. */
	accepted: boolean;
	reason: string;
}

interface QuizReviewOutcome {
	id: string;
	/** The user accepts the model's revised verdict. */
	accepted: boolean;
	/** Set when they do not: their reply, for another round. */
	note: string | null;
}

interface QuizReviewResult {
	topic: string;
	outcomes: QuizReviewOutcome[];
	cancelled: boolean;
}

interface QuizResult {
	topic: string;
	answers: QuizAnswer[];
	cancelled: boolean;
}

const QuizParams = Type.Object({
	topic: Type.String({ description: "Short label for what this quiz probes, e.g. 'vector calculus prerequisites'" }),
	questions: Type.Array(
		Type.Object({
			id: Type.String({ description: "Unique id, e.g. 'q1'" }),
			prompt: Type.String({ description: "The question text. LaTeX allowed ($...$); keep it terminal-readable too." }),
			options: Type.Array(Type.String(), {
				description: "2-5 answer options. Do NOT include an 'I don't know' option — the widget adds it.",
			}),
			correctIndex: Type.Number({ description: "0-based index of the correct option" }),
			explanation: Type.Optional(
				Type.String({ description: "1-3 sentence explanation shown after answering. Always provide it." }),
			),
		}),
		{ description: "Questions to ask, in order" },
	),
	mode: Type.Optional(
		Type.Union([Type.Literal("ask"), Type.Literal("review")], {
			description:
				"ask (default): pose the questions. review: render your revised verdict on answers the user contested, and take their answer to it. Use review after a round comes back with a contested answer — and call it again if the user pushes back, so the model and the learner alternate until it is settled.",
		}),
	),
	reviews: Type.Optional(
		Type.Array(
			Type.Object({
				id: Type.String({ description: "The question's id, as it appeared in the round that raised it." }),
				prompt: Type.String({ description: "The question text, repeated so the review renders standalone." }),
				chosen: Type.String({ description: "The option the user picked, as text." }),
				correct: Type.String({ description: "The option that round originally marked correct, as text." }),
				accepted: Type.Boolean({
					description:
						"True when the user's reasoning was right and the earlier grading was wrong, so full marks are restored. False when the original verdict stands.",
				}),
				reason: Type.String({
					description: "1-3 sentences answering the user's reason directly. Shown in the widget, so it is read before they reply.",
				}),
			}),
			{ description: "Required when mode is 'review'. One entry per contested answer." },
		),
	),
});

async function runQuiz(
	params: { topic: string; questions?: QuizQuestion[]; mode?: "ask" | "review"; reviews?: QuizReview[] },
	ctx: ExtensionContext,
): Promise<{ content: { type: "text"; text: string }[]; details: QuizResult | QuizReviewResult }> {
	if (params.mode === "review") {
		return runQuizReview({ topic: params.topic, reviews: params.reviews ?? [] }, ctx);
	}
	const questions = (params.questions ?? []) as QuizQuestion[];
	const error = (message: string): { content: { type: "text"; text: string }[]; details: QuizResult } => ({
		content: [{ type: "text", text: message }],
		details: { topic: params.topic, answers: [], cancelled: true },
	});

	if (ctx.mode !== "tui") {
		return error(`${NON_TUI_ERROR} Ask the questions as plain numbered text instead.`);
	}
	if (questions.length === 0) return error("Error: no questions provided");
	for (const q of questions) {
		if (q.correctIndex < 0 || q.correctIndex >= q.options.length) {
			throw new Error(`Question ${q.id}: correctIndex ${q.correctIndex} out of range for ${q.options.length} options`);
		}
	}

	const result = await ctx.ui.custom<QuizResult>((tui, theme, _kb, done) => {
		let qIndex = 0;
		let optionIndex = 0;
		let revealed = false;
		/** The reveal's second phase: the user is typing why they chose it. */
		let notePhase = false;
		const cache = new ScreenCache();
		const answers: QuizAnswer[] = [];
		const editor = new Editor(tui, editorTheme(theme));
		const win = new BodyWindow();

		const IDK = "I don't know";

		function refresh() {
			cache.clear();
			win.follow = true;
			tui.requestRender();
		}

		/** Repaint without re-following the focus, for a hand scroll. */
		function repaint() {
			cache.clear();
			tui.requestRender();
		}

		function currentOptions(): string[] {
			return [...questions[qIndex].options, IDK];
		}

		function selectCurrent() {
			const q = questions[qIndex];
			const idk = optionIndex === q.options.length;
			answers.push({
				id: q.id,
				selectedIndex: idk ? null : optionIndex,
				correct: !idk && optionIndex === q.correctIndex,
				idk,
				note: null,
				contested: false,
			});
			revealed = true;
			refresh();
		}

		/** The answer being revealed. Every phase after the selection edits this one. */
		function currentAnswer(): QuizAnswer | undefined {
			return answers[answers.length - 1];
		}

		editor.onChange = () => refresh();
		editor.onSubmit = (value) => {
			const answer = currentAnswer();
			if (answer) answer.note = value.trim().length > 0 ? value.trim() : null;
			editor.setText("");
			notePhase = false;
			refresh();
		};

		function advance() {
			editor.setText("");
			notePhase = false;
			if (qIndex < questions.length - 1) {
				qIndex++;
				optionIndex = 0;
				revealed = false;
				win.reset();
				refresh();
			} else {
				done({ topic: params.topic, answers, cancelled: false });
			}
		}

		function handleInput(data: string) {
			if (notePhase) {
				if (matchesKey(data, Key.escape)) {
					editor.setText("");
					notePhase = false;
					refresh();
					return;
				}
				editor.handleInput(data);
				refresh();
				return;
			}
			if (matchesKey(data, Key.escape)) {
				done({ topic: params.topic, answers, cancelled: true });
				return;
			}
			// Nothing here collides with the answer keys: digits and plain letters never arrive as
			// PageUp/PageDown or a ctrl combination.
			if (win.scroll(data)) {
				repaint();
				return;
			}
			if (revealed) {
				// `c` marks the grading disputed, which is what the model acts on. Without it a
				// wrong mark stands unless the learner argues in prose outside the quiz.
				if (data === "c") {
					const answer = currentAnswer();
					if (answer) answer.contested = !answer.contested;
					refresh();
					return;
				}
				if (data === "n") {
					notePhase = true;
					refresh();
					return;
				}
				if (matchesKey(data, Key.enter) || data === " ") advance();
				return;
			}
			const opts = currentOptions();
			if (matchesKey(data, Key.up)) {
				optionIndex = Math.max(0, optionIndex - 1);
				refresh();
				return;
			}
			if (matchesKey(data, Key.down)) {
				optionIndex = Math.min(opts.length - 1, optionIndex + 1);
				refresh();
				return;
			}
			// Number keys 1-9 jump-select
			const n = Number.parseInt(data, 10);
			if (!Number.isNaN(n) && n >= 1 && n <= opts.length) {
				optionIndex = n - 1;
				selectCurrent();
				return;
			}
			if (matchesKey(data, Key.enter)) {
				selectCurrent();
			}
		}

		function render(width: number): string[] {
			const hit = cache.get(width, terminalRows(tui));
			if (hit) return hit;
			const w = Math.max(1, width);
			const q = questions[qIndex];
			const opts = currentOptions();
			const answer = answers[answers.length - 1];

			const head: string[] = [theme.fg("accent", "─".repeat(w))];
			const score = answers.filter((a) => a.correct).length;
			addPrefixed(
				head,
				w,
				" ",
				theme.fg("muted", `${params.topic} · question ${qIndex + 1}/${questions.length} · ${score} correct`),
			);
			head.push("");

			const body: string[] = [];
			addPrefixed(body, w, " ", theme.fg("text", theme.bold(q.prompt)));
			body.push("");

			for (let i = 0; i < opts.length; i++) {
				const isIdk = i === q.options.length;
				const selected = i === optionIndex;
				const from = body.length;
				let prefix = selected && !revealed ? theme.fg("accent", "> ") : "  ";
				let color: Parameters<Theme["fg"]>[0] = selected && !revealed ? "accent" : isIdk ? "muted" : "text";
				let suffix = "";
				if (revealed && answer) {
					if (i === q.correctIndex) {
						color = "success";
						suffix = "  ✓";
					} else if (i === answer.selectedIndex) {
						color = "error";
						suffix = "  ✗ your answer";
					} else if (answer.idk && isIdk) {
						color = "warning";
						suffix = "  — your answer";
					} else {
						color = "dim";
					}
					prefix = "  ";
				}
				addPrefixed(body, w, prefix, theme.fg(color, `${i + 1}. ${opts[i]}${suffix}`));
				if (selected && !revealed) win.focus(from, body.length - 1);
			}

			body.push("");
			let hint: string;
			if (revealed && answer) {
				// Once the answer is out, the verdict is what the reader is looking at, not the list.
				const from = body.length;
				const verdict = answer.correct
					? theme.fg("success", "✓ Correct")
					: answer.idk
						? theme.fg("warning", `— The answer: ${q.options[q.correctIndex]}`)
						: theme.fg("error", `✗ Incorrect — correct: ${q.options[q.correctIndex]}`);
				addPrefixed(body, w, " ", verdict);
				if (q.explanation) {
					body.push("");
					addPrefixed(body, w, " ", theme.fg("muted", q.explanation));
				}
				body.push("");
				if (notePhase) {
					addPrefixed(body, w, " ", theme.fg("dim", "Why you chose it — Enter to save · Esc to cancel"));
					for (const editorLine of editor.render(w)) body.push(editorLine);
					hint = "";
				} else {
					if (answer.contested) {
						addPrefixed(body, w, " ", theme.fg("warning", "⚑ You have contested this grading"));
					}
					if (answer.note) {
						addPrefixed(
							body,
							w,
							" ",
							theme.fg("dim", `your reason: ${answer.note.replace(/\s*\n\s*/g, " / ")}`),
						);
					}
					hint =
						answer.correct && !answer.contested
							? "Enter/Space for next · n add a reason · Esc to stop"
							: "Enter/Space for next · n add a reason · c contest · Esc to stop";
				}
				win.focus(from, body.length - 1);
			} else {
				hint = "↑↓ or 1-9 · Enter to answer · Esc to stop";
			}

			const foot: string[] = [];
			if (hint !== "") addPrefixed(foot, w, " ", theme.fg("dim", hint));
			const view = win.cut(body, widgetRows(tui) - head.length - foot.length - 1);
			head[0] = topRule(w, view, theme);
			const lines = [...head, ...view.lines, ...foot, theme.fg("accent", "─".repeat(w))];
			cache.set(width, terminalRows(tui), lines);
			return lines;
		}

		return {
			render,
			invalidate: () => {
				cache.clear();
			},
			handleInput,
		};
	});

	const summary = result.answers.map((a) => {
		const q = questions.find((x) => x.id === a.id);
		const chosen = a.idk ? "I don't know" : (q?.options[a.selectedIndex ?? 0] ?? "?");
		const verdict = a.correct ? "CORRECT" : a.idk ? "IDK" : `INCORRECT (correct: ${q?.options[q.correctIndex]})`;
		const note = a.note ? ` — reason: ${a.note.replace(/\s*\n\s*/g, " / ")}` : "";
		const contested = a.contested ? " — CONTESTED" : "";
		return `${a.id}: "${chosen}" — ${verdict}${contested}${note}`;
	});
	const score = result.answers.filter((a) => a.correct).length;
	const contested = result.answers.filter((a) => a.contested);
	const header = result.cancelled
		? `Quiz stopped early after ${result.answers.length}/${questions.length} question(s).`
		: `Quiz complete: ${score}/${questions.length} correct.`;
	// A contest is not a score correction — it is a claim that the grading was wrong, and it is
	// only settled by re-examining the reasoning. So say so in the result the model reads.
	const resolution = contested.length
		? `\n${contested.length} answer(s) contested (${contested.map((a) => a.id).join(", ")}) — weigh the reason given against the explanation, call quiz again with mode: "review" and your revised verdict, and restore full marks if the learner was right.`
		: "";
	return {
		content: [{ type: "text" as const, text: [header, ...summary].join("\n") + resolution }],
		details: result,
	};
}

/* ─────────────────── quiz review: the model re-examines a contest ─────────────────── */

/**
 * Render the model's revised verdict on the answers the learner disputed, and take their answer
 * to it.
 *
 * Deliberately not a discussion, because a widget cannot host a model turn: the discussion is the
 * conversation, and this is the round-trip that carries one turn of it. The learner accepts and it
 * is settled, or pushes back with a note and the model calls this again with a fresh verdict —
 * which is how the two alternate until it is.
 */
async function runQuizReview(
	params: { topic: string; reviews: QuizReview[] },
	ctx: ExtensionContext,
): Promise<{ content: { type: "text"; text: string }[]; details: QuizReviewResult }> {
	const reviews = params.reviews;
	const error = (message: string): { content: { type: "text"; text: string }[]; details: QuizReviewResult } => ({
		content: [{ type: "text", text: message }],
		details: { topic: params.topic, outcomes: [], cancelled: true },
	});

	if (ctx.mode !== "tui") {
		return error(`${NON_TUI_ERROR} State each revised verdict in plain text instead.`);
	}
	if (reviews.length === 0) {
		return error("Error: mode 'review' needs at least one entry in reviews");
	}

	const result = await ctx.ui.custom<QuizReviewResult>((tui, theme, _kb, done) => {
		let index = 0;
		let notePhase = false;
		const cache = new ScreenCache();
		const outcomes: QuizReviewOutcome[] = [];
		const editor = new Editor(tui, editorTheme(theme));
		const win = new BodyWindow();

		function refresh() {
			cache.clear();
			win.follow = true;
			tui.requestRender();
		}

		/** Repaint without re-following the focus, for a hand scroll. */
		function repaint() {
			cache.clear();
			tui.requestRender();
		}

		function record(accepted: boolean, note: string | null) {
			outcomes.push({ id: reviews[index].id, accepted, note: note && note.length > 0 ? note : null });
		}

		function advance() {
			editor.setText("");
			notePhase = false;
			if (index < reviews.length - 1) {
				index++;
				win.reset();
				refresh();
				return;
			}
			done({ topic: params.topic, outcomes, cancelled: false });
		}

		editor.onChange = () => refresh();
		editor.onSubmit = (value) => {
			record(false, value.trim());
			advance();
		};

		function handleInput(data: string) {
			if (notePhase) {
				if (matchesKey(data, Key.escape)) {
					editor.setText("");
					notePhase = false;
					refresh();
					return;
				}
				editor.handleInput(data);
				refresh();
				return;
			}
			if (matchesKey(data, Key.escape)) {
				done({ topic: params.topic, outcomes, cancelled: true });
				return;
			}
			if (win.scroll(data)) {
				repaint();
				return;
			}
			if (matchesKey(data, Key.enter) || data === " ") {
				record(true, null);
				advance();
				return;
			}
			if (data === "p") {
				notePhase = true;
				refresh();
			}
		}

		function render(width: number): string[] {
			const hit = cache.get(width, terminalRows(tui));
			if (hit) return hit;
			const w = Math.max(1, width);
			const review = reviews[index];

			const head: string[] = [theme.fg("accent", "─".repeat(w))];
			addPrefixed(
				head,
				w,
				" ",
				theme.fg("muted", `${params.topic} · review ${index + 1}/${reviews.length}`),
			);
			head.push("");

			const body: string[] = [];
			addPrefixed(body, w, " ", theme.fg("text", theme.bold(review.prompt)));
			body.push("");
			addPrefixed(body, w, " ", theme.fg("dim", `you: ${review.chosen}`));
			if (review.accepted) {
				addPrefixed(body, w, " ", theme.fg("success", "model now: agrees — full marks"));
			} else {
				addPrefixed(body, w, " ", theme.fg("warning", `model stands by: ${review.correct}`));
			}
			body.push("");
			addPrefixed(body, w, " ", theme.fg("muted", review.reason));
			body.push("");
			let hint = "Enter to accept · p to push back · Esc to stop";
			if (notePhase) {
				// Only the reply is anchored: with nothing typed yet the verdict above is what the
				// reader needs, and the window starts at the top of the body for that reason.
				const from = body.length;
				addPrefixed(body, w, " ", theme.fg("dim", "Your reply — Enter to send · Esc to cancel"));
				for (const editorLine of editor.render(w)) body.push(editorLine);
				win.focus(from, body.length - 1);
				hint = "";
			}

			const foot: string[] = [];
			if (hint !== "") addPrefixed(foot, w, " ", theme.fg("dim", hint));
			const view = win.cut(body, widgetRows(tui) - head.length - foot.length - 1);
			head[0] = topRule(w, view, theme);
			const lines = [...head, ...view.lines, ...foot, theme.fg("accent", "─".repeat(w))];
			cache.set(width, terminalRows(tui), lines);
			return lines;
		}

		return {
			render,
			invalidate: () => {
				cache.clear();
			},
			handleInput,
		};
	});

	const pushed = result.outcomes.filter((o) => !o.accepted);
	const header = result.cancelled
		? `Review stopped after ${result.outcomes.length}/${reviews.length}.`
		: pushed.length > 0
			? `Review: ${reviews.length - pushed.length}/${reviews.length} accepted, ${pushed.length} pushed back.`
			: `Review: all ${reviews.length} accepted.`;
	const summary = result.outcomes.map((o) => {
		const review = reviews.find((r) => r.id === o.id);
		const verdict = o.accepted ? "accepted" : `PUSHED BACK (model still says: ${review?.correct ?? "?"})`;
		const note = o.note ? ` — reply: ${o.note.replace(/\s*\n\s*/g, " / ")}` : "";
		return `${o.id}: ${verdict}${note}`;
	});
	// A pushback is not a rejection of the learner — it is the next turn. Say that in the result so
	// the model answers the reply rather than closing the exchange.
	const next = pushed.length
		? "\nStill contested: answer the reply above in the conversation, and call quiz again with mode: 'review' if the verdict changes. Do not move on while a contest is open."
		: "";
	return {
		content: [{ type: "text" as const, text: [header, ...summary].join("\n") + next }],
		details: result,
	};
}

/* ─────────────────── narrow: winnow a pile of candidates ─────────────────── */

/**
 * Divergence produces more than it needs; this is where the excess goes.
 *
 * `ask` cannot express it: that tool is single-select per question and caps at nine options, so
 * "keep 3 of these 30" has no shape there.
 *
 * The gesture set is built for a linear pass, because the failure mode of a long list is that
 * nobody winnows it and every extra keystroke per item makes that more likely: space or Enter
 * keeps the focused candidate and advances, `s` skips it and advances, `x` rejects it with a
 * reason and advances, and `d` finishes. Arrow keys move without deciding, and `u` retracts a
 * verdict.
 *
 * Two things were wrong until 2026-09-23, and both produced the same empty result. Space used to
 * toggle in place, so a pass over eleven candidates cost two keystrokes each and pressing space
 * twice on one candidate undecided it. And Enter finished the pass — so the reflex `ask` trains,
 * where Enter accepts the focused option, ended the whole thing instead. The finish key is now
 * `d`, so no accept gesture can end a pass. `tools/pack-extensions.test.ts` drives the widget
 * with keystrokes and is what stops either returning; its `enter keeps like space` and
 * `space keeps and advances` cases are the regressions written down.
 */
interface NarrowCandidate {
	id: string;
	oneLine: string;
	/** Which generator produced it. Two candidates sharing one are the same idea twice. */
	generator?: string;
	cost?: string;
	firstTest?: string;
	preview?: string;
}

interface NarrowVerdict {
	id: string;
	kept: boolean;
	/** Set when the candidate was rejected: why, in the user's words. */
	why: string | null;
}

/*
 * The verdict state of one pass, as pure functions.
 *
 * They live in this file rather than in a sibling module because
 * `tools/pack-extensions.test.ts` discovers every `Packs/<pack>/extensions/*.ts` as an extension
 * and asserts each one registers a tool under a stub api — so a helper sitting next to
 * `questions.ts` is read as a broken extension. Extracting them was tried on 2026-09-23 and
 * reverted for that reason; the fix is unchanged either way.
 *
 * Every one returns a new array rather than mutating in place. The version before this pushed,
 * spliced and reassigned a single captured array from four call sites, and a reducer that returns
 * the next state is the version that can be reasoned about.
 */
function verdictIn(verdicts: NarrowVerdict[], id: string): NarrowVerdict | undefined {
	return verdicts.find((v) => v.id === id);
}

/** Keep a candidate, replacing whatever verdict it had. Idempotent, so a repeated keep on one
 * candidate cannot retract it — `u` is the key that does that. */
function keepVerdict(verdicts: NarrowVerdict[], id: string): NarrowVerdict[] {
	return [...verdicts.filter((v) => v.id !== id), { id, kept: true, why: null }];
}

/** Remove any verdict, leaving the candidate undecided again. */
function undecideVerdict(verdicts: NarrowVerdict[], id: string): NarrowVerdict[] {
	return verdicts.filter((v) => v.id !== id);
}

/** Reject a candidate. An empty or whitespace-only reason becomes null rather than "", so
 * "rejected with no reason given" and "rejected with an empty reason" are one state. */
function dropVerdict(verdicts: NarrowVerdict[], id: string, why: string): NarrowVerdict[] {
	const text = why.trim();
	return [...verdicts.filter((v) => v.id !== id), { id, kept: false, why: text.length > 0 ? text : null }];
}

/** Where focus goes after a verdict is recorded. Stops at the last candidate rather than
 * wrapping, which would silently revisit candidates already decided. */
function advanceIndex(index: number, total: number): number {
	return Math.min(index + 1, total - 1);
}

interface NarrowResult {
	topic: string;
	verdicts: NarrowVerdict[];
	cancelled: boolean;
}

const NarrowParams = Type.Object({
	topic: Type.String({ description: "What is being narrowed, e.g. 'Round 1 — how to cut session cost'" }),
	candidates: Type.Array(
		Type.Object({
			id: Type.String({ description: "Short id, e.g. 'c1'." }),
			oneLine: Type.String({ description: "The candidate in one line. This is what gets read." }),
			generator: Type.Optional(
				Type.String({
					description:
						"Which generator produced it (analogy, inversion, constraint-removal, extreme-scale, recombination). Shown beside the id so two candidates from one generator are visible as one idea twice.",
				}),
			),
			cost: Type.Optional(Type.String({ description: "One line on what it costs, if that is known." })),
			firstTest: Type.Optional(
				Type.String({ description: "The cheapest thing that would show whether it works." }),
			),
			preview: Type.Optional(Type.String({ description: "Optional block shown while the candidate is highlighted." })),
		}),
		{ description: "Candidates to winnow. Order matters: put the strongest first." },
	),
});

async function runNarrow(
	params: { topic: string; candidates: NarrowCandidate[] },
	ctx: ExtensionContext,
): Promise<{ content: { type: "text"; text: string }[]; details: NarrowResult }> {
	const candidates = params.candidates ?? [];
	const error = (message: string): { content: { type: "text"; text: string }[]; details: NarrowResult } => ({
		content: [{ type: "text", text: message }],
		details: { topic: params.topic, verdicts: [], cancelled: true },
	});

	if (ctx.mode !== "tui") {
		return error(`${NON_TUI_ERROR} List the candidates as plain numbered text and ask which to keep.`);
	}
	if (candidates.length === 0) return error("Error: no candidates provided");
	const seen = new Set<string>();
	for (const c of candidates) {
		if (seen.has(c.id)) throw new Error(`Candidate ${c.id}: duplicate id`);
		seen.add(c.id);
	}

	const result = await ctx.ui.custom<NarrowResult>((tui, theme, _kb, done) => {
		let index = 0;
		const cache = new ScreenCache();
		let reasonPhase = false;
		let verdicts: NarrowVerdict[] = [];
		const editor = new Editor(tui, editorTheme(theme));
		const win = new BodyWindow();

		function refresh() {
			cache.clear();
			win.follow = true;
			tui.requestRender();
		}

		/** Repaint without re-following the focus, for a hand scroll. */
		function repaint() {
			cache.clear();
			tui.requestRender();
		}

		function verdictOf(id: string): NarrowVerdict | undefined {
			return verdictIn(verdicts, id);
		}

		/**
		 * Keep the focused candidate and move on.
		 *
		 * The advance is the whole point: a pass over a long list should cost one keystroke per
		 * candidate, and a keep that stays put turns every pass into two.
		 */
		function keepAndAdvance() {
			verdicts = keepVerdict(verdicts, candidates[index].id);
			index = advanceIndex(index, candidates.length);
			refresh();
		}

		/** Leave the focused candidate undecided and move on. */
		function skipAndAdvance() {
			index = advanceIndex(index, candidates.length);
			refresh();
		}

		function move(delta: number) {
			index = Math.max(0, Math.min(candidates.length - 1, index + delta));
			refresh();
		}

		editor.onChange = () => refresh();
		editor.onSubmit = (value) => {
			verdicts = dropVerdict(verdicts, candidates[index].id, value);
			reasonPhase = false;
			editor.setText("");
			// Move on automatically: rejecting with a reason is the long part of the pass, and the
			// next candidate is almost always the next thing wanted.
			index = advanceIndex(index, candidates.length);
			refresh();
		};

		function handleInput(data: string) {
			if (reasonPhase) {
				if (matchesKey(data, Key.escape)) {
					editor.setText("");
					reasonPhase = false;
					refresh();
					return;
				}
				editor.handleInput(data);
				refresh();
				return;
			}
			if (matchesKey(data, Key.escape)) {
				done({ topic: params.topic, verdicts, cancelled: true });
				return;
			}
			// A hand scroll outranks the movement keys: none of them is `j`, `k` or a verdict key.
			if (win.scroll(data)) {
				repaint();
				return;
			}
			if (matchesKey(data, Key.up) || data === "k") return move(-1);
			if (matchesKey(data, Key.down) || data === "j") return move(1);
			// `matchesKey` first, with the raw byte as a fallback. Every other widget in this file
			// pairs the two, and this one used to check only `data === " "` — which is how a pass
			// over eleven candidates recorded nothing at all. pi's own space-invaders example
			// hedges the same way, so the raw comparison alone is not a reliable detector.
			//
			// Enter is the same gesture as space, deliberately. `ask` accepts its focused option
			// on Enter, so the reflex a user arrives with is to press Enter to keep a candidate —
			// and while Enter finished the pass instead, that reflex recorded one verdict at most
			// and usually none. The finish key is `d`, which is why no accept gesture can end a
			// pass any more.
			if (matchesKey(data, Key.space) || data === " " || matchesKey(data, Key.enter)) {
				keepAndAdvance();
				return;
			}
			if (data === "s") {
				skipAndAdvance();
				return;
			}
			if (data === "u") {
				verdicts = undecideVerdict(verdicts, candidates[index].id);
				refresh();
				return;
			}
			if (data === "x") {
				reasonPhase = true;
				editor.setText(verdictOf(candidates[index].id)?.why ?? "");
				refresh();
				return;
			}
			if (data === "d") {
				done({ topic: params.topic, verdicts, cancelled: false });
			}
		}

		function render(width: number): string[] {
			const hit = cache.get(width, terminalRows(tui));
			if (hit) return hit;
			const w = Math.max(1, width);
			const keptCount = verdicts.filter((v) => v.kept).length;
			const rejectedCount = verdicts.filter((v) => !v.kept).length;

			const head: string[] = [theme.fg("accent", "─".repeat(w))];
			addPrefixed(
				head,
				w,
				" ",
				theme.fg(
					"muted",
					`${params.topic} · ${candidates.length} candidate(s) · ${keptCount} kept · ${rejectedCount} rejected · ${candidates.length - keptCount - rejectedCount} undecided`,
				),
			);
			head.push("");

			const body: string[] = [];
			for (let i = 0; i < candidates.length; i++) {
				const c = candidates[i];
				const focused = i === index;
				const verdict = verdictOf(c.id);
				const from = body.length;
				const box = verdict ? (verdict.kept ? theme.fg("success", "[keep] ") : theme.fg("error", "[drop] ")) : "[    ] ";
				let line = `${box}${theme.fg(focused ? "accent" : "text", c.oneLine)}`;
				if (c.generator) line += "  " + theme.fg("dim", `(${c.generator})`);
				addPrefixed(body, w, focused ? theme.fg("accent", "> ") : "  ", line);
				if (c.cost) addPrefixed(body, w, "     ", theme.fg("muted", c.cost));
				if (verdict && !verdict.kept && verdict.why) {
					addPrefixed(body, w, "     ", theme.fg("muted", `rejected: ${verdict.why}`));
				}
				if (focused) {
					if (c.firstTest) addPrefixed(body, w, "     ", theme.fg("dim", `first test: ${c.firstTest}`));
					if (c.preview) {
						for (const raw of c.preview.split("\n")) addPrefixed(body, w, "     │ ", theme.fg("dim", raw));
					}
					// A preview runs to many lines, so the window anchors the whole block: the focused
					// candidate stays in view even when its detail does not fit beside it.
					win.focus(from, body.length - 1);
				}
			}

			body.push("");
			let hint: string;
			if (reasonPhase) {
				const from = body.length;
				addPrefixed(body, w, " ", theme.fg("text", `Why drop it? (${candidates[index].id})`));
				for (const editorLine of editor.render(Math.max(1, w - 1))) body.push(` ${editorLine}`);
				body.push("");
				win.focus(from, body.length - 1);
				hint = "Enter to record · an empty reason is fine · Esc to cancel";
			} else {
				hint = "↑↓ move · space/Enter keep → · s skip → · x drop → · u undecide · d finish · Esc stop";
			}

			const foot: string[] = [];
			addPrefixed(foot, w, " ", theme.fg("dim", hint));
			const view = win.cut(body, widgetRows(tui) - head.length - foot.length - 1);
			head[0] = topRule(w, view, theme);
			const lines = [...head, ...view.lines, ...foot, theme.fg("accent", "─".repeat(w))];
			cache.set(width, terminalRows(tui), lines);
			return lines;
		}

		return {
			render,
			invalidate: () => {
				cache.clear();
			},
			handleInput,
		};
	});

	const kept = result.verdicts.filter((v) => v.kept);
	const rejected = result.verdicts.filter((v) => !v.kept);
	const undecided = candidates.filter((c) => !verdictIn(result.verdicts, c.id));
	const header = result.cancelled
		? `Narrowing stopped early: ${kept.length} kept, ${rejected.length} rejected, ${undecided.length} not reached.`
		: `Narrowed: ${kept.length} kept, ${rejected.length} rejected, ${undecided.length} undecided.`;
	const lines = [header];
	if (kept.length > 0) {
		lines.push("Kept:");
		for (const v of kept) {
			const c = candidates.find((x) => x.id === v.id);
			lines.push(`  ${v.id}: ${c?.oneLine ?? "?"}${c?.generator ? ` (${c.generator})` : ""}`);
		}
		lines.push("These are the options on the table. Weigh them with council, or take them to crystallize if the decision is already clear.");
	}
	if (rejected.length > 0) {
		lines.push("Rejected:");
		for (const v of rejected) {
			const c = candidates.find((x) => x.id === v.id);
			lines.push(`  ${v.id}: ${c?.oneLine ?? "?"}${v.why ? ` — because ${v.why}` : ""}`);
		}
		// The rejection table is the artifact that proves the pass happened, and the reasons are
		// what stop the next round producing the same candidates again.
		lines.push("Do not re-propose a rejected candidate in the next round unless the reason no longer holds.");
	}
	if (undecided.length > 0) lines.push(`Undecided, so not on the table: ${undecided.map((c) => c.id).join(", ")}`);

	return { content: [{ type: "text" as const, text: lines.join("\n") }], details: result };
}

/* ────────────────────────────── registration ────────────────────────────── */

export default function questions(pi: ExtensionAPI) {
	pi.registerTool({
		name: "ask",
		label: "Ask",
		description:
			"Ask the user a batched round of decision questions. Each question gets 2-4 mutually exclusive options (up to 9), a one-line consequence per option (what happens if it is chosen, including the cost), an optional preview block shown for the highlighted option (folder layout, config, data model), and your recommendation — marked ★ and focused, so Enter accepts it. The user picks one option and can attach a free-text note to that decision; if none of the options fits they can answer in their own words instead, which comes back as `freeText` (not as a skip — a rejected option set is a decision, and it means ask differently if you ask again); `s` skips a question and leaves it open. Returns every decision with its note and whether it overruled your recommendation. Use for the forks only the user can settle; never for what you can measure yourself, and never for a question whose answer changes nothing.",
		promptSnippet: "Ask the user a round of decision questions, each with options, your recommendation and a note",
		promptGuidelines: [
			"Use ask when a fork belongs to the user and not to you: 2-4 mutually exclusive options, a one-line consequence each, your recommendation marked and given first, and a preview when the choice has shape. The user can attach a note to each decision — those notes are their reasoning, so carry them into whatever you write next.",
			"Use ask never for what you can measure yourself, and never for a question whose answer changes nothing: decide those, say you decided, and move on.",
		],
		parameters: AskParams,

		async execute(_toolCallId, params, _signal, _onUpdate, ctx) {
			return runAsk(params as { topic: string; questions: AskQuestion[] }, ctx);
		},

		renderCall(args, theme, _context) {
			const count = Array.isArray(args.questions) ? args.questions.length : 0;
			let text = theme.fg("toolTitle", theme.bold("ask "));
			text += theme.fg("muted", `${args.topic ?? ""} `);
			text += theme.fg("dim", `(${count} question${count !== 1 ? "s" : ""})`);
			return new Text(text, 0, 0);
		},

		renderResult(result, { expanded }, theme, _context) {
			const details = result.details as AskResult | undefined;
			if (!details || details.answers.length === 0) {
				const t = result.content[0];
				return new Text(theme.fg("warning", t?.type === "text" ? t.text : ""), 0, 0);
			}
			const answered = details.answers.filter((a) => !a.skipped).length;
			const overruled = details.answers.filter((a) => a.followedRecommendation === false).length;
			let text = theme.fg("success", `◆ ${answered}/${details.answers.length} decided`);
			if (overruled > 0) text += theme.fg("warning", ` · ${overruled} overruled your recommendation`);
			if (details.cancelled) text += theme.fg("warning", " (stopped early)");
			for (const a of details.answers) {
				const label = a.skipped
					? theme.fg("warning", "skipped — still open")
					: a.freeText
						? theme.fg(
								"accent",
								`none of the options: ${expanded ? a.freeText : a.freeText.replace(/\s*\n\s*/g, " / ")}`,
							)
						: theme.fg("text", a.label ?? "");
				text += `\n  ${theme.fg("muted", `${a.title ?? a.id}:`)} ${label}`;
				if (a.note) {
					const note = expanded ? a.note : a.note.replace(/\s*\n\s*/g, " / ");
					text += `\n    ${theme.fg("dim", `note: ${note}`)}`;
				}
			}
			return new Text(text, 0, 0);
		},
	});

	pi.registerTool({
		name: "quiz",
		label: "Quiz",
		description:
			"Ask the user graded multiple-choice questions with immediate right/wrong feedback. Use during learning sessions: to probe the edge of the user's understanding (broad → specific, binary-searching each dependency strand) and to verify understanding after each teaching step. Each question carries its correct answer and an explanation; the widget grades on selection and always offers 'I don't know'. After a wrong answer the user can add why they chose it and mark the grading contested; the result says which were contested. Settle a contest with mode: 'review' — pass the model's revised verdict and the widget renders it for the user to accept or push back on, so a disputed answer becomes a short exchange instead of a mark that stands because nobody argued. Returns per-question results (correct / incorrect / IDK, plus note and contested), or per-review outcomes in review mode.",
		promptSnippet: "Ask graded multiple-choice questions and grade them on the spot",
		promptGuidelines: [
			"Use quiz to measure what a learner already holds: give every question its correct answer and a one-to-three sentence explanation, and always add the 'I don't know' escape by leaving it out of the options.",
		],
		parameters: QuizParams,

		async execute(_toolCallId, params, _signal, _onUpdate, ctx) {
			return runQuiz(
				params as { topic: string; questions?: QuizQuestion[]; mode?: "ask" | "review"; reviews?: QuizReview[] },
				ctx,
			);
		},

		renderCall(args, theme, _context) {
			const count = Array.isArray(args.questions) ? args.questions.length : 0;
			let text = theme.fg("toolTitle", theme.bold("quiz "));
			text += theme.fg("muted", `${args.topic ?? ""} `);
			text += theme.fg("dim", `(${count} question${count !== 1 ? "s" : ""})`);
			return new Text(text, 0, 0);
		},

		renderResult(result, _options, theme, _context) {
			const details = result.details as QuizResult | QuizReviewResult | undefined;
			if (details && "outcomes" in details) {
				const pushed = details.outcomes.filter((o) => !o.accepted).length;
				const status = details.cancelled ? theme.fg("warning", " (stopped early)") : "";
				return new Text(
					pushed === 0
						? `${theme.fg("success", "✓")}  ${theme.fg("text", `review: ${details.outcomes.length} accepted`)}${status}`
						: `${theme.fg("warning", "⚑")}  ${theme.fg("text", `review: ${pushed} pushed back`)}${status}`,
					0,
					0,
				);
			}
			if (!details || details.answers.length === 0) {
				const t = result.content[0];
				return new Text(theme.fg("warning", t?.type === "text" ? t.text : ""), 0, 0);
			}
			const score = details.answers.filter((a) => a.correct).length;
			const marks = details.answers
				.map((a) => {
					const mark = a.correct ? theme.fg("success", "✓") : a.idk ? theme.fg("warning", "?") : theme.fg("error", "✗");
					return a.contested ? `${mark}${theme.fg("warning", "⚑")}` : mark;
				})
				.join(" ");
			const status = details.cancelled ? theme.fg("warning", " (stopped early)") : "";
			return new Text(`${marks}  ${theme.fg("text", `${score}/${details.answers.length} correct`)}${status}`, 0, 0);
		},
	});

	pi.registerTool({
		name: "narrow",
		label: "Winnow Candidates",
		description:
			"Show the user a pile of candidates and let them keep, drop or leave each one, one keystroke per candidate. Use after divergence produces more than the decision needs: `ask` cannot express this, because it is single-select per question and caps at nine options, so \"keep 3 of these 30\" has no shape there. Space keeps, `x` drops with a reason, Enter finishes. Returns the kept candidates, the rejected ones with their reasons, and however many were left undecided. The rejected list is the artifact that proves the winnowing happened — do not re-propose a rejected candidate next round unless its reason no longer holds.",
		promptSnippet: "narrow(candidates) — let the user keep/drop a pile of candidates one keystroke at a time",
		promptGuidelines: [
			"Use narrow when there are more candidates than the decision can hold. Give each a different generator, no two sharing a rationale, and put the strongest first. The kept set is what council then weighs; if nothing is kept, the round produced nothing worth deciding about and saying so is the honest result.",
		],
		parameters: NarrowParams,

		async execute(_toolCallId, params, _signal, _onUpdate, ctx) {
			return runNarrow(params as { topic: string; candidates: NarrowCandidate[] }, ctx);
		},

		renderCall(args, theme, _context) {
			const count = Array.isArray(args.candidates) ? args.candidates.length : 0;
			let text = theme.fg("toolTitle", theme.bold("narrow "));
			text += theme.fg("muted", `${args.topic ?? ""} `);
			text += theme.fg("dim", `(${count} candidate${count !== 1 ? "s" : ""})`);
			return new Text(text, 0, 0);
		},

		renderResult(result, _options, theme, _context) {
			const details = result.details as NarrowResult | undefined;
			if (!details || details.verdicts.length === 0) {
				const t = result.content[0];
				return new Text(theme.fg("warning", t?.type === "text" ? t.text : ""), 0, 0);
			}
			const kept = details.verdicts.filter((v) => v.kept).length;
			const dropped = details.verdicts.filter((v) => !v.kept).length;
			const status = details.cancelled ? theme.fg("warning", " (stopped early)") : "";
			return new Text(
				`${theme.fg("success", `◆ ${kept} kept`)}${dropped ? theme.fg("muted", ` · ${dropped} dropped`) : ""}${status}`,
				0,
				0,
			);
		},
	});
}
