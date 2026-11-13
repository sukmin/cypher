// 문서 항목: Markdown·글자 파일을 읽기 좋은 모습으로 보여 준다(고치지는 않는다).
//
// Claude가 만든 계획·보고서(.md)를 작업영역에서 서식 있는 모습으로 읽고, 파일이 바뀌면 다시 그린다.
// 파일 내용은 신뢰할 수 없는 입력으로 다룬다: 글자는 모두 이스케이프하고, 링크는 주소를 속성(data-href)에만
// 적어 두었다가 눌렀을 때 앱의 링크 열기(link_open)로 넘긴다. 본창이 다른 곳으로 이동하는 일은 없다.

let api = null; // { invoke, make, setStatus, showMenu, baseName, openMemo(path), toClaude(item, path, ev) }

export function initReader(given) {
  api = given;
}

// ───────────────────────── Markdown → HTML ─────────────────────────
// DOM 없이 도는 순수 함수들. 만들어 내는 태그는 여기서 직접 적은 것뿐이고, 파일에서 온 글자는 전부 esc()를 거친다.

const ESCAPES = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };
const esc = (text) => String(text).replace(/[&<>"']/g, (c) => ESCAPES[c]);

/** 서식으로 그릴 수 있는 가장 긴 글(글자 수). 이보다 길면 글자 그대로 보여 준다. */
const MAX_MARKDOWN = 1_500_000;
/** 한 문단이 이보다 길면 강조·링크를 찾지 않는다(병적인 입력에서 화면이 멈추지 않게). */
const MAX_INLINE = 20_000;
/** 한 문단 안에 공백이 이만큼 이어져 있으면 강조·링크를 찾지 않는다(링크 괄호 안의 공백을 되짚느라 멈추지 않게). */
const MAX_BLANK_RUN = 200;
/** 한 줄이 이보다 길면 서식으로 그리지 않고 글자 그대로 보여 준다(줄 길이에 따라 느려지는 곳이 남아 있어도 화면이 멈추지 않게). */
const MAX_LINE = 100_000;
const MAX_DEPTH = 30;

/** 정규식의 `\s`에 맞는 글자인지(글자 번호로 본다). */
const isBlankCode = (c) => c === 32 || (c >= 9 && c <= 13) || c === 0xA0 || c === 0x1680 || (c >= 0x2000 && c <= 0x200A)
  || c === 0x2028 || c === 0x2029 || c === 0x202F || c === 0x205F || c === 0x3000 || c === 0xFEFF;

/** 공백이 `limit`개 이상 이어진 곳이 있는지. */
function hasBlankRun(text, limit) {
  let run = 0;
  for (let i = 0; i < text.length; i++) {
    if (!isBlankCode(text.charCodeAt(i))) run = 0;
    else if (++run >= limit) return true;
  }
  return false;
}

const LINE_SEPARATOR = String.fromCharCode(0x2028), PARAGRAPH_SEPARATOR = String.fromCharCode(0x2029);
/** 줄 구분 문자(정규식의 `.`이 건너지 못하는 글자)가 들어 있는지. */
const hasLineBreakChar = (text) => text.includes(LINE_SEPARATOR) || text.includes(PARAGRAPH_SEPARATOR);

/** `limit`자보다 긴 줄이 있는지. */
export function hasLongLine(text, limit = MAX_LINE) {
  let start = 0;
  for (;;) {
    const end = text.indexOf('\n', start);
    if ((end < 0 ? text.length : end) - start > limit) return true;
    if (end < 0) return false;
    start = end + 1;
  }
}

// 이미 만든 HTML 조각을 잠시 치워 두는 자리 표시: 사용자 영역 문자만 쓴다(입력에서는 미리 지운다).
const HOLD_OPEN = '\uE000', HOLD_CLOSE = '\uE001';
const HOLD = /\uE000([\uE010-\uE019]+)\uE001/g;
const holdKey = (n) => HOLD_OPEN + String(n).replace(/\d/g, (d) => String.fromCharCode(0xE010 + Number(d))) + HOLD_CLOSE;
const holdIndex = (code) => Number([...code].map((c) => c.charCodeAt(0) - 0xE010).join(''));

const destSource = (name) => `(?<${name}>${/<[^>\n]*>|(?:[^\s()\\]|\\.|\([^\s()]*\))*/.source})`;
const TITLE_SOURCE = /(?:\s+(?:"[^"]*"|'[^']*'|\([^()]*\)))?/.source;
/** 문단 안에서 먼저 골라낼 것들(왼쪽부터 먼저 걸리는 것이 이긴다): 이스케이프, 코드, 그림, 링크, 참조 링크, 주소. */
const INLINE = new RegExp([
  /\\(?<esc>[!-\/:-@\[-`{-~])/.source,
  /(?<!`)(?<ticks>`+)(?!`)(?<code>[\s\S]*?[^`])\k<ticks>(?!`)/.source,
  `!\\[(?<ialt>(?:[^\\[\\]\\\\]|\\\\.)*)\\]\\(\\s*${destSource('idest')}${TITLE_SOURCE}\\s*\\)`,
  `\\[(?<ltext>(?:[^\\[\\]\\\\]|\\\\.|\\[[^\\[\\]]*\\])*)\\]\\(\\s*${destSource('ldest')}${TITLE_SOURCE}\\s*\\)`,
  /\[(?<rtext>(?:[^\[\]\\]|\\.)+)\]\[(?<rname>[^\[\]]*)\]/.source,
  /<(?<auto>https?:\/\/[^\s<>]+)>/.source,
  /(?<![A-Za-z0-9_\/:.@-])(?<bare>https?:\/\/[^\s<>\uE000-\uE01F]+)/.source,
].join('|'), 'g');

/**
 * 링크로 만들어도 되는 주소인지 보고 다듬는다. 안 되면 빈 글.
 * 웹 주소(http, https), 문서 안의 제목(#…), 파일 경로만 받는다. `javascript:` 같은 다른 방식과 네트워크 경로는 받지 않는다.
 */
export function safeTarget(raw) {
  const dest = String(raw || '').trim();
  if (!dest || /[\u0000-\u001f\u007f\uE000-\uE01F]/.test(dest)) return '';
  if (dest.startsWith('#')) return dest;
  if (/^https?:\/\//i.test(dest)) return /\s/.test(dest) ? '' : dest;
  // 두 글자 이상의 방식(javascript:, data:, file:, mailto: …)은 열지 않는다. 한 글자는 드라이브(C:\)다.
  if (/^[a-z][a-z0-9+.-]+:/i.test(dest)) return '';
  if (/^[\\/]{2}/.test(dest)) return ''; // 네트워크 경로
  return dest;
}

/**
 * Markdown에 적힌 링크 주소를 다듬어 검사한다: 꺾쇠를 벗기고, 괄호·꺾쇠·공백 앞의 역슬래시를 뗀다.
 * (다른 부호 앞의 역슬래시는 그대로 둔다. Windows 경로의 구분자일 수 있다.)
 */
function cleanDest(raw) {
  let dest = String(raw || '').trim();
  if (dest.startsWith('<') && dest.endsWith('>')) dest = dest.slice(1, -1).trim();
  return safeTarget(dest.replace(/\\([()[\]<> ])/g, '$1'));
}

/** 글 속에 그냥 적힌 주소의 끝에 붙어 있으면 주소에서 빼는 문장 부호. */
const URL_TAIL = '.,;:!?\'"*_~';

const anchor = (target, inner, extra = '') =>
  `<a class="reader-link${extra}" data-href="${esc(target)}" title="${esc(target)}">${inner}</a>`;

/** 문단 안의 서식(굵게·기울임·취소선·코드·링크)을 HTML로 바꾼다. `nested`이면(링크 글 안) 링크를 다시 만들지 않는다. */
function inline(source, ctx, nested = false) {
  if (source.length > MAX_INLINE || hasBlankRun(source, MAX_BLANK_RUN)) return esc(source);
  const held = [];
  const hold = (html) => { held.push(html); return holdKey(held.length - 1); };
  const link = (dest, inner, extra) => {
    const target = nested ? '' : cleanDest(dest);
    return hold(target ? anchor(target, inner, extra) : inner);
  };

  let text = source.replace(INLINE, (match, ...rest) => {
    const g = rest[rest.length - 1];
    if (g.esc != null) return hold(esc(g.esc));
    if (g.ticks != null) {
      let code = g.code.replace(/\n/g, ' ');
      if (code.length > 2 && code.startsWith(' ') && code.endsWith(' ') && code.trim()) code = code.slice(1, -1);
      return hold(`<code>${esc(code)}</code>`);
    }
    if (g.ialt != null) {
      // 그림은 불러오지 않는다(본창은 바깥 그림을 막는다). 이름만 보여 주고, 누르면 그 파일·주소를 연다.
      const name = `🖼 ${esc(g.ialt.replace(/\\(.)/g, '$1') || '그림')}`;
      const target = nested ? '' : cleanDest(g.idest);
      return hold(target ? anchor(target, name, ' reader-image') : `<span class="reader-image">${name}</span>`);
    }
    if (g.ltext != null) return link(g.ldest, inline(g.ltext, ctx, true));
    if (g.rtext != null) {
      const dest = ctx.refs.get((g.rname || g.rtext).trim().toLowerCase());
      return dest ? link(dest, inline(g.rtext, ctx, true)) : match;
    }
    if (g.auto != null) return link(g.auto, esc(g.auto));
    // 글 속에 그냥 적힌 주소: 끝에 붙은 문장 부호와 짝이 맞지 않는 닫는 괄호는 주소에서 뺀다.
    // (끝에서부터 한 번만 훑는다. 부호가 아주 길게 붙어 있어도 느려지지 않는다.)
    const bare = g.bare;
    let end = bare.length;
    const opens = bare.split('(').length - 1;
    let closes = bare.split(')').length - 1;
    for (; end > 0; end--) {
      const c = bare[end - 1];
      if (URL_TAIL.includes(c)) continue;
      if (c === ')' && closes > opens) { closes--; continue; }
      break;
    }
    const url = bare.slice(0, end);
    return link(url, esc(url)) + bare.slice(end);
  });

  text = esc(text)
    // 자주 쓰는 문자 표기(&nbsp; 등)와 줄 바꿈 표시(<br>)는 뜻대로 보여 준다. 글자로만 바뀌므로 안전하다.
    .replace(/&amp;(nbsp|amp|lt|gt|quot|copy|reg|trade|larr|rarr|uarr|darr|harr|mdash|ndash|hellip|middot|times|check|#\d{1,6}|#x[0-9a-fA-F]{1,5});/g, '&$1;')
    .replace(/&lt;br\s*\/?&gt;/gi, '<br>')
    .replace(/\*\*\*(?=\S)([\s\S]*?\S)\*\*\*/g, '<strong><em>$1</em></strong>')
    .replace(/\*\*(?=\S)([\s\S]*?\S)\*\*/g, '<strong>$1</strong>')
    .replace(/(?<![\p{L}\p{N}_])__(?=\S)([\s\S]*?\S)__(?![\p{L}\p{N}_])/gu, '<strong>$1</strong>')
    .replace(/\*(?=[^\s*])([\s\S]*?[^\s*])\*/g, '<em>$1</em>')
    .replace(/(?<![\p{L}\p{N}_])_(?=[^\s_])([\s\S]*?[^\s_])_(?![\p{L}\p{N}_])/gu, '<em>$1</em>')
    .replace(/~~(?=\S)([\s\S]*?\S)~~/g, '<del>$1</del>')
    // 줄 끝의 공백 둘이나 역슬래시는 줄 바꿈이다.
    .replace(/(?: {2,}|\\)\n/g, '<br>\n');
  return text.replace(HOLD, (_, code) => held[holdIndex(code)] ?? '');
}

const FENCE = /^( {0,3})(`{3,}|~{3,})[ \t]*(.*)$/;
const HEADING_OPEN = /^ {0,3}(#{1,6})(?=[ \t]|$)/;
const RULE = /^ {0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$/;
const QUOTE = /^ {0,3}>/;
const LIST = /^( *)([-*+]|\d{1,9}[.)])( +|$)(.*)$/;
const ALERTS = { note: '참고', tip: '팁', important: '중요', warning: '경고', caution: '주의' };

const isGap = (c) => c === ' ' || c === '\t';

/**
 * 그 줄이 제목(`# 제목`, `## 제목 ##`)이면 { level, text }, 아니면 null.
 * 제목 글 앞뒤의 공백과, 공백을 두고 끝에 붙인 닫는 `#`들은 뗀다. 정규식 하나로 하면 줄 가운데에 공백이 아주 길게
 * 이어진 줄에서 길이의 제곱만큼 느려지므로, 여는 `#`만 정규식으로 보고 나머지는 앞뒤에서 직접 센다.
 */
function headingOf(line) {
  const m = HEADING_OPEN.exec(line);
  if (!m) return null;
  const rest = line.slice(m[0].length);
  if (hasLineBreakChar(rest)) return null; // 줄 구분 문자(U+2028, U+2029)가 든 줄은 제목으로 보지 않는다.
  let start = 0, end = rest.length;
  while (start < end && isGap(rest[start])) start++;
  while (end > start && isGap(rest[end - 1])) end--;
  let cut = end;
  while (cut > start && rest[cut - 1] === '#') cut--;
  if (cut < end && cut > start && isGap(rest[cut - 1])) {
    end = cut;
    while (end > start && isGap(rest[end - 1])) end--;
  }
  return { level: m[1].length, text: rest.slice(start, end) };
}

/**
 * 표의 머리와 본문을 가르는 줄(`|---|:-:|`)인지: 칸마다 `-`가 하나 이상 있고 양 끝에 `:`를 붙일 수 있다.
 * 맨 앞의 `|`는 공백 세 칸까지만 들여쓸 수 있다. (정규식 하나로 하면 공백이 길게 이어진 줄에서 느려져 직접 읽는다.)
 */
function isTableSep(line) {
  const n = line.length;
  let i = 0;
  const gap = () => { while (i < n && isGap(line[i])) i++; };
  gap();
  if (line[i] === '|') {
    if (i > 3 || line.slice(0, i).includes('\t')) return false;
    i++;
  }
  for (;;) {
    gap();
    if (line[i] === ':') i++;
    const dashes = i;
    while (line[i] === '-') i++;
    if (i === dashes) return false;
    if (line[i] === ':') i++;
    gap();
    if (i >= n) return true;
    if (line[i] !== '|') return false;
    i++;
    gap();
    if (i >= n) return true; // 맨 끝의 `|`
  }
}

const indentOf = (line) => line.length - line.trimStart().length;
/** 그 줄이 코드 울타리를 여는지. 열면 { indent, mark, info }. */
function fenceOf(line) {
  const m = FENCE.exec(line);
  if (!m || (m[2][0] === '`' && m[3].includes('`'))) return null;
  return { indent: m[1].length, mark: m[2], info: m[3].trim() };
}
const closesFence = (line, mark) => {
  const m = /^ {0,3}(`{3,}|~{3,})[ \t]*$/.exec(line);
  return !!m && m[1][0] === mark[0] && m[1].length >= mark.length;
};
/** 문단을 끊고 새 덩어리를 시작하는 줄인지(제목, 코드 울타리, 가로줄, 인용). */
const startsBlock = (line) => !!headingOf(line) || !!fenceOf(line) || RULE.test(line) || QUOTE.test(line);

function codeBlock(text, lang) {
  const name = lang ? `<span class="reader-lang">${esc(lang)}</span>` : '';
  return `<div class="reader-code">${name}<button class="reader-copy" data-copy title="이 코드를 복사">복사</button><pre><code>${esc(text)}</code></pre></div>\n`;
}

/** 제목 글로 문서 안 링크(#…)가 가리킬 이름을 만든다: 소문자, 부호는 빼고 공백은 `-`. 같은 이름이 또 나오면 번호를 붙인다. */
function slugOf(html, ctx) {
  const plain = html.replace(/<[^>]*>/g, '').replace(/&(amp|lt|gt|quot|#39);/g, (_, name) => ({ amp: '&', lt: '<', gt: '>', quot: '"', '#39': "'" })[name]);
  const base = plain.trim().toLowerCase().replace(/[^\p{L}\p{N}\s_-]/gu, '').replace(/\s/g, '-');
  const seen = ctx.slugs.get(base) || 0;
  ctx.slugs.set(base, seen + 1);
  return seen ? `${base}-${seen}` : base;
}

function heading(level, raw, ctx) {
  const html = inline(raw.trim(), ctx);
  return `<h${level} class="reader-h" data-slug="${esc(slugOf(html, ctx))}">${html}</h${level}>\n`;
}

/** 표의 한 줄을 칸으로 나눈다(`\|`는 칸 안의 글자). */
function splitRow(line) {
  let row = line.trim();
  if (row.startsWith('|')) row = row.slice(1);
  if (row.endsWith('|') && !row.endsWith('\\|')) row = row.slice(0, -1);
  const cells = [];
  let cell = '';
  for (let i = 0; i < row.length; i++) {
    if (row[i] === '\\' && row[i + 1] === '|') { cell += '|'; i++; }
    else if (row[i] === '|') { cells.push(cell.trim()); cell = ''; }
    else cell += row[i];
  }
  cells.push(cell.trim());
  return cells;
}

function table(lines, start, ctx) {
  const head = splitRow(lines[start]);
  const aligns = splitRow(lines[start + 1]).map((cell) => (
    cell.startsWith(':') && cell.endsWith(':') ? ' class="reader-al-c"' : cell.endsWith(':') ? ' class="reader-al-r"' : ''));
  const row = (cells, tag) => `<tr>${head.map((_, index) => `<${tag}${aligns[index] || ''}>${inline(cells[index] || '', ctx)}</${tag}>`).join('')}</tr>`;
  let i = start + 2, body = '';
  while (i < lines.length && lines[i].trim() && !startsBlock(lines[i])) body += row(splitRow(lines[i++]), 'td');
  return { html: `<div class="reader-table"><table><thead>${row(head, 'th')}</thead><tbody>${body}</tbody></table></div>\n`, next: i };
}

/** `start` 줄에서 시작하는 목록을 읽는다. 하위 목록과 항목 안의 문단·코드는 들여쓰기를 걷어 내고 다시 덩어리로 읽는다. */
function list(lines, start, ctx, depth) {
  const first = LIST.exec(lines[start]);
  const ordered = /\d/.test(first[2]);
  const kindOf = (m) => (/\d/.test(m[2]) ? m[2].slice(-1) : m[2]);
  const kind = kindOf(first);
  const sibling = (line) => {
    const m = LIST.exec(line);
    return m && m[1].length <= 3 && kindOf(m) === kind && !RULE.test(line) ? m : null;
  };
  const items = [];
  let loose = false, i = start;
  while (i < lines.length) {
    const m = sibling(lines[i]);
    if (!m) break;
    const indent = m[1].length, gap = m[3].length;
    const width = indent + m[2].length + (m[4] && gap <= 4 ? gap : 1);
    const body = [(gap > 4 ? ' '.repeat(gap - 1) : '') + m[4]];
    let fence = null; // 항목 안에서 열려 있는 코드 울타리
    i++;
    while (i < lines.length) {
      const line = lines[i];
      if (!line.trim()) {
        // 빈 줄: 그 뒤에 이 항목에 속하는(충분히 들여쓴) 줄이 이어질 때만 항목에 넣는다.
        let j = i + 1;
        while (j < lines.length && !lines[j].trim()) j++;
        if (j >= lines.length || indentOf(lines[j]) < width) break;
        if (!fence) loose = true;
        for (; i < j; i++) body.push('');
        continue;
      }
      if (indentOf(line) >= width) {
        const inner = line.slice(width);
        if (fence) { if (closesFence(inner, fence)) fence = null; } else fence = fenceOf(inner)?.mark || null;
        body.push(inner);
      } else if (!fence && indentOf(line) > indent && LIST.test(line)) {
        body.push(line.trimStart()); // 덜 들여쓴 하위 목록(번호 목록 아래의 두 칸 들여쓴 `-` 등)도 이 항목의 것으로 본다.
      } else if (!fence && body[body.length - 1].trim() && !startsBlock(line) && !LIST.test(line)) {
        body.push(line.trim()); // 들여쓰지 않고 이어 쓴 문단
      } else break;
      i++;
    }
    items.push(body);
    let j = i;
    while (j < lines.length && !lines[j].trim()) j++;
    if (j === i) continue;
    if (j < lines.length && sibling(lines[j])) { loose = true; i = j; continue; } // 항목 사이를 빈 줄로 띄운 목록
    break;
  }
  const rows = items.map((body) => {
    const task = /^\[([ xX])\](?:[ \t]+|$)/.exec(body[0]);
    const content = task ? [body[0].slice(task[0].length), ...body.slice(1)] : body;
    const done = task && task[1] !== ' ';
    const box = task ? `<span class="reader-check${done ? ' on' : ''}">${done ? '✓' : ''}</span>` : '';
    return `<li${task ? ' class="reader-task"' : ''}>${box}${blocks(content, ctx, !loose, depth + 1)}</li>`;
  }).join('\n');
  const number = ordered ? Number(first[2].slice(0, -1)) : 1;
  const open = ordered ? `<ol${number !== 1 ? ` start="${number}"` : ''}>` : '<ul>';
  return { html: `${open}\n${rows}\n</${ordered ? 'ol' : 'ul'}>\n`, next: i };
}

/** 줄들을 덩어리(제목, 문단, 목록, 인용, 코드, 표 …)로 읽어 HTML로 바꾼다. `tight`이면(빽빽한 목록 안) 문단을 <p>로 감싸지 않는다. */
function blocks(lines, ctx, tight, depth = 0, raw = null) {
  if (depth > MAX_DEPTH) return `<p>${esc(lines.join('\n'))}</p>\n`;
  let out = '', para = [], i = 0;
  let openComment = false; // 닫는 표시(-->)가 끝까지 없는 주석 줄을 이미 만났다.
  const flush = () => {
    if (!para.length) return;
    const html = inline(para.map((line) => line.trimStart()).join('\n').trimEnd(), ctx);
    out += tight ? `${html}\n` : `<p>${html}</p>\n`;
    para = [];
  };
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) { flush(); i++; continue; }

    const fence = fenceOf(line);
    if (fence) {
      flush();
      const body = [];
      // 문서 맨 바깥의 들여쓰지 않은 울타리 안은 탭을 바꾸기 전의 줄을 그대로 쓴다(Makefile처럼 탭이 뜻을 가진 코드를 복사해 쓸 수 있게).
      const asWritten = !!raw && !fence.indent;
      for (i++; i < lines.length && !closesFence(lines[i], fence.mark); i++) {
        body.push(asWritten ? raw[i] : lines[i].slice(Math.min(indentOf(lines[i]), fence.indent)));
      }
      i++; // 닫는 줄(닫히지 않았으면 글 끝까지가 코드다)
      out += codeBlock(body.join('\n'), fence.info.split(/\s+/)[0]);
      continue;
    }
    const head = headingOf(line);
    if (head) { flush(); out += heading(head.level, head.text, ctx); i++; continue; }
    // 문단 바로 아래의 === 나 --- 는 그 문단을 제목으로 만든다.
    if (para.length && /^ {0,3}(?:=+|-+)[ \t]*$/.test(line)) {
      const text = para.map((l) => l.trim()).join(' ');
      para = [];
      out += heading(line.trim()[0] === '=' ? 1 : 2, text, ctx);
      i++;
      continue;
    }
    if (RULE.test(line)) { flush(); out += '<hr>\n'; i++; continue; }
    // 주석(<!-- … -->)은 보이지 않게 넘긴다.
    if (!openComment && /^ {0,3}<!--/.test(line)) {
      let end = i;
      while (end < lines.length && !lines[end].includes('-->')) end++;
      if (end < lines.length) { flush(); i = end + 1; continue; } // 닫히지 않은 주석은 글로 둔다.
      openComment = true; // 여기서부터 끝까지 닫는 표시가 없다. 뒤의 주석 줄에서 다시 찾지 않는다.
    }
    if (QUOTE.test(line)) {
      flush();
      const inner = [];
      // `>` 없이 이어 쓴 문단 줄도 인용에 넣는다.
      while (i < lines.length && (QUOTE.test(lines[i])
        || (lines[i].trim() && inner.length && inner[inner.length - 1].trim() && !startsBlock(lines[i]) && !LIST.test(lines[i])))) {
        inner.push(lines[i].replace(/^ {0,3}> ?/, ''));
        i++;
      }
      const alert = /^\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\][ \t]*$/i.exec(inner[0] || '');
      const kind = alert?.[1].toLowerCase();
      out += alert
        ? `<blockquote class="reader-alert reader-alert-${kind}"><p class="reader-alert-name">${ALERTS[kind]}</p>\n${blocks(inner.slice(1), ctx, false, depth + 1)}</blockquote>\n`
        : `<blockquote>\n${blocks(inner, ctx, false, depth + 1)}</blockquote>\n`;
      continue;
    }
    if (line.includes('|') && i + 1 < lines.length && lines[i + 1].includes('-') && isTableSep(lines[i + 1])
      && (lines[i + 1].includes('|') || splitRow(line).length === 1) && splitRow(line).length === splitRow(lines[i + 1]).length) {
      flush();
      const made = table(lines, i, ctx);
      out += made.html;
      i = made.next;
      continue;
    }
    const m = LIST.exec(line);
    // 문단을 끊고 시작하는 목록은 글이 있는 글머리 목록이나 1번으로 시작하는 번호 목록뿐이다.
    if (m && m[1].length <= 3 && (!para.length || (m[4] && (!/\d/.test(m[2]) || /^1[.)]$/.test(m[2]))))) {
      flush();
      const made = list(lines, i, ctx, depth);
      out += made.html;
      i = made.next;
      continue;
    }
    if (!para.length && indentOf(line) >= 4) {
      // 네 칸 들여쓴 줄들은 코드다.
      const body = [];
      while (i < lines.length && (indentOf(lines[i]) >= 4 || !lines[i].trim())) body.push(lines[i++].slice(4));
      while (body.length && !body[body.length - 1].trim()) body.pop();
      out += codeBlock(body.join('\n'), '');
      continue;
    }
    para.push(line);
    i++;
  }
  flush();
  return out;
}

/**
 * `[이름]: 주소` 꼴의 참조 정의를 모은다(코드 울타리 안은 건드리지 않는다).
 * 줄마다 남길 줄인지(참조 정의가 아닌지)를 돌려준다. 참조 정의 줄은 화면에 그리지 않는다.
 */
function takeRefs(lines, ctx) {
  let fence = null;
  return lines.map((line) => {
    if (fence) { if (closesFence(line, fence)) fence = null; return true; }
    fence = fenceOf(line)?.mark || null;
    if (fence) return true;
    const m = /^ {0,3}\[([^\]^][^\]]*)\]:[ \t]*<?(\S+?)>?(?:[ \t]+(?:"[^"]*"|'[^']*'|\([^)]*\)))?[ \t]*$/.exec(line);
    if (!m) return true;
    const name = m[1].trim().toLowerCase();
    if (!ctx.refs.has(name)) ctx.refs.set(name, m[2]);
    return false;
  });
}

/** Markdown 글을 HTML로 바꾼다. 글 속의 HTML은 실행되지 않고 글자 그대로 보인다. */
export function renderMarkdown(text) {
  const ctx = { slugs: new Map(), refs: new Map() };
  const clean = String(text ?? '')
    .replace(/\r\n?/g, '\n')
    .replace(/[\uE000-\uE01F]/g, '');
  // 줄 앞의 탭은 공백 네 칸으로 센다(들여쓰기를 칸 수로 가리기 위해). 탭을 바꾸기 전의 줄(raw)도 나란히 들고 있다가,
  // 들여쓰지 않은 코드 울타리 안의 줄은 그것으로 보여 준다(복사한 코드에 탭이 살아 있게).
  let raw = clean.split('\n');
  let lines = clean.replace(/^[ ]*\t[ \t]*/gm, (lead) => lead.replace(/\t/g, '    ')).split('\n');
  let out = '';
  // 맨 앞의 머리말(--- 사이의 `이름: 값` 줄들)은 코드처럼 그대로 보여 준다.
  if (lines[0]?.trim() === '---' && /^[A-Za-z_][\w-]*[ \t]*:/.test(lines[1] || '')) {
    const end = lines.findIndex((line, index) => index > 0 && /^(?:---|\.\.\.)[ \t]*$/.test(line));
    if (end > 0) {
      out += codeBlock(lines.slice(1, end).join('\n'), '머리말');
      lines = lines.slice(end + 1);
      raw = raw.slice(end + 1);
    }
  }
  const keep = takeRefs(lines, ctx);
  const kept = (list) => list.filter((_, index) => keep[index]);
  return out + blocks(kept(lines), ctx, false, 0, kept(raw));
}

/** 글자 그대로(고정폭) 보여 줄 HTML. */
export const renderPlain = (text) => `<pre class="reader-plain">${esc(String(text ?? '').replace(/\r\n?/g, '\n'))}</pre>`;

// ───────────────────────── 경로 ─────────────────────────

export const isMarkdown = (path) => /\.(md|markdown|mdown|mkd)$/i.test(path);
const dirOf = (path) => path.replace(/[\\/][^\\/]*$/, '');

/** 문서 폴더(dir) 기준의 상대 경로를 전체 경로로 바꾼다(`.`·`..`를 정리한다). */
export function resolvePath(dir, relative) {
  let full = relative.replace(/\//g, '\\');
  if (/^\\(?!\\)/.test(full)) full = `${/^[a-zA-Z]:/.exec(dir)?.[0] || ''}${full}`; // 드라이브 없는 절대 경로는 문서의 드라이브를 따른다.
  else if (!/^[a-zA-Z]:\\/.test(full) && !full.startsWith('\\\\')) full = `${dir.replace(/\//g, '\\').replace(/\\+$/, '')}\\${full}`;
  const head = full.startsWith('\\\\') ? '\\\\' : '';
  const kept = [];
  for (const part of full.slice(head.length).split('\\')) {
    if (!part || part === '.') continue;
    if (part === '..') { if (kept.length > 1) kept.pop(); continue; }
    kept.push(part);
  }
  return head + kept.join('\\');
}

// ───────────────────────── 글자 크기·찾은 곳 표시(문서 항목들이 함께 쓴다) ─────────────────────────

const ZOOM_KEY = 'cypher.reader.zoom';
const BASE_SIZE = 14;
const views = new Set();   // 열려 있는 문서 화면들
let zoom = 0;              // 글자 크기(%). 0이면 아직 읽지 않았다.

function currentZoom() {
  if (!zoom) {
    try { zoom = Number(localStorage.getItem(ZOOM_KEY)) || 100; } catch { zoom = 100; }
    zoom = Math.min(300, Math.max(50, zoom));
  }
  return zoom;
}

function setZoom(next) {
  zoom = Math.min(300, Math.max(50, Math.round(next / 10) * 10));
  try { localStorage.setItem(ZOOM_KEY, String(zoom)); } catch { /* 저장 공간 문제는 무시 */ }
  for (const view of views) view.applyZoom();
}

const MAX_HITS = 2000;
const hits = new Map(); // 문서 화면 → { ranges, at }

/** 찾은 곳을 칠한다(CSS Custom Highlight). 칠하는 이름은 문서 전체에 하나씩이라, 열려 있는 문서 화면의 것을 모아서 올린다. */
function paintHits() {
  const registry = globalThis.CSS?.highlights;
  if (!registry || !globalThis.Highlight) return;
  const all = [], current = [];
  for (const found of hits.values()) {
    all.push(...found.ranges);
    if (found.ranges[found.at]) current.push(found.ranges[found.at]);
  }
  if (all.length) registry.set('reader-hit', new Highlight(...all)); else registry.delete('reader-hit');
  if (current.length) {
    const mark = new Highlight(...current);
    mark.priority = 1;
    registry.set('reader-hit-cur', mark);
  } else registry.delete('reader-hit-cur');
}

const BLOCK_TAGS = 'p, li, h1, h2, h3, h4, h5, h6, pre, td, th, blockquote';

// ───────────────────────── 문서 항목 ─────────────────────────

/**
 * 문서 항목의 화면을 host 안에 만든다.
 * saved: { path(보던 파일), scroll(보던 자리) }. 파일이 없으면 [파일 열기]만 있는 빈 화면으로 시작한다.
 */
export function createReader(item, host, saved = {}, onChange = () => {}) {
  const { invoke, make, setStatus, showMenu } = api;
  let path = saved.path || '';
  let seen;                  // 마지막으로 읽어 본(읽으려 한) 파일의 고친 시각. 파일이 없었으면 null
  let loaded = false;        // 내용을 한 번이라도 그렸다
  const trail = [];          // 링크를 따라 오기 전에 보던 문서들 { path, scroll }
  let lastScroll = Number(saved.scroll) || 0;
  let shown = false;         // 지금 화면에 보이는지
  let loadSeq = 0, loading = 0, checking = false, disposed = false;
  let findOpen = false;

  const button = (text, tip, action) => {
    const b = make('button', '', text);
    b.title = tip;
    b.onclick = action;
    return b;
  };
  const $title = make('span', 'doc-title');
  const $note = make('span', 'reader-note');
  const $open = button('파일 열기', 'Markdown·글자 파일 열기 (Ctrl+O)', () => pick());
  const $back = button('← 뒤로', '이전 문서로 (Alt+←)', () => back());
  const $zoom = make('span', 'reader-zoom');
  const tools = [
    button('새로 고침', '다시 읽기 (바뀌면 자동 갱신)', () => reload()),
    button('목차', '목차에서 골라 이동', (ev) => contents(ev)),
    button('찾기', '찾기 (Ctrl+F)', () => find()),
    button('메모로 편집', '메모 항목으로 열어 고치기', () => api.openMemo?.(path)),
    button('Claude에 넣기', '파일 경로를 Claude 세션 입력창에 넣기', (ev) => api.toClaude?.(item, path, ev)),
    button('탐색기에서 보기', '탐색기에서 보기', () => outside(path, false)),
    button('−', '글자 작게 (Ctrl+−, Ctrl+휠)', () => setZoom(currentZoom() - 10)),
    $zoom,
    button('+', '글자 크게 (Ctrl++, Ctrl+휠) · 기본 크기 (Ctrl+0)', () => setZoom(currentZoom() + 10)),
  ];
  const bar = make('div', 'doc-bar');
  bar.append($title, $note, $open, $back, ...tools);

  // 찾기 줄
  const $query = make('input');
  Object.assign($query, { placeholder: '이 문서에서 찾기', spellcheck: false, autocomplete: 'off' });
  const $count = make('span', 'find-count');
  const $close = button('×', '닫기 (Esc)', () => closeFind());
  $close.className = 'close';
  const findRow = make('div', 'find-row');
  findRow.append($query, $count, button('↑', '이전 (Shift+Enter)', () => step(-1)), button('↓', '다음 (Enter)', () => step(1)), $close);
  const $find = make('div', 'reader-find');
  $find.hidden = true;
  $find.append(findRow);

  const $body = make('div', 'reader-body');
  const $empty = make('div', 'reader-empty',
    '[파일 열기]로 Markdown(.md)이나 글자 파일을 고르면 서식 있는 모습으로 보여 줍니다. 파일이 바뀌면 화면도 따라 바뀝니다.');
  const $scroll = make('div', 'reader-scroll');
  $scroll.tabIndex = 0;
  $scroll.append($empty, $body);
  const root = make('div', 'doc reader');
  root.append(bar, $find, $scroll);
  host.append(root);

  const title = () => (path ? api.baseName(path) : '문서');
  const folder = () => (path ? dirOf(path) : item.cwd || '');
  const note = (text, fail = false) => {
    $note.textContent = text;
    $note.title = text;
    $note.classList.toggle('fail', fail);
  };

  function refreshBar() {
    $title.textContent = title();
    $title.title = path || '문서';
    for (const tool of tools) tool.hidden = !path;
    $back.hidden = !trail.length;
    $empty.hidden = !!path;
    $zoom.textContent = `${currentZoom()}%`;
  }

  function applyZoom() {
    root.style.setProperty('--reader-size', `${(BASE_SIZE * currentZoom()) / 100}px`);
    $zoom.textContent = `${currentZoom()}%`;
  }

  /** 보던 자리로 옮긴다. 화면에 없는 동안에는 적어만 두었다가 다시 보일 때 옮긴다. */
  function setScroll(top) {
    lastScroll = Math.max(0, top || 0);
    if (shown) $scroll.scrollTop = lastScroll;
  }

  function draw(text, scroll) {
    // 아주 긴 줄이 있는 글은 서식으로 그리지 않는다(줄 길이에 따라 느려지는 곳이 남아 있어도 화면이 멈추지 않게).
    const longLine = isMarkdown(path) && text.length <= MAX_MARKDOWN && hasLongLine(text);
    let rich = isMarkdown(path) && text.length <= MAX_MARKDOWN && !longLine;
    let html;
    try {
      html = rich ? renderMarkdown(text) : renderPlain(text);
    } catch {
      rich = false;
      html = renderPlain(text); // 서식으로 그리지 못하면 글자 그대로 보여 준다.
    }
    $body.className = `reader-body${rich ? '' : ' plain'}`;
    $body.innerHTML = html;
    loaded = true;
    if (longLine) note(`한 줄이 ${MAX_LINE.toLocaleString('ko-KR')}자를 넘는 글이라 서식 없이 글자 그대로 보여 줍니다`);
    setScroll(scroll);
    if (findOpen) search(true);
  }

  /**
   * 파일을 읽어 그린다. 성공하면 참.
   * opts.push: 지금 보던 문서를 [뒤로]에 남긴다 · opts.keep: 보던 자리를 지킨다(바뀐 파일을 다시 읽을 때) · opts.scroll: 옮길 자리
   */
  async function load(target, opts = {}) {
    const seq = ++loadSeq;
    let file;
    loading += 1;
    try {
      file = await invoke('reader_read', { path: target });
    } catch (err) {
      loading -= 1;
      if (disposed || seq !== loadSeq) return false;
      if (target !== path) { setStatus(`문서를 열지 못했습니다: ${err}`); return false; }
      // 보던 파일을 다시 읽지 못했다: 마지막으로 읽은 내용은 그대로 둔다.
      note(loaded ? `파일을 읽지 못했습니다(마지막으로 읽은 내용입니다): ${err}` : `파일을 읽지 못했습니다: ${err}`, true);
      refreshBar();
      return false;
    }
    loading -= 1;
    if (disposed || seq !== loadSeq) return false;
    const moved = file.path !== path;
    if (opts.push && path && moved) trail.push({ path, scroll: lastScroll });
    path = file.path;
    seen = file.modified;
    note('');
    draw(file.text, opts.keep && !moved ? lastScroll : opts.scroll ?? 0);
    refreshBar();
    if (moved) onChange();
    return true;
  }

  /** 파일이 바뀌었는지 보고, 바뀌었으면 보던 자리를 지킨 채 다시 그린다. */
  async function check() {
    if (disposed || !path || checking || loading) return;
    checking = true;
    const watched = path;
    try {
      const stamp = await invoke('reader_stamp', { path: watched });
      if (disposed || watched !== path || loading || stamp === seen) return;
      seen = stamp;
      if (stamp == null) {
        note(loaded ? '파일이 없어졌습니다. 마지막으로 읽은 내용을 보여 주고 있습니다.' : '파일이 없습니다. 생기면 바로 보여 줍니다.', true);
      } else {
        // 읽지 못하면(너무 크거나 글자 파일이 아님) 파일이 다시 바뀔 때까지는 또 읽지 않는다.
        if (await load(watched, { keep: true })) {
          const d = new Date();
          const two = (n) => String(n).padStart(2, '0');
          note(`${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())} 바뀐 내용 반영`);
        }
      }
    } catch { /* 다음 번에 다시 본다 */ } finally {
      checking = false;
    }
  }

  async function reload() {
    if (!path) return;
    if (await load(path, { keep: true })) note('다시 읽었습니다');
  }

  async function pick() {
    const picked = await invoke('reader_pick', { start: folder() || null });
    if (picked) await open(picked);
  }

  /** 다른 파일을 이 화면에서 연다. 보던 문서는 [뒤로]로 돌아갈 수 있다. */
  async function open(target) {
    const ok = await load(target, { push: true });
    if (ok) focus();
    return ok;
  }

  async function back() {
    const last = trail[trail.length - 1];
    if (!last) return;
    if (await load(last.path, { scroll: last.scroll })) {
      trail.pop();
      refreshBar();
    }
  }

  /** 웹 주소나 파일을 앱의 링크 열기로 넘긴다(웹 주소는 기본 브라우저, 파일은 연결 프로그램·탐색기). */
  function outside(target, openFile = true) {
    if (!target) return;
    invoke('link_open', { target, cwd: folder(), openFile, line: null, col: null }).catch((err) => setStatus(String(err)));
  }

  /** 그 요소가 화면 위쪽에 오게 스크롤한다. */
  function scrollTo(el) {
    setScroll($scroll.scrollTop + el.getBoundingClientRect().top - $scroll.getBoundingClientRect().top - 10);
  }

  /** 문서 안의 제목(#…)으로 간다. */
  function jump(name) {
    let wanted = name;
    try { wanted = decodeURIComponent(name); } catch { /* 그대로 쓴다 */ }
    wanted = wanted.toLowerCase();
    const target = [...$body.querySelectorAll('[data-slug]')].find((el) => el.dataset.slug === wanted);
    if (target) scrollTo(target); else setStatus(`이 문서에 그런 제목이 없습니다: #${name}`);
  }

  /** 문서 안의 링크를 눌렀을 때. 주소는 그릴 때 이미 걸렀지만(safeTarget) 여기서도 다시 본다. */
  async function follow(href) {
    const target = safeTarget(href);
    if (!target) return setStatus('열 수 없는 링크입니다.');
    if (target.startsWith('#')) return jump(target.slice(1));
    if (/^https?:\/\//i.test(target)) return outside(target);
    const hash = target.indexOf('#');
    let file = (hash >= 0 ? target.slice(0, hash) : target).replace(/\?.*$/, '');
    const name = hash >= 0 ? target.slice(hash + 1) : '';
    try { file = decodeURIComponent(file); } catch { /* 그대로 쓴다 */ }
    if (!file) return name ? jump(name) : undefined;
    if (/^[\\/]{2}/.test(file) || /[\u0000-\u001f]/.test(file)) return setStatus('네트워크 경로의 링크는 열지 않습니다.');
    if (!path) return outside(file);
    if (isMarkdown(file)) {
      // 다른 Markdown 문서는 이 화면에서 연다.
      if (await load(resolvePath(folder(), file), { push: true }) && name) jump(name);
      return undefined;
    }
    return outside(resolvePath(folder(), file));
  }

  /** [목차]: 제목 목록을 메뉴로 보여 주고, 고르면 그 제목으로 간다. */
  function contents(ev) {
    const heads = [...$body.querySelectorAll('[data-slug]')];
    if (!heads.length) return setStatus('이 문서에는 제목이 없습니다.');
    const top = Math.min(...heads.map((el) => Number(el.tagName[1])));
    showMenu(ev, heads.slice(0, 200).map((el) => {
      const text = el.textContent.trim() || '(제목 없음)';
      return {
        label: `${'   '.repeat(Number(el.tagName[1]) - top)}${text.length > 60 ? `${text.slice(0, 60)}…` : text}`,
        action: () => { scrollTo(el); focus(); },
      };
    }));
  }

  function copyText(text) {
    if (!text) return;
    invoke('clipboard_write', { text }).then(() => setStatus('복사했습니다.')).catch((err) => setStatus(String(err)));
  }

  /** 이 문서 안에서 고른 글. */
  function selected() {
    const selection = getSelection();
    return selection && selection.rangeCount && $body.contains(selection.anchorNode) ? selection.toString() : '';
  }

  // ── 찾기 ──

  /** 문서의 글자를 하나로 이어 붙이고, 글자 위치에서 글자 마디를 찾을 수 있게 한다. 덩어리(문단, 칸 …) 사이에는 줄 바꿈을 넣는다. */
  function textIndex() {
    const walker = document.createTreeWalker($body, NodeFilter.SHOW_TEXT, {
      acceptNode: (node) => (node.parentElement.closest('.reader-copy, .reader-lang') ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
    });
    const nodes = [];
    let text = '', lastBlock = null;
    while (walker.nextNode()) {
      const node = walker.currentNode;
      const block = node.parentElement.closest(BLOCK_TAGS);
      if (nodes.length && block !== lastBlock) text += '\n';
      lastBlock = block;
      nodes.push({ node, at: text.length });
      text += node.data;
    }
    return { text, nodes };
  }

  function search(keepPlace) {
    const query = $query.value;
    const before = hits.get(view);
    const ranges = [];
    if (query) {
      const { text, nodes } = textIndex();
      const locate = (position) => {
        let low = 0, high = nodes.length - 1;
        while (low < high) {
          const mid = (low + high + 1) >> 1;
          if (nodes[mid].at <= position) low = mid; else high = mid - 1;
        }
        return nodes[low];
      };
      const pattern = new RegExp(query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'gi');
      for (let m = pattern.exec(text); m && ranges.length < MAX_HITS; m = pattern.exec(text)) {
        const first = locate(m.index), last = locate(m.index + m[0].length - 1);
        const range = document.createRange();
        range.setStart(first.node, Math.min(first.node.length, m.index - first.at));
        range.setEnd(last.node, Math.min(last.node.length, m.index + m[0].length - last.at));
        ranges.push(range);
      }
    }
    let at = 0;
    if (ranges.length && !keepPlace) {
      // 지금 보이는 자리에서 가장 가까운(아래쪽) 것부터 시작한다.
      const top = $scroll.getBoundingClientRect().top;
      const near = ranges.findIndex((range) => range.getBoundingClientRect().bottom >= top);
      at = near < 0 ? 0 : near;
    } else if (keepPlace && before) {
      at = Math.min(before.at, Math.max(0, ranges.length - 1));
    }
    hits.set(view, { ranges, at });
    paintHits();
    showCount();
    if (ranges.length && !keepPlace) reveal();
  }

  function showCount() {
    const found = hits.get(view);
    const total = found?.ranges.length || 0;
    $count.textContent = !$query.value ? '' : !total ? '없음' : `${found.at + 1}/${total}${total >= MAX_HITS ? '+' : ''}`;
    $count.classList.toggle('fail', !!$query.value && !total);
  }

  /** 지금 가리키는 찾은 곳이 화면에 보이게 한다. */
  function reveal() {
    const found = hits.get(view);
    const range = found?.ranges[found.at];
    if (!range || !shown) return;
    const rect = range.getBoundingClientRect(), box = $scroll.getBoundingClientRect();
    if (rect.top < box.top + 20 || rect.bottom > box.bottom - 20) {
      setScroll($scroll.scrollTop + rect.top - box.top - box.height / 3);
    }
  }

  function step(by) {
    const found = hits.get(view);
    if (!found?.ranges.length) return;
    found.at = (found.at + by + found.ranges.length) % found.ranges.length;
    paintHits();
    showCount();
    reveal();
  }

  function find() {
    if (!path) return;
    const picked = selected().trim();
    findOpen = true;
    $find.hidden = false;
    if (picked && picked.length <= 80 && !picked.includes('\n')) $query.value = picked;
    $query.focus();
    $query.select();
    search(false);
  }

  function closeFind() {
    findOpen = false;
    $find.hidden = true;
    hits.delete(view);
    paintHits();
    focus();
  }

  $query.addEventListener('input', () => search(false));
  $query.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter') { ev.preventDefault(); step(ev.shiftKey ? -1 : 1); }
  });

  // ── 마우스·키보드 ──

  $body.addEventListener('click', (ev) => {
    const copy = ev.target.closest?.('[data-copy]');
    if (copy) {
      ev.preventDefault();
      return copyText(copy.parentElement.querySelector('pre')?.textContent || '');
    }
    const link = ev.target.closest?.('[data-href]');
    if (!link) return undefined;
    ev.preventDefault(); // 본창이 다른 곳으로 이동하지 않게 한다(링크에는 주소 속성도 두지 않는다).
    if (selected()) return undefined; // 글을 고르려고 끈 것이다.
    return follow(link.dataset.href);
  });
  $body.addEventListener('auxclick', (ev) => { if (ev.target.closest?.('[data-href]')) ev.preventDefault(); });

  $scroll.addEventListener('contextmenu', (ev) => {
    ev.preventDefault();
    if (!path) return;
    const picked = selected();
    const link = ev.target.closest?.('[data-href]');
    showMenu(ev, [
      ...(picked ? [{ label: '선택한 글 복사  (Ctrl+C)', action: () => copyText(picked) }] : []),
      ...(link ? [{ label: '링크 주소 복사', action: () => copyText(link.dataset.href) }] : []),
      { label: '찾기…  (Ctrl+F)', action: () => find() },
      { label: '문서 경로 복사', action: () => copyText(path) },
      { label: '메모로 편집', action: () => api.openMemo?.(path) },
    ]);
  });

  $scroll.addEventListener('scroll', () => {
    if (shown && $scroll.clientHeight > 0) lastScroll = $scroll.scrollTop;
  });

  // Ctrl+휠: 글자 크기
  $scroll.addEventListener('wheel', (ev) => {
    if (!ev.ctrlKey) return;
    ev.preventDefault();
    setZoom(currentZoom() + (ev.deltaY < 0 ? 10 : -10));
  }, { passive: false });

  root.addEventListener('keydown', (ev) => {
    const key = ev.key.toLowerCase();
    let handled = true;
    if (ev.ctrlKey && !ev.altKey && (key === '=' || key === '+')) setZoom(currentZoom() + 10);
    else if (ev.ctrlKey && !ev.altKey && key === '-') setZoom(currentZoom() - 10);
    else if (ev.ctrlKey && !ev.altKey && !ev.shiftKey && key === '0') setZoom(100);
    else if (ev.ctrlKey && !ev.altKey && !ev.shiftKey && key === 'f') find();
    else if (ev.ctrlKey && !ev.altKey && !ev.shiftKey && key === 'o') pick();
    else if (ev.ctrlKey && !ev.altKey && !ev.shiftKey && key === 'c' && ev.target !== $query && selected()) copyText(selected());
    else if (ev.altKey && !ev.ctrlKey && key === 'arrowleft') back();
    else if (key === 'escape' && findOpen) closeFind();
    else handled = false;
    if (handled) { ev.preventDefault(); ev.stopPropagation(); }
  });

  // 숨겨졌다 다시 보이면 보던 자리로 돌려놓고(숨기면 브라우저가 스크롤을 맨 위로 되돌린다) 파일이 바뀌었는지 바로 본다.
  const watcher = new ResizeObserver(() => {
    const now = $scroll.clientHeight > 0;
    if (now && !shown) {
      shown = true;
      if (Math.abs($scroll.scrollTop - lastScroll) > 1) $scroll.scrollTop = lastScroll;
      check();
    } else if (!now) {
      shown = false;
    }
  });
  watcher.observe($scroll);
  // 화면에 보이는 동안만 파일이 바뀌었는지 살핀다.
  const timer = setInterval(() => { if (shown && !document.hidden) check(); }, 1500);

  function focus() {
    $scroll.focus({ preventScroll: true });
  }

  const view = {
    ready: null,
    focus,
    title,
    state: () => ({ path, scroll: Math.round(lastScroll) }),
    close: () => Promise.resolve(true),
    dispose() {
      disposed = true;
      clearInterval(timer);
      watcher.disconnect();
      views.delete(view);
      hits.delete(view);
      paintHits();
    },
    path: () => path,
    find,
    open,
    reload,
    applyZoom,
  };
  views.add(view);
  applyZoom();
  refreshBar();
  view.ready = path ? load(path, { scroll: lastScroll }).then(() => {}, () => {}) : Promise.resolve();
  return view;
}
