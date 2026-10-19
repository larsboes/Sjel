import type { Component } from 'svelte';
import MieleW961Filter from './MieleW961Filter.svelte';
import MieleW961Machine from './MieleW961Machine.svelte';

export interface FigureEntry {
  title: string;
  component: Component<{ title?: string }>;
}

export const figures: Record<string, FigureEntry> = {
  'miele-w961-filter': {
    title: 'W 961 mains filter',
    component: MieleW961Filter,
  },
  'miele-w961-machine': {
    title: 'W 961 whole machine system',
    component: MieleW961Machine,
  },
};

