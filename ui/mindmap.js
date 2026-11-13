// 브레인스토밍(마인드맵) 항목: 생각을 가지로 뻗어 가며 정리한다.
//
// 두 화면을 오가며 만든다. 둘은 같은 내용을 가리킨다.
// - 그림: 가지를 누르고 Tab(하위)·Enter(형제)로 더하고, 글자를 치면 고치고, 끌어서 옮긴다. 글이 없는 가지도 둘 수 있다.
// - 코드(개요 글): 첫 줄 "# 중심 주제", 그 아래 "- 가지", 두 칸 들여쓰면 하위 가지. 줄 끝의 {…}는 꾸밈이다.
//     {#e06c75} 색 · {fold} 접힘 · {left}/{right} 중심 주제의 어느 쪽에 놓을지 · 중심 주제의 {logic} 오른쪽으로만 펼치기
//     {flag-red p1 ✅} 표식(아래 "표식" 참고)
//   코드 화면에서는 엄격하게 읽어(readCode) 잘못된 줄을 알려 주고, 고치기 전에는 그림으로 돌아가지 않는다.
//   파일을 열거나 붙여 넣을 때는 너그럽게 읽는다(parseOutline): Markdown 개요(#, ##, - …)나 들여쓴 글도 받아들인다.
//
// 내용은 계속 자동 저장되고(mindmap.rs), 파일(.md)로 저장하거나 PNG·SVG·PDF로 내보낼 수 있다.

let api = null; // { invoke, make, setStatus, showMenu, baseName, addImages(list, opts) }

export function initMindmap(given) {
  api = given;
}

const SVG = 'http://www.w3.org/2000/svg';
function el(name, attrs = {}, text) {
  const node = document.createElementNS(SVG, name);
  for (const [key, value] of Object.entries(attrs)) if (value != null && value !== '') node.setAttribute(key, value);
  if (text != null) node.textContent = text;
  return node;
}

// ───────────────────────── 표식 ─────────────────────────
// 가지의 글 앞에 붙이는 작은 그림. 색만 다른 같은 모양(깃발·별·순위·진행)은 한 가지에 하나씩만 붙고, 이모지는 여럿 붙일 수 있다.
// 개요 글에서는 줄 끝 꾸밈에 이름을 적는다: "- 로그인 {flag-red p1 ✅}"

const MARK_COLORS = [
  ['red', '빨강', '#e5484d'], ['orange', '주황', '#f08c2e'], ['yellow', '노랑', '#dba400'], ['green', '초록', '#3fae5a'],
  ['blue', '파랑', '#3b8fe6'], ['purple', '보라', '#9b6be0'], ['gray', '회색', '#8a939e'],
];
const MARK_EMOJI = [
  ['✅', '완료'], ['❌', '안 함'], ['❗', '중요'], ['❓', '물음'], ['⚠️', '주의'], ['💡', '생각'], ['🔥', '급함'],
  ['📌', '고정'], ['👍', '좋음'], ['👎', '나쁨'], ['❤️', '마음'], ['🎯', '목표'], ['⏰', '기한'], ['🤝', '의논'],
  ['🔒', '막힘'], ['🐞', '버그'], ['🚀', '출시'], ['💰', '비용'], ['👀', '검토'], ['🙂', '만족'], ['😟', '걱정'],
];
const STAR_POINTS = Array.from({ length: 10 }, (_, i) => {
  const radius = i % 2 ? 3.1 : 7.3, angle = -Math.PI / 2 + (i * Math.PI) / 5;
  return `${(8 + radius * Math.cos(angle)).toFixed(2)},${(8.6 + radius * Math.sin(angle)).toFixed(2)}`;
}).join(' ');

/** 표식 그림: 16×16 칸 안에 그린다. */
const MARK_DRAW = {
  flag: (color) => [
    el('path', { d: 'M3.6 1.6V14.6', stroke: '#7a828c', 'stroke-width': 1.5, 'stroke-linecap': 'round', fill: 'none' }),
    el('path', { d: 'M4.4 2.3H13.6L10.9 5.5L13.6 8.7H4.4Z', fill: color }),
  ],
  star: (color) => [el('polygon', { points: STAR_POINTS, fill: color })],
  rank: (color, number) => [
    el('circle', { cx: 8, cy: 8, r: 7.2, fill: color }),
    el('text', { x: 8, y: 11.7, 'text-anchor': 'middle', 'font-size': 10.5, 'font-weight': 'bold', 'font-family': 'Segoe UI, Arial, sans-serif', fill: '#ffffff' }, String(number)),
  ],
  task: (percent) => {
    const green = '#3fae5a';
    if (percent >= 100) {
      return [
        el('circle', { cx: 8, cy: 8, r: 7.2, fill: green }),
        el('path', { d: 'M4.6 8.3L7.1 10.8L11.5 5.7', stroke: '#ffffff', 'stroke-width': 1.8, 'stroke-linecap': 'round', 'stroke-linejoin': 'round', fill: 'none' }),
      ];
    }
    const out = [el('circle', { cx: 8, cy: 8, r: 6.5, fill: '#ffffff', stroke: green, 'stroke-width': 1.4 })];
    if (percent > 0) {
      const angle = (percent / 100) * Math.PI * 2;
      out.push(el('path', {
        d: `M8 8L8 3.2A4.8 4.8 0 ${percent > 50 ? 1 : 0} 1 ${(8 + 4.8 * Math.sin(angle)).toFixed(2)} ${(8 - 4.8 * Math.cos(angle)).toFixed(2)}Z`,
        fill: green,
      }));
    }
    return out;
  },
  emoji: (char) => [
    el('text', { x: 8, y: 12.4, 'text-anchor': 'middle', 'font-size': 12.5, 'font-family': '"Segoe UI Emoji", "Apple Color Emoji", "Noto Color Emoji", sans-serif' }, char),
  ],
};

const MARKS = new Map(); // 이름 → { id, group, only(같은 무리에서 하나만), label, draw() }
const MARK_ROWS = [];    // 메뉴에 보이는 줄
{
  const row = (list) => { for (const mark of list) MARKS.set(mark.id, mark); MARK_ROWS.push(list); };
  for (const [shape, name] of [['flag', '깃발'], ['star', '별']]) {
    row(MARK_COLORS.map(([key, label, color]) => ({ id: `${shape}-${key}`, group: shape, only: true, label: `${label} ${name}`, draw: () => MARK_DRAW[shape](color) })));
  }
  row(MARK_COLORS.map(([, , color], index) => ({ id: `p${index + 1}`, group: 'rank', only: true, label: `순위 ${index + 1}`, draw: () => MARK_DRAW.rank(color, index + 1) })));
  row([0, 25, 50, 75, 100].map((percent) => ({ id: `task-${percent}`, group: 'task', only: true, label: percent === 100 ? '진행: 끝남' : `진행 ${percent}%`, draw: () => MARK_DRAW.task(percent) })));
  for (let i = 0; i < MARK_EMOJI.length; i += 7) {
    row(MARK_EMOJI.slice(i, i + 7).map(([char, label]) => ({ id: char, group: 'emoji', only: false, label, draw: () => MARK_DRAW.emoji(char) })));
  }
}
/** 이모지는 뒤에 붙는 모양 선택 문자(U+FE0F)가 있든 없든 같은 것으로 본다. */
const markKey = (token) => token.replace(/️/g, '').toLowerCase();
const MARK_KEYS = new Map([...MARKS.keys()].map((id) => [markKey(id), id]));

/** 표식 하나를 작은 그림(SVG)으로. 메뉴에 쓴다. */
function markIcon(id, size = 18) {
  const svg = el('svg', { width: size, height: size, viewBox: '0 0 16 16' });
  svg.append(...MARKS.get(id).draw());
  return svg;
}

/** 표식을 붙이거나 뗀다. 색만 다른 같은 모양은 그 자리에서 바뀐다. */
function toggledMarks(marks, id) {
  const mark = MARKS.get(id);
  if (!mark) return marks;
  if (marks.includes(id)) return marks.filter((other) => other !== id);
  const same = mark.only ? marks.findIndex((other) => MARKS.get(other)?.group === mark.group) : -1;
  return same < 0 ? [...marks, id] : marks.map((other, index) => (index === same ? id : other));
}

// ───────────────────────── 개요 글 ↔ 가지 ─────────────────────────

const newNode = (text = '') => ({ text, children: [], color: '', fold: false, side: '', marks: [] });

// 글 안의 줄 바꿈은 "\n"으로 적는다. 글자 그대로의 "\n"이나 "\\"는 역슬래시를 하나 더 붙여 구분한다.
const escapeText = (text) => text.replace(/\\(?=[n\\\r\n])/g, '\\\\').replace(/\r?\n/g, '\\n');
const unescapeText = (text) => text.replace(/\\([n\\])/g, (all, char) => (char === 'n' ? '\n' : '\\'));
/** 그림(SVG·XML)에 넣을 수 없는 제어 문자를 걸러 낸다(탭·줄 바꿈은 둔다). 세로 탭·쪽 바꿈은 빈칸으로 본다. */
const cleanControls = (text) => text.replace(/[\v\f]/g, ' ').replace(/[\u0000-\u0008\u000e-\u001f]/g, '');
/** 글의 앞 `count` 글자. 글자(코드 포인트) 단위로 세어 이모지 같은 대리 문자 쌍을 반으로 자르지 않는다. */
const clip = (text, count) => Array.from(text).slice(0, count).join('');

