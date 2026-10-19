import { THEME_VARS } from './types';

/** Collect the current theme variables from computed styles on body. */
export function collectThemeVars(computed: CSSStyleDeclaration): string {
  const out: string[] = [];
  for (const name of THEME_VARS) {
    const v = computed.getPropertyValue(name);
    if (v && v.trim()) out.push(`${name}:${v.trim()};`);
  }
  const scheme = computed.getPropertyValue('color-scheme');
  if (scheme && scheme.trim()) out.push(`color-scheme:${scheme.trim()};`);
  return out.join('');
}

/** Generates the full CSS string injected into sandboxed iframes. */
export function generateBaseCss(themeCss: string, isDark: boolean): string {
  return (
    '<style id="html-embed-theme">' +
    ':root{' +
    themeCss +
    '--anatomy-page:var(--background-primary);--anatomy-card:var(--background-secondary);' +
    '--anatomy-text:var(--text-normal);--anatomy-strong:var(--text-normal);' +
    '--anatomy-muted:var(--text-muted);--anatomy-mono:var(--font-monospace);}' +
    'html,body{margin:0;padding:0;background:transparent;' +
    'color:var(--text-normal,' + (isDark ? '#dcddde' : '#222') + ');' +
    'font-family:var(--font-text,-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif);' +
    'font-size:var(--font-text-size,16px);line-height:var(--line-height-normal,1.5);}' +
    '*,*::before,*::after{box-sizing:border-box}' +
    'html{height:auto}body{height:auto;overflow:hidden;padding:2px 1px}' +
    'button{font:inherit;font-size:13px;padding:6px 10px;border-radius:var(--radius-s,6px);' +
    'border:1px solid var(--background-modifier-border,#ccc);' +
    'background:var(--interactive-normal,#f2f2f2);color:var(--text-normal,#222);cursor:pointer}' +
    'button:hover{background:var(--interactive-hover,#e6e6e6)}' +
    'input[type=range]{accent-color:var(--interactive-accent,#2d96e6)}' +
    'code,.mono{font-family:var(--font-monospace,ui-monospace,Menlo,Consolas,monospace)}' +
    '</style>'
  );
}
