import { BlockOptions, MESSAGE_TYPE, MAX_HEIGHT } from './types';
import { generateBaseCss } from './theme';
import { escapeHtml } from './parser';

/** The script that runs inside the iframe and reports its content height via postMessage. */
export function resizeScript(id: string): string {
  return (
    '<script>(function(){' +
    'var last=-1;' +
    'function send(){var b=document.body;if(!b)return;var r=b.getBoundingClientRect();' +
    'var h=Math.ceil(Math.max(r.height,b.scrollHeight||0));' +
    'if(h===last)return;last=h;' +
    'try{parent.postMessage({type:' + JSON.stringify(MESSAGE_TYPE) + ',id:' + JSON.stringify(id) + ',height:h},"*");}catch(e){}}' +
    'if(window.ResizeObserver){new ResizeObserver(send).observe(document.documentElement);' +
    'new ResizeObserver(send).observe(document.body);}' +
    'window.addEventListener("load",send);' +
    'var mo=new MutationObserver(send);mo.observe(document.body,{childList:true,subtree:true,attributes:true});' +
    'send();setTimeout(send,50);setTimeout(send,400);' +
    '})();</script>'
  );
}

/** Build the complete srcdoc HTML for one iframe. */
export function buildSrcdoc(html: string, themeCss: string, id: string, opts?: BlockOptions): string {
  const isDark = /color-scheme:\s*dark/.test(themeCss);
  const baseCss = generateBaseCss(themeCss, isDark);
  const meta = '<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">';
  const script = resizeScript(id);
  const theme = isDark ? 'dark' : 'light';

  if (/<html[\s>]/i.test(html)) {
    let doc = html;
    // Synchronize data-theme on <body> for components like Anatomy (.iso)
    if (/<body[^>]*\bdata-theme=/i.test(doc)) {
      doc = doc.replace(/(<body[^>]*\bdata-theme=["'])[^"']*(["'])/i, `$1${theme}$2`);
    } else if (/<body/i.test(doc)) {
      doc = doc.replace(/<body([^>]*)>/i, `<body$1 data-theme="${theme}">`);
    }
    doc = /<head[^>]*>/i.test(doc)
      ? doc.replace(/<head[^>]*>/i, (m) => m + meta + baseCss)
      : doc.replace(/<html[^>]*>/i, (m) => m + '<head>' + meta + baseCss + '</head>');
    doc = /<\/body>/i.test(doc)
      ? doc.replace(/<\/body>/i, script + '</body>')
      : doc + script;
    return doc;
  }

  const title = opts && opts.title ? `<title>${escapeHtml(String(opts.title))}</title>` : '';
  return `<!DOCTYPE html><html lang="en" data-theme="${theme}"><head>${meta}${title}${baseCss}</head><body data-theme="${theme}">${html}${script}</body></html>`;
}
