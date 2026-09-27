<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { base } from '$app/paths';
  import { page } from '$app/state';
  import { goto } from '$app/navigation';
  import { link } from '$lib/nav';
  import Icon from '$lib/Icon.svelte';
  import { assistantStore } from './assistant.svelte';
  import { extractRouteContext } from './context';
  import GenerativeWidgetRenderer from './cards/GenerativeWidgetRenderer.svelte';

  interface CommandItem {
    id: string;
    label: string;
    hint: string;
    icon: string;
    run: () => void | Promise<void>;
  }

  const COMMANDS: CommandItem[] = [
    { id: 'c-home', label: '/home', hint: 'Jump to Home & Today', icon: 'home', run: () => { void goto(link('/')); assistantStore.closeDrawer(); } },
    { id: 'c-cal', label: '/calendar', hint: 'Jump to Calendar & Agenda', icon: 'calendar', run: () => { void goto(link('/calendar')); assistantStore.closeDrawer(); } },
    { id: 'c-trav', label: '/travel', hint: 'Jump to Travel & Connections', icon: 'train', run: () => { void goto(link('/travel')); assistantStore.closeDrawer(); } },
    { id: 'c-feed', label: '/feed', hint: 'Jump to Reading Feed', icon: 'feed', run: () => { void goto(link('/feed')); assistantStore.closeDrawer(); } },
    { id: 'c-sys', label: '/systems', hint: 'Jump to Systems & Hardware', icon: 'server', run: () => { void goto(link('/systems')); assistantStore.closeDrawer(); } },
    { id: 'c-fin', label: '/finance', hint: 'Jump to Finance & Ledger', icon: 'wallet', run: () => { void goto(link('/finance')); assistantStore.closeDrawer(); } },
    { id: 'c-int', label: '/interior', hint: 'Jump to 3D RoomPlan Interior', icon: 'layout', run: () => { void goto(link('/interior')); assistantStore.closeDrawer(); } },
    { id: 'c-scout', label: '/scout', hint: 'Jump to Scouting Opportunities', icon: 'compass', run: () => { void goto(link('/scout')); assistantStore.closeDrawer(); } },
    { id: 'c-theme', label: '> theme', hint: 'Toggle Dark / Light theme', icon: 'sun', run: () => { document.documentElement.classList.toggle('dark'); } },
    { id: 'c-clear', label: '> clear', hint: 'Clear chat history', icon: 'refresh', run: () => { assistantStore.clearHistory(); } },
    { id: 'c-doc', label: '> doctor', hint: 'Probe system & capability health', icon: 'cpu', run: () => { void assistantStore.send('system health', routePath); } },
  ];

  let inputPrompt = $state('');
  let selectedCommandIndex = $state(0);
  let chatBodyEl: HTMLElement | undefined = $state();
  let inputEl: HTMLTextAreaElement | undefined = $state();
  let drawerEl: HTMLElement | undefined = $state();

  let touchStartY = 0;
  let touchDeltaY = $state(0);

  // The route without the demo base (SJEL_DEMO_BASE), so /travel is /travel on Pages too.
  const routePath = $derived(
    base && page.url.pathname.startsWith(base) ? page.url.pathname.slice(base.length) || '/' : page.url.pathname,
  );
  const currentContext = $derived(extractRouteContext(routePath));

  const isCommandInput = $derived(inputPrompt.startsWith('/') || inputPrompt.startsWith('>'));
  const filteredCommands = $derived.by(() => {
    if (!isCommandInput) return [];
    const query = inputPrompt.toLowerCase().trim();
    return COMMANDS.filter(
      (cmd) => cmd.label.toLowerCase().includes(query) || cmd.hint.toLowerCase().includes(query)
    );
  });

  async function executeCommand(cmd: CommandItem) {
    inputPrompt = '';
    await cmd.run();
  }

  function handleTouchStart(e: TouchEvent) {
    if (e.touches.length === 1) {
      touchStartY = e.touches[0].clientY;
    }
  }

  function handleTouchMove(e: TouchEvent) {
    if (e.touches.length === 1) {
      const delta = e.touches[0].clientY - touchStartY;
      if (delta > 0) {
        touchDeltaY = delta;
      }
    }
  }

  function handleTouchEnd() {
    if (touchDeltaY > 110) {
      assistantStore.closeDrawer();
    }
    touchDeltaY = 0;
  }

  async function scrollToBottom() {
    await tick();
    if (chatBodyEl) {
      chatBodyEl.scrollTop = chatBodyEl.scrollHeight;
    }
  }

  async function handleSend() {
    const text = inputPrompt.trim();
    if (!text || assistantStore.loading) return;

    inputPrompt = '';
    await assistantStore.send(text, routePath);
    await scrollToBottom();
  }

  function handleKeyDown(event: KeyboardEvent) {
    // Global Cmd+K / Ctrl+K shortcut to toggle drawer
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
      event.preventDefault();
      assistantStore.toggle();
      if (assistantStore.isOpen) {
        tick().then(() => inputEl?.focus());
      }
      return;
    }

    if (event.key === 'Escape' && assistantStore.isOpen) {
      event.preventDefault();
      assistantStore.closeDrawer();
    }
  }

  function handleInputKeyDown(event: KeyboardEvent) {
    if (isCommandInput && filteredCommands.length > 0) {
      if (event.key === 'ArrowDown') {
        event.preventDefault();
        selectedCommandIndex = (selectedCommandIndex + 1) % filteredCommands.length;
        return;
      }
      if (event.key === 'ArrowUp') {
        event.preventDefault();
        selectedCommandIndex = (selectedCommandIndex - 1 + filteredCommands.length) % filteredCommands.length;
        return;
      }
      if (event.key === 'Enter' && !event.shiftKey) {
        event.preventDefault();
        const cmd = filteredCommands[selectedCommandIndex] || filteredCommands[0];
        if (cmd) {
          void executeCommand(cmd);
          return;
        }
      }
    }

    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      void handleSend();
    }
  }

  function sendQuickPrompt(promptText: string) {
    inputPrompt = promptText;
    if (promptText.startsWith('/') || promptText.startsWith('>')) {
      tick().then(() => inputEl?.focus());
      return;
    }
    void handleSend();
  }

  $effect(() => {
    // When messages change, auto-scroll to bottom
    if (assistantStore.messages.length > 0) {
      void scrollToBottom();
    }
  });

  onMount(() => {
    assistantStore.loadFloatingVisibility();
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  });
</script>

