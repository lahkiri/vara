// Minimal, safe markdown renderer: escape first, then transform.
// Citations [n] become styled chips. No dangerously external HTML.

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export function mdToHtml(md: string): string {
  const lines = md.replace(/\r\n/g, "\n").split("\n");
  const out: string[] = [];
  let inList: "ul" | "ol" | null = null;
  let inQuote = false;

  const closeList = () => {
    if (inList) {
      out.push(`</${inList}>`);
      inList = null;
    }
  };
  const closeQuote = () => {
    if (inQuote) {
      out.push("</blockquote>");
      inQuote = false;
    }
  };

  const inline = (raw: string): string => {
    let s = escapeHtml(raw);
    // links [text](url)
    s = s.replace(/\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>');
    // bare urls
    s = s.replace(/(^|[\s(])((?:https?:\/\/)[^\s<)"'،؛]+)/g, '$1<a href="$2" target="_blank" rel="noopener noreferrer">$2</a>');
    // bold / italic / code
    s = s.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
    s = s.replace(/(^|[^*])\*([^*\n]+)\*/g, "$1<em>$2</em>");
    s = s.replace(/`([^`]+)`/g, "<code>$1</code>");
    // citations [12] — after links so [t](u) is untouched
    s = s.replace(/\[(\d{1,2})\]/g, '<span class="md-cite">$1</span>');
    return s;
  };

  for (const line of lines) {
    const l = line.trimEnd();
    if (/^\s*$/.test(l)) {
      closeList();
      closeQuote();
      continue;
    }
    const h = l.match(/^(#{1,3})\s+(.*)$/);
    if (h) {
      closeList();
      closeQuote();
      const level = h[1].length;
      out.push(`<h${level}>${inline(h[2])}</h${level}>`);
      continue;
    }
    if (/^\s*(---+|\*\*\*+)\s*$/.test(l)) {
      closeList();
      closeQuote();
      out.push("<hr/>");
      continue;
    }
    const ul = l.match(/^\s*[-*•]\s+(.*)$/);
    if (ul) {
      closeQuote();
      if (inList !== "ul") {
        closeList();
        out.push("<ul>");
        inList = "ul";
      }
      out.push(`<li>${inline(ul[1])}</li>`);
      continue;
    }
    const ol = l.match(/^\s*\d{1,2}[.)]\s+(.*)$/);
    if (ol) {
      closeQuote();
      if (inList !== "ol") {
        closeList();
        out.push("<ol>");
        inList = "ol";
      }
      out.push(`<li>${inline(ol[1])}</li>`);
      continue;
    }
    const q = l.match(/^\s*>\s?(.*)$/);
    if (q) {
      closeList();
      if (!inQuote) {
        out.push("<blockquote>");
        inQuote = true;
      }
      out.push(`<p>${inline(q[1])}</p>`);
      continue;
    }
    closeList();
    closeQuote();
    out.push(`<p>${inline(l)}</p>`);
  }
  closeList();
  closeQuote();
  return out.join("\n");
}
