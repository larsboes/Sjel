import { App, FuzzySuggestModal } from 'obsidian';
import { modelKey, type PiModel } from './rpc';

/** The empty choice: keep pi's own default model. */
const DEFAULT = { provider: '', id: '', name: 'pi default' } as PiModel;

const fmt = (n?: number) => (!n ? '' : n >= 1e6 ? `${(n / 1e6).toFixed(n % 1e6 ? 1 : 0)}M` : `${Math.round(n / 1e3)}k`);

/**
 * A fuzzy model picker over everything `get_available_models` lists: several hundred entries, too
 * many for a dropdown. `onPick` receives `provider/id`, or '' for pi's default.
 */
export class ModelPicker extends FuzzySuggestModal<PiModel> {
  constructor(
    app: App,
    private readonly models: PiModel[],
    private readonly onPick: (key: string) => void,
    private readonly allowDefault: boolean,
  ) {
    super(app);
    this.setPlaceholder('Type a model or provider, e.g. "deepseek flash"');
  }

  getItems(): PiModel[] {
    return this.allowDefault ? [DEFAULT, ...this.models] : this.models;
  }

  getItemText(m: PiModel): string {
    return m === DEFAULT ? 'pi default' : `${modelKey(m)} ${m.name ?? ''}`;
  }

  renderSuggestion(match: { item: PiModel }, el: HTMLElement) {
    const m = match.item;
    el.addClass('pi-learn-suggest');
    el.createDiv({ cls: 'pi-learn-suggest-name', text: m === DEFAULT ? 'pi default' : modelKey(m) });
    if (m !== DEFAULT) {
      const meta = [m.reasoning ? 'reasoning' : '', fmt(m.contextWindow) && `${fmt(m.contextWindow)} context`]
        .filter(Boolean)
        .join(' · ');
      if (meta) el.createDiv({ cls: 'pi-learn-suggest-meta', text: meta });
    }
  }

  onChooseItem(m: PiModel) {
    this.onPick(m === DEFAULT ? '' : modelKey(m));
  }
}
