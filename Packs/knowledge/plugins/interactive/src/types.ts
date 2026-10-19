export interface BlockOptions {
  file?: string;
  height?: number | string;
  title?: string;
  [key: string]: string | number | undefined;
}

export interface ParsedBlock {
  opts: BlockOptions;
  content: string;
}

export const THEME_VARS = [
  '--background-primary', '--background-primary-alt', '--background-secondary',
  '--background-secondary-alt', '--background-modifier-border', '--background-modifier-hover',
  '--background-modifier-border-hover', '--text-normal', '--text-muted', '--text-faint',
  '--text-accent', '--text-accent-hover', '--text-on-accent', '--text-error', '--text-warning',
  '--text-success', '--text-highlight-bg', '--interactive-accent', '--interactive-accent-hover',
  '--interactive-normal', '--interactive-hover', '--font-text', '--font-monospace',
  '--font-interface', '--font-text-size', '--font-smaller', '--font-small', '--font-ui-small',
  '--radius-s', '--radius-m', '--radius-l', '--line-height-normal', '--line-height-tight',
  '--color-red', '--color-orange', '--color-yellow', '--color-green', '--color-cyan',
  '--color-blue', '--color-purple', '--color-pink', '--color-red-rgb', '--color-orange-rgb',
  '--color-yellow-rgb', '--color-green-rgb', '--color-cyan-rgb', '--color-blue-rgb',
  '--color-purple-rgb', '--color-pink-rgb', '--color-base-00', '--color-base-10',
  '--color-base-20', '--color-base-30', '--color-base-40', '--color-base-50', '--color-base-60',
  '--color-base-70', '--color-base-100', '--divider-color', '--table-border-color',
] as const;

export const MESSAGE_TYPE = 'html-embed:size';
export const MAX_HEIGHT = 4000;