/** 꾸밈 낱말 하나를 읽어 out에 담는다. 모르는 낱말이면 false. */
function readToken(token, out) {
  const t = token.toLowerCase();
  const mark = MARK_KEYS.get(markKey(token));
  if (/^#([0-9a-f]{3}|[0-9a-f]{6})$/.test(t)) out.color = t;
  else if (t === 'fold' || t === '접힘') out.fold = true;
  else if (t === 'left' || t === '왼쪽') out.side = 'left';
  else if (t === 'right' || t === '오른쪽') out.side = 'right';
  else if (t === 'logic' || t === '한쪽') out.logic = true;
  else if (mark) out.marks = toggledMarks((out.marks || []).filter((other) => other !== mark), mark);
  else return false;
  return true;
}

/** 표식을 적으려다 틀린 것으로 보이는 낱말(없는 색·번호, 목록에 없는 이모지). */
const looksLikeMark = (token) => /^(flag|star|task)(-\w*)?$|^p\d+$/i.test(token) || /^\p{Extended_Pictographic}/u.test(token);
const MARK_HELP = `쓸 수 있는 표식: \`flag-\`·\`star-\` 뒤에 ${MARK_COLORS.map(([key]) => key).join('/')}, \`p1\`~\`p7\`, \`task-0\`/\`25\`/\`50\`/\`75\`/\`100\`, 이모지 ${MARK_EMOJI.map(([char]) => char).join(' ')}. 표식이 아니라 글의 일부라면 줄 끝에 \` {}\`를 덧붙여 주세요.`;
const ATTR_HELP = '쓸 수 있는 꾸밈: 색 `#rgb`·`#rrggbb`, `fold`, `left`·`right`, 표식(`flag-red`, `star-blue`, `p1`, `task-50`, `✅` …). 꾸밈이 아니라 글의 일부라면 줄 끝에 ` {}`를 덧붙여 주세요.';

/**
 * 줄 끝의 {…} 꾸밈을 떼어 낸다. 아는 낱말이 하나도 없으면 꾸밈이 아니라 글의 일부로 본다.
 * `problems`(배열)를 주면 엄격하게 읽는다: 아는 낱말과 모르는 낱말이 섞였거나 닫지 않은 꾸밈을 { message, fix }로 담는다.
 * 주지 않으면 모르는 낱말이 하나라도 있을 때 통째로 글로 본다.
 */
function readAttrs(raw, problems) {
  const plain = { text: raw.trim() };
  const m = /\s*\{([^{}]*)\}\s*$/.exec(raw);
  if (!m) {
    const open = problems && /\{([^{}]*)$/.exec(raw);
    const words = open ? open[1].split(/[\s,]+/).filter(Boolean) : [];
    if (words.length && words.every((word) => readToken(word, {}))) {
      problems.push({ message: '꾸밈을 `}`로 닫지 않았습니다.', fix: '줄 끝에 `}`를 붙여 주세요.' });
    }
    return plain;
  }
  const out = {}, unknown = [];
  const tokens = m[1].split(/[\s,]+/).filter(Boolean);
  for (const token of tokens) if (!readToken(token, out)) unknown.push(token);
  if (unknown.length) {
    const meant = unknown.length < tokens.length || unknown.some((token) => token.startsWith('#') || looksLikeMark(token));
    if (!problems || !meant) return plain;
    for (const token of unknown) {
      problems.push(token.startsWith('#')
        ? { message: `읽을 수 없는 색입니다: \`${token}\``, fix: '색은 `#` 다음에 16진수 3자리나 6자리로 적습니다. 예: `{#3b9fe6}`' }
        : looksLikeMark(token)
          ? { message: `없는 표식입니다: \`${token}\``, fix: MARK_HELP }
          : { message: `모르는 꾸밈입니다: \`${token}\``, fix: ATTR_HELP });
    }
  }
  return { ...out, text: raw.slice(0, m.index).trim() };
}

const nodeOf = (attrs) => Object.assign(newNode(attrs.text), { color: attrs.color || '', fold: !!attrs.fold, side: attrs.side || '', marks: attrs.marks || [] });

/** 개요 글을 너그럽게 읽어 { root, logic }을 만든다. 빈 글이면 중심 주제 하나만 있는 그림이 된다. */
export function parseOutline(source) {
  const rows = [];
  for (const raw of String(source ?? '').replace(/^﻿/, '').replace(/\r\n?/g, '\n').split('\n')) {
    const line = cleanControls(raw);
    if (!line.trim()) continue;
    const lead = /^\s*/.exec(line)[0];
    const indent = lead.replace(/\t/g, '    ').length;
    let body = line.slice(lead.length).trim(), heading = 0;
    // `s` 깃발: 글 안의 줄 구분 문자(U+2028·U+2029)도 `.`이 받게 한다(없으면 그 줄을 가지로 읽지 못한다).
    const h = /^(#{1,6})(?:\s+(.*))?$/s.exec(body);
    if (h) { heading = h[1].length; body = h[2] ?? ''; }
    else body = (/^(?:[-*+]|\d+[.)])(?:\s+(.*))?$/s.exec(body) || [null, body])[1] ?? '';
    rows.push({ indent, heading, ...readAttrs(unescapeText(body)) });
  }
  if (!rows.length) return { root: newNode('중심 주제'), logic: false };

  const [first, ...rest] = rows;
  const root = nodeOf(first);
  root.fold = false;
  root.side = '';
  // 들여쓰기 한 단계의 폭: 가장 작은 들여쓰기 차이(보통 2칸이나 4칸).
  const listRows = rest.filter((row) => !row.heading);
  const indents = [...new Set([...(first.heading ? [] : [first.indent]), ...listRows.map((row) => row.indent)])].sort((a, b) => a - b);
  let unit = 0;
  for (let i = 1; i < indents.length; i++) unit = unit ? Math.min(unit, indents[i] - indents[i - 1]) : indents[i] - indents[i - 1];
  unit = unit || 2;
  const lowest = listRows.length ? Math.min(...listRows.map((row) => row.indent)) : 0;
  // 중심 주제가 머리글(#)이 아니고 나머지가 모두 그보다 들여쓰였으면 들여쓰기가 곧 깊이다.
  const plainTree = !first.heading && lowest > first.indent;
  const origin = plainTree ? first.indent : lowest - unit;

  const stack = [root]; // stack[d] = 지금 줄기에서 깊이 d의 가지
  let section = 0;      // 마지막 머리글의 깊이(그 아래 목록 줄은 이보다 깊다)
  for (const row of rest) {
    let depth = row.heading
      ? Math.max(1, row.heading - (first.heading || 1))
      : section + Math.max(1, Math.round((row.indent - origin) / unit));
    depth = Math.min(depth, stack.length); // 단계를 건너뛰지 않는다.
    const node = nodeOf(row);
    if (depth !== 1) node.side = '';
    stack[depth - 1].children.push(node);
    stack.length = depth;
    stack.push(node);
    if (row.heading) section = depth;
  }
  return { root, logic: !!first.logic };
}

/**
 * 코드 화면에서 쓴 개요 글을 엄격하게 읽는다(그림으로 바꾸기 전의 검사).
 * parseOutline은 무엇이든 받아들여 가지로 만들지만, 여기서는 뜻이 분명하지 않은 줄을 오류로 알려 준다.
 * 돌려주는 값: { root, logic, errors: [{ line, message, fix }] }. 가지마다 `line`(몇째 줄에서 왔는지)이 붙는다.
 */
export function readCode(source) {
  const errors = [];
  let root = null, logic = false;
  const stack = []; // 지금 줄기: [{ indent, node }] (중심 주제는 뺀다)
  String(source ?? '').replace(/^﻿/, '').replace(/\r\n?/g, '\n').split('\n').forEach((given, index) => {
    const raw = cleanControls(given);
    if (!raw.trim()) return;
    const line = index + 1;
    const problems = [];
    const fail = (message, fix) => problems.push({ message, fix });
    const lead = /^[ \t 　]*/.exec(raw)[0];
    if (/[ 　]/.test(lead)) fail('줄 앞에 보통 빈칸이 아닌 빈칸(전각 빈칸 등)이 있습니다.', '줄 앞의 빈칸을 지우고 스페이스 글쇠로 다시 들여 주세요.');
    const body = raw.slice(lead.length).trimEnd();
    const report = () => { for (const problem of problems) errors.push({ line, ...problem }); };

    if (!root) {
      const h = /^#(?:\s+(.*))?$/s.exec(body); // `s`: 글 안의 U+2028·U+2029도 받는다(parseOutline과 같다).
      let words = h ? h[1] ?? '' : '';
      if (!h) {
        words = body.replace(/^(#+|[-*+])\s*/, '');
        fail('첫 줄은 중심 주제여야 합니다.', `\`# \`(샵 하나와 빈칸)로 시작하게 적어 주세요. 예: \`# ${words || '중심 주제'}\``);
      }
      const attrs = readAttrs(unescapeText(words), problems);
      if (attrs.fold || attrs.side) fail('중심 주제에는 `fold`·`left`·`right`를 쓸 수 없습니다.', '첫 줄에서 그 낱말을 지워 주세요.');
      root = Object.assign(nodeOf(attrs), { fold: false, side: '', line });
      logic = !!attrs.logic;
      return report();
    }

    if (lead.includes('\t')) fail('들여쓰기에 탭 문자가 있습니다.', '탭을 지우고 빈칸으로 들여 주세요(한 단계에 두 칸).');
    const indent = lead.replace(/\t/g, '  ').length; // 탭은 두 칸으로 센다(오류로 알린 뒤 나머지를 계속 읽으려고).
    const bullet = /^[-*+](?:\s+(.*))?$/s.exec(body);
    let words = bullet ? bullet[1] ?? '' : body;
    if (bullet) {
      // 그대로 읽는다.
    } else if (body.startsWith('#')) {
      fail('중심 주제(`#`로 시작하는 줄)는 첫 줄 하나만 둘 수 있습니다.', '가지로 적으려면 `#` 대신 `- `로 시작해 주세요.');
      words = body.replace(/^#+\s*/, '');
    } else if (/^[-*+][^\s\-*+]/.test(body)) {
      words = body.slice(1);
      fail('`-` 다음에 한 칸을 띄워야 합니다.', `이렇게 적어 주세요: \`- ${words}\``);
    } else {
      fail('가지는 `- `로 시작해야 합니다.', '줄 앞에 `- `를 붙여 주세요. 윗줄의 글을 잇는 것이라면 윗줄 끝에 `\\n`을 적고 이어서 쓰세요.');
    }

    // 들여쓰기: 바로 윗줄보다 깊으면 그 하위, 위쪽 어느 가지와 같으면 그 형제. 그 밖에는 어디에 붙을지 알 수 없다.
    const before = stack.map((entry) => entry.indent);
    while (stack.length && stack[stack.length - 1].indent > indent) stack.pop();
    if (stack.length && stack[stack.length - 1].indent === indent) stack.pop();
    else if (before.length && before[before.length - 1] >= indent) {
      fail(`들여쓰기(${indent}칸)가 위쪽 가지들과 맞지 않습니다.`, `이 줄기에서 쓴 들여쓰기는 ${[...new Set(before)].join('·')}칸입니다. 그중 하나에 맞춰 주세요.`);
    }
    const parent = stack.length ? stack[stack.length - 1].node : root;
    const depth = stack.length + 1;
    const attrs = readAttrs(unescapeText(words), problems);
    if (attrs.logic) fail('`logic`은 중심 주제(첫 줄)에만 쓸 수 있습니다.', '이 줄에서 지우고, 필요하면 첫 줄 끝에 `{logic}`을 적어 주세요.');
    if (attrs.side && depth > 1) fail('`left`·`right`는 중심 주제 바로 아래 가지에만 쓸 수 있습니다.', '이 줄에서 그 낱말을 지워 주세요.');
    const node = Object.assign(nodeOf(attrs), { line });
    if (depth > 1) node.side = '';
    parent.children.push(node);
    stack.push({ indent, node });
    report();
  });
  if (!root) errors.push({ line: 1, message: '내용이 없습니다.', fix: '첫 줄에 `# 중심 주제`를 적어 주세요.' });
  return { root: root || newNode('중심 주제'), logic, errors };
}

/** 가지를 개요 글로 적는다. */
export function toOutline(root, logic = false) {
  const row = (lead, node, tokens) => {
    // 글이 꾸밈처럼(또는 틀린 꾸밈처럼) 끝나면(예: "값 {fold}", "이슈 {#12}") 빈 꾸밈 {}을 덧붙여 글임을 밝힌다.
    const problems = [];
    const tail = tokens.length ? `{${tokens.join(' ')}}`
      : readAttrs(node.text, problems).text !== node.text.trim() || problems.length ? '{}' : '';
    return [lead, escapeText(node.text), tail].filter(Boolean).join(' ');
  };
  const looks = (node) => [...(node.marks || []), ...(node.color ? [node.color] : [])];
  const out = [row('#', root, [...(logic ? ['logic'] : []), ...looks(root)])];
  (function walk(node, depth) {
    for (const child of node.children) {
      // 쪽(left·right)은 중심 주제 바로 아래 가지에만 적는다(더 깊은 곳에 적으면 엄격하게 읽을 때 오류가 된다).
      out.push(row(`${'  '.repeat(depth)}-`, child, [...looks(child), ...(child.fold && child.children.length ? ['fold'] : []), ...(child.side && depth === 0 ? [child.side] : [])]));
      walk(child, depth + 1);
    }
  })(root, 0);
  return `${out.join('\n')}\n`;
}

// ───────────────────────── 배치 ─────────────────────────

const BRANCH_COLORS = ['#e06c75', '#e5a23c', '#4caf6a', '#3b9fe6', '#b07be0', '#2fb5b5', '#d98a57', '#8c9aa8'];
const SIZE = [16, 14, 13];                        // 글자 크기: 중심 주제, 첫 가지, 그 아래
const PAD = [[18, 11], [12, 7], [9, 5]];          // 안쪽 여백(가로, 세로)
const LINE = 1.4;                                 // 줄 높이
const MAX_TEXT_WIDTH = 280;                       // 이보다 긴 글은 줄을 바꾼다
const H_GAP = 46, V_GAP = [0, 18, 9];             // 단계 사이의 가로 간격, 형제 사이의 세로 간격
const EMPTY_WIDTH = 34;                           // 글도 표식도 없는 가지의 안쪽 폭
const MARK_GAP = 3;                               // 표식 사이의 틈
const level = (depth) => Math.min(depth, 2);
const markSize = (l) => SIZE[l] + 4;              // 표식 한 칸의 크기

let measurer = null;
function measure(text, font) {
  measurer ||= document.createElement('canvas').getContext('2d');
  measurer.font = font;
  return measurer.measureText(text).width;
}

/** 글을 폭에 맞춰 줄로 나눈다. 한글·한자는 글자 단위로, 그 밖에는 낱말 단위로 끊는다. */
function wrap(text, font) {
  const lines = [];
  for (const paragraph of text.split('\n')) {
    const tokens = paragraph.match(/[　-鿿가-힯＀-￯]|[^\s　-鿿가-힯＀-￯]+|\s+/g) || [''];
    let line = '';
    for (let token of tokens) {
      if (measure(line + token, font) <= MAX_TEXT_WIDTH) { line += token; continue; }
      if (line.trim()) lines.push(line.trimEnd());
      line = '';
      token = token.trimStart();
      // 한 낱말이 한 줄보다 길면(긴 주소 등) 글자 단위로 자른다. 글자는 코드 포인트로 세어 이모지를 반으로 자르지 않는다.
      while (measure(token, font) > MAX_TEXT_WIDTH) {
        const chars = Array.from(token);
        let cut = 1;
        while (cut < chars.length && measure(chars.slice(0, cut + 1).join(''), font) <= MAX_TEXT_WIDTH) cut += 1;
        const head = chars.slice(0, cut).join('');
        lines.push(head);
        token = token.slice(head.length);
      }
      line = token;
    }
    lines.push(line.trimEnd());
  }
  return lines;
}

/**
 * 가지마다 크기와 자리를 정한다. 중심 주제의 가운데가 (0, 0)이다.
 * 중심 주제의 가지는 오른쪽·왼쪽으로 나뉘고(`logic`이면 모두 오른쪽), 그 아래는 같은 쪽으로 뻗는다.
 * 돌려주는 값: 화면에 보이는(접힌 가지 아래는 뺀) 가지 목록.
 */
function layoutTree(root, logic, family) {
  const shown = [];
  const size = (node, depth) => {
    const l = level(depth);
    node.depth = depth;
    node.font = `${depth ? 'normal' : 'bold'} ${SIZE[l]}px ${family}`;
    node.lines = node.text ? wrap(node.text, node.font) : [''];
    node.marks = (node.marks || []).filter((mark) => MARKS.has(mark));
    // 표식은 글 앞에 한 줄로 놓는다. mw: 표식이 차지하는 폭(글과의 틈까지).
    const marks = node.marks.length * (markSize(l) + MARK_GAP);
    node.mw = marks && node.text ? marks + 2 : Math.max(0, marks - MARK_GAP);
    const width = node.text ? Math.max(...node.lines.map((line) => measure(line, node.font))) : marks ? 0 : EMPTY_WIDTH;
    node.w = Math.ceil(width + node.mw) + PAD[l][0] * 2;
    node.h = Math.ceil(node.lines.length * SIZE[l] * LINE) + PAD[l][1] * 2;
    shown.push(node);
    const open = depth === 0 || !node.fold;
    node.kids = open ? node.children : [];
    for (const child of node.children) {
      child.parent = node;
      if (open) size(child, depth + 1);
    }
  };
  size(root, 0);
  root.parent = null;

  const gap = (depth) => V_GAP[level(depth)];
  const span = (node) => {
    const kids = node.kids.reduce((sum, child) => sum + span(child), 0) + Math.max(0, node.kids.length - 1) * gap(node.depth + 1);
    node.kidSpan = kids;
    node.span = Math.max(node.h, kids);
    return node.span;
  };
  const place = (node, edge, top, dir, color) => {
    node.dir = dir;
    node.branch = node.color || color;
    const center = top + node.span / 2;
    node.x = dir > 0 ? edge : edge - node.w;
    node.y = center - node.h / 2;
    let y = center - node.kidSpan / 2;
    for (const child of node.kids) {
      place(child, dir > 0 ? node.x + node.w + H_GAP : node.x - H_GAP, y, dir, node.branch);
      y += child.span + gap(node.depth + 1);
    }
  };

  // 중심 주제의 가지를 양쪽으로 나눈다: 쪽을 정해 둔 것은 그대로, 나머지는 앞쪽 절반이 오른쪽.
  const free = root.children.filter((child) => !child.side);
  const rightCount = Math.ceil(free.length / 2);
  const sides = { 1: [], [-1]: [] };
  root.children.forEach((child, index) => {
    const dir = logic ? 1 : child.side === 'left' ? -1 : child.side === 'right' ? 1 : free.indexOf(child) < rightCount ? 1 : -1;
    child.order = index;
    sides[dir].push(child);
  });
  Object.assign(root, { x: -root.w / 2, y: -root.h / 2, dir: 0, branch: '' });
  for (const dir of [1, -1]) {
    const list = sides[dir];
    list.forEach(span);
    const total = list.reduce((sum, child) => sum + child.span, 0) + Math.max(0, list.length - 1) * gap(1);
    let y = -total / 2;
    for (const child of list) {
      const color = child.color || BRANCH_COLORS[child.order % BRANCH_COLORS.length];
      place(child, dir > 0 ? root.w / 2 + H_GAP : -root.w / 2 - H_GAP, y, dir, color);
      y += child.span + gap(1);
    }
  }
  return shown;
}

function boundsOf(nodes) {
  const box = { left: Infinity, top: Infinity, right: -Infinity, bottom: -Infinity };
  for (const node of nodes) {
    box.left = Math.min(box.left, node.x);
    box.top = Math.min(box.top, node.y);
    box.right = Math.max(box.right, node.x + node.w);
    box.bottom = Math.max(box.bottom, node.y + node.h);
  }
  return { ...box, width: box.right - box.left, height: box.bottom - box.top };
}

// ───────────────────────── 그리기(SVG) ─────────────────────────

/** 내보낼 때 쓰는 색(밝은 바탕). 화면에서는 지금 테마의 색을 읽어 쓴다. */
const PRINT_COLORS = { bg: '#ffffff', panel: '#f6f7f9', text: '#1f2328', dim: '#656d76', accent: '#c2410c', onAccent: '#ffffff', ring: '#0969da', line: '#c9ced6' };

function screenColors(host) {
  const style = getComputedStyle(host);
  const value = (name) => style.getPropertyValue(name).trim();
  return {
    bg: value('--field'), panel: value('--panel'), text: value('--text'), dim: value('--dim'), accent: value('--accent'),
    onAccent: value('--on-accent'), ring: value('--attention'), line: value('--line'),
  };
}

const descendants = (node) => node.children.reduce((sum, child) => sum + 1 + descendants(child), 0);

/**
 * 보이는 가지와 이음선을 `group` 안에 그린다. `selected`(대표 가지)와 `picked`(함께 고른 가지들)에는 테두리를 두른다.
 * 고른 가지의 요소에는 class `picked`가, 대표 가지에는 `main`도 붙는다. 내보낼 때는 둘 다 주지 않는다.
 */
function drawMap(group, nodes, colors, family, selected, picked = null) {
  const links = el('g', { fill: 'none', 'stroke-linecap': 'round' });
  const boxes = el('g', { 'font-family': family });
  for (const node of nodes) {
    const l = level(node.depth);
    const parent = node.parent;
    if (parent) {
      const fromX = parent.depth === 0 ? (node.dir > 0 ? parent.x + parent.w : parent.x) : (node.dir > 0 ? parent.x + parent.w : parent.x);
      const fromY = parent.y + parent.h / 2;
      const toX = node.dir > 0 ? node.x : node.x + node.w;
      const toY = node.y + node.h / 2;
      const mid = (fromX + toX) / 2;
      links.append(el('path', {
        d: `M${fromX} ${fromY} C${mid} ${fromY} ${mid} ${toY} ${toX} ${toY}`,
        stroke: node.branch, 'stroke-width': node.depth === 1 ? 2.6 : 1.8, 'stroke-opacity': 0.9,
      }));
    }
    const chosen = node === selected || !!picked?.has(node);
    const g = el('g', {
      class: `mm-node${chosen ? ' picked' : ''}${node === selected ? ' main' : ''}`, 'data-id': node.id,
      transform: `translate(${node.x} ${node.y})`,
    });
    const rect = { width: node.w, height: node.h, rx: [12, 9, 7][l] };
    if (node.depth === 0) g.append(el('rect', { ...rect, fill: node.color || colors.accent }));
    else if (node.depth === 1) g.append(el('rect', { ...rect, fill: colors.bg }), el('rect', { ...rect, fill: node.branch, 'fill-opacity': 0.2, stroke: node.branch, 'stroke-width': 1.6 }));
    else g.append(el('rect', { ...rect, fill: colors.panel, stroke: node.branch, 'stroke-width': 1.2, 'stroke-opacity': 0.75 }));
    if (chosen) {
      // 함께 고른 가지는 대표 가지보다 조금 옅은 테두리로 그린다.
      g.append(el('rect', {
        class: 'mm-ring', x: -3.5, y: -3.5, width: node.w + 7, height: node.h + 7, rx: rect.rx + 3, fill: 'none',
        stroke: colors.ring, 'stroke-width': 2, 'stroke-opacity': node === selected ? 1 : 0.6,
      }));
    }
    const lineHeight = SIZE[l] * LINE;
    const text = el('text', {
      class: 'mm-text', 'font-size': SIZE[l], 'font-weight': node.depth ? 'normal' : 'bold',
      fill: node.depth === 0 ? colors.onAccent : colors.text,
    });
    // 글이 없는 가지는 빈 상자로 둔다.
    (node.text ? node.lines : []).forEach((line, index) => {
      text.append(el('tspan', { x: PAD[l][0] + node.mw, y: PAD[l][1] + lineHeight * index + lineHeight / 2 + SIZE[l] * 0.35 }, line));
    });
    g.append(text);
    node.marks.forEach((id, index) => {
      const size = markSize(l);
      const mark = el('g', {
        class: 'mm-mark', 'data-mark': id,
        transform: `translate(${PAD[l][0] + index * (size + MARK_GAP)} ${PAD[l][1] + (lineHeight - size) / 2}) scale(${size / 16})`,
      });
      mark.append(el('title', {}, MARKS.get(id).label), ...MARKS.get(id).draw());
      g.append(mark);
    });
    // 접힌 가지: 바깥쪽에 숨은 가지 수를 보여 준다. 펼친 가지는 골랐을 때만 접는 단추를 보여 준다.
    if (node.depth && node.children.length && (node.fold || node === selected)) {
      const toggle = el('g', { class: 'mm-fold', transform: `translate(${node.dir > 0 ? node.w + 11 : -11} ${node.h / 2})` });
      toggle.append(
        el('circle', { r: 8, fill: node.fold ? node.branch : colors.bg, stroke: node.branch, 'stroke-width': 1.4 }),
        el('text', { 'text-anchor': 'middle', y: 3.5, 'font-size': 10, 'font-weight': 'bold', fill: node.fold ? '#ffffff' : colors.text },
          node.fold ? String(Math.min(99, descendants(node))) : '−'),
      );
      g.append(toggle);
    }
    boxes.append(g);
  }
  group.replaceChildren(links, boxes);
}

/** 내보낼 그림(혼자 설 수 있는 SVG)을 만든다. 밝은 바탕에 그리며, 고른 가지 표시는 넣지 않는다. */
function exportSvg(root, logic, family) {
  const nodes = layoutTree(root, logic, family);
  nodes.forEach((node, index) => { node.id = index; });
  const box = boundsOf(nodes);
  const margin = 28;
  const width = Math.ceil(box.width + margin * 2), height = Math.ceil(box.height + margin * 2);
  const svg = el('svg', { xmlns: SVG, width, height, viewBox: `${Math.floor(box.left - margin)} ${Math.floor(box.top - margin)} ${width} ${height}` });
  const group = el('g');
  drawMap(group, nodes, PRINT_COLORS, family, null);
  svg.append(el('rect', { x: Math.floor(box.left - margin), y: Math.floor(box.top - margin), width, height, fill: PRINT_COLORS.bg }), group);
  return { svg, width, height };
}

/** SVG를 PNG로 바꾼다(가로·세로 2배로 그려 또렷하게). */
async function svgToPng(svg, width, height) {
  const scale = Math.min(2, 16000 / Math.max(width, height));
  const image = new Image();
  image.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(new XMLSerializer().serializeToString(svg))}`;
  await image.decode();
  const canvas = document.createElement('canvas');
  canvas.width = Math.round(width * scale);
  canvas.height = Math.round(height * scale);
  canvas.getContext('2d').drawImage(image, 0, 0, canvas.width, canvas.height);
  const blob = await new Promise((resolve) => canvas.toBlob(resolve, 'image/png'));
  if (!blob) throw new Error('그림을 만들지 못했습니다');
  return new Uint8Array(await blob.arrayBuffer());
}

// ───────────────────────── 화면 ─────────────────────────

const newId = () => `b${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
const MIN_ZOOM = 0.2, MAX_ZOOM = 3;
const MAX_UNDO = 200;
const SAVE_RETRY_MS = 5000;       // 자동 저장에 실패했을 때 다시 해 보기까지의 시간
const MAX_PDF_SIDE = 190 * 96;    // PDF 한 쪽의 한 변 한도(CSS 픽셀). mindmap.rs의 MAX_PAGE_INCHES와 같다.

/**
 * 브레인스토밍 항목의 화면을 host 안에 만든다.
 * saved: { map(저장 번호), path(연결된 파일), code(개요 글 창을 열어 두었는지), text(처음 넣을 내용) }
 */
export function createMindmap(item, host, saved = {}, onChange = () => {}) {
  const { invoke, make, setStatus, showMenu, baseName } = api;
  const id = saved.map || newId();
  let root = newNode('중심 주제');
  let logic = false;
  let path = saved.path || '';     // 연결된 파일(저장하면 여기에 쓴다)
  let fileText = null;             // 그 파일에 마지막으로 쓴(또는 읽은) 내용
  let stored = '';                 // 자동 저장소에 마지막으로 쓴 내용(쓰지 못했으면 null: 저장소에 무엇이 들었는지 모른다)
  let unread = false;              // 시작할 때 저장소를 읽지 못했다(다시 읽을 때까지 자동 저장하지 않는다)
  let nodes = [];                  // 화면에 보이는 가지
  let selected = null;             // 고른 가지(여러 개를 골랐으면 그 대표: 키보드 이동·고치기·접기의 기준)
  let picked = new Set();          // 함께 고른 가지들. 대표 가지를 늘 품고, 보이는 가지만 담는다(render가 맞춘다).
  let spaceHeld = false;           // Space를 누르고 있다(이때 끌면 그림이 움직인다)
  let spaceUsed = false;           // Space를 누른 채 그림을 끌었다(떼어도 글 고치기를 시작하지 않는다)
  let editing = null;              // 글을 고치고 있는 가지
  let pristine = false;            // 글자를 쳐서 고치기 시작했지만 아직 글자가 하나도 들어오지 않았다
  let seq = 0;
  let view = { x: 0, y: 0, k: 1 }; // 그림의 이동·배율
  let fitted = false;
  let auto = true;                 // 사용자가 그림을 옮기거나 확대하지 않았다(창 크기가 바뀌면 다시 맞춘다)
  let fitMax = 1;
  const undo = [], redo = [];
  let saveTimer = 0, codeTimer = 0;
  let disposed = false;
  let codeMode = false;            // 코드 화면을 보고 있다
  let codeApplied = '';            // 그림에 반영된 마지막 코드(코드 칸의 글이 이것과 다르면 오류가 있어 아직 반영하지 못한 것이다)
  let codeErrors = [];             // 지금 코드의 오류
  let codeUndo = false;            // 코드 화면에 들어온 뒤 되돌리기를 남겼다(한 번만 남긴다)
  let codeChanged = false;         // 코드 화면에서 내용을 바꿨다(그림으로 돌아가면 다시 맞춘다)

  const button = (text, tip, action) => {
    const b = make('button', '', text);
    b.title = tip;
    b.onclick = (ev) => { action(ev); if (!editing && document.activeElement === b) (codeMode ? $codeText : $stage).focus(); };
    return b;
  };
  const $title = make('span', 'doc-title', '');
  const $zoom = make('span', 'image-zoom', '100%');
  // 그림 ↔ 코드 전환: 지금 화면 쪽이 눌린 모양이다.
  const $modeMap = button('그림', '그림으로 봅니다 (Ctrl+E) · 코드에 오류가 있으면 고친 뒤에 바뀝니다', () => setMode(false));
  const $modeCode = button('코드', '같은 내용을 코드(개요 글)로 보고 고칩니다 (Ctrl+E)', () => setMode(true));
  const $mode = make('span', 'mm-mode');
  $mode.append($modeMap, $modeCode);
  const $fold = button('접기', '고른 가지 아래를 접거나 폅니다 (Ctrl+/)', () => toggleFold());
  // 그림에서만 쓰는 단추(코드 화면에서는 숨긴다).
  const mapOnly = [
    button('+ 하위', '고른 가지 아래에 새 가지 (Tab) · 글을 적지 않고 Enter를 누르면 빈 가지로 남습니다', () => addChild()),
    button('+ 형제', '고른 가지 다음에 새 가지 (Enter)', () => addSibling(1)),
    button('삭제', '고른 가지와 그 아래를 지웁니다 (Delete)', () => removeSelected()),
    button('색', '고른 가지의 색', (ev) => colorMenu(ev)),
    button('표식', '고른 가지에 깃발·별·순위·진행·이모지 표식을 붙이거나 뗍니다 (가지를 오른쪽 클릭해도 됩니다)', (ev) => showMenu(ev, markItems(selected || root))),
    $fold,
    button('맞춤', '그림 전체가 보이게 (Ctrl+0)', () => fit()),
    button('축소', '축소 (Ctrl+-, Ctrl+휠)', () => zoomBy(1 / 1.25)),
    $zoom,
    button('확대', '확대 (Ctrl++, Ctrl+휠)', () => zoomBy(1.25)),
    button('구조', '가지를 양쪽으로 펼칠지, 오른쪽으로만 펼칠지', (ev) => showMenu(ev, [
      { label: `${logic ? '' : '● '}양쪽으로 펼치기 (마인드맵)`, action: () => change(() => { logic = false; }) },
      { label: `${logic ? '● ' : ''}오른쪽으로만 펼치기 (논리도)`, action: () => change(() => { logic = true; }) },
    ])),
  ];
  const bar = make('div', 'doc-bar');
  bar.append(
    $mode,
    ...mapOnly,
    $title,
    button('열기', '개요 파일(.md, .txt)을 이 항목으로 엽니다 (Ctrl+O)', () => openFile()),
    button('저장', '파일로 저장합니다 (Ctrl+S) · 내용은 저장하지 않아도 자동으로 보관됩니다', () => saveFile(false)),
    button('내보내기', 'PNG 이미지·PDF·SVG로 내보내거나, 사이드바의 이미지 항목으로 넣습니다', (ev) => showMenu(ev, [
      { label: 'PNG 이미지로 내보내기…', action: () => exportAs('png') },
      { label: 'PDF로 내보내기…', action: () => exportAs('pdf') },
      { label: 'SVG로 내보내기…', action: () => exportAs('svg') },
      { label: '사이드바의 이미지 항목으로 넣기', action: () => exportAs('item') },
      { label: '코드(개요 글)를 클립보드에 복사', action: () => { if (settle('복사')) invoke('clipboard_write', { text: text() }).then(() => setStatus('코드(개요 글)를 복사했습니다.')); } },
      { label: '다른 이름으로 저장…', action: () => saveFile(true) },
    ])),
  );

  // 코드 화면: 줄 번호 · 글 · 아래에 검사 결과와 쓰는 법.
  const $codeText = make('textarea', 'mm-code');
  Object.assign($codeText, { spellcheck: false, autocomplete: 'off', wrap: 'off' });
  $codeText.placeholder = '# 중심 주제\n- 가지\n  - 하위 가지\n- 다른 가지 {#3b9fe6}';
  const $gutter = make('div', 'mm-gutter');
  const $problems = make('div', 'mm-problems');
  const $guide = make('div', 'mm-guide');
  const $guideHead = make('button', 'mm-guide-head');
  $guideHead.title = '코드 쓰는 법을 펴거나 접습니다';
  const $guideBody = make('div', 'mm-guide-body');
  for (const [sample, what] of [
    ['# 중심 주제', '첫 줄에 하나만'],
    ['- 가지', '`- ` 다음에 글. 글 없이 `-`만 적으면 빈 가지'],
    ['  - 하위 가지', '윗줄보다 두 칸 더 들여쓰기'],
    ['- 첫 줄\\n둘째 줄', '`\\n`은 가지 안의 줄 바꿈'],
    ['- 가지 {#3b9fe6 fold}', '줄 끝 꾸밈: 색 · `fold` 접힘 · `left`/`right` 놓을 쪽 · 첫 줄의 `logic` 오른쪽으로만'],
    ['- 가지 {flag-red p1 ✅}', `표식: flag-·star- + ${MARK_COLORS.map(([key]) => key).join('/')} · p1~p7 · task-0/25/50/75/100 · ${MARK_EMOJI.map(([char]) => char).join(' ')}`],
  ]) {
    const row = make('div', 'mm-guide-row');
    row.title = `${sample}  —  ${what.replace(/`/g, '')}`;
    row.append(make('code', '', sample), rich('', what));
    $guideBody.append(row);
  }
  $guideBody.lastChild.title = `표식 이름 — 깃발 flag-색 · 별 star-색 (색: ${MARK_COLORS.map(([key, label]) => `${key} ${label}`).join(', ')}) · 순위 p1~p7 · 진행 task-0/25/50/75/100 · ${MARK_EMOJI.map(([char, label]) => `${char} ${label}`).join(' · ')}`;
  // 쓰는 법은 접어 둘 수 있다(작은 창에서 글 칸이 좁아지지 않게). 접었는지는 기억해 둔다.
  const showGuide = (open) => {
    $guideBody.hidden = !open;
    $guideHead.textContent = `${open ? '▾' : '▸'} 코드 쓰는 법`;
    try { localStorage.setItem('cypher.mindmap.guide', open ? '1' : '0'); } catch { /* 저장하지 못해도 된다 */ }
  };
  $guideHead.onclick = () => { showGuide($guideBody.hidden); $codeText.focus(); };
  let guideOpen = true;
  try { guideOpen = localStorage.getItem('cypher.mindmap.guide') !== '0'; } catch { /* 기본값으로 */ }
  showGuide(guideOpen);
  $guide.append($guideHead, $guideBody);
  const codeMain = make('div', 'mm-codemain');
  codeMain.append($gutter, $codeText);
  const $codeView = make('div', 'mm-codeview');
  $codeView.hidden = true;
  $codeView.append(codeMain, $problems, $guide);
  const $stage = make('div', 'mm-stage');
  $stage.tabIndex = 0;
  const $svg = el('svg', { class: 'mm-svg' });
  const $view = el('g');
  $svg.append($view);
  const $edit = make('textarea', 'mm-edit');
  Object.assign($edit, { spellcheck: false, autocomplete: 'off', hidden: true, rows: 1 });
  const $hint = make('div', 'mm-hint', 'Tab: 하위 · Enter: 형제 · 글자 입력·F2·두 번 누르기: 고치기(Shift+Enter: 줄 바꿈, Esc: 취소) · Delete: 지우기 · 끌기: 옮기기 · 빈 곳 끌기·Ctrl+클릭: 여러 개 고르기 · Space+끌기·가운데 단추: 그림 옮기기 · 오른쪽 클릭: 표식·색 · Ctrl+E: 코드');
  $stage.append($svg, $edit, $hint);
  const body = make('div', 'mm-body');
  body.append($codeView, $stage);
  const rootEl = make('div', 'doc mindmap');
  rootEl.append(bar, body);
  host.append(rootEl);

  const text = () => toOutline(root, logic);
  const family = () => getComputedStyle($stage).fontFamily;
  const byId = (nodeId) => nodes.find((node) => String(node.id) === String(nodeId)) || null;
  /** 가지의 자리(중심 주제에서 내려오는 순번들). 다시 읽은 뒤에도 같은 가지를 찾는 데 쓴다. */
  const pathOf = (node) => {
    const out = [];
    for (let n = node; n && n.parent; n = n.parent) out.unshift(n.parent.children.indexOf(n));
    return out;
  };
  const nodeAt = (steps) => steps.reduce((node, index) => node?.children[index] || node, root);

  // ── 그리기 ──

  function render() {
    if (disposed) return;
    nodes = layoutTree(root, logic, family());
    for (const node of nodes) node.id ??= ++seq;
    if (selected && !nodes.includes(selected)) {
      // 고른 가지가 접혀 보이지 않게 됐으면 보이는 조상을 고른다.
      let up = selected;
      while (up && !nodes.includes(up)) up = up.parent;
      selected = up || root;
    }
    // 대표 가지가 고른 것들 밖으로 바뀌었으면(방향키, 새 가지, 되돌리기 등) 하나만 고른 상태로 돌아간다.
    if (!picked.has(selected)) picked = new Set();
    for (const node of [...picked]) if (!nodes.includes(node)) picked.delete(node);
    if (selected) picked.add(selected);
    drawMap($view, nodes, screenColors($stage), family(), editing ? null : selected, editing ? null : picked);
    applyView();
    $title.textContent = `${path ? `${baseName(path)}${fileDirty() ? ' *' : ''} · ` : ''}가지 ${countAll(root) - 1}개${picked.size > 1 ? ` · ${picked.size}개 고름` : ''}`;
    $fold.textContent = selected?.fold && selected.children.length ? '펴기' : '접기';
    $title.title = path || '아직 파일로 저장하지 않았습니다(내용은 자동으로 보관됩니다)';
  }
  const countAll = (node) => 1 + node.children.reduce((sum, child) => sum + countAll(child), 0);

  function applyView() {
    $view.setAttribute('transform', `translate(${view.x} ${view.y}) scale(${view.k})`);
    $zoom.textContent = `${Math.round(view.k * 100)}%`;
    if (editing) placeEditor();
  }

  function fit(maxZoom = 1.5) {
    const rect = $stage.getBoundingClientRect();
    if (rect.width < 20 || rect.height < 20 || !nodes.length) return;
    const box = boundsOf(nodes);
    const k = Math.min(maxZoom, Math.max(MIN_ZOOM, Math.min((rect.width - 60) / box.width, (rect.height - 70) / box.height)));
    view = { k, x: rect.width / 2 - (box.left + box.width / 2) * k, y: rect.height / 2 - (box.top + box.height / 2) * k };
    fitted = true;
    auto = true;
    fitMax = maxZoom;
    applyView();
  }

  function zoomBy(factor, at) {
    const rect = $stage.getBoundingClientRect();
    const point = at || { x: rect.width / 2, y: rect.height / 2 };
    const k = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, view.k * factor));
    view = { k, x: point.x - ((point.x - view.x) / view.k) * k, y: point.y - ((point.y - view.y) / view.k) * k };
    auto = false;
    applyView();
  }

  /** 가지가 화면 밖이면 보이는 곳까지만 그림을 옮긴다. */
  function reveal(node) {
    if (!node || !nodes.includes(node)) return;
    const rect = $stage.getBoundingClientRect();
    const margin = 24;
    const left = view.x + node.x * view.k, top = view.y + node.y * view.k;
    const right = left + node.w * view.k, bottom = top + node.h * view.k;
    if (left < margin) view.x += margin - left; else if (right > rect.width - margin) view.x -= right - (rect.width - margin);
    if (top < margin) view.y += margin - top; else if (bottom > rect.height - margin) view.y -= bottom - (rect.height - margin);
    applyView();
  }

  /** 가지 하나만 고른다. */
  function select(node, show = true) {
    selected = node || root;
    picked = new Set([selected]);
    render();
    if (show) reveal(selected);
  }

  /** 가지를 고른 것들에 넣는다. `toggle`이면 이미 고른 가지는 뺀다(하나는 늘 고른 채로 둔다). 방금 넣은 가지가 대표가 된다. */
  function pick(node, toggle) {
    if (!picked.has(node)) { picked.add(node); selected = node; }
    else if (toggle && picked.size > 1) {
      picked.delete(node);
      if (selected === node) selected = [...picked].pop();
    } else selected = node;
    render();
  }

  /** 고른 가지 가운데 맨 위의 것들(조상도 함께 골랐으면 조상만)을 그림에 적힌 차례로. 중심 주제는 뺀다. */
  function topPicked() {
    const out = [];
    (function walk(node) {
      if (node.parent && picked.has(node)) out.push(node);
      else node.children.forEach(walk);
    })(root);
    return out;
  }

  /** 색·표식처럼 고른 가지 모두에 하는 일의 대상: 그 가지가 여러 개 고른 것 가운데 하나면 고른 것 모두, 아니면 그 가지만. */
  const groupOf = (node) => (picked.size > 1 && picked.has(node) ? [...picked] : [node]);

  // ── 고치기(되돌릴 수 있게) ──

  const snapshot = () => ({ text: text(), at: pathOf(selected || root) });

  function load(source, at) {
    const parsed = parseOutline(source);
    root = parsed.root;
    logic = parsed.logic;
    seq = 0;
    render();
    selected = at ? nodeAt(at) : root;
    if (!nodes.includes(selected)) selected = root;
    render();
  }

  /** 내용을 바꾼다: 바꾸기 전 모습을 되돌리기 목록에 남기고, 다시 그리고, 저장한다. */
  function change(action) {
    undo.push(snapshot());
    if (undo.length > MAX_UNDO) undo.shift();
    redo.length = 0;
    setStatus(''); // 앞서 띄운 안내("…지울 수 없습니다" 등)가 방금 한 일의 결과처럼 남지 않게 한다.
    action();
    changed();
  }

  function changed() {
    render();
    if (codeMode) syncCode();
    scheduleSave();
    onChange();
  }

  function step(from, to) {
    const state = from.pop();
    if (!state) return;
    to.push(snapshot());
    load(state.text, state.at);
    changed();
    reveal(selected);
  }

  function scheduleSave() {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(flush, 400);
  }
  async function flush() {
    clearTimeout(saveTimer);
    const now = text();
    if (disposed || unread || now === stored) return;
    stored = now;
    try {
      await invoke('mindmap_write', { id, text: now });
    } catch (err) {
      setStatus(`브레인스토밍 자동 저장 실패: ${err}`);
      // 쓰지 못했으면 저장소의 내용을 모르는 것으로 둔다: 저장소의 옛 내용을 새것인 줄 알고 읽어 들이지 않고(reload),
      // 같은 내용이라도 다시 쓴다. 그 사이 더 새 내용을 쓰고 있으면 그쪽에 맡긴다.
      if (stored !== now) return;
      stored = null;
      if (!disposed) saveTimer = setTimeout(flush, SAVE_RETRY_MS);
    }
  }

  // ── 가지 더하기·지우기·옮기기 ──

  function addChild() {
    if (editing) commitEdit();
    const parent = selected || root;
    const node = newNode('');
    change(() => { parent.fold = false; parent.children.push(node); selected = node; });
    beginEdit(node, { fresh: true });
  }

  function addSibling(offset) {
    if (editing) commitEdit();
    const here = selected || root;
    if (!here.parent) return addChild();
    const node = newNode('');
    // 중심 주제의 가지는 옆에 있던 가지와 같은 쪽에 놓는다.
    if (here.parent === root && !logic) node.side = here.dir < 0 ? 'left' : 'right';
    change(() => {
      if (here.parent === root) pinSides();
      const list = here.parent.children;
      list.splice(list.indexOf(here) + (offset > 0 ? 1 : 0), 0, node);
      selected = node;
    });
    beginEdit(node, { fresh: true });
  }

  /** 가지들을(그 아래까지) 한 번에 지운다(되돌리기 한 번). `list`는 서로 조상·자손이 아닌 가지들이다. 중심 주제는 지우지 않는다. */
  function removeNodes(list) {
    list = list.filter((node) => node?.parent);
    if (!list.length) return setStatus('중심 주제는 지울 수 없습니다.');
    const home = list[0].parent, index = home.children.indexOf(list[0]);
    change(() => {
      if (list.some((node) => node.parent === root)) pinSides(); // 남은 가지가 반대쪽으로 넘어가지 않게 한다.
      for (const node of list) {
        const from = node.parent.children, at = from.indexOf(node);
        if (at >= 0) from.splice(at, 1);
      }
      // 지운 자리의 다음 가지(없으면 앞 가지, 그것도 없으면 부모)를 고른다.
      selected = home.children[index] || home.children[index - 1] || home;
      picked = new Set();
    });
  }
  const removeNode = (node) => removeNodes([node]);
  /** 고른 가지를 지운다. 여러 개를 골랐으면 모두(조상도 함께 골랐으면 조상째로). */
  const removeSelected = () => { if (editing) cancelEdit(); removeNodes(picked.size > 1 ? topPicked() : [selected]); };

  function toggleFold(node = selected) {
    if (!node?.parent || !node.children.length) return;
    change(() => { node.fold = !node.fold; });
  }

  function moveAmongSiblings(offset) {
    const node = selected;
    if (!node?.parent) return;
    // 화면에서 위아래로 이웃한(같은 쪽의) 형제와 자리를 바꾼다.
    const list = node.parent.children;
    const same = list.filter((other) => nodes.includes(other) && other.dir === node.dir);
    const other = same[same.indexOf(node) + offset];
    if (!other) return;
    change(() => {
      // 쪽을 정하지 않은 가지는 순서에 따라 좌우로 나뉘므로, 순서를 바꾸기 전에 지금 놓인 쪽을 못박는다.
      if (node.parent === root) pinSides();
      const a = list.indexOf(node), b = list.indexOf(other);
      [list[a], list[b]] = [list[b], list[a]];
    });
    reveal(node);
  }

  const within = (node, ancestor) => { for (let n = node; n; n = n.parent) if (n === ancestor) return true; return false; };

  /** 그 자리에 놓을 수 있는가: 옮기는 가지 자신이나 그 자손 위에는 놓을 수 없다. */
  const canDrop = (list, target) => !!target && !list.some((node) => within(target, node));

  /**
   * 가지들을 다른 가지로 한 번에 옮긴다(되돌리기 한 번). zone: 'child'(그 아래로), 'before'/'after'(그 가지의 앞·뒤 형제로).
   * `list`는 서로 조상·자손이 아닌 가지들이고, 적힌 차례 그대로 놓인다. 중심 주제는 옮기지 않는다.
   */
  function moveNodes(list, target, zone, leftOfRoot) {
    list = list.filter((node) => node.parent);
    if (!list.length || !canDrop(list, target)) return;
    if (zone !== 'child' && !target.parent) zone = 'child';
    change(() => {
      // 중심 주제의 가지가 늘거나 줄거나 자리를 바꾸면 나머지는 지금 있는 쪽에 그대로 둔다.
      if (target === root || (zone !== 'child' && target.parent === root) || list.some((node) => node.parent === root)) pinSides();
      for (const node of list) {
        const from = node.parent.children;
        from.splice(from.indexOf(node), 1);
      }
      const home = zone === 'child' ? target : target.parent;
      const side = home !== root || logic ? '' : zone === 'child' ? (leftOfRoot ? 'left' : 'right') : (target.dir < 0 ? 'left' : 'right');
      for (const node of list) node.side = side;
      if (zone === 'child') {
        target.fold = false;
        home.children.push(...list);
      } else {
        home.children.splice(home.children.indexOf(target) + (zone === 'after' ? 1 : 0), 0, ...list);
      }
      // 옮긴 가지들은 고른 채로 둔다. 대표 가지가 옮긴 것 밖에 있었으면 첫 가지를 대표로 삼는다.
      if (!list.some((node) => within(selected, node))) selected = list[0];
      picked.add(selected);
    });
  }

  /**
   * 중심 주제의 가지들이 지금 놓인 쪽을 못박는다. 쪽을 정하지 않은 가지는 앞쪽 절반이 오른쪽으로 나뉘므로,
   * 한 가지의 쪽만 정하면 나머지가 다시 나뉘어 엉뚱한 가지가 반대쪽으로 넘어간다.
   */
  function pinSides() {
    if (logic) return;
    for (const child of root.children) if (!child.side && child.dir) child.side = child.dir < 0 ? 'left' : 'right';
  }

  /** 색 메뉴의 한 줄. 여러 개를 골랐으면 고른 가지 모두의 색을 바꾼다. */
  const colorItem = (node) => {
    const list = groupOf(node);
    return { colors: ['', ...BRANCH_COLORS.slice(0, 7)], action: (color) => change(() => { for (const other of list) other.color = color; }) };
  };
  const colorMenu = (ev) => showMenu(ev, [colorItem(selected || root)]);

  /**
   * 표식 메뉴: 줄마다 같은 모양의 색 다른 표식(깃발·별·순위·진행), 그 아래 이모지. 붙어 있는 것을 다시 누르면 뗀다.
   * 여러 개를 골랐으면 `node`(누른 가지)를 기준으로 고른 가지 모두에 붙이거나 모두에서 뗀다.
   */
  function markItems(node) {
    const list = groupOf(node);
    const toggle = (id) => change(() => {
      const off = node.marks.includes(id);
      for (const other of list) {
        if (off) other.marks = other.marks.filter((mark) => mark !== id);
        else if (!other.marks.includes(id)) other.marks = toggledMarks(other.marks, id);
      }
    });
    return [
      { caption: `표식${list.length > 1 ? ` · 고른 가지 ${list.length}개` : ''} · 붙은 것을 다시 누르면 뗍니다` },
      ...MARK_ROWS.map((row) => ({
        icons: row.map((mark) => ({
          node: markIcon(mark.id), title: `${mark.label}  {${mark.id}}`, on: node.marks.includes(mark.id),
          action: () => toggle(mark.id),
        })),
      })),
      ...(list.some((other) => other.marks.length) ? [{ label: '표식 모두 떼기', action: () => change(() => { for (const other of list) other.marks = []; }) }] : []),
    ];
  }

  // ── 글 고치기 ──

  function placeEditor() {
    const node = editing;
    if (!node) return;
    const l = level(node.depth);
    const size = SIZE[l] * view.k;
    // 표식이 있으면 그 바로 오른쪽(글 자리)부터 놓아 표식이 가려지지 않게 한다.
    const inset = 4; // 표식 옆에 놓을 때의 왼쪽 안여백
    const lead = node.mw ? PAD[l][0] + node.mw - (node.text ? inset : -2) : 0;
    Object.assign($edit.style, {
      left: `${view.x + (node.x + lead) * view.k}px`,
      top: `${view.y + node.y * view.k}px`,
      minWidth: `${Math.max((node.w - lead) * view.k, 90)}px`,
      minHeight: `${node.h * view.k}px`,
      padding: `${PAD[l][1] * view.k}px ${PAD[l][0] * view.k}px ${PAD[l][1] * view.k}px ${(node.mw ? inset : PAD[l][0]) * view.k}px`,
      font: `${node.depth ? 'normal' : 'bold'} ${size}px/${LINE} ${family()}`,
      borderRadius: `${[12, 9, 7][l] * view.k}px`,
    });
    growEditor();
  }
  /** 고치는 칸의 크기를 글에 맞춘다. 가지 글과 같은 폭에서 줄을 바꾼다(긴 글이 한 줄로 끝없이 늘어나지 않게). */
  function growEditor() {
    const node = editing;
    if (!node) return;
    const style = getComputedStyle($edit);
    const border = style.boxSizing === 'border-box' ? parseFloat(style.borderLeftWidth) * 2 : 0;
    const sides = style.boxSizing === 'border-box' ? parseFloat(style.paddingLeft) + parseFloat(style.paddingRight) + border : 0;
    const widest = Math.max(0, ...$edit.value.split('\n').map((line) => measure(line, node.font)));
    $edit.style.width = `${Math.ceil((Math.min(widest, MAX_TEXT_WIDTH) + 12) * view.k + sides)}px`;
    $edit.style.height = '0px';
    $edit.style.height = `${$edit.scrollHeight + border}px`;
  }

  /** 가지의 글을 그 자리에서 고친다. `fresh`: 방금 만든 가지(취소하면 지운다) · `typed`: 글자를 쳐서 시작(있던 글을 바꾼다). */
  function beginEdit(node, { fresh = false, typed = false } = {}) {
    if (!node) return;
    selected = node;
    editing = node;
    editing.fresh = fresh;
    pristine = typed;
    render();
    reveal(node);
    $edit.value = typed ? '' : node.text;
    $edit.hidden = false;
    placeEditor();
    $edit.focus();
    if (!typed) $edit.select();
  }

  function endEdit() {
    const node = editing;
    editing = null;
    $edit.hidden = true;
    return node;
  }

  function commitEdit() {
    // 아무것도 적지 않아도 가지는 남는다(글 없는 가지). 만들지 않으려면 Esc.
    const node = endEdit();
    if (!node) return;
    const fresh = !!node.fresh;
    delete node.fresh;
    // 글자를 쳐서 시작했는데 글자가 하나도 들어오지 않았으면(한/영 글쇠처럼 글자를 내지 않는 글쇠) 있던 글을 지우지 않고 그만둔다.
    if (pristine && !$edit.value) { pristine = false; return render(); }
    // 줄 구분 문자(U+2028·U+2029)와 세로 탭·쪽 바꿈은 줄 바꿈으로, 그 밖의 제어 문자는 걸러 낸다.
    const value = cleanControls($edit.value.replace(/\r\n?|[\p{Zl}\p{Zp}\v\f]/gu, '\n')).trim();
    if (value !== node.text) {
      // 방금 만든 가지에 글을 넣는 것은 만들 때 남긴 되돌리기 한 번으로 묶는다(전부터 있던 빈 가지는 따로 되돌린다).
      if (fresh && undo.length) { node.text = value; changed(); }
      else change(() => { node.text = value; });
    } else render();
  }

  function cancelEdit() {
    const node = endEdit();
    if (!node) return;
    if (node.fresh && !node.text && node.parent) {
      // 방금 만들고 아무것도 적지 않은 가지는 없던 일로 한다.
      const before = undo.pop();
      if (before) { load(before.text, before.at); changed(); }
    } else {
      delete node.fresh;
      render();
    }
  }

  $edit.addEventListener('input', () => { pristine = false; growEditor(); });
  $edit.addEventListener('compositionstart', () => { pristine = false; }); // 조합 중인 글자도 들어온 글자로 친다.
  $edit.addEventListener('keydown', (ev) => {
    ev.stopPropagation();
    if (ev.isComposing || ev.keyCode === 229) return; // 한글 조합 중
    if (ev.key === 'Enter' && !ev.shiftKey) { ev.preventDefault(); commitEdit(); $stage.focus(); }
    else if (ev.key === 'Escape') { ev.preventDefault(); cancelEdit(); $stage.focus(); }
    else if (ev.key === 'Tab') { ev.preventDefault(); commitEdit(); $stage.focus(); addChild(); }
  });
  $edit.addEventListener('blur', () => { if (editing) commitEdit(); });
  $edit.addEventListener('mousedown', (ev) => ev.stopPropagation());

  // ── 키보드 ──

  /** 화면에서 그 방향으로 가장 가까운 가지. */
  function neighbor(node, key) {
    if (key === 'ArrowLeft' || key === 'ArrowRight') {
      const outward = key === 'ArrowRight' ? 1 : -1;
      if (!node.parent) return node.kids.find((child) => child.dir === outward) || null;
      if (node.dir !== outward) return node.parent;
      if (node.fold && node.children.length) { toggleFold(node); return node.children[0]; }
      return node.kids[0] || null;
    }
    const up = key === 'ArrowUp';
    const cx = node.x + node.w / 2, cy = node.y + node.h / 2;
    let best = null, bestScore = Infinity;
    for (const other of nodes) {
      if (other === node) continue;
      const dy = (other.y + other.h / 2) - cy;
      if (up ? dy >= -1 : dy <= 1) continue;
      const score = Math.abs(dy) + Math.abs((other.x + other.w / 2) - cx) * 3 + (other.dir === node.dir ? 0 : 400);
      if (score < bestScore) { best = other; bestScore = score; }
    }
    return best;
  }

  $stage.addEventListener('keydown', (ev) => {
    if (editing) return;
    const key = ev.key, mod = ev.ctrlKey && !ev.altKey;
    const node = selected || root;
    let handled = true;
    if (mod && ev.shiftKey && key.toLowerCase() !== 'z') return; // Ctrl+Shift+… 는 앱 단축키
    if (mod && key.toLowerCase() === 'z') step(ev.shiftKey ? redo : undo, ev.shiftKey ? undo : redo);
    else if (mod && key.toLowerCase() === 'y') step(redo, undo);
    else if (mod && key.toLowerCase() === 's') saveFile(false);
    else if (mod && key.toLowerCase() === 'o') openFile();
    else if (mod && key.toLowerCase() === 'c') copyNode(false);
    else if (mod && key.toLowerCase() === 'x') copyNode(true);
    else if (mod && key.toLowerCase() === 'v') pasteNode();
    else if (mod && key.toLowerCase() === 'e') setMode(true);
    else if (mod && key === '/') toggleFold();
    else if (mod && key === '0') fit();
    else if (mod && (key === '=' || key === '+')) zoomBy(1.25);
    else if (mod && key === '-') zoomBy(1 / 1.25);
    else if (ev.ctrlKey || ev.metaKey) handled = false;
    else if (ev.altKey && (key === 'ArrowUp' || key === 'ArrowDown')) moveAmongSiblings(key === 'ArrowUp' ? -1 : 1);
    else if (ev.altKey) handled = false;
    else if (key === 'Tab') addChild();
    else if (key === 'Enter') addSibling(ev.shiftKey ? -1 : 1);
    else if (key === 'F2') beginEdit(node);
    else if (key === 'Delete' || key === 'Backspace') removeSelected();
    else if (key === 'Home') { select(root); fit(Math.max(view.k, 1)); }
    else if (key.startsWith('Arrow')) { const next = neighbor(node, key); if (next) select(next); }
    else if (key === 'Escape' && picked.size > 1) select(node, false); // 여러 개 고른 것을 풀고 대표 가지만 남긴다.
    // Space: 누른 채 끌면 그림이 움직인다. 끌지 않고 떼면 고른 가지의 글을 고친다(onKeyUp).
    else if (key === ' ') { if (!spaceHeld) { spaceHeld = true; spaceUsed = false; $stage.classList.add('grab'); } }
    // 글자를 치면 바로 고치기 시작한다(있던 글을 바꾼다). 한글 입력기의 첫 글쇠도 여기로 온다.
    else if (key.length === 1 || key === 'Process') { beginEdit(node, { typed: true }); return; }
    else handled = false;
    if (handled) { ev.preventDefault(); ev.stopPropagation(); }
  });

  /** Space를 뗐다. 누른 동안 그림을 끌지 않았으면 전처럼 고른 가지의 글을 고친다. */
  const onKeyUp = (ev) => {
    if (ev.key !== ' ' || !spaceHeld) return;
    endSpace();
    if (!spaceUsed && !press && !editing && !codeMode && !disposed && document.activeElement === $stage) beginEdit(selected || root);
  };
  const endSpace = () => { spaceHeld = false; $stage.classList.remove('grab'); };
  // 그림 밖에서 떼거나 창이 가려져도 놓치지 않게 문서와 창에서 듣는다(dispose에서 뗀다).
  document.addEventListener('keyup', onKeyUp);
  window.addEventListener('blur', endSpace);

  async function copyNode(cut) {
    // 여러 개를 골랐으면 맨 위의 것들을 모두 복사한다: 중심 주제 줄 없이 가지들만 적는다(붙여 넣으면 줄마다 가지가 된다).
    const tops = picked.size > 1 ? topPicked() : [];
    if (tops.length > 1) {
      await invoke('clipboard_write', { text: toOutline({ ...newNode(''), children: tops }, false).replace(/^.*\n/, '') });
      if (cut) removeNodes(tops);
      return setStatus(cut ? `가지 ${tops.length}개를 잘라 냈습니다.` : `가지 ${tops.length}개를 개요 글로 복사했습니다.`);
    }
    const node = tops[0] || selected || root;
    await invoke('clipboard_write', { text: toOutline(node, false) });
    if (cut && node.parent) removeNode(node);
    setStatus(cut && node.parent ? '가지를 잘라 냈습니다.' : '가지를 개요 글로 복사했습니다.');
  }

  async function pasteNode() {
    const clip = await invoke('clipboard_read').catch(() => null);
    if (!clip?.text?.trim() || clip.kind === 'image') return setStatus('붙여 넣을 글이 없습니다.');
    const source = clip.text.replace(/\r\n?/g, '\n');
    let forest;
    if (/^#{1,6}(\s|$)/.test(source.trimStart())) {
      // 가지를 복사한 글(첫 줄이 "# …")이나 Markdown 개요: 첫 줄이 가지 하나가 되고 나머지는 그 하위로 붙는다.
      forest = [parseOutline(source).root];
    } else {
      // 그 밖의 글: 줄마다 가지가 된다(들여쓰기·목록 기호가 있으면 그대로 하위 가지로).
      const lines = source.split('\n').map((line) => line.replace(/^(\s*)#{1,6}\s+/, '$1- '));
      forest = parseOutline(`# 붙여넣기\n${lines.join('\n')}`).root.children;
    }
    if (!forest.length) return;
    const parent = selected || root;
    // 쪽(left·right)은 중심 주제 바로 아래에서만 뜻이 있다. 중심 주제를 복사한 글에는 그 가지들의 쪽이 적혀 있으므로 하위까지 지운다.
    const clearSides = (node) => { node.side = ''; node.children.forEach(clearSides); };
    change(() => {
      parent.fold = false;
      for (const node of forest) { clearSides(node); parent.children.push(node); }
      // 붙여 넣은 가지들을 고른 채로 둔다(여러 개면 이어서 함께 옮기거나 지울 수 있다).
      selected = forest[0];
      picked = new Set(forest);
    });
    reveal(selected);
  }

  // ── 마우스 ──

  const pointOf = (ev) => { const rect = $stage.getBoundingClientRect(); return { x: ev.clientX - rect.left, y: ev.clientY - rect.top }; };
  const nodeOfEvent = (ev) => byId(ev.target.closest?.('.mm-node')?.dataset.id);
  // 누르고 있는 것: { pan } 그림 옮기기 · { box } 빈 곳에서 범위 상자로 고르기 · { node } 가지 누르기·끌어 옮기기. 공통: x, y, moved
  let press = null;
  let lastDown = null; // 바로 앞에 누른 곳 { node, at, x, y }

  function dropZone(ev) {
    const target = nodeOfEvent(ev);
    if (!target) return null;
    const point = pointOf(ev);
    const top = view.y + target.y * view.k, height = target.h * view.k;
    const ratio = (point.y - top) / height;
    const zone = !target.parent ? 'child' : ratio < 0.25 ? 'before' : ratio > 0.75 ? 'after' : 'child';
    return { target, zone, leftOfRoot: point.x < view.x };
  }

  $stage.addEventListener('mousedown', (ev) => {
    if ((ev.button !== 0 && ev.button !== 1) || ev.target === $edit) return;
    if (editing) commitEdit();
    $stage.focus();
    if (ev.button === 1 || spaceHeld) {
      // 가운데 단추로 끌거나 Space를 누른 채 끌면 어디를 눌렀든 그림이 움직인다.
      ev.preventDefault();
      lastDown = null;
      if (spaceHeld) spaceUsed = true;
      press = { pan: true, x: ev.clientX, y: ev.clientY, moved: false, from: { ...view } };
      return;
    }
    const fold = ev.target.closest?.('.mm-fold');
    const node = nodeOfEvent(ev);
    if (fold && node) { ev.preventDefault(); lastDown = null; toggleFold(node); return; }
    ev.preventDefault();
    const add = ev.ctrlKey || ev.shiftKey;
    if (node && add) {
      // Ctrl+클릭: 고른 것에 넣거나 뺀다. Shift+클릭: 넣기만 한다.
      lastDown = null;
      pick(node, ev.ctrlKey && !ev.shiftKey);
      return;
    }
    // 두 번 누르기: 가지를 고르면 그림을 다시 그려 눌린 요소가 바뀌므로 브라우저의 dblclick에 기대지 않고 직접 센다.
    const again = node && lastDown && lastDown.node === node && ev.timeStamp - lastDown.at < 450 && Math.hypot(ev.clientX - lastDown.x, ev.clientY - lastDown.y) < 6;
    lastDown = again ? null : { node, at: ev.timeStamp, x: ev.clientX, y: ev.clientY };
    if (again) { beginEdit(node); return; }
    if (!node) {
      // 빈 곳: 끌면 범위 상자로 고른다(Ctrl·Shift를 누르고 있으면 이미 고른 것에 더한다). 끌지 않고 떼면 여러 개 고른 것이 풀린다.
      press = { box: true, x: ev.clientX, y: ev.clientY, moved: false, at: pointOf(ev), add, base: new Set(add ? picked : []), main: selected };
      return;
    }
    // 표식은 이미 고른 가지에서 누를 때만 메뉴를 띄운다(표식이 많은 가지를 고르려고 누를 때 방해되지 않게).
    const mark = node === selected && !!ev.target.closest?.('.mm-mark');
    // 여러 개 고른 것 가운데 하나를 누르면 고른 것을 그대로 둔다(함께 끌어 옮길 수 있게). 끌지 않고 떼면 그 가지만 고른다.
    if (picked.size > 1 && picked.has(node)) { selected = node; render(); } else select(node, false);
    press = { node, x: ev.clientX, y: ev.clientY, moved: false, from: { ...view }, mark };
  });

  /** 범위 상자를 끄는 동안: 상자를 그리고, 상자에 닿은 보이는 가지들을 바로 고른 모습으로 보여 준다. */
  function dragBox(ev) {
    const from = press.at, now = pointOf(ev);
    const left = Math.min(from.x, now.x), top = Math.min(from.y, now.y), width = Math.abs(now.x - from.x), height = Math.abs(now.y - from.y);
    if (!press.band) { press.band = el('rect', { class: 'mm-band' }); $svg.append(press.band); }
    for (const [key, value] of Object.entries({ x: left, y: top, width, height })) press.band.setAttribute(key, value);
    // 상자를 그림의 좌표로 바꿔 가지의 상자와 겹치는지 본다.
    const x0 = (left - view.x) / view.k, y0 = (top - view.y) / view.k, x1 = x0 + width / view.k, y1 = y0 + height / view.k;
    const hits = nodes.filter((node) => node.x < x1 && node.x + node.w > x0 && node.y < y1 && node.y + node.h > y0);
    const next = new Set([...press.base, ...hits]);
    if (!next.size) next.add(press.main || root); // 하나는 늘 고른 채로 둔다.
    const main = next.has(selected) ? selected : next.has(press.main) ? press.main : hits[0];
    if (main === selected && next.size === picked.size && [...next].every((node) => picked.has(node))) return;
    picked = next;
    selected = main;
    render();
  }

  const onMove = (ev) => {
    if (!press) return;
    const dx = ev.clientX - press.x, dy = ev.clientY - press.y;
    if (!press.moved && Math.hypot(dx, dy) < 5) return;
    press.moved = true;
    if (press.pan || (press.node && !press.node.parent)) {
      // 그림 옮기기: 가운데 단추, Space+끌기, 중심 주제 끌기.
      view = { ...view, x: press.from.x + dx, y: press.from.y + dy };
      auto = false;
      $stage.classList.add('panning');
      return applyView();
    }
    if (press.box) return dragBox(ev);
    if (!press.moving) {
      // 여러 개 고른 것 가운데 하나를 끌면 고른 것 모두를 옮긴다(조상도 함께 골랐으면 조상만: 그 아래는 따라간다).
      press.moving = picked.size > 1 && picked.has(press.node) ? topPicked() : [press.node];
      const label = press.moving.length > 1 ? `가지 ${press.moving.length}개` : clip(press.moving[0].text.split('\n')[0], 30) || '(빈 가지)';
      press.ghost = make('div', 'drag-ghost', label);
      document.body.append(press.ghost);
    }
    press.ghost.style.left = `${ev.clientX + 12}px`;
    press.ghost.style.top = `${ev.clientY + 8}px`;
    const hit = dropZone(ev);
    const ok = hit && canDrop(press.moving, hit.target);
    $view.querySelector('.mm-drop')?.remove();
    if (ok) {
      const { target, zone } = hit;
      const mark = zone === 'child'
        ? el('rect', { x: target.x - 4, y: target.y - 4, width: target.w + 8, height: target.h + 8, rx: 10 })
        : el('rect', { x: target.x, y: zone === 'before' ? target.y - 6 : target.y + target.h + 2, width: target.w, height: 4, rx: 2 });
      mark.setAttribute('class', `mm-drop ${zone}`);
      $view.append(mark);
    }
  };
  const onUp = (ev) => {
    if (!press) return;
    const { node, moved, ghost, mark, band, box, pan, add, moving } = press;
    press = null;
    $stage.classList.remove('panning');
    ghost?.remove();
    band?.remove();
    $view.querySelector('.mm-drop')?.remove();
    if (pan) return;
    if (box) {
      // 범위 상자로 고른 것은 끄는 동안 이미 반영됐다. 빈 곳을 끌지 않고 눌렀다 뗐으면 여러 개 고른 것을 풀고 대표 가지만 남긴다.
      if (!moved && !add && picked.size > 1) select(selected, false);
      return;
    }
    // 표식을 누르면 표식 메뉴가 뜬다(바꾸거나 떼기).
    if (!moved && mark) return showMenu(ev, markItems(node));
    // 여러 개 고른 것 가운데 하나를 끌지 않고 눌렀다 뗐으면 그 가지만 고른다.
    if (!moved) { if (picked.size > 1) select(node, false); return; }
    if (!node.parent) return;
    const hit = dropZone(ev);
    if (hit) moveNodes(moving, hit.target, hit.zone, hit.leftOfRoot);
  };
  document.addEventListener('mousemove', onMove);
  document.addEventListener('mouseup', onUp);

  // 고치는 칸이 그림 밖으로 나가도 초점 때문에 그림 영역이 스크롤되어 밀리지 않게 한다.
  $stage.addEventListener('scroll', () => { $stage.scrollLeft = 0; $stage.scrollTop = 0; });

  $stage.addEventListener('wheel', (ev) => {
    ev.preventDefault();
    if (ev.ctrlKey || ev.altKey) return zoomBy(ev.deltaY < 0 ? 1.12 : 1 / 1.12, pointOf(ev));
    view = { ...view, x: view.x - (ev.shiftKey ? ev.deltaY : ev.deltaX), y: view.y - (ev.shiftKey ? 0 : ev.deltaY) };
    auto = false;
    applyView();
  }, { passive: false });

  $stage.addEventListener('contextmenu', (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    if (editing) commitEdit();
    const node = nodeOfEvent(ev);
    if (!node) {
      return showMenu(ev, [
        { label: '그림 전체가 보이게 (맞춤)', action: () => fit() },
        { label: '붙여넣기', action: () => pasteNode() },
        { label: '되돌리기', action: () => step(undo, redo) },
      ]);
    }
    // 여러 개 고른 것 가운데 하나를 오른쪽 클릭하면 고른 것을 그대로 두고(그 가지가 대표가 된다), 아니면 그 가지만 고른다.
    if (picked.size > 1 && picked.has(node)) { selected = node; render(); } else select(node, false);
    const onRoot = !node.parent;
    const tops = picked.size > 1 ? topPicked() : []; // 여러 개를 골랐으면 복사·지우기는 고른 것 모두에, 색·표식도 모두에 한다.
    const many = tops.length > 1 ? `고른 가지 ${tops.length}개 ` : '';
    showMenu(ev, [
      { label: '하위 가지 더하기 (Tab)', action: () => addChild() },
      ...(onRoot ? [] : [{ label: '형제 가지 더하기 (Enter)', action: () => addSibling(1) }]),
      { label: '글 고치기 (F2)', action: () => beginEdit(node) },
      colorItem(node),
      ...markItems(node),
      ...(!onRoot && node.children.length ? [{ label: node.fold ? '펴기' : '접기', action: () => toggleFold(node) }] : []),
      ...(node.parent === root && !logic ? [{
        label: node.dir > 0 ? '왼쪽으로 옮기기' : '오른쪽으로 옮기기',
        action: () => { const side = node.dir > 0 ? 'left' : 'right'; change(() => { pinSides(); node.side = side; }); },
      }] : []),
      { label: `${many}복사 (Ctrl+C)`, action: () => copyNode(false) },
      { label: '붙여넣기 (Ctrl+V)', action: () => pasteNode() },
      ...(tops.length ? [{ label: `${many}지우기 (Delete)`, action: () => removeNodes(tops) }]
        : onRoot ? [] : [{ label: '지우기 (Delete)', action: () => removeNode(node) }]),
    ]);
  });

  // ── 코드 화면 ──

  const LINE_HEIGHT = 20; // 코드 칸의 줄 높이(items.css의 .mm-code, .mm-gutter와 같다)

  /** 화면 종류에 맞춰 영역과 단추를 보이거나 숨긴다. */
  function showMode() {
    $codeView.hidden = !codeMode;
    $stage.hidden = codeMode;
    $modeMap.classList.toggle('on', !codeMode);
    $modeCode.classList.toggle('on', codeMode);
    for (const control of mapOnly) control.hidden = codeMode;
  }

  /** `낱말`처럼 적은 곳을 코드 글꼴로 보여 준다. */
  function rich(className, source) {
    const span = make('span', className);
    source.split('`').forEach((part, index) => { if (part) span.append(index % 2 ? make('code', '', part) : part); });
    return span;
  }

  /** 가지가 코드의 몇째 줄(0부터)에 적히는지: 중심 주제부터 차례로 한 줄씩이다. */
  function rowOf(target) {
    let row = 0, found = 0;
    (function walk(node) { if (node === target) found = row; row += 1; node.children.forEach(walk); })(root);
    return found;
  }

  /** 코드 칸의 커서를 그 줄(1부터)의 끝으로 옮기고 보이게 한다. `whole`: 줄 전체를 고른다. */
  function jumpTo(line, whole = false) {
    const lines = $codeText.value.split('\n');
    const index = Math.max(0, Math.min(lines.length - 1, line - 1));
    const start = lines.slice(0, index).reduce((sum, row) => sum + row.length + 1, 0);
    $codeText.focus();
    $codeText.setSelectionRange(whole ? start : start + lines[index].length, start + lines[index].length);
    const top = index * LINE_HEIGHT, room = $codeText.clientHeight;
    if (top < $codeText.scrollTop || top > $codeText.scrollTop + room - LINE_HEIGHT * 3) $codeText.scrollTop = Math.max(0, top - room / 2);
    $gutter.scrollTop = $codeText.scrollTop;
  }
  const caretLine = () => $codeText.value.slice(0, $codeText.selectionStart).split('\n').length;

  /** 줄 번호를 줄 수에 맞춘다(오류 표시는 다음 검사 때 다시 칠한다). */
  function fitGutter() {
    const count = $codeText.value.split('\n').length;
    while ($gutter.childElementCount > count) $gutter.lastChild.remove();
    while ($gutter.childElementCount < count) $gutter.append(make('div', '', String($gutter.childElementCount + 1)));
    $gutter.scrollTop = $codeText.scrollTop;
  }

  /** 검사 결과를 줄 번호 옆과 아래 칸에 보여 준다. */
  function showProblems() {
    const lines = $codeText.value.split('\n');
    const bad = new Map();
    for (const error of codeErrors) if (!bad.has(error.line)) bad.set(error.line, error);
    $gutter.replaceChildren(...lines.map((_, index) => {
      const error = bad.get(index + 1);
      const row = make('div', error ? 'bad' : '', String(index + 1));
      if (error) { row.title = error.message.replace(/`/g, ''); row.onclick = () => jumpTo(index + 1, true); }
      return row;
    }));
    $gutter.scrollTop = $codeText.scrollTop;
    $problems.className = `mm-problems ${codeErrors.length ? 'bad' : 'ok'}`;
    $modeMap.classList.toggle('blocked', codeErrors.length > 0);
    $modeMap.title = codeErrors.length
      ? '코드에 오류가 있어 그림으로 바꿀 수 없습니다. 아래 안내대로 고쳐 주세요'
      : '그림으로 봅니다 (Ctrl+E) · 코드에 오류가 있으면 고친 뒤에 바뀝니다';
    if (!codeErrors.length) {
      $problems.replaceChildren(make('div', 'head', `✓ 오류 없음 · 가지 ${countAll(root) - 1}개 — 그림에 반영됐습니다. [그림](Ctrl+E)을 누르면 그림으로 돌아갑니다.`));
      return;
    }
    $problems.replaceChildren(
      make('div', 'head', `오류 ${codeErrors.length}개 — 고치기 전에는 그림으로 바뀌지 않습니다. 그림과 저장된 내용은 마지막으로 올바르던 코드 그대로입니다.`),
      ...codeErrors.slice(0, 50).map((error) => {
        const row = make('button');
        row.title = '이 줄로 가기';
        row.append(make('span', 'where', `${error.line}번째 줄`), rich('', error.message),
          make('span', 'src', lines[error.line - 1] ?? ''), rich('fix', `→ ${error.fix}`));
        row.onclick = () => jumpTo(error.line, true);
        return row;
      }),
    );
  }

  /** 코드 칸의 글을 검사하고, 오류가 없으면 그림에 반영한다. 오류가 없으면 true. */
  function checkCode() {
    clearTimeout(codeTimer);
    const source = $codeText.value;
    const read = readCode(source);
    codeErrors = read.errors;
    if (!read.errors.length && source !== codeApplied) {
      if (toOutline(read.root, read.logic) !== text()) {
        // 코드 화면에서 고친 것은 되돌리기 한 번으로 묶는다.
        if (!codeUndo) { undo.push(snapshot()); if (undo.length > MAX_UNDO) undo.shift(); redo.length = 0; codeUndo = true; }
        codeChanged = true;
      }
      root = read.root;
      logic = read.logic;
      seq = 0;
      selected = root;
      codeApplied = source;
      render();
      scheduleSave();
    }
    showProblems();
    onChange();
    return !read.errors.length;
  }

  /** 그림 쪽에서 내용이 바뀌었으면(열기, 밖에서 쓰기 등) 코드 칸을 그 내용으로 다시 채운다. */
  function syncCode() {
    clearTimeout(codeTimer);
    $codeText.value = text();
    codeApplied = $codeText.value;
    codeErrors = readCode(codeApplied).errors; // 그림이 만든 글은 늘 통과해야 한다. 그렇지 않다면 숨기지 않고 보여 준다.
    showProblems();
  }

  /** 오류가 있어 하려던 일을 하지 못했음을 알리고 첫 오류 줄로 데려간다. */
  function pointAtErrors(message) {
    setStatus(message);
    $problems.classList.remove('flash');
    void $problems.offsetWidth; // 깜박임을 처음부터 다시
    $problems.classList.add('flash');
    $problems.scrollTop = 0;
    jumpTo(codeErrors[0].line, true);
  }

  /** 코드 화면에 반영하지 못한 글(오류)이 있으면 알려 주고 false. 파일로 쓰기 전에 부른다. `what`: "저장하지"처럼. */
  function settle(what) {
    if (!codeMode || checkCode()) return true;
    pointAtErrors(`코드에 오류가 ${codeErrors.length}개 있어 ${what} 않았습니다. 아래 안내대로 고친 뒤 다시 해 주세요.`);
    return false;
  }

  /** 그림 ↔ 코드. 코드에 오류가 있으면 그림으로 바꾸지 않고 어디를 고칠지 알려 준다. 바뀌었으면 true. */
  function setMode(code, focus = true) {
    if (code === codeMode) {
      if (focus) (codeMode ? $codeText : $stage).focus();
      return true;
    }
    if (code) {
      if (editing) commitEdit();
      const row = rowOf(selected || root);
      codeMode = true;
      codeUndo = false;
      codeChanged = false;
      showMode();
      syncCode();
      if (focus) jumpTo(row + 1);
    } else {
      if (!checkCode()) {
        pointAtErrors(`코드에 오류가 ${codeErrors.length}개 있어 그림으로 바꾸지 않았습니다. 아래 안내대로 고쳐 주세요.`);
        return false;
      }
      // 커서가 있던 줄의 가지를 고른다: 그 줄까지 적힌 가지의 수가 곧 차례다.
      const line = caretLine();
      let order = -1;
      (function walk(node) { if (node.line <= line) order += 1; node.children.forEach(walk); })(readCode($codeText.value).root);
      let here = root;
      (function walk(node) { if (order === 0) here = node; order -= 1; node.children.forEach(walk); })(root);
      codeMode = false;
      showMode();
      selected = here;
      render();
      if (codeChanged || !fitted) fit(1);
      reveal(selected);
      if (focus) $stage.focus();
      setStatus('');
    }
    onChange();
    return true;
  }

  /** 코드 칸에 글을 넣는다(칸의 되돌리기 Ctrl+Z가 살아 있게). */
  function insert(words) {
    if (!document.execCommand('insertText', false, words)) {
      $codeText.setRangeText(words, $codeText.selectionStart, $codeText.selectionEnd, 'end');
      $codeText.dispatchEvent(new Event('input'));
    }
  }

  /** Tab·Shift+Tab: 커서가 있는 줄(고른 줄들)을 두 칸 들이거나 내어 쓴다. */
  function indentLines(dir) {
    const value = $codeText.value, from = $codeText.selectionStart, to = $codeText.selectionEnd;
    const start = value.lastIndexOf('\n', from - 1) + 1;
    let end = value.indexOf('\n', to > from && value[to - 1] === '\n' ? to - 1 : to);
    if (end < 0) end = value.length;
    const before = value.slice(start, end).split('\n');
    const rootAt = value.length - value.trimStart().length; // 중심 주제 줄은 들이지 않는다.
    const first = start <= rootAt && rootAt <= end ? value.slice(start, rootAt).split('\n').length - 1 : -1;
    const after = before.map((row, index) => (dir > 0 ? (row.trim() && index !== first ? `  ${row}` : row) : row.replace(/^( {1,2}|\t)/, '')));
    if (after.join('\n') === before.join('\n')) return;
    $codeText.setSelectionRange(start, end);
    insert(after.join('\n'));
    if (from === to) {
      const caret = Math.max(start, from + after[0].length - before[0].length);
      $codeText.setSelectionRange(caret, caret);
    } else {
      $codeText.setSelectionRange(start, start + after.join('\n').length);
    }
  }

  /** Enter: 새 줄을 같은 깊이의 가지("- ")로 시작한다. */
  function newLine() {
    const value = $codeText.value, at = $codeText.selectionStart;
    const start = value.lastIndexOf('\n', at - 1) + 1;
    const row = value.slice(start, at);
    if (!row.trim()) return insert('\n');
    const onRoot = /^\s*#/.test(row) && !value.slice(0, start).trim();
    let lead = onRoot ? '' : /^\s*/.exec(row)[0];
    const end = value.indexOf('\n', at);
    if (!onRoot && end >= 0 && !value.slice(at, end).trim()) {
      // 줄 끝에서 눌렀고 아래에 이 줄의 하위가 있으면, 새 줄을 같은 깊이로 넣을 때 그 하위가 새 줄 밑으로 넘어간다.
      // 그러지 않게 새 줄을 첫 하위로 넣는다.
      const next = value.slice(end + 1).split('\n').find((line) => line.trim());
      const deeper = next ? /^\s*/.exec(next)[0] : '';
      if (deeper.length > lead.length) lead = deeper;
    }
    insert(`\n${lead}- `);
  }

  $codeText.addEventListener('input', () => {
    fitGutter();
    clearTimeout(codeTimer);
    codeTimer = setTimeout(checkCode, 200);
  });
  $codeText.addEventListener('scroll', () => { $gutter.scrollTop = $codeText.scrollTop; });
  $codeText.addEventListener('keydown', (ev) => {
    if (ev.ctrlKey && ev.shiftKey) return; // Ctrl+Shift+… 는 앱 단축키
    ev.stopPropagation();
    if (ev.isComposing || ev.keyCode === 229) return; // 한글 조합 중
    const key = ev.key.toLowerCase(), mod = ev.ctrlKey && !ev.altKey;
    if (ev.key === 'Tab' && !ev.ctrlKey && !ev.altKey) { ev.preventDefault(); indentLines(ev.shiftKey ? -1 : 1); }
    else if (ev.key === 'Enter' && !ev.ctrlKey && !ev.altKey && !ev.shiftKey) { ev.preventDefault(); newLine(); }
    else if (mod && key === 'e') { ev.preventDefault(); setMode(false); }
    else if (mod && key === 's') { ev.preventDefault(); saveFile(false); }
    else if (mod && key === 'o') { ev.preventDefault(); openFile(); }
  });

  // ── 파일 ──

  const fileDirty = () => !!path && fileText !== null && text() !== fileText;
  const startDir = () => (path ? path.slice(0, path.lastIndexOf('\\')) : item.startCwd);
  const fileName = () => (clip(root.text.split('\n')[0].trim().replace(/[\\/:*?"<>|]/g, '_'), 60) || '브레인스토밍');

  async function openFile(given) {
    try {
      const file = await invoke('text_open', { path: given || null, start: startDir(), kind: 'mindmap' });
      if (!file) return;
      undo.push(snapshot());
      redo.length = 0;
      unread = false; // 내용을 통째로 바꾸므로, 읽지 못한 저장소의 내용을 기다리지 않는다.
      load(file.text);
      path = file.path;
      fileText = text();
      changed();
      fit(1);
      if (codeMode) jumpTo(1);
      setStatus(`열었습니다: ${file.path}`);
    } catch (err) {
      setStatus(`열지 못했습니다: ${err}`);
    }
  }

  async function saveFile(askName) {
    if (!settle('저장하지')) return;
    try {
      const now = text();
      const target = await invoke('text_save', {
        path: askName ? null : path || null, name: `${fileName()}.md`, start: startDir(), text: now, encoding: 'UTF-8', eol: 'LF', kind: 'mindmap',
      });
      if (!target) return;
      path = target;
      fileText = now;
      render();
      onChange();
      setStatus(`저장했습니다: ${target} (UTF-8)`);
    } catch (err) {
      setStatus(`저장하지 못했습니다: ${err}`);
    }
  }

  async function exportAs(kind) {
    if (editing) commitEdit();
    if (!settle('내보내지')) return;
    const made = exportSvg(parseOutline(text()).root, logic, family());
    render(); // 내보내려고 다시 잰 값이 화면 가지에 섞이지 않게 다시 그린다.
    const headers = (ext) => ({ 'x-name': encodeURIComponent(fileName()), 'x-ext': ext, 'x-start': encodeURIComponent(startDir()) });
    try {
      setStatus('내보내는 중…');
      let saved = null;
      if (kind === 'svg') {
        const xml = `<?xml version="1.0" encoding="UTF-8"?>\n${new XMLSerializer().serializeToString(made.svg)}\n`;
        saved = await invoke('mindmap_export', new TextEncoder().encode(xml), { headers: headers('svg') });
      } else if (kind === 'png') {
        saved = await invoke('mindmap_export', await svgToPng(made.svg, made.width, made.height), { headers: headers('png') });
      } else if (kind === 'item') {
        const image = await invoke('image_store', await svgToPng(made.svg, made.width, made.height), {
          headers: { 'x-name': encodeURIComponent(fileName()), 'x-ext': 'png' },
        });
        await api.addImages([image], { near: item, source: `브레인스토밍: ${fileName()}` });
        return setStatus('이 항목과 같은 묶음에 이미지 항목으로 넣었습니다.');
      } else {
        saved = await printPdf(made);
      }
      setStatus(saved ? `내보냈습니다: ${saved}` : '');
    } catch (err) {
      setStatus(`내보내지 못했습니다: ${err}`);
    }
  }

  let pdfTarget = null; // 자동 검증용: 저장할 곳을 묻지 않고 이 경로에 쓴다.
  /** 인쇄용 영역에 그림만 펼쳐 놓고 PDF로 만든다(글자가 그대로 살아 있는 PDF). */
  async function printPdf(made) {
    // 한 변이 한도를 넘는 큰 그림은 넘친 쪽이 잘려 나가므로, 같은 비율로 줄여 한 쪽에 담는다(벡터라 줄여도 또렷하다).
    const scale = Math.min(1, MAX_PDF_SIDE / Math.max(made.width, made.height));
    const width = Math.max(1, Math.floor(made.width * scale)), height = Math.max(1, Math.floor(made.height * scale));
    if (scale < 1) { made.svg.setAttribute('width', width); made.svg.setAttribute('height', height); }
    const box = document.getElementById('print');
    const page = document.createElement('style');
    page.textContent = `@page { size: ${width}px ${height}px; margin: 0; }`;
    box.replaceChildren(made.svg);
    document.head.append(page);
    try {
      return await invoke('mindmap_pdf', { name: fileName(), start: startDir(), width, height, target: pdfTarget });
    } finally {
      box.replaceChildren();
      page.remove();
    }
  }

  // ── 시작 ──

  const retheme = () => render();
  window.addEventListener('cypher-look', retheme);
  const resize = new ResizeObserver(() => { if (!fitted || auto) fit(fitMax); });
  resize.observe($stage);

  /** 밖에서(같은 그룹의 Claude 세션 등) 저장소의 내용을 바꿨으면 그 내용을 읽어 들인다. */
  async function reload() {
    if (disposed || editing) return;
    if (unread) return reread();
    if (text() !== stored) return;
    if (codeMode && $codeText.value !== codeApplied) return; // 코드 화면에서 고치는 중인 글은 덮어쓰지 않는다.
    const base = stored;
    const found = await invoke('mindmap_read', { id }).catch(() => null);
    // 읽는 사이에 여기서 고쳤으면(저장했든 아니든) 읽어 온 옛 내용으로 덮어쓰지 않는다.
    if (found == null || disposed || editing || found === base || stored !== base || text() !== base) return;
    if (codeMode && $codeText.value !== codeApplied) return;
    undo.push(snapshot());
    redo.length = 0;
    load(found);
    stored = found;
    changed();
  }

  /**
   * 시작할 때 읽지 못한 저장소를 다시 읽어 본다. 읽히면 그 내용으로 바꾸고(그 사이 여기서 고친 것은 되돌리기에 남긴다)
   * 자동 저장을 다시 켠다. 아직 읽지 못하면 그대로 둔다(다음에 이 항목을 볼 때 다시 해 본다).
   */
  async function reread() {
    if (codeMode && $codeText.value !== codeApplied) return;
    let found = null;
    try { found = await invoke('mindmap_read', { id }); } catch { return; }
    if (disposed || editing || !unread) return;
    if (codeMode && $codeText.value !== codeApplied) return;
    unread = false;
    stored = found ?? '';
    const parsed = parseOutline(found ?? '');
    if (found != null && toOutline(parsed.root, parsed.logic) !== text()) {
      undo.push(snapshot());
      redo.length = 0;
      load(found);
      changed();
      fit(1);
    } else if (text() !== stored) scheduleSave();
    setStatus('브레인스토밍의 내용을 다시 읽었습니다.');
  }

  showMode();
  const ready = (async () => {
    // 처음 넣을 내용이 주어졌으면(끌어다 놓기, Claude 세션이 만든 것) 그것으로, 아니면 자동 저장소에서 읽는다.
    const given = saved.text ?? null;
    let kept = null;
    if (given == null) {
      try {
        kept = await invoke('mindmap_read', { id }); // 아직 적어 둔 것이 없으면 null, 읽지 못하면 오류다.
      } catch (err) {
        // 읽지 못한 것은 내용이 없는 것과 다르다. 기본 내용으로 덮어쓰지 않게 자동 저장을 멈추고, 이 항목을 다시 볼 때 다시 읽는다.
        unread = true;
        setStatus(`브레인스토밍의 내용을 읽지 못했습니다: ${err} · 이 항목을 다시 보면 다시 읽습니다(그때까지 자동 저장하지 않습니다).`);
      }
    }
    load(given ?? kept ?? '# 중심 주제\n');
    stored = kept ?? ''; // 저장소에 실제로 들어 있는 내용. 지금 내용과 다르면 아래에서 바로 적는다.
    if (saved.code) {
      // 코드 화면을 보던 항목은 코드 화면으로 되살린다. 오류가 있어 반영하지 못한 글도 그대로 돌려놓는다.
      setMode(true, false);
      if (typeof saved.draft === 'string' && saved.draft !== codeApplied) { $codeText.value = saved.draft; checkCode(); }
    }
    if (path) {
      // 연결된 파일과 내용이 같은지 알아 둔다(다르면 제목에 * 표시).
      const file = await invoke('text_open', { path, start: null, kind: 'mindmap' }).catch(() => null);
      if (file) { const parsed = parseOutline(file.text); fileText = toOutline(parsed.root, parsed.logic); } else path = '';
    }
    render();
    fit(1);
    if (text() !== stored) await flush();
    onChange();
  })();

  return {
    ready,
    focus: () => { if (codeMode) $codeText.focus(); else if (!editing) $stage.focus(); if (!fitted) fit(1); reload(); },
    /** 사이드바에 보일 기본 이름: 중심 주제. */
    title: () => clip(root.text.split('\n')[0].trim(), 40) || '브레인스토밍',
    state: () => ({ map: id, path, code: codeMode, ...(codeMode && $codeText.value !== codeApplied ? { draft: $codeText.value } : {}) }),
    /** 그림 ↔ 코드 화면을 바꾼다. 코드에 오류가 있어 그림으로 바꾸지 못했으면 false. */
    setMode: (code) => setMode(!!code),
    /** 지금 코드의 오류: [{ line, message, fix }] */
    problems: () => codeErrors.map((error) => ({ ...error })),
    /** 저장할 것을 바로 저장한다(앱이 꺼지기 직전). 고치던 가지 글도 넣는다. */
    flush: () => { if (editing) commitEdit(); return flush(); },
    /** 개요 글. */
    text,
    /** 내용을 통째로 바꾼다(되돌릴 수 있다). 같은 그룹의 Claude 세션이 고칠 때 쓴다. */
    setText: (source) => {
      // 코드 화면에 오류가 있어 반영하지 못한 글이 있으면 덮어쓰지 않는다(고치던 글이 사라진다). 바꾸지 못했으면 false.
      if (codeMode) checkCode(); // 방금 쳐서 아직 검사하지 않은 올바른 글은 먼저 반영한다(오류로 잘못 보지 않게).
      if (codeMode && $codeText.value !== codeApplied) return false;
      // 고치던 가지 글은 버리지 않고 넣어 둔다: 그 글까지 든 모습이 되돌리기 목록에 남는다.
      if (editing) commitEdit();
      undo.push(snapshot());
      redo.length = 0;
      unread = false; // 내용을 통째로 바꾸므로, 읽지 못한 저장소의 내용을 기다리지 않는다.
      load(source);
      changed();
      fit(1);
      return true;
    },
    /** 자동 저장소의 번호(파일 이름). */
    store: () => id,
    file: () => path,
    openFile,
    /** PDF로 내보낸다. `target`을 주면 저장할 곳을 묻지 않는다(자동 검증용). */
    exportPdf: async (target) => { pdfTarget = target || null; try { await exportAs('pdf'); } finally { pdfTarget = null; } },
    close: async () => {
      if (editing) commitEdit(); // 고치던 가지 글도 보관되게 한다.
      if (codeMode) checkCode(); // 방금 쳐서 아직 검사하지 않은 올바른 글은 먼저 반영한다.
      if (codeMode && $codeText.value !== codeApplied
        && !confirm('코드에 오류가 있어 아직 그림에 반영되지 않은 글이 있습니다.\n닫으면 그 글은 사라집니다(마지막으로 올바르던 내용은 30일 동안 보관됩니다). 닫을까요?')) return false;
      await flush();
      return true;
    },
    dispose: () => {
      disposed = true;
      clearTimeout(saveTimer);
      clearTimeout(codeTimer);
      resize.disconnect();
      window.removeEventListener('cypher-look', retheme);
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      document.removeEventListener('keyup', onKeyUp);
      window.removeEventListener('blur', endSpace);
      press?.ghost?.remove(); // 끌던 중에 닫히면 문서에 붙여 둔 딱지를 남기지 않는다.
      // 닫은 항목의 내용은 바로 지우지 않고 보관해 둔다(30일).
      invoke('mindmap_remove', { id, keep: true }).catch(() => {});
    },
  };
}
