import type { TripPlan } from '$lib/api';
import { assistantEngine } from './assistant-engine';
import { extractRouteContext } from './context';
import { executeCalendarAccept, executeJourneyPin } from './executor';
import type { ActionResult, AssistantMessage, CalendarSlotCardData, JourneyOptionCardData } from './types';

/**
 * Drawer state that survives page navigation (PRD §8.1, Q111). Every card action reports
 * its outcome into the conversation, failure included: a write that did not happen is
 * said as plainly as one that did.
 */
class AssistantStore {
  isOpen = $state(false);
  isMinimized = $state(false);
  loading = $state(false);
  messages = $state<AssistantMessage[]>([this.welcome()]);

  private welcome(): AssistantMessage {
    return {
      id: 'welcome',
      role: 'assistant',
      content:
        'Ask for a train connection, a free calendar block or your interior layouts. Answers come from the capabilities on this machine; when one does not answer, the reply names it.',
      timestamp: new Date().toISOString(),
    };
  }

  toggle(): void {
    this.isOpen = !this.isOpen;
    if (this.isOpen) this.isMinimized = false;
  }

  openDrawer(): void {
    this.isOpen = true;
    this.isMinimized = false;
  }

  closeDrawer(): void {
    this.isOpen = false;
  }

  toggleMinimize(): void {
    this.isMinimized = !this.isMinimized;
  }

  clearHistory(): void {
    this.messages = [this.welcome()];
  }

  private note(result: ActionResult): void {
    this.messages.push({
      id: `sys-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
      role: 'system',
      content: result.ok ? result.message : `Failed: ${result.message}`,
      timestamp: new Date().toISOString(),
    });
  }

  async send(rawPrompt: string, pathname: string): Promise<void> {
    const text = rawPrompt.trim();
    if (!text || this.loading) return;

    this.messages.push({
      id: `usr-${Date.now()}`,
      role: 'user',
      content: text,
      timestamp: new Date().toISOString(),
    });
    this.loading = true;
    try {
      this.messages.push(await assistantEngine.processQuery(text, extractRouteContext(pathname)));
    } catch (err) {
      this.messages.push({
        id: `err-${Date.now()}`,
        role: 'system',
        content: `The drawer failed: ${err instanceof Error ? err.message : String(err)}`,
        timestamp: new Date().toISOString(),
      });
    } finally {
      this.loading = false;
    }
  }

  async pinJourney(card: JourneyOptionCardData, plan: TripPlan | null): Promise<ActionResult> {
    const result = await executeJourneyPin(card, plan);
    this.note(result);
    return result;
  }

  async acceptCalendar(card: CalendarSlotCardData): Promise<ActionResult> {
    const result = await executeCalendarAccept(card);
    this.note(result);
    return result;
  }
}

export const assistantStore = new AssistantStore();
