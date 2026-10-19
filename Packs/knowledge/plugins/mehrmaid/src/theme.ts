export interface MermaidThemeConfig {
  theme: string;
  themeVariables: Record<string, string>;
  securityLevel: string;
  startOnLoad: boolean;
  flowchart: {
    htmlLabels: boolean;
  };
}

export function getThemeConfig(isDark: boolean): MermaidThemeConfig {
  const cs = getComputedStyle(document.body);
  const text = cs.getPropertyValue('--text-normal').trim() || (isDark ? '#dcddde' : '#222222');
  const muted = cs.getPropertyValue('--text-muted').trim() || (isDark ? '#888888' : '#666666');
  const bg = cs.getPropertyValue('--background-primary').trim() || (isDark ? '#141414' : '#ffffff');
  const bgAlt = cs.getPropertyValue('--background-secondary').trim() || (isDark ? '#1e1e1e' : '#f5f5f5');
  const border = cs.getPropertyValue('--background-modifier-border').trim() || (isDark ? '#333333' : '#e0e0e0');

  return {
    theme: isDark ? 'dark' : 'neutral',
    themeVariables: {
      primaryColor: bgAlt,
      primaryTextColor: text,
      primaryBorderColor: border,
      lineColor: muted,
      secondaryColor: bgAlt,
      tertiaryColor: bg,
      clusterBkg: bg,
      clusterBorder: border,
      nodeBorder: border,
      edgeLabelBackground: bg,
      fontFamily: cs.getPropertyValue('--font-text').trim() || 'inherit',
      fontSize: '13px'
    },
    securityLevel: 'loose',
    startOnLoad: true,
    flowchart: {
      htmlLabels: true
    }
  };
}