<!-- Small edge trigger. The × hides it until the primary Ask control is used. -->
{#if !assistantStore.isOpen && assistantStore.floatingVisible}
  <div class="assistant-edge-tab">
    <button
      class="assistant-edge-open"
      onclick={() => assistantStore.openDrawer()}
      aria-label="Open Sjel Assistant (Cmd+K)"
      title="Sjel Assistant (Cmd+K)"
    >
      <Icon name="sparkles" size={14} />
      <span>Ask</span>
      <kbd>⌘K</kbd>
    </button>
    <button
      class="assistant-edge-hide"
      onclick={(event) => { event.stopPropagation(); assistantStore.hideFloating(); }}
      aria-label="Hide floating Assistant"
      title="Hide floating Assistant"
    >
      ×
    </button>
  </div>
{/if}

<!-- Assistant Drawer Overlay / Sheet -->
{#if assistantStore.isOpen}
  <!-- Scrim / Backdrop -->
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="assistant-scrim" onclick={() => assistantStore.closeDrawer()}></div>

  <aside
    bind:this={drawerEl}
    class="assistant-drawer"
    class:minimized={assistantStore.isMinimized}
    style={touchDeltaY > 0 ? `transform: translateY(${touchDeltaY}px); transition: none;` : ''}
    aria-label="Sjel Assistant"
  >
    <!-- Mobile Drag Handle Bar -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="mobile-drag-bar-wrap"
      ontouchstart={handleTouchStart}
      ontouchmove={handleTouchMove}
      ontouchend={handleTouchEnd}
    >
      <div class="mobile-drag-bar"></div>
    </div>

    <!-- Header -->
    <header class="drawer-header">
      <div class="header-left">
        <div class="assistant-title">
          <Icon name="sparkles" size={16} />
          <span>Sjel Assistant</span>
        </div>
        <div class="context-pill" title={currentContext.contextSummary}>
          <span class="context-dot"></span>
          <span class="context-text">{currentContext.label}</span>
        </div>
      </div>

      <div class="header-actions">
        <button
          class="btn-icon"
          onclick={() => assistantStore.clearHistory()}
          aria-label="Clear conversation"
          title="Clear history"
        >
          <Icon name="refresh" size={14} />
        </button>
        <button
          class="btn-icon"
          onclick={() => assistantStore.toggleMinimize()}
          aria-label={assistantStore.isMinimized ? 'Expand drawer' : 'Minimize drawer'}
          title={assistantStore.isMinimized ? 'Expand' : 'Minimize'}
        >
          <Icon name={assistantStore.isMinimized ? 'plus' : 'square'} size={12} />
        </button>
        <button
          class="btn-icon close-btn"
          onclick={() => assistantStore.closeDrawer()}
          aria-label="Close assistant"
          title="Close (Esc)"
        >
          <Icon name="close" size={16} />
        </button>
      </div>
    </header>

    {#if !assistantStore.isMinimized}
      <!-- Chat Body -->
      <div class="drawer-body" bind:this={chatBodyEl}>
        {#each assistantStore.messages as msg (msg.id)}
          <div class="message-row" class:user={msg.role === 'user'} class:system={msg.role === 'system'}>
            {#if msg.role === 'assistant' && msg.routing}
              <div class="msg-meta-bar">
                <span class="route-tag" title={msg.routing.reason}>{msg.routing.domain}</span>
              </div>
            {/if}

            <div class="bubble">
              <p class="bubble-text">{msg.content}</p>

              <!-- Cards carry capability data only; every write waits for a tap. -->
              {#if msg.cards && msg.cards.length > 0}
                <div class="cards-stream">
                  {#each msg.cards as card}
                    <GenerativeWidgetRenderer
                      {card}
                      onApplyJourney={(c, plan) => assistantStore.pinJourney(c, plan)}
                      onApplyCalendar={(c) => assistantStore.acceptCalendar(c)}
                      onPrompt={(p) => sendQuickPrompt(p)}
                    />
                  {/each}
                </div>
              {/if}
            </div>

            <span class="msg-time mono">
              {new Date(msg.timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
            </span>
          </div>
        {/each}

        {#if assistantStore.loading}
          <div class="loading-row">
            <span class="spinner-dot"></span>
            <span class="spinner-dot"></span>
            <span class="spinner-dot"></span>
            <span class="loading-label">Asking the capabilities…</span>
          </div>
        {/if}
      </div>

      <!-- Quick Suggestion Chips based on Context -->
      <div class="quick-chips">
        <button class="chip-btn chip-command" onclick={() => sendQuickPrompt('/')}>
          <Icon name="search" size={11} />
          <span>/ Commands</span>
        </button>
        {#each currentContext.quickPrompts as qp}
          <button class="chip-btn" onclick={() => sendQuickPrompt(qp)}>
            {qp}
          </button>
        {/each}
      </div>

      <!-- Footer / Input Box -->
      <footer class="drawer-footer">
        {#if filteredCommands.length > 0}
          <div class="command-palette-popover" role="listbox" aria-label="Command suggestions">
            {#each filteredCommands as cmd, i (cmd.id)}
              <button
                type="button"
                class="command-item-btn"
                class:active={i === selectedCommandIndex}
                onclick={() => void executeCommand(cmd)}
                role="option"
                aria-selected={i === selectedCommandIndex}
              >
                <div class="cmd-icon-wrap">
                  <Icon name={cmd.icon as any} size={13} />
                </div>
                <div class="cmd-meta">
                  <span class="cmd-label mono">{cmd.label}</span>
                  <span class="cmd-hint">{cmd.hint}</span>
                </div>
              </button>
            {/each}
          </div>
        {/if}

        <div class="input-container">
          <textarea
            bind:this={inputEl}
            bind:value={inputPrompt}
            onkeydown={handleInputKeyDown}
            placeholder={`Ask Sjel Assistant or type / for commands... (Enter to send)`}
            rows={1}
            aria-label="Message Sjel Assistant"
          ></textarea>

          <button
            class="send-btn"
            disabled={!inputPrompt.trim() || assistantStore.loading}
            onclick={handleSend}
            aria-label="Send message"
          >
            <Icon name="send" size={15} />
          </button>
        </div>
      </footer>
    {/if}
  </aside>
{/if}

<style>
  .assistant-edge-tab {
    position: fixed;
    top: 50%;
    right: 0;
    z-index: 60;
    display: flex;
    align-items: stretch;
    transform: translateY(-50%);
    border: 1px solid var(--card-border);
    border-right: 0;
    border-radius: var(--radius-md) 0 0 var(--radius-md);
    background: var(--card-bg);
    box-shadow: -4px 4px 16px rgb(0 0 0 / 18%);
    overflow: hidden;
  }

  .assistant-edge-open,
  .assistant-edge-hide {
    border: 0;
    color: var(--text-primary);
    background: transparent;
    cursor: pointer;
  }

  .assistant-edge-open {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.55rem 0.65rem;
    color: var(--primary);
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .assistant-edge-open:hover,
  .assistant-edge-hide:hover {
    background: var(--primary-soft);
  }

  .assistant-edge-open kbd {
    padding: 0.05rem 0.25rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    font: inherit;
    font-size: var(--text-2xs);
  }

  .assistant-edge-hide {
    padding: 0 0.45rem;
    border-left: 1px solid var(--card-border);
    color: var(--text-tertiary);
    font-size: 1rem;
  }

  /* Scrim */
  .assistant-scrim {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 45%);
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
    z-index: 80;
  }

  /* Drawer / Bottom Sheet */
  .assistant-drawer {
    position: fixed;
    top: 0;
    right: 0;
    width: clamp(380px, 34vw, 560px);
    height: 100vh;
    background: var(--page-bg);
    border-left: 1px solid var(--card-border);
    box-shadow: -8px 0 36px rgb(0 0 0 / 35%);
    z-index: 90;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .assistant-drawer.minimized {
    height: auto;
    top: auto;
    bottom: 0;
    border-top: 1px solid var(--card-border);
    border-radius: var(--radius-lg) var(--radius-lg) 0 0;
  }

  /* Header */
  .drawer-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.85rem 1.15rem;
    background: var(--header-bg);
    backdrop-filter: var(--glass-blur);
    -webkit-backdrop-filter: var(--glass-blur);
    border-bottom: 1px solid var(--header-border);
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .assistant-title {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    font-size: var(--text-sm);
    font-weight: 700;
    color: var(--primary);
  }

  .context-pill {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    background: var(--primary-soft);
    border: 1px solid color-mix(in srgb, var(--primary) 25%, transparent);
    padding: 0.15rem 0.55rem;
    border-radius: var(--radius-full, 9999px);
    font-size: var(--text-2xs);
    font-weight: 550;
    color: var(--text-secondary);
    cursor: default;
  }

  .context-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--primary);
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 0.35rem;
  }

  .btn-icon {
    background: transparent;
    border: none;
    color: var(--text-tertiary);
    padding: 0.4rem;
    border-radius: var(--radius-sm);
    cursor: pointer;
    display: grid;
    place-items: center;
    transition:
      color 0.15s ease,
      background-color 0.15s ease;
  }

  .btn-icon:hover {
    color: var(--text-primary);
    background-color: var(--surface);
  }

  .close-btn:hover {
    color: var(--danger);
    background-color: var(--danger-soft);
  }

  /* Drawer Body */
  .drawer-body {
    flex: 1;
    overflow-y: auto;
    padding: 1.15rem;
    display: flex;
    flex-direction: column;
    gap: 1.15rem;
  }

  .message-row {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    max-width: 92%;
  }

  .message-row.user {
    align-self: flex-end;
    align-items: flex-end;
  }

  .message-row.system {
    align-self: center;
    align-items: center;
    max-width: 100%;
  }

  .msg-meta-bar {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin-bottom: 0.25rem;
    font-size: var(--text-2xs);
  }

  .route-tag {
    color: var(--text-tertiary);
    padding: 0.05rem 0.4rem;
    border-radius: var(--radius-sm);
    border: 1px solid var(--card-border);
    font-weight: 500;
  }

  .bubble {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-lg) var(--radius-lg) var(--radius-lg) var(--radius-sm);
    padding: 0.75rem 1rem;
    color: var(--text-primary);
    font-size: var(--text-xs);
    line-height: var(--leading-normal);
    word-break: break-word;
    box-shadow: var(--card-shadow);
  }

  .message-row.user .bubble {
    background: color-mix(in srgb, var(--primary) 12%, var(--card-bg));
    border-color: color-mix(in srgb, var(--primary) 28%, var(--card-border));
    border-radius: var(--radius-lg) var(--radius-lg) var(--radius-sm) var(--radius-lg);
    color: var(--text-primary);
  }

  .message-row.system .bubble {
    background: transparent;
    border: 1px dashed var(--card-border);
    border-radius: var(--radius-md);
    color: var(--text-tertiary);
    font-size: var(--text-xs);
    text-align: center;
  }

  .bubble-text {
    margin: 0;
    white-space: pre-wrap;
  }

  .cards-stream {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .msg-time {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
    margin-top: 0.25rem;
  }

  .loading-row {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    font-size: var(--text-xs);
    color: var(--text-tertiary);
    padding: 0.5rem 0;
  }

  .spinner-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--primary);
    animation: bounce 1.2s infinite ease-in-out;
  }

  .spinner-dot:nth-child(2) {
    animation-delay: 0.2s;
  }

  .spinner-dot:nth-child(3) {
    animation-delay: 0.4s;
  }

  @keyframes bounce {
    0%, 80%, 100% {
      transform: scale(0);
      opacity: 0.3;
    }
    40% {
      transform: scale(1);
      opacity: 1;
    }
  }

  /* Quick Suggestion Chips */
  .quick-chips {
    display: flex;
    gap: var(--space-2);
    overflow-x: auto;
    padding: 0.6rem 1.15rem;
    background: var(--page-bg);
    border-top: 1px solid var(--card-border);
    scrollbar-width: none;
  }

  .quick-chips::-webkit-scrollbar {
    display: none;
  }

  .chip-btn {
    white-space: nowrap;
    background: var(--surface);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-full, 9999px);
    padding: 0.3rem 0.75rem;
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    transition:
      background-color 0.15s ease,
      border-color 0.15s ease,
      color 0.15s ease;
  }

  .chip-btn:hover {
    border-color: var(--primary);
    color: var(--primary);
    background-color: var(--primary-soft);
  }

  .chip-command {
    background-color: var(--primary-soft);
    color: var(--primary);
    border-color: color-mix(in srgb, var(--primary) 30%, transparent);
    font-weight: 600;
  }

  /* Input Footer */
  .drawer-footer {
    position: relative;
    padding: 0.85rem 1.15rem;
    background: var(--header-bg);
    backdrop-filter: var(--glass-blur);
    -webkit-backdrop-filter: var(--glass-blur);
    border-top: 1px solid var(--header-border);
  }

  .command-palette-popover {
    position: absolute;
    bottom: calc(100% + 0.5rem);
    left: 1.15rem;
    right: 1.15rem;
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    box-shadow: var(--card-shadow-hover);
    max-height: 240px;
    overflow-y: auto;
    padding: 0.35rem;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    z-index: 20;
  }

  .command-item-btn {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4rem 0.6rem;
    border-radius: var(--radius-sm);
    border: none;
    background: transparent;
    cursor: pointer;
    font: inherit;
    text-align: left;
    width: 100%;
    transition: background-color 0.1s ease;
  }

  .command-item-btn.active,
  .command-item-btn:hover {
    background-color: var(--surface);
  }

  .cmd-icon-wrap {
    display: grid;
    place-items: center;
    width: 1.5rem;
    height: 1.5rem;
    border-radius: var(--radius-sm);
    background-color: var(--primary-soft);
    color: var(--primary);
    flex-shrink: 0;
  }

  .cmd-meta {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    flex: 1;
    min-width: 0;
  }

  .cmd-label {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--primary);
  }

  .cmd-hint {
    font-size: var(--text-2xs);
    color: var(--text-secondary);
    margin-left: auto;
  }

  .input-container {
    display: flex;
    align-items: center;
    background: var(--surface);
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    padding: 0.45rem 0.75rem;
    gap: 0.5rem;
    transition: border-color 0.15s ease, box-shadow 0.15s ease;
  }

  .input-container:focus-within {
    border-color: var(--primary);
    box-shadow: 0 0 0 1px var(--primary);
  }

  textarea {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    color: var(--text-primary);
    font-family: inherit;
    font-size: var(--text-xs);
    resize: none;
    line-height: var(--leading-normal);
    max-height: 120px;
  }

  .send-btn {
    background: var(--primary);
    color: var(--text-inverse);
    border: none;
    border-radius: var(--radius-sm);
    padding: 0.35rem 0.55rem;
    cursor: pointer;
    display: grid;
    place-items: center;
    transition: opacity 0.15s ease, transform 0.15s ease, background-color 0.15s ease;
  }

  .send-btn:hover:not(:disabled) {
    background: var(--primary-hover);
    transform: scale(1.04);
  }

  .send-btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  /* Drag handle on mobile */
  .mobile-drag-bar-wrap {
    display: none;
    width: 100%;
    padding: 0.5rem 0 0.25rem;
    cursor: grab;
    touch-action: pan-y;
    justify-content: center;
    align-items: center;
  }

  .mobile-drag-bar {
    width: 38px;
    height: 4px;
    border-radius: 9999px;
    background: var(--card-border);
  }

  /* Responsive Mobile Bottom Sheet */
  @media (width < 48rem) {
    .mobile-drag-bar-wrap {
      display: flex;
    }

    .assistant-drawer {
      top: auto;
      bottom: 0;
      width: 100%;
      height: min(88dvh, calc(100dvh - env(safe-area-inset-top, 24px)));
      border-left: none;
      border-top: 1px solid var(--card-border);
      border-radius: var(--radius-xl) var(--radius-xl) 0 0;
      transition: transform 0.2s cubic-bezier(0.16, 1, 0.3, 1);
      overscroll-behavior: contain;
    }

    .drawer-footer {
      padding-bottom: max(0.85rem, env(safe-area-inset-bottom, 16px));
    }

  }
</style>
