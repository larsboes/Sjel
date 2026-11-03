// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import {
	isAiStudio,
	extractAiStudioTitle,
	nodeToMarkdownWithCitations,
	extractAiStudioTurns,
	extractAiStudio,
	turnsToMarkdown,
	detectRole,
	extractUserContent,
	extractModelContent,
	mergeTurns,
	mergeTurnsForConversation,
	findChatScrollContainer,
	scanFullAiStudioChat,
} from './ai-studio-extractor';

describe('ai-studio-extractor', () => {
	it('identifies AI Studio documents by URL or DOM elements', () => {
		const doc1 = document.implementation.createHTMLDocument();
		doc1.body.innerHTML = '<div>Hello</div>';
		expect(isAiStudio(doc1)).toBe(false);

		const doc2 = document.implementation.createHTMLDocument();
		doc2.body.innerHTML = '<ms-chat-turn></ms-chat-turn>';
		expect(isAiStudio(doc2)).toBe(true);
	});

	it('extracts AI Studio prompt titles from various selectors', () => {
		const doc1 = document.implementation.createHTMLDocument();
		doc1.title = 'LLM Privacy and Routing Strategy - Google AI Studio';
		doc1.body.innerHTML = '<div data-testid="prompt-name">LLM Privacy and Routing Strategy</div>';
		expect(extractAiStudioTitle(doc1)).toBe('LLM Privacy and Routing Strategy');

		const doc2 = document.implementation.createHTMLDocument();
		doc2.title = 'Research Note - Google AI Studio';
		expect(extractAiStudioTitle(doc2)).toBe('Research Note');
	});

	it('preserves footnote citation links [N](url) and unwraps Google redirect URLs', () => {
		const container = document.createElement('div');
		container.innerHTML = `
			<p>commercial/API privacy<a href="https://www.google.com/url?q=https://openai.com/policies&sa=D">[1]</a><a href="https://anthropic.com/privacy">[2]</a>.</p>
			<p>training defaults<a href="https://policies.google.com">3</a><a href="https://deepseek.com">4</a>.</p>
			<p>See <a href="https://sjel.org">Sjel documentation</a> for details.</p>
			<p>Grounding redirect: <a href="https://www.google.com/url?sa=E&q=https%3A%2F%2Fvertexaisearch.cloud.google.com%2Fgrounding-api-redirect%2FAUZIYQHr1rAgghLiyvOlnmUtStinv7TEPq4h4uiU_npZJTTJaXGTfJ1mVqtuKqFeueTPcQ31VW1okiJnEvXNPopRmYCf2-_giaFIqyH0xgI0I-fgVAgh5F7FmJa0gEYSzuTltfs%3D">[15]</a></p>
		`;
		const md = nodeToMarkdownWithCitations(container);
		expect(md).toContain('[1](https://openai.com/policies)');
		expect(md).toContain('[2](https://anthropic.com/privacy)');
		expect(md).toContain('[3](https://policies.google.com)');
		expect(md).toContain('[4](https://deepseek.com)');
		expect(md).toContain('[Sjel documentation](https://sjel.org)');
		expect(md).toContain('[15](https://vertexaisearch.cloud.google.com/grounding-api-redirect/AUZIYQHr1rAgghLiyvOlnmUtStinv7TEPq4h4uiU_npZJTTJaXGTfJ1mVqtuKqFeueTPcQ31VW1okiJnEvXNPopRmYCf2-_giaFIqyH0xgI0I-fgVAgh5F7FmJa0gEYSzuTltfs=)');
	});

	it('extracts AI Studio conversation correctly: prompt 1 in prompt 1, thoughts in thoughts, response 1 in response 1', () => {
		const doc = document.implementation.createHTMLDocument();
		doc.title = 'LLM Privacy and Routing Strategy - Google AI Studio';
		doc.body.innerHTML = `
			<div data-testid="prompt-name">LLM Privacy and Routing Strategy</div>
			<div class="chat-viewport">
				<!-- Turn 1: User message -->
				<ms-chat-turn>
					<div class="turn-header">User • 2:40 PM</div>
					<ms-cmark-node>
						<p>Compare current provider plans (subscriptions and api) of several LLM providers. OpenAI (sub), Anthropic (sub), google AI pro (sub), deepseek (api)...</p>
					</ms-cmark-node>
				</ms-chat-turn>

				<!-- Turn 1: Model response with thoughts and grounded answer -->
				<ms-chat-turn>
					<div class="turn-header">Model • 2:40 PM</div>
					<ms-thought-chunk>
						<button>✦ Thoughts</button>
						<div class="subtitle">Expand to view model thoughts</div>
						<ms-cmark-node>
							<p>Analyzing LLM Plan Details... Okay, I'm now comparing specific LLM provider plans...</p>
						</ms-cmark-node>
					</ms-thought-chunk>
					<ms-cmark-node>
						<p>Building a local-first system like Sjel sits right at the fault line<a href="https://sjel.dev">[1]</a><a href="https://example.com/2">[2]</a>.</p>
						<p>Paying for a consumer subscription does not buy you API privacy<a href="https://example.com/3">[3]</a>.</p>
					</ms-cmark-node>
				</ms-chat-turn>

				<!-- Turn 2: User follow up -->
				<ms-chat-turn>
					<div class="turn-header">User • 2:42 PM</div>
					<ms-cmark-node>
						<p>What about self-hosted Ollama models?</p>
					</ms-cmark-node>
				</ms-chat-turn>

				<!-- Turn 2: Model response -->
				<ms-chat-turn>
					<div class="turn-header">Model • 2:43 PM</div>
					<ms-cmark-node>
						<p>Self-hosted models guarantee zero data retention to external vendors.</p>
					</ms-cmark-node>
				</ms-chat-turn>
			</div>
		`;

		const result = extractAiStudio(doc);
		expect(result).not.toBeNull();
		expect(result!.title).toBe('LLM Privacy and Routing Strategy');
		expect(result!.turns).toHaveLength(2);

		// Turn 1 assertions
		const turn1 = result!.turns[0];
		expect(turn1.index).toBe(1);
		expect(turn1.prompt).toContain('Compare current provider plans');
		expect(turn1.prompt).not.toContain('User •');
		expect(turn1.prompt).not.toContain('Building a local-first');

		expect(turn1.thoughts).toContain('Analyzing LLM Plan Details');
		expect(turn1.thoughts).not.toContain('✦ Thoughts');
		expect(turn1.thoughts).not.toContain('Expand to view model thoughts');

		expect(turn1.response).toContain('Building a local-first system like Sjel');
		expect(turn1.response).toContain('[1](https://sjel.dev)');
		expect(turn1.response).toContain('[2](https://example.com/2)');
		expect(turn1.response).toContain('[3](https://example.com/3)');
		expect(turn1.response).not.toContain('Analyzing LLM Plan Details');
		expect(turn1.response).not.toContain('Compare current provider plans');

		// Turn 2 assertions
		const turn2 = result!.turns[1];
		expect(turn2.index).toBe(2);
		expect(turn2.prompt).toBe('What about self-hosted Ollama models?');
		expect(turn2.response).toBe('Self-hosted models guarantee zero data retention to external vendors.');
		expect(turn2.thoughts).toBeUndefined();

		// Formatted Markdown assertions
		const md = result!.markdown;
		expect(md).toContain('## Prompt 1\nCompare current provider plans');
		expect(md).toContain('> [!NOTE] Thoughts\n> Analyzing LLM Plan Details');
		expect(md).toContain('## Response 1\nBuilding a local-first system');
		expect(md).toContain('## Prompt 2\nWhat about self-hosted Ollama models?');
		expect(md).toContain('## Response 2\nSelf-hosted models guarantee zero data retention');
	});

	it('correctly classifies model turns even when model response contains words like User Data Protection', () => {
		const doc = document.implementation.createHTMLDocument();
		doc.title = 'AI Studio Test';
		doc.body.innerHTML = `
			<ms-chat-turn>
				<div class="turn-header">User • 10:00 AM</div>
				<ms-cmark-node><p>What is the user policy?</p></ms-cmark-node>
			</ms-chat-turn>
			<ms-chat-turn>
				<div class="turn-header">Model • 10:01 AM</div>
				<ms-thought-chunk>
					<button>✦ Thoughts</button>
					<ms-cmark-node><p>Checking policies...</p></ms-cmark-node>
				</ms-thought-chunk>
				<ms-cmark-node>
					<h3>User Data Protection & Privacy</h3>
					<p>User privacy is preserved per agreement<a href="https://example.com/privacy">[1]</a>.</p>
				</ms-cmark-node>
			</ms-chat-turn>
		`;

		const result = extractAiStudio(doc);
		expect(result).not.toBeNull();
		expect(result!.turns).toHaveLength(1);
		expect(result!.turns[0].prompt).toBe('What is the user policy?');
		expect(result!.turns[0].thoughts).toContain('Checking policies...');
		expect(result!.turns[0].response).toContain('User Data Protection & Privacy');
		expect(result!.turns[0].response).toContain('[1](https://example.com/privacy)');
	});

	it('correctly uses data-turn-role attribute on ms-chat-turn', () => {
		const doc = document.implementation.createHTMLDocument();
		doc.title = 'Attribute Test';
		doc.body.innerHTML = `
			<ms-chat-turn data-turn-role="User">
				<ms-cmark-node><p>Prompt 1 paragraph 1</p></ms-cmark-node>
				<ms-cmark-node><p>Prompt 1 paragraph 2</p></ms-cmark-node>
			</ms-chat-turn>
			<ms-chat-turn data-turn-role="Model">
				<ms-cmark-node><p>Model answer here with <a href="https://example.com/cite">1</a>.</p></ms-cmark-node>
			</ms-chat-turn>
		`;

		const result = extractAiStudio(doc);
		expect(result).not.toBeNull();
		expect(result!.turns).toHaveLength(1);
		expect(result!.turns[0].prompt).toContain('Prompt 1 paragraph 1');
		expect(result!.turns[0].prompt).toContain('Prompt 1 paragraph 2');
		expect(result!.turns[0].response).toContain('Model answer here with [1](https://example.com/cite).');
	});

	it('does not extract nested cmark content twice', () => {
		const doc = document.implementation.createHTMLDocument();
		const model = doc.createElement('ms-chat-turn');
		model.innerHTML = `
			<ms-thought-chunk>
				<ms-cmark-node><p>One thought paragraph.</p></ms-cmark-node>
			</ms-thought-chunk>
			<ms-cmark-node>
				<div><ms-cmark-node><p>One answer paragraph.</p></ms-cmark-node></div>
			</ms-cmark-node>
		`;

		const extracted = extractModelContent(model);
		expect(extracted.thoughts).toBe('One thought paragraph.');
		expect(extracted.response).toBe('One answer paragraph.');
	});

	it('handles combined turn containers where user and model are in the same parent', () => {
		const doc = document.implementation.createHTMLDocument();
		doc.title = 'Combined Test';
		doc.body.innerHTML = `
			<ms-chat-turn>
				<div class="user-query">
					<div class="turn-header">User • 1:00 PM</div>
					<ms-cmark-node><p>Combined user prompt</p></ms-cmark-node>
				</div>
				<div class="model-response">
					<div class="turn-header">Model • 1:00 PM</div>
					<ms-thought-chunk>
						<ms-cmark-node><p>Internal thoughts</p></ms-cmark-node>
					</ms-thought-chunk>
					<ms-cmark-node><p>Combined model answer</p></ms-cmark-node>
				</div>
			</ms-chat-turn>
		`;

		const result = extractAiStudio(doc);
		expect(result).not.toBeNull();
		expect(result!.turns).toHaveLength(1);
		expect(result!.turns[0].prompt).toBe('Combined user prompt');
		expect(result!.turns[0].thoughts).toBe('Internal thoughts');
		expect(result!.turns[0].response).toBe('Combined model answer');
	});

	it('converts HTML table to markdown with preserved citation links', () => {
		const doc = document.implementation.createHTMLDocument();
		const tableContainer = doc.createElement('div');
		tableContainer.innerHTML = `
			<table>
				<thead>
					<tr><th>Provider</th><th>Price</th><th>Flagship</th></tr>
				</thead>
				<tbody>
					<tr>
						<td>OpenAI (Sub)</td>
						<td>$20/mo<a href="https://openai.com">[8]</a></td>
						<td>GPT-6 Sol<a href="https://example.com/sol">[12]</a></td>
					</tr>
					<tr>
						<td>Anthropic</td>
						<td>$20/mo</td>
						<td>Claude Opus 5.5<a href="https://anthropic.com">[18]</a></td>
					</tr>
				</tbody>
			</table>
		`;

		const md = nodeToMarkdownWithCitations(tableContainer);
		expect(md).toContain('| Provider | Price | Flagship |');
		expect(md).toContain('| --- | --- | --- |');
		expect(md).toContain('| OpenAI (Sub) | $20/mo[8](https://openai.com) | GPT-6 Sol[12](https://example.com/sol) |');
		expect(md).toContain('| Anthropic | $20/mo | Claude Opus 5.5[18](https://anthropic.com) |');
	});

	it('converts a table whose cells AI Studio wraps in ms-cmark-node', () => {
		const doc = document.implementation.createHTMLDocument();
		const container = doc.createElement('div');
		container.innerHTML = `
			<table><tbody>
				<tr><ms-cmark-node><td><ms-cmark-node><span>Task</span></ms-cmark-node></td></ms-cmark-node>
					<ms-cmark-node><td><ms-cmark-node><span>Runs On</span></ms-cmark-node></td></ms-cmark-node></tr>
				<tr><ms-cmark-node><td><ms-cmark-node><strong><ms-cmark-node><span>Sorting</span></ms-cmark-node></strong></ms-cmark-node></td></ms-cmark-node>
					<ms-cmark-node><td><ms-cmark-node><span>Rust Core</span></ms-cmark-node></td></ms-cmark-node></tr>
			</tbody></table>
		`;

		const md = nodeToMarkdownWithCitations(container);
		expect(md).toContain('| Task | Runs On |\n| --- | --- |\n| **Sorting** | Rust Core |');
		expect(md.match(/Sorting/g)).toHaveLength(1);
	});

	it('merges consecutive model chunks into one single dialogue turn', () => {
		const doc = document.implementation.createHTMLDocument();
		doc.title = 'Multi-chunk Test';
		doc.body.innerHTML = `
			<ms-chat-turn data-turn-role="User">
				<ms-cmark-node><p>Compare LLM providers</p></ms-cmark-node>
			</ms-chat-turn>
			<!-- Model chunk 1: Thoughts + intro -->
			<ms-chat-turn data-turn-role="Model">
				<ms-thought-chunk><ms-cmark-node><p>Analyzing options...</p></ms-cmark-node></ms-thought-chunk>
				<ms-cmark-node><p>Introductory paragraph 1</p></ms-cmark-node>
			</ms-chat-turn>
			<!-- Model chunk 2: Continuation with table -->
			<ms-chat-turn data-turn-role="Model">
				<ms-cmark-node>
					<p>Below is the comparison table:</p>
					<table>
						<tr><th>Vendor</th><th>Cost</th></tr>
						<tr><td>Mistral</td><td>$15<a href="https://mistral.ai">[1]</a></td></tr>
					</table>
				</ms-cmark-node>
			</ms-chat-turn>
			<!-- Model chunk 3: Continuation with concluding advice -->
			<ms-chat-turn data-turn-role="Model">
				<ms-cmark-node><p>Specific recommendations for Sjel...</p></ms-cmark-node>
			</ms-chat-turn>
		`;

		const result = extractAiStudio(doc);
		expect(result).not.toBeNull();
		// All 3 model chunks MUST be merged into Turn 1 (NOT 3 separate turns!)
		expect(result!.turns).toHaveLength(1);
		const turn1 = result!.turns[0];
		expect(turn1.prompt).toBe('Compare LLM providers');
		expect(turn1.thoughts).toBe('Analyzing options...');
		expect(turn1.response).toContain('Introductory paragraph 1');
		expect(turn1.response).toContain('| Vendor | Cost |');
		expect(turn1.response).toContain('| Mistral | $15[1](https://mistral.ai) |');
		expect(turn1.response).toContain('Specific recommendations for Sjel...');
	});

	it('merges continuation chunks without any role or header into the active model response (real AI Studio structure)', () => {
		const doc = document.implementation.createHTMLDocument();
		doc.title = 'AI Studio Real Continuation Test';
		doc.body.innerHTML = `
			<!-- Element 0: User prompt with header -->
			<ms-chat-turn>
				<div class="turn-header">User • 2:40 PM</div>
				<ms-cmark-node><p>Compare LLM privacy for Sjel</p></ms-cmark-node>
			</ms-chat-turn>

			<!-- Element 1: Model thoughts chunk with header -->
			<ms-chat-turn>
				<div class="turn-header">Model • 2:40 PM</div>
				<ms-thought-chunk>
					<button>✦ Thoughts</button>
					<ms-cmark-node><p>ToS, and architectural blind spots...</p></ms-cmark-node>
				</ms-thought-chunk>
			</ms-chat-turn>

			<!-- Element 2: Model response chunk 1 with NO header, only citations -->
			<ms-chat-turn>
				<ms-cmark-node>
					<p>Building a local-first system like Sjel sits right at the fault line<a href="https://vertexaisearch.cloud.google.com/grounding-api-redirect/abc">[1]</a><a href="https://vertexaisearch.cloud.google.com/grounding-api-redirect/def">[2]</a>.</p>
				</ms-cmark-node>
			</ms-chat-turn>

			<!-- Element 3: Model response chunk 2 (table) with NO header -->
			<ms-chat-turn>
				<ms-cmark-node>
					<p>Provider comparison:</p>
					<table>
						<tr><th>Vendor</th><th>Tier</th></tr>
						<tr><td>OpenAI</td><td>API<a href="https://openai.com">[3]</a></td></tr>
					</table>
				</ms-cmark-node>
			</ms-chat-turn>
		`;

		const result = extractAiStudio(doc);
		expect(result).not.toBeNull();
		// Must be 1 dialogue turn, NOT 4 turns!
		expect(result!.turns).toHaveLength(1);
		const turn = result!.turns[0];
		expect(turn.prompt).toBe('Compare LLM privacy for Sjel');
		expect(turn.thoughts).toContain('ToS, and architectural blind spots...');
		expect(turn.response).toContain('Building a local-first system like Sjel');
		expect(turn.response).toContain('[1](https://vertexaisearch.cloud.google.com/grounding-api-redirect/abc)');
		expect(turn.response).toContain('[2](https://vertexaisearch.cloud.google.com/grounding-api-redirect/def)');
		expect(turn.response).toContain('| OpenAI | API[3](https://openai.com) |');
	});

	it('preserves complete response across internal horizontal rule dividers without splitting turns', () => {
		const doc = document.implementation.createHTMLDocument();
		doc.title = 'Divider Test';
		doc.body.innerHTML = `
			<ms-chat-turn>
				<div class="turn-header">User • 1:00 PM</div>
				<ms-cmark-node><p>Overview of providers</p></ms-cmark-node>
			</ms-chat-turn>
			<ms-chat-turn>
				<div class="turn-header">Model • 1:00 PM</div>
				<ms-cmark-node>
					<p>Section A: Overview</p>
					<hr/>
					<p>Section B: Table</p>
					<hr/>
					<p>Section C: Recommendations</p>
				</ms-cmark-node>
			</ms-chat-turn>
		`;

		const result = extractAiStudio(doc);
		expect(result).not.toBeNull();
		expect(result!.turns).toHaveLength(1);
		const turn = result!.turns[0];
		expect(turn.response).toContain('Section A: Overview');
		expect(turn.response).toContain('---');
		expect(turn.response).toContain('Section B: Table');
		expect(turn.response).toContain('Section C: Recommendations');
	});

	it('extracts multi-turn chat with plain-text user query, thoughts-only model turn, and skips bottom composer draft', () => {
		const doc = document.implementation.createHTMLDocument();
		doc.title = 'AI Studio Multi-Turn';
		doc.body.innerHTML = `
			<ms-chat-turn>
				<div class="turn-header">User • 2:40 PM</div>
				<ms-user-query>
					<p>Compare current provider plans</p>
				</ms-user-query>
			</ms-chat-turn>

			<ms-chat-turn>
				<div class="turn-header">Model • 2:40 PM</div>
				<ms-thought-chunk>
					<button>✦ Thoughts</button>
					<ms-cmark-node><p>Analyzing plan details...</p></ms-cmark-node>
				</ms-thought-chunk>
				<ms-cmark-node>
					<p>Building a local-first system like Sjel<a href="https://example.com/1">[1]</a>.</p>
				</ms-cmark-node>
			</ms-chat-turn>

			<!-- Turn 2: Plain text user query without ms-cmark-node -->
			<ms-chat-turn>
				<div class="turn-header">User • 3:17 PM</div>
				<ms-user-query>
					<p>Ok the problem I have: I want to build myself a system which enables me. It pre-sorts, suggests, help me finding something...</p>
					<p>Are there any projects already in this direction?</p>
				</ms-user-query>
			</ms-chat-turn>

			<!-- Turn 2: Model has thoughts, but response is still generating or empty -->
			<ms-chat-turn>
				<div class="turn-header">Model • 3:17 PM</div>
				<ms-thought-chunk>
					<button>✦ Thoughts</button>
					<div class="subtitle">Expand to view model thoughts</div>
					<ms-cmark-node>
						<p>Thinking about personal local-first knowledge management architectures...</p>
					</ms-cmark-node>
				</ms-thought-chunk>
			</ms-chat-turn>

			<!-- Bottom prompt composer box — must NOT become Turn 3! -->
			<ms-chat-turn class="user-turn composer-turn">
				<div class="prompt-box">
					Start typing a prompt to see what our models can do
				</div>
			</ms-chat-turn>
		`;

		const result = extractAiStudio(doc);
		expect(result).not.toBeNull();
		// Exactly 2 turns, NOT 3 or 4!
		expect(result!.turns).toHaveLength(2);

		// Turn 1
		expect(result!.turns[0].index).toBe(1);
		expect(result!.turns[0].prompt).toBe('Compare current provider plans');
		expect(result!.turns[0].thoughts).toContain('Analyzing plan details...');
		expect(result!.turns[0].response).toContain('Building a local-first system like Sjel');

		// Turn 2
		expect(result!.turns[1].index).toBe(2);
		expect(result!.turns[1].prompt).toContain('Ok the problem I have: I want to build myself a system');
		expect(result!.turns[1].prompt).toContain('Are there any projects already in this direction?');
		expect(result!.turns[1].thoughts).toContain('Thinking about personal local-first knowledge management');
		expect(result!.turns[1].response).toBe('');
	});

	it('mergeTurns deduplicates by prompt and upgrades fields with richer content', () => {
		const existingTurns = [
			{ index: 1, prompt: 'How does Sjel work?', response: 'Sjel is a local-first system' },
			{ index: 2, prompt: 'What about local models?', response: 'They run on Mac' },
		];

		const newTurns = [
			// Turn 2 with richer response and thoughts
			{ index: 1, prompt: 'What about local models?', thoughts: 'Deep thought', response: 'They run on Mac Studio or Mac Mini with unified memory.' },
			// Turn 3 (new)
			{ index: 2, prompt: 'How does indexing work?', response: 'SQLite with vector embeddings.' },
		];

		const merged = mergeTurns(existingTurns, newTurns);
		expect(merged).toHaveLength(3);
		expect(merged[0].index).toBe(1);
		expect(merged[0].prompt).toBe('How does Sjel work?');

		expect(merged[1].index).toBe(2);
		expect(merged[1].prompt).toBe('What about local models?');
		expect(merged[1].thoughts).toBe('Deep thought');
		expect(merged[1].response).toBe('They run on Mac Studio or Mac Mini with unified memory.');

		expect(merged[2].index).toBe(3);
		expect(merged[2].prompt).toBe('How does indexing work?');
		expect(merged[2].response).toBe('SQLite with vector embeddings.');
	});

	it('mergeTurns compares cleaned length so duplicate-heavy scans cannot discard unique content', () => {
		const repeated = 'This substantial response paragraph is repeated in the raw source. '.repeat(5);
		const first = mergeTurns([], [{
			id: 'turn-1',
			index: 1,
			prompt: 'Prompt',
			response: `${repeated}\n\n${repeated}\n\nUnique conclusion with [1](https://example.com/source).`,
		}]);
		const second = mergeTurns(first, [{
			id: 'turn-1',
			index: 1,
			prompt: 'Prompt',
			response: `${repeated}\n\n${repeated}\n\n${repeated}`,
		}]);

		expect(second[0].response).toContain('Unique conclusion');
		expect(turnsToMarkdown(second)).toContain('[1](https://example.com/source)');
	});

	it('findChatScrollContainer locates CDK viewport or scrollable parent', () => {
		const doc = document.implementation.createHTMLDocument();
		doc.body.innerHTML = `
			<cdk-virtual-scroll-viewport class="chat-viewport">
				<ms-chat-turn></ms-chat-turn>
			</cdk-virtual-scroll-viewport>
		`;
		const container = findChatScrollContainer(doc);
		expect(container).not.toBeNull();
		expect(container?.tagName.toLowerCase()).toBe('cdk-virtual-scroll-viewport');
	});

	it('deduplicates KaTeX math expressions without duplicating MathML and HTML', () => {
		const container = document.createElement('div');
		container.innerHTML = `
			<p>Paying for a consumer subscription (<span class="katex"><span class="katex-mathml"><math><annotation encoding="application/x-tex">10–200/month</annotation></math></span><span class="katex-html" aria-hidden="true"><span class="mord">10–200/month</span></span></span>) does not buy you commercial privacy.</p>
		`;
		const md = nodeToMarkdownWithCitations(container);
		expect(md).toContain('10–200/month');
		expect(md).not.toContain('10–200/month10–200/month');
	});

	it('does not create fake empty placeholder turns when turns are unmounted in virtual scroll', () => {
		const doc = document.implementation.createHTMLDocument();
		doc.body.innerHTML = `
			<ms-items-scrollbar>
				<button aria-controls="turn-1" aria-label="First prompt snippet...">1</button>
				<button aria-controls="turn-2" aria-label="Second prompt snippet...">2</button>
				<button aria-controls="turn-3" aria-label="Third prompt snippet...">3</button>
				<button aria-controls="turn-4" aria-label="Fourth prompt snippet...">4</button>
			</ms-items-scrollbar>

			<!-- Only Turn 1 is mounted in DOM -->
			<ms-chat-turn id="turn-1">
				<div class="turn-header">User • 1:00 PM</div>
				<ms-cmark-node><p>Full prompt 1 text</p></ms-cmark-node>
			</ms-chat-turn>
			<ms-chat-turn>
				<div class="turn-header">Model • 1:01 PM</div>
				<ms-cmark-node><p>Full response 1 text</p></ms-cmark-node>
			</ms-chat-turn>

			<!-- Turn 2, 3, 4 are unmounted (empty DOM shells with only actions) -->
			<ms-chat-turn id="turn-2" class="text-chunk-host"></ms-chat-turn>
			<ms-chat-turn class="text-chunk-host"></ms-chat-turn>
		`;

		const turns = extractAiStudioTurns(doc);
		// Should ONLY return mounted Turn 1, NOT 4 turns with empty responses!
		expect(turns).toHaveLength(1);
		expect(turns[0].id).toBe('turn-1');
		expect(turns[0].index).toBe(1);
		expect(turns[0].prompt).toBe('Full prompt 1 text');
		expect(turns[0].response).toBe('Full response 1 text');
	});

	it('removes exact long duplicate prompt and response blocks without changing structured content', () => {
		const repeated = `${'This paragraph is an exact duplicate candidate. '.repeat(6)}[1](https://example.com/source)`;
		const short = 'A short repeated phrase.';
		const code = `\`\`\`text\n${'preserve repeated code content '.repeat(10)}\n\`\`\``;
		const quotedCode = `> \`\`\`text\n> ${'preserve quoted code content '.repeat(10)}\n> \`\`\``;
		const table = `| ${'preserve repeated table content '.repeat(10)} |\n| --- |`;
		const markdown = turnsToMarkdown([
			{
				index: 1,
				prompt: `${repeated}\n\n${repeated}\n\n${short}\n\n${short}`,
				response: `${repeated}\n\n${repeated}\n\n${code}\n\n${code}\n\n${quotedCode}\n\n${quotedCode}\n\n${table}\n\n${table}`,
			},
			{ index: 2, prompt: 'Next prompt', response: repeated },
		]);

		const count = (text: string, fragment: string) => text.split(fragment).length - 1;
		expect(count(markdown, repeated)).toBe(3);
		expect(count(markdown, short)).toBe(2);
		expect(count(markdown, code)).toBe(2);
		expect(count(markdown, quotedCode)).toBe(2);
		expect(count(markdown, table)).toBe(2);
		expect(markdown).toContain('[1](https://example.com/source)');
	});

	it('mergeTurns preserves source indices when partial scans arrive out of order', () => {
		const fourth = mergeTurns([], [{ id: 'turn-4', index: 4, prompt: 'Prompt 4', response: 'Response 4' }]);
		const firstAndFourth = mergeTurns(fourth, [
			{ id: 'turn-1', index: 1, prompt: 'Prompt 1', response: 'Response 1' },
		]);
		const turns = mergeTurns(firstAndFourth, [
			{ id: 'turn-2', index: 2, prompt: 'Prompt 2', response: 'Response 2' },
		]);

		expect(turns.map(turn => turn.index)).toEqual([1, 2, 4]);
		expect(turns.map(turn => turn.prompt)).toEqual(['Prompt 1', 'Prompt 2', 'Prompt 4']);
	});

	it('mergeTurns appends unseen fallback turns instead of sorting viewport-local indices', () => {
		const existing = mergeTurns([], [
			{ index: 1, prompt: 'Prompt A', response: 'Response A' },
			{ index: 2, prompt: 'Prompt B', response: 'Response B' },
			{ index: 3, prompt: 'Prompt C', response: 'Response C' },
		]);
		const turns = mergeTurns(existing, [
			{ index: 1, prompt: 'Prompt D', response: 'Response D' },
		]);

		expect(turns.map(turn => turn.prompt)).toEqual(['Prompt A', 'Prompt B', 'Prompt C', 'Prompt D']);
		expect(turns.map(turn => turn.index)).toEqual([1, 2, 3, 4]);
	});

	it('mergeTurns promotes a fallback turn to its stable source index when an ID appears', () => {
		const fallback = mergeTurns([], [
			{ index: 1, prompt: 'Prompt 4', response: 'Response 4' },
		]);
		const promoted = mergeTurns(fallback, [
			{ id: 'turn-4', index: 4, prompt: 'Prompt 4', response: 'Response 4' },
		]);

		expect(promoted).toHaveLength(1);
		expect(promoted[0].id).toBe('turn-4');
		expect(promoted[0].index).toBe(4);
	});

	it('mergeTurnsForConversation discards cached turns when the conversation URL changes', () => {
		const firstConversation = mergeTurnsForConversation([], [
			{ id: 'turn-1', index: 1, prompt: 'Old prompt', response: 'Old response with a much longer body' },
		], '', '/prompts/old');
		const secondConversation = mergeTurnsForConversation(firstConversation, [
			{ id: 'turn-1', index: 1, prompt: 'New prompt', response: 'New response' },
		], '/prompts/old', '/prompts/new');

		expect(secondConversation).toHaveLength(1);
		expect(secondConversation[0].prompt).toBe('New prompt');
		expect(secondConversation[0].response).toBe('New response');
	});

	it('mergeTurns accurately matches turns by ID and preserves distinct prompts', () => {
		const existing = [
			{ id: 'turn-1', index: 1, prompt: 'Prompt 1', response: 'Response 1' },
		];

		const scanned = [
			// Turn 1 mounted again
			{ id: 'turn-1', index: 1, prompt: 'Prompt 1', response: 'Response 1 extended' },
			// Turn 2 newly mounted
			{ id: 'turn-2', index: 2, prompt: 'Prompt 2', response: 'Response 2' },
		];

		const merged = mergeTurns(existing, scanned);
		expect(merged).toHaveLength(2);
		expect(merged[0].id).toBe('turn-1');
		expect(merged[0].response).toBe('Response 1 extended');
		expect(merged[1].id).toBe('turn-2');
		expect(merged[1].prompt).toBe('Prompt 2');
	});
});
