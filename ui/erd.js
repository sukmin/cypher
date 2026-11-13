// ERD 항목: 글(코드)로 쓴 개체 관계도를 그림으로 본다.
//
// 글의 표기는 Mermaid의 erDiagram을 따른다(GitHub·GitLab 등이 그대로 그려 주는 표기라 파일을 그대로 옮겨 쓸 수 있다).
// 읽기(parseErd), 배치(layoutErd), 그리기(drawErd)는 모두 이 파일에서 직접 한다. 바깥 라이브러리를 쓰지 않는다.
//
//   erDiagram
//       CUSTOMER ||--o{ ORDER : "주문한다"
//       CUSTOMER {
//           bigint id PK
//           varchar name "고객 이름"
//       }
//
// 내용은 글 그대로 자동 저장된다(erd.rs). 글에 잘못된 줄이 있어도 글은 저장되고, 읽을 수 있는 부분만 그린다.

let api = null; // { invoke, make, setStatus, showMenu, addImages }

export function initErd(given) {
  api = given;
}

// ───────────────────────── 글 읽기 ─────────────────────────

/** 관계선 끝의 표기 → 뜻. one: 정확히 하나, zero-one: 없거나 하나, many: 하나 이상, zero-many: 없거나 여럿. */
const CARDINALITY = { '||': 'one', '|o': 'zero-one', 'o|': 'zero-one', '}|': 'many', '|{': 'many', '}o': 'zero-many', 'o{': 'zero-many' };
const NAME = String.raw`(?:"[^"]+"|[\p{L}\p{N}_][\p{L}\p{N}_.\-]*)`;
/** 관계: A ||--o{ B : 설명 */
const RELATION = new RegExp(String.raw`^(${NAME})\s*(\|\||\|o|\}o|\}\|)(--|\.\.)(\|\||o\||o\{|\|\{)\s*(${NAME})\s*(?::\s*(.*))?$`, 'u');
/** 개체: 이름, 이름["보일 이름"], 이름 {  */
const ENTITY = new RegExp(String.raw`^(${NAME})(?:\s*\[(?:\s*"([^"]*)"\s*|([^\]"]*))\])?(?::::[\w-]+)?(?:\s*(\{)\s*(\})?)?$`, 'u');
/** 속성: 자료형 이름 PK,FK "설명" */
const ATTRIBUTE = /^([^\s("{}]+(?:\([^)]*\)[^\s("{}]*)*)\s+([^\s"{}]+?)(?:\s+((?:PK|FK|UK)(?:\s*,\s*(?:PK|FK|UK))*))?(?:\s+"([^"]*)")?$/i;
/** 그리는 데 쓰지 않는 Mermaid 구문(접근성 설명, 모양 지정). 오류로 치지 않고 넘어간다. */
const IGNORED = /^(?:accTitle|accDescr)\s*:|^(?:style|classDef|class)\s+[^{\s]/;
/** 방향: direction TB · BT · LR · RL */
const DIRECTION = /^direction\s+(\S+)$/;
const MAX_LINE = 600; // 이보다 긴 줄은 읽지 않는다(한 줄이 터무니없이 길면 읽는 데 오래 걸린다).

const unquote = (text) => (text.length >= 2 && text.startsWith('"') && text.endsWith('"') ? text.slice(1, -1) : text);
const clip = (text, max = 40) => (text.length > max ? `${text.slice(0, max)}…` : text);

/**
 * erDiagram 글을 읽는다.
 * → { title, direction('TB'|'LR'), entities: [{ id, label, attrs: [{ type, name, keys, comment, line }], line }],
 *     relations: [{ from, to, left, right, identifying, label, line }], errors: [{ line, message, fix }] }
 * 잘못된 줄은 errors에 줄 번호와 고치는 법을 적고 건너뛴다(읽을 수 있는 나머지는 그대로 읽는다).
 */
export function parseErd(source) {
  const lines = String(source ?? '').replace(/\r\n?/g, '\n').split('\n');
  const errors = [];
  const fail = (line, message, fix) => errors.push({ line, message, fix });
  const entities = new Map();
  const relations = [];
  const entity = (name, line) => {
    const id = unquote(name);
    if (!entities.has(id)) entities.set(id, { id, label: '', attrs: [], line, declared: false });
    return entities.get(id);
  };
  let title = '', direction = 'TB', started = false, open = null, openLine = 0;

  let at = 0;
  while (at < lines.length && !lines[at].trim()) at += 1;
  // 머리말: --- 와 --- 사이의 "title: 제목"
  if (lines[at]?.trim() === '---') {
    const from = at;
    at += 1;
    while (at < lines.length && lines[at].trim() !== '---') {
      const found = /^\s*title\s*:\s*(.*)$/.exec(lines[at]);
      if (found) title = unquote(found[1].trim());
      at += 1;
    }
    if (at >= lines.length) fail(from + 1, '머리말(---)이 닫히지 않았습니다', '제목 아래 줄에 ---를 한 번 더 적어 닫습니다');
    at += 1;
  }

  for (; at < lines.length; at += 1) {
    let text = lines[at].trim();
    const no = at + 1;
    if (!text || text.startsWith('%%')) continue;
    if (text.length > MAX_LINE) {
      fail(no, `줄이 너무 깁니다(${text.length}자)`, `한 줄은 ${MAX_LINE}자까지 읽습니다. 긴 설명은 줄이거나 나눠 적습니다`);
      continue;
    }
    let wrongStart = false;
    if (!started) {
      started = true;
      if (/^erDiagram(?:\s|$)/.test(text)) {
        text = text.slice('erDiagram'.length).trim();
        if (!text) continue;
      } else {
        fail(no, '첫 줄이 erDiagram이 아닙니다', '맨 위에 erDiagram 한 줄을 적습니다');
        wrongStart = true;
      }
    }
    if (text === 'erDiagram') continue; // 한 번 더 적은 것은 넘어간다.
    const relation = RELATION.exec(text);
    const head = relation ? null : ENTITY.exec(text);
    if (open) {
      if (text === '}') { open = null; continue; }
      if (relation || head?.[4]) {
        // 관계나 다른 개체가 시작됐다: 앞 개체를 닫는 }를 빠뜨린 것이다. 알리고, 이 줄은 블록 밖의 줄로 읽는다.
        fail(openLine, `개체 "${open.id}"의 { 가 닫히지 않았습니다`, `${no}번째 줄 앞에 }를 적어 닫습니다`);
        open = null;
      } else {
        const found = ATTRIBUTE.exec(text);
        if (!found) {
          fail(no, `속성을 읽지 못했습니다: ${clip(text)}`, '속성은 한 줄에 하나씩 "자료형 이름"으로 적고, 뒤에 PK·FK·UK와 "설명"을 붙일 수 있습니다 (예: bigint id PK "번호")');
          continue;
        }
        open.attrs.push({
          type: found[1], name: found[2],
          keys: (found[3] || '').toUpperCase().split(/\s*,\s*/).filter(Boolean),
          comment: found[4] ?? '', line: no,
        });
        continue;
      }
    }
    if (relation) {
      const from = entity(relation[1], no), to = entity(relation[5], no);
      relations.push({
        from: from.id, to: to.id, left: CARDINALITY[relation[2]], right: CARDINALITY[relation[4]],
        identifying: relation[3] === '--', label: unquote((relation[6] || '').trim()), line: no,
      });
      continue;
    }
    if (head) {
      const target = entity(head[1], no);
      if (!target.declared) { target.declared = true; target.line = no; }
      const alias = head[2] ?? head[3];
      if (alias != null && alias.trim()) target.label = alias.trim();
      if (head[4] && !head[5]) { open = target; openLine = no; }
      continue;
    }
    // 관계도 개체도 아닌 줄: 방향, 그리는 데 쓰지 않는 구문, 그 밖에는 알 수 없는 줄
    // (direction·class·style이라는 이름의 개체나 그 개체의 관계는 위에서 이미 읽혔다).
    const way = DIRECTION.exec(text)?.[1].toUpperCase();
    if (way) {
      if (way === 'TB' || way === 'BT') direction = 'TB';
      else if (way === 'LR' || way === 'RL') direction = 'LR';
      else fail(no, `방향을 읽지 못했습니다: ${clip(text)}`, 'direction TB(위에서 아래로) 또는 direction LR(왼쪽에서 오른쪽으로)로 적습니다');
      continue;
    }
    if (IGNORED.test(text) || wrongStart) continue; // 첫 줄이 틀렸다고 이미 알린 줄은 한 번 더 알리지 않는다.
    fail(no, `알 수 없는 줄입니다: ${clip(text)}`, '관계는 "A ||--o{ B : 설명", 개체는 "이름 {" 다음 줄부터 속성을 적고 "}"로 닫습니다');
  }
  if (open) fail(openLine, `개체 "${open.id}"의 { 가 닫히지 않았습니다`, '속성 아래 줄에 }를 적어 닫습니다');
  return { title, direction, entities: [...entities.values()], relations, errors };
}

// ───────────────────────── 배치 ─────────────────────────

const PAD = 10;       // 상자 안쪽 여백
const HEAD_H = 30;    // 개체 이름 줄의 높이
const ROW_H = 22;     // 속성 한 줄의 높이
const COL_GAP = 14;   // 속성의 칸 사이
const MAX_COMMENT = 28; // 상자에 보일 설명의 글자 수(나머지는 풍선말)
const SHELF = 1500;   // 서로 이어지지 않은 덩어리들을 옆으로 늘어놓는 너비 한도

const END_PITCH = 24; // 한 변에 붙는 관계선 사이의 간격(끝 표시의 너비 14 + 여유)

/**
 * 층 a의 상자에서 층 b의 상자로 가는 관계선이 a의 어느 변에 붙는지. before: 층이 쌓이는 방향의 앞쪽 변(TB면 위, LR이면 왼쪽),
 * after: 뒤쪽 변. 같은 층끼리의 관계는 가운데 개체에서 먼 쪽 변에 붙어 바깥으로 돌아간다.
 */
const sideOf = (a, b) => (a === b ? (a > 0 ? 'after' : 'before') : b > a ? 'after' : 'before');

/** 개체 상자의 크기와 칸 위치. measure(text, kind) → 너비(px). kind: head | name | type | key | comment */
function boxOf(entity, measure) {
  const widest = (pick, kind) => Math.max(0, ...entity.attrs.map((attr) => { const text = pick(attr); return text ? measure(text, kind) : 0; }));
  const keyW = widest((attr) => attr.keys.join(','), 'key');
  const nameW = widest((attr) => attr.name, 'name');
  const typeW = widest((attr) => attr.type, 'type');
  const commentW = widest((attr) => clip(attr.comment, MAX_COMMENT), 'comment');
  const nameX = PAD + (keyW ? keyW + 8 : 0);
  const typeX = nameX + nameW + COL_GAP;
  const commentX = typeX + typeW + COL_GAP;
  const body = (commentW ? commentX + commentW : typeX + typeW) + PAD;
  const w = Math.ceil(Math.max(110, measure(entity.label || entity.id, 'head') + PAD * 2, entity.attrs.length ? body : 0));
  const h = HEAD_H + (entity.attrs.length ? entity.attrs.length * ROW_H + 6 : 0);
  return { id: entity.id, w, h, x: 0, y: 0, nameX, typeX, commentX };
}

/**
 * 개체들의 자리를 정한다. → { boxes: Map(id → 상자), width, height }
 *
 * 관계로 이어진 덩어리마다, 관계가 가장 많은 개체를 가운데에 두고 거기서 몇 걸음 떨어졌는지로 층을 나눈다.
 * 가운데 개체를 빼면 서로 끊어지는 가지들은 양쪽(위·아래, LR이면 왼쪽·오른쪽)으로 나눠 놓아 한 층이 너무 길어지지 않게 한다.
 * 이렇게 하면 모든 관계선이 같은 층이나 바로 옆 층의 개체끼리만 이어진다.
 */
export function layoutErd(model, measure) {
  const lr = model.direction === 'LR';
  const FLOW_GAP = lr ? 124 : 100; // 층과 층 사이(관계선, 그 끝 표시, 같은 층끼리의 관계가 돌아가는 자리)
  const CROSS_GAP = lr ? 30 : 46;  // 한 층 안의 상자 사이
  const boxes = new Map(model.entities.map((entity) => [entity.id, boxOf(entity, measure)]));
  const order = new Map(model.entities.map((entity, index) => [entity.id, index]));
  const near = new Map(model.entities.map((entity) => [entity.id, new Set()]));
  for (const relation of model.relations) {
    if (relation.from === relation.to) continue;
    near.get(relation.from).add(relation.to);
    near.get(relation.to).add(relation.from);
  }
  // 층이 쌓이는 방향(flow)과 층 안에서 늘어서는 방향(cross)의 크기
  const flowSize = (box) => (lr ? box.w : box.h);
  const crossSize = (box) => (lr ? box.h : box.w);

  /** start에서 이어진 것들(skip은 지나가지 않는다)을 가까운 순서로. → Map(id → 걸음 수) */
  const reach = (start, skip, seen) => {
    const steps = new Map([[start, 0]]);
    const queue = [start];
    seen.add(start);
    for (let head = 0; head < queue.length; head += 1) {
      const sorted = [...near.get(queue[head])].sort((a, b) => order.get(a) - order.get(b));
      for (const next of sorted) {
        if (next === skip || seen.has(next)) continue;
        seen.add(next);
        steps.set(next, steps.get(queue[head]) + 1);
        queue.push(next);
      }
    }
    return steps;
  };

  /** 이어진 덩어리 하나를 (0,0)에서 시작하게 놓고 그 크기를 돌려준다. */
  const place = (ids) => {
    const root = ids.reduce((best, id) => (near.get(id).size > near.get(best).size ? id : best), ids[0]);
    // 가운데 개체를 빼면 끊어지는 가지들
    const seen = new Set([root]);
    const branches = [];
    for (const id of [...near.get(root)].sort((a, b) => order.get(a) - order.get(b))) {
      if (!seen.has(id)) branches.push([...reach(id, root, seen).keys()]);
    }
    // 큰 가지부터 개체 수가 적은 쪽에 놓는다(가지가 하나뿐이면 한쪽으로만 뻗는다).
    const side = new Map();
    const weight = { 1: 0, [-1]: 0 };
    branches.map((branch, index) => ({ branch, index })).sort((a, b) => b.branch.length - a.branch.length || a.index - b.index)
      .forEach(({ branch, index }) => {
        const to = weight[1] <= weight[-1] ? 1 : -1;
        weight[to] += branch.length;
        for (const id of branch) side.set(id, { to, branch: index });
      });
    const steps = reach(root, null, new Set());
    const layers = new Map([[0, [root]]]);
    for (const id of ids) {
      if (id === root) continue;
      const layer = side.get(id).to * steps.get(id);
      boxes.get(id).layer = layer;
      if (!layers.has(layer)) layers.set(layer, []);
      layers.get(layer).push(id);
    }
    boxes.get(root).layer = 0;
    // 한 변에 관계선이 많이 붙는 상자는 그 변을 늘린다(선 끝의 표시가 서로 겹치지 않게).
    const inBlock = new Set(ids);
    const ends = new Map(ids.map((id) => [id, { before: 0, after: 0 }]));
    for (const relation of model.relations) {
      if (relation.from === relation.to || !inBlock.has(relation.from)) continue;
      const a = boxes.get(relation.from).layer, b = boxes.get(relation.to).layer;
      ends.get(relation.from)[sideOf(a, b)] += 1;
      ends.get(relation.to)[sideOf(b, a)] += 1;
    }
    for (const id of ids) {
      const need = Math.max(ends.get(id).before, ends.get(id).after) * END_PITCH + 16;
      const box = boxes.get(id);
      if (lr) box.h = Math.max(box.h, need); else box.w = Math.max(box.w, need);
    }
    // 층 안의 순서와 자리: 가운데에서 가까운 층부터, 안쪽 층에서 이어진 개체들의 가운데 가까이에 놓는다.
    const center = new Map([[root, 0]]); // 층 안에서의 가운데 자리(cross 방향)
    const span = Math.max(...layers.keys(), ...[...layers.keys()].map((layer) => -layer));
    for (let depth = 1; depth <= span; depth += 1) {
      for (const sign of [1, -1]) {
        const list = layers.get(sign * depth);
        if (!list) continue;
        const wish = new Map(list.map((id) => {
          const inner = [...near.get(id)].filter((other) => boxes.get(other).layer === sign * (depth - 1));
          return [id, inner.length ? inner.reduce((sum, other) => sum + center.get(other), 0) / inner.length : 0];
        }));
        list.sort((a, b) => wish.get(a) - wish.get(b) || side.get(a).branch - side.get(b).branch || order.get(a) - order.get(b));
        // 바라는 자리에 놓되 겹치면 옆으로 밀고, 민 만큼의 평균을 층 전체에서 되돌려 한쪽으로 쏠리지 않게 한다.
        let edge = -Infinity, pushed = 0;
        const spots = list.map((id) => {
          const half = crossSize(boxes.get(id)) / 2;
          const spot = Math.max(wish.get(id), edge + half);
          pushed += spot - wish.get(id);
          edge = spot + half + CROSS_GAP;
          return spot;
        });
        list.forEach((id, index) => {
          center.set(id, spots[index] - pushed / list.length);
          boxes.get(id).order = index;
        });
      }
    }
    boxes.get(root).order = 0;
    // 층이 쌓이는 방향의 자리: 층마다 가장 큰 상자만큼 차지하고, 상자는 가운데 개체 쪽 변에 맞춰 줄을 세운다
    // (그 변들이 한 줄에 있어야 안쪽 층에서 오는 관계선이 이웃 상자를 지나가지 않는다).
    let flow = 0;
    for (const layer of [...layers.keys()].sort((a, b) => a - b)) {
      const list = layers.get(layer);
      const thick = Math.max(...list.map((id) => flowSize(boxes.get(id))));
      for (const id of list) {
        const box = boxes.get(id);
        const along = flow + (layer > 0 ? 0 : layer < 0 ? thick - flowSize(box) : (thick - flowSize(box)) / 2);
        const across = center.get(id) - crossSize(box) / 2;
        box.x = lr ? along : across;
        box.y = lr ? across : along;
        box.band = thick;
      }
      flow += thick + FLOW_GAP;
    }
    const list = ids.map((id) => boxes.get(id));
    const left = Math.min(...list.map((box) => box.x)), top = Math.min(...list.map((box) => box.y));
    for (const box of list) { box.x -= left; box.y -= top; }
    return { list, w: Math.max(...list.map((box) => box.x + box.w)), h: Math.max(...list.map((box) => box.y + box.h)) };
  };

  // 이어진 덩어리들을 찾는다. 관계가 없는 개체들은 한데 모아 맨 뒤에 늘어놓는다.
  const seen = new Set();
  const blocks = [];
  const alone = [];
  for (const entity of model.entities) {
    if (seen.has(entity.id)) continue;
    const ids = [...reach(entity.id, null, seen).keys()];
    if (ids.length === 1) alone.push(boxes.get(entity.id)); else blocks.push(place(ids));
  }
  if (alone.length) {
    let x = 0, y = 0, tall = 0, wide = 0;
    for (const box of alone) {
      if (x && x + box.w > SHELF) { x = 0; y += tall + 30; tall = 0; }
      box.x = x; box.y = y; box.layer = 0; box.order = 0; box.band = lr ? box.w : box.h;
      x += box.w + 30;
      tall = Math.max(tall, box.h);
      wide = Math.max(wide, x - 30);
    }
    blocks.push({ list: alone, w: wide, h: y + tall });
  }
  // 덩어리들을 왼쪽에서 오른쪽으로 늘어놓고, 너비 한도를 넘으면 아랫줄로 내린다.
  let x = 0, y = 0, tall = 0, width = 0;
  blocks.forEach((block, index) => {
    if (x && x + block.w > Math.max(SHELF, block.w)) { x = 0; y += tall + 70; tall = 0; }
    for (const box of block.list) { box.x += x; box.y += y; box.block = index; }
    x += block.w + 70;
    tall = Math.max(tall, block.h);
    width = Math.max(width, x - 70);
  });
  return { boxes, width, height: y + tall, lr, flowGap: FLOW_GAP };
}

// ───────────────────────── 그리기 ─────────────────────────

const SVG = 'http://www.w3.org/2000/svg';
const MARGIN = 28; // 그림 둘레의 여백

function svg(tag, attrs = {}, text) {
  const node = document.createElementNS(SVG, tag);
  for (const [key, value] of Object.entries(attrs)) if (value != null && value !== false) node.setAttribute(key, String(value));
  if (text != null) node.textContent = text;
  return node;
}

const round = (value) => Math.round(value * 10) / 10;


/**
 * 관계선 끝의 표시(까마귀발 표기). at: 상자 테두리의 점, (ux, uy): 상자에서 멀어지는 방향.
 * one ‖ · zero-one ○| · many >| · zero-many >○
 */
function endMark(group, kind, at, ux, uy, theme) {
  const nx = -uy, ny = ux; // 선에 수직인 방향
  const point = (along, across) => `${round(at.x + ux * along + nx * across)},${round(at.y + uy * along + ny * across)}`;
  const line = (d) => group.append(svg('path', { d, fill: 'none', stroke: theme.line, 'stroke-width': 1.4, 'stroke-linecap': 'round' }));
  const bar = (along) => line(`M${point(along, -7)} L${point(along, 7)}`);
  const ring = (along) => group.append(svg('circle', {
    cx: round(at.x + ux * along), cy: round(at.y + uy * along), r: 4.2, fill: theme.bg, stroke: theme.line, 'stroke-width': 1.4,
  }));
  if (kind === 'one') { bar(8); bar(13); }
  else if (kind === 'zero-one') { bar(8); ring(17); }
  else {
    // 까마귀발: 조금 떨어진 한 점에서 상자 테두리의 세 곳으로 벌어진다.
    line(`M${point(14, 0)} L${point(0, -7)} M${point(14, 0)} L${point(0, 7)}`);
    if (kind === 'many') bar(18); else ring(21);
  }
}

/**
 * 관계선의 길을 정한다. → { routes: [{ relation, d(SVG 경로), start: { at, dir }, end: { at, dir }, label: { x, y, w } | null }],
 *   bounds: { x0, y0, x1, y1 }(상자·선·설명을 모두 담는 테두리) }
 *
 * 선은 상자 가운데끼리 잇지 않고, 상자의 마주 보는 변에 자리를 나눠 붙인다(한 변에 여럿이 붙으면 고르게 벌린다).
 * - 이웃한 층끼리: 한쪽의 뒤쪽 변에서 나와 다른 쪽의 앞쪽 변으로 들어가는 곡선. 층 사이의 빈 띠 안에서만 지나간다.
 * - 같은 층끼리: 둘 다 가운데 개체에서 먼 쪽 변에서 나와, 그 층의 바깥으로 돌아 들어간다.
 * - 자기 자신과: 상자 옆으로 나갔다 돌아오는 고리.
 */
export function routeErd(model, layout, measure) {
  const { boxes, lr, flowGap = 100 } = layout;
  // f: 층이 쌓이는 방향, c: 층 안에서 늘어서는 방향
  const span = (box) => (lr
    ? { f0: box.x, f1: box.x + box.w, c0: box.y, c1: box.y + box.h }
    : { f0: box.y, f1: box.y + box.h, c0: box.x, c1: box.x + box.w });
  const xy = (f, c) => (lr ? { x: f, y: c } : { x: c, y: f });
  const bounds = { x0: Infinity, y0: Infinity, x1: -Infinity, y1: -Infinity };
  const cover = (x, y, pad = 0) => {
    bounds.x0 = Math.min(bounds.x0, x - pad); bounds.y0 = Math.min(bounds.y0, y - pad);
    bounds.x1 = Math.max(bounds.x1, x + pad); bounds.y1 = Math.max(bounds.y1, y + pad);
  };
  // 층마다 바깥쪽 끝(같은 층끼리의 관계가 돌아 나가는 기준)
  const edges = new Map();
  for (const box of boxes.values()) {
    cover(box.x, box.y); cover(box.x + box.w, box.y + box.h);
    const key = `${box.block}/${box.layer}`, at = span(box);
    const edge = edges.get(key) || { before: Infinity, after: -Infinity };
    edge.before = Math.min(edge.before, at.f0);
    edge.after = Math.max(edge.after, at.f1);
    edges.set(key, edge);
  }

  // 상자의 변마다 붙을 선을 모아, 상대 개체가 놓인 순서대로 자리를 나눈다.
  const slots = new Map();
  const seat = (box, side, other, index, end) => {
    const key = `${box.id}/${side}`;
    if (!slots.has(key)) slots.set(key, []);
    const at = span(other);
    slots.get(key).push({ index, end, order: (at.c0 + at.c1) / 2 });
  };
  model.relations.forEach((relation, index) => {
    const a = boxes.get(relation.from), b = boxes.get(relation.to);
    if (!a || !b || a === b) return;
    seat(a, sideOf(a.layer, b.layer), b, index, 'from');
    seat(b, sideOf(b.layer, a.layer), a, index, 'to');
  });
  const seats = new Map(); // `${관계 순번}/${from|to}` → 그 변에서의 자리(c)
  for (const [key, list] of slots) {
    const at = span(boxes.get(key.slice(0, key.lastIndexOf('/'))));
    list.sort((p, q) => p.order - q.order || p.index - q.index);
    list.forEach((slot, nth) => seats.set(`${slot.index}/${slot.end}`, at.c0 + ((nth + 1) * (at.c1 - at.c0)) / (list.length + 1)));
  }

  const pairs = new Map(), pairUsed = new Map(), loops = new Map(), loopUsed = new Map(), arcs = new Map();
  for (const relation of model.relations) {
    if (relation.from === relation.to) loops.set(relation.from, (loops.get(relation.from) || 0) + 1);
    const key = [relation.from, relation.to].sort().join('\n');
    pairs.set(key, (pairs.get(key) || 0) + 1);
  }
  const cubic = (p0, p1, p2, p3, t) => {
    const u = 1 - t;
    return {
      f: u * u * u * p0.f + 3 * u * u * t * p1.f + 3 * u * t * t * p2.f + t * t * t * p3.f,
      c: u * u * u * p0.c + 3 * u * u * t * p1.c + 3 * u * t * t * p2.c + t * t * t * p3.c,
    };
  };
  const routes = [];
  model.relations.forEach((relation, index) => {
    const a = boxes.get(relation.from), b = boxes.get(relation.to);
    if (!a || !b) return;
    const ga = span(a), gb = span(b);
    let p0, p1, p2, p3, dir0, dir3, labelAt = 0.5, labelShift = { f: 0, c: 0 }, labelLeft = false;
    if (a === b) {
      // 자기 자신과의 관계: 상자 옆(TB면 오른쪽, LR이면 아래)으로 나갔다 돌아오는 고리
      const nth = loopUsed.get(a.id) || 0;
      loopUsed.set(a.id, nth + 1);
      const reach = 38 + nth * 26;
      const inset = Math.min((ga.f1 - ga.f0) * 0.3, 14) + nth * 4;
      const from = ga.f0 + inset, to = Math.max(ga.f1 - inset, from + 10);
      p0 = { f: from, c: ga.c1 }; p3 = { f: to, c: ga.c1 };
      p1 = { f: from - 6, c: ga.c1 + reach }; p2 = { f: to + 6, c: ga.c1 + reach };
      dir0 = dir3 = { f: 0, c: 1 };
      // 설명은 고리 바깥에 둔다. 고리가 여럿이면 설명끼리 겹치지 않게 어긋나게 놓는다.
      labelShift = lr ? { f: 0, c: reach * 0.25 + 14 } : { f: (nth - (loops.get(a.id) - 1) / 2) * 20, c: reach * 0.25 + 8 };
      labelLeft = !lr;
    } else {
      const sideA = sideOf(a.layer, b.layer), sideB = sideOf(b.layer, a.layer);
      const wayA = sideA === 'after' ? 1 : -1, wayB = sideB === 'after' ? 1 : -1;
      p0 = { f: sideA === 'after' ? ga.f1 : ga.f0, c: seats.get(`${index}/from`) };
      p3 = { f: sideB === 'after' ? gb.f1 : gb.f0, c: seats.get(`${index}/to`) };
      dir0 = { f: wayA, c: 0 }; dir3 = { f: wayB, c: 0 };
      if (a.layer === b.layer) {
        // 같은 층: 그 층의 바깥쪽 끝을 넘어 돌아간다. 여럿이면 조금씩 더 멀리 돌려 겹치지 않게 한다.
        const key = `${a.block}/${a.layer}`;
        const nth = arcs.get(key) || 0;
        arcs.set(key, nth + 1);
        const bulge = Math.min(40 + nth * 14, flowGap - 22);
        const peak = (wayA > 0 ? edges.get(key).after : edges.get(key).before) + wayA * bulge;
        const control = (peak - 0.125 * (p0.f + p3.f)) / 0.75; // 곡선의 가운데가 peak에 닿게 하는 조절점
        p1 = { f: control, c: p0.c }; p2 = { f: control, c: p3.c };
      } else {
        const reach = Math.max(34, Math.abs(p3.f - p0.f) * 0.5);
        p1 = { f: p0.f + wayA * reach, c: p0.c }; p2 = { f: p3.f + wayB * reach, c: p3.c };
        // 같은 두 개체를 잇는 관계가 여럿이면 설명이 겹치지 않게 선을 따라 어긋나게 놓는다.
        const key = [relation.from, relation.to].sort().join('\n');
        const nth = pairUsed.get(key) || 0;
        pairUsed.set(key, nth + 1);
        labelAt = Math.min(0.8, Math.max(0.2, 0.5 + (nth - (pairs.get(key) - 1) / 2) * 0.27));
      }
    }
    for (let step = 0; step <= 12; step += 1) {
      const at = cubic(p0, p1, p2, p3, step / 12), point = xy(at.f, at.c);
      cover(point.x, point.y, 9); // 선 끝의 표시가 선 양옆으로 7만큼 나온다.
    }
    const point = (p) => { const at = xy(p.f, p.c); return `${round(at.x)},${round(at.y)}`; };
    const unit = (dir) => (lr ? { x: dir.f, y: dir.c } : { x: dir.c, y: dir.f });
    let label = null;
    if (relation.label) {
      const w = measure(relation.label, 'label') + 10;
      const mid = cubic(p0, p1, p2, p3, labelAt);
      const at = xy(mid.f + labelShift.f, mid.c + labelShift.c);
      label = { x: labelLeft ? at.x + w / 2 : at.x, y: at.y, w };
      cover(label.x - w / 2, label.y - 9); cover(label.x + w / 2, label.y + 9);
    }
    routes.push({
      relation, d: `M${point(p0)} C${point(p1)} ${point(p2)} ${point(p3)}`,
      start: { at: xy(p0.f, p0.c), dir: unit(dir0) }, end: { at: xy(p3.f, p3.c), dir: unit(dir3) }, label,
    });
  });
  if (!Number.isFinite(bounds.x0)) Object.assign(bounds, { x0: 0, y0: 0, x1: 0, y1: 0 });
  return { routes, bounds };
}

/**
 * 그림을 그린다. → { node: <g>(상자와 관계선), width, height(둘레 여백을 넣은 그림의 크기) }
 * theme: { bg, panel, head, line, text, dim, accent, key, family, mono }
 */
export function drawErd(model, layout, theme, measure) {
  const { routes, bounds } = routeErd(model, layout, measure);
  const root = svg('g', { transform: `translate(${round(MARGIN - bounds.x0)},${round(MARGIN - bounds.y0)})`, 'font-family': theme.family });
  const lines = svg('g'), marks = svg('g'), labels = svg('g'), cards = svg('g');
  const { boxes } = layout;
  for (const { relation, d, start, end, label } of routes) {
    const tag = { class: 'erd-rel', 'data-from': relation.from, 'data-to': relation.to, 'data-line': relation.line };
    const line = svg('g', tag);
    line.append(svg('path', { d, fill: 'none', stroke: theme.line, 'stroke-width': 1.4, 'stroke-dasharray': relation.identifying ? null : '5 4' }));
    lines.append(line);
    const ends = svg('g', tag);
    endMark(ends, relation.left, start.at, start.dir.x, start.dir.y, theme);
    endMark(ends, relation.right, end.at, end.dir.x, end.dir.y, theme);
    marks.append(ends);
    if (label) {
      const text = svg('g', tag);
      text.append(
        svg('rect', { x: round(label.x - label.w / 2), y: round(label.y - 9), width: round(label.w), height: 18, rx: 4, fill: theme.bg }),
        svg('text', { x: round(label.x), y: round(label.y + 4), 'text-anchor': 'middle', 'font-size': 11, fill: theme.dim }, relation.label),
      );
      labels.append(text);
    }
  }

  for (const entity of model.entities) {
    const box = boxes.get(entity.id);
    const card = svg('g', { class: 'erd-entity', 'data-id': entity.id, 'data-line': entity.line, transform: `translate(${round(box.x)},${round(box.y)})` });
    card.append(svg('rect', { width: box.w, height: box.h, rx: 6, fill: theme.panel, stroke: theme.line, 'stroke-width': 1.2 }));
    // 이름 줄: 위쪽 모서리만 둥근 바탕
    card.append(svg('path', {
      d: `M0,${HEAD_H} V6 Q0,0 6,0 H${box.w - 6} Q${box.w},0 ${box.w},6 V${HEAD_H} Z`, fill: theme.head, stroke: theme.line, 'stroke-width': 1.2,
    }));
    const name = svg('text', { x: box.w / 2, y: 20, 'text-anchor': 'middle', 'font-size': 13, 'font-weight': 600, fill: theme.text }, entity.label || entity.id);
    if (entity.label && entity.label !== entity.id) name.append(svg('title', {}, entity.id));
    card.append(name);
    entity.attrs.forEach((attr, index) => {
      const y = HEAD_H + 3 + index * ROW_H;
      const row = svg('g', { class: 'erd-attr', 'data-line': attr.line });
      if (index) row.append(svg('path', { d: `M1,${y} H${box.w - 1}`, stroke: theme.line, 'stroke-width': 0.6, opacity: 0.5 }));
      const base = y + 15;
      const primary = attr.keys.includes('PK');
      if (attr.keys.length) {
        row.append(svg('text', { x: PAD, y: base, 'font-size': 10, 'font-weight': 600, fill: primary ? theme.accent : theme.key }, attr.keys.join(',')));
      }
      row.append(svg('text', { x: box.nameX, y: base, 'font-size': 12, 'font-weight': primary ? 600 : null, fill: theme.text }, attr.name));
      row.append(svg('text', { x: box.typeX, y: base, 'font-size': 12, 'font-family': theme.mono, fill: theme.dim }, attr.type));
      if (attr.comment) {
        row.append(svg('text', { x: box.commentX, y: base, 'font-size': 11, fill: theme.dim, opacity: 0.85 }, clip(attr.comment, MAX_COMMENT)));
        row.append(svg('title', {}, attr.comment));
      }
      card.append(row);
    });
    cards.append(card);
  }
  root.append(lines, marks, cards, labels);
  return { node: root, width: bounds.x1 - bounds.x0 + MARGIN * 2, height: bounds.y1 - bounds.y0 + MARGIN * 2 };
}

/** 따로 쓸 수 있는 SVG 그림(내보내기용): 크기와 바탕색이 들어 있다. → { svg, width, height } */
function standalone(model, layout, theme, measure) {
  const drawn = drawErd(model, layout, theme, measure);
  const width = Math.ceil(drawn.width), height = Math.ceil(drawn.height);
  const node = svg('svg', { xmlns: SVG, width, height, viewBox: `0 0 ${width} ${height}` });
  node.append(svg('rect', { width, height, fill: theme.bg }), drawn.node);
  return { svg: node, width, height };
}

/** SVG 그림을 PNG로 바꾼다(큰 그림은 배율을 낮춘다). */
async function svgToPng(node, width, height) {
  const scale = Math.min(2, 16000 / Math.max(width, height));
  const image = new Image();
  image.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(new XMLSerializer().serializeToString(node))}`;
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

const newId = () => `e${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
const MIN_ZOOM = 0.03, MAX_ZOOM = 3;
const DRAW_MS = 200;          // 치는 동안 그림을 다시 그리기까지 기다리는 시간
const SAVE_MS = 600;          // 자동 저장까지 기다리는 시간
const SAVE_RETRY_MS = 5000;   // 자동 저장에 실패했을 때 다시 해 보기까지의 시간

/** 새 ERD 항목의 처음 내용. 표기를 한눈에 볼 수 있는 작은 예시다. */
const SAMPLE = `erDiagram
    CUSTOMER ||--o{ ORDER : "주문한다"
    ORDER ||--|{ ORDER_ITEM : "포함한다"
    PRODUCT ||--o{ ORDER_ITEM : "담긴다"

    CUSTOMER {
        bigint id PK
        varchar name "고객 이름"
        varchar email UK
    }
    ORDER {
        bigint id PK
        bigint customer_id FK
        datetime ordered_at "주문한 때"
    }
    ORDER_ITEM {
        bigint order_id PK,FK
        bigint product_id PK,FK
        int quantity
    }
    PRODUCT {
        bigint id PK
        varchar name
        int price
    }
`;

/** [문법] 단추가 보여 주는 쓰는 법. */
const GUIDE = [
  '첫 줄: erDiagram',
  '관계: A ||--o{ B : "설명"',
  '  왼쪽 끝  ||  하나     |o  없거나 하나     }|  하나 이상     }o  없거나 여럿',
  '  오른쪽 끝  ||  하나     o|  없거나 하나     |{  하나 이상     o{  없거나 여럿',
  '  선  --  식별 관계(실선)     ..  비식별 관계(점선)',
  '개체: 이름 {  다음 줄부터 속성,  } 로 닫기',
  '속성: 자료형 이름 PK,FK,UK "설명"   (예: bigint id PK "번호")',
  '보일 이름: CUSTOMER["고객"] {',
  '방향: direction LR (왼쪽→오른쪽) · direction TB (위→아래, 기본)',
  '제목: 맨 위에 ---  /  title: 제목  /  ---',
  '주석: %% 로 시작하는 줄',
];

/**
 * ERD 항목의 화면을 host 안에 만든다.
 * saved: { erd(저장 번호), code(코드 칸을 열어 두었는지), text(처음 넣을 내용) }
 */
export function createErd(item, host, saved = {}, onChange = () => {}) {
  const { invoke, make, setStatus, showMenu } = api;
  const id = typeof saved.erd === 'string' && /^[a-z0-9]{1,40}$/i.test(saved.erd) ? saved.erd : newId();
  let stored = '';             // 저장소에 실제로 들어 있는 글
  let unread = false;          // 시작할 때 저장소를 읽지 못했다(그동안은 자동 저장하지 않는다)
  let model = parseErd('');
  let size = null;             // 지금 그림의 크기(둘레 여백 포함). 그릴 것이 없으면 null
  let view = { x: 0, y: 0, k: 1 }; // 그림의 이동·배율
  let auto = true;             // 사용자가 그림을 옮기거나 확대하지 않았다(다시 그리거나 창 크기가 바뀌면 다시 맞춘다)
  let drawTimer = 0, saveTimer = 0;
  let disposed = false;
  let codeShown = saved.code !== false;
  let drag = null;

  const button = (text, tip, action) => {
    const b = make('button', '', text);
    b.title = tip;
    b.onclick = action;
    return b;
  };
  const $code = make('textarea', 'erd-code');
  Object.assign($code, { spellcheck: false, autocomplete: 'off', wrap: 'off' });
  $code.placeholder = 'erDiagram\n    A ||--o{ B : "설명"';
  const $problems = make('div', 'erd-problems');
  const $codeView = make('div', 'erd-codeview');
  $codeView.append($code, $problems);
  const $stage = make('div', 'erd-stage');
  $stage.tabIndex = 0;
  const $svg = svg('svg', { class: 'erd-svg' });
  const $view = svg('g');
  $svg.append($view);
  const $empty = make('div', 'erd-empty');
  $stage.append($svg, $empty);
  const $info = make('span', 'doc-title');
  const $toggle = button('코드', '코드 칸 보이기/숨기기 (Ctrl+E)', () => showCode(!codeShown));
  const bar = make('div', 'doc-bar');
  bar.append(
    $toggle,
    button('맞춤', '전체 맞춤 (Ctrl+0)', () => fit()),
    button('−', '축소 (Ctrl+휠)', () => zoomBy(1 / 1.2)),
    button('+', '확대 (Ctrl+휠)', () => zoomBy(1.2)),
    $info,
    button('문법', 'erDiagram 쓰는 법과 예시', (ev) => showMenu(ev, [
      ...GUIDE.map((caption) => ({ caption })),
      { label: '예시로 바꾸기 (지금 글은 Ctrl+Z로 되살립니다)', action: () => { replaceText(SAMPLE, true); $code.focus(); } },
    ])),
    button('복사', '코드 복사', () => invoke('clipboard_write', { text: $code.value }).then(() => setStatus('ERD 코드를 복사했습니다.'))),
    button('내보내기', 'PNG·SVG로 저장 · 이미지 항목으로 넣기', (ev) => showMenu(ev, [
      { label: 'PNG로 저장…', action: () => exportAs('png') },
      { label: 'SVG로 저장…', action: () => exportAs('svg') },
      { label: '이미지 항목으로 넣기 (같은 묶음)', action: () => exportAs('item') },
    ])),
  );
  const body = make('div', 'erd-body');
  body.append($codeView, $stage);
  const box = make('div', 'doc erd');
  box.append(bar, body);
  host.append(box);

  // 글자 너비는 화면의 글꼴로 잰다(테마·글꼴이 바뀌면 다시 잰다).
  const canvas = document.createElement('canvas').getContext('2d');
  let fonts = null;
  const readTheme = () => {
    const style = getComputedStyle($stage);
    const color = (name, fallback) => style.getPropertyValue(name).trim() || fallback;
    const family = style.fontFamily || 'sans-serif';
    const mono = color('--mono-font', 'monospace');
    fonts = {
      head: `600 13px ${family}`, name: `600 12px ${family}`, type: `12px ${mono}`,
      key: `600 10px ${family}`, comment: `11px ${family}`, label: `11px ${family}`,
    };
    // 바탕으로 칠하는 색은 불투명해야 한다. 반투명한 바탕(매트릭스 테마)을 그대로 쓰면 설명 뒤로 관계선이 비치고,
    // 내보낸 그림의 바탕이 비친다. 검은 바탕 위에 올렸을 때의 색으로 바꿔 쓴다.
    const solid = (value) => {
      canvas.fillStyle = '#000';
      canvas.fillRect(0, 0, 1, 1);
      canvas.fillStyle = value;
      canvas.fillRect(0, 0, 1, 1);
      const [r, g, b] = canvas.getImageData(0, 0, 1, 1).data;
      return `rgb(${r},${g},${b})`;
    };
    return {
      bg: solid(color('--field', color('--bg', '#16181d'))), panel: solid(color('--panel', '#1e2127')), head: solid(color('--group', '#2a2e36')),
      line: color('--idle', '#7f848e'), text: color('--text', '#d7dae0'), dim: color('--dim', '#9aa0aa'),
      accent: color('--accent', '#e5c07b'), key: color('--attention', '#61afef'), family, mono,
    };
  };
  const measure = (text, kind) => {
    canvas.font = fonts[kind] || fonts.name;
    return canvas.measureText(text).width;
  };

  const applyView = () => $view.setAttribute('transform', `translate(${round(view.x)},${round(view.y)}) scale(${view.k})`);

  /** 그림 전체가 보이게 맞춘다(작은 그림은 키우지 않는다). */
  function fit() {
    auto = true;
    if (!size || !$stage.clientWidth) return;
    const { width, height } = size;
    const k = Math.min(1, $stage.clientWidth / width, $stage.clientHeight / height);
    view = { k: Math.max(MIN_ZOOM, k), x: 0, y: 0 };
    view.x = ($stage.clientWidth - width * view.k) / 2;
    view.y = Math.max(0, ($stage.clientHeight - height * view.k) / 2);
    applyView();
  }

  /** 배율을 바꾼다. 기준점(없으면 화면 가운데)이 제자리에 있게 한다. */
  function zoomBy(factor, at) {
    const point = at || { x: $stage.clientWidth / 2, y: $stage.clientHeight / 2 };
    const k = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, view.k * factor));
    view.x = point.x - (point.x - view.x) * (k / view.k);
    view.y = point.y - (point.y - view.y) * (k / view.k);
    view.k = k;
    auto = false;
    applyView();
  }

  /** 글을 읽어 그림을 다시 그린다. */
  function draw() {
    clearTimeout(drawTimer);
    drawTimer = 0;
    if (disposed) return;
    model = parseErd($code.value);
    const theme = readTheme();
    const drawn = model.entities.length ? drawErd(model, layoutErd(model, measure), theme, measure) : null;
    size = drawn && { width: drawn.width, height: drawn.height };
    $view.replaceChildren(...(drawn ? [drawn.node] : []));
    $empty.textContent = drawn ? '' : (model.errors.length ? '그릴 수 있는 개체가 없습니다. 아래의 오류를 고쳐 보세요.' : '왼쪽에 erDiagram 글을 쓰면 여기에 그림이 나옵니다. ([문법] 단추: 쓰는 법과 예시)');
    $empty.hidden = !!drawn;
    $info.textContent = `개체 ${model.entities.length} · 관계 ${model.relations.length}${model.errors.length ? ` · 오류 ${model.errors.length}` : ''}`;
    $info.classList.toggle('bad', model.errors.length > 0);
    $problems.replaceChildren(...model.errors.map((error) => {
      const row = make('div', 'erd-problem');
      row.append(make('b', '', `${error.line}번째 줄`), make('span', '', error.message), make('small', '', error.fix));
      row.title = '이 줄로 가기';
      row.onclick = () => goToLine(error.line);
      return row;
    }));
    $problems.hidden = !model.errors.length;
    if (auto) fit(); else applyView();
  }

  const drawSoon = () => { clearTimeout(drawTimer); drawTimer = setTimeout(draw, DRAW_MS); };

  /** 코드 칸에서 그 줄을 고른다(코드 칸이 숨겨져 있으면 연다). */
  function goToLine(line) {
    if (!codeShown) showCode(true);
    const lines = $code.value.split('\n');
    const start = lines.slice(0, line - 1).reduce((sum, text) => sum + text.length + 1, 0);
    $code.focus();
    $code.setSelectionRange(start, start + (lines[line - 1] || '').length);
    // 고른 줄이 보이게 스크롤한다.
    const height = parseFloat(getComputedStyle($code).lineHeight) || 20;
    $code.scrollTop = Math.max(0, (line - 1) * height - $code.clientHeight / 2);
  }

  function showCode(shown) {
    codeShown = shown;
    $codeView.hidden = !shown;
    $toggle.classList.toggle('on', shown);
    if (auto) fit();
    if (shown) $code.focus(); else $stage.focus();
    onChange();
  }

  /**
   * 글을 통째로 바꾼다. `undoable`이면 코드 칸의 되돌리기(Ctrl+Z)에 남게 편집 명령으로 넣는다.
   *
   * 편집 명령은 초점이 있는 곳에 글을 넣는다. 이 항목이 화면에 없으면 코드 칸이 초점을 받지 못해, 그 글이
   * 사용자가 보고 있던 터미널이나 다른 편집기에 입력된다. 그래서 편집 명령은 사용자가 이 화면에서 직접 시킨 경우에만 쓰고,
   * 밖에서 바꾸는 경우(Claude 세션의 erd_write, 저장소 다시 읽기)는 초점을 건드리지 않고 값을 바로 넣는다.
   */
  function replaceText(text, undoable = false) {
    let typed = false;
    if (undoable && $code.offsetParent) {
      $code.focus();
      if (document.activeElement === $code) {
        $code.select();
        typed = text ? document.execCommand('insertText', false, text) : document.execCommand('delete');
        $code.setSelectionRange(0, 0);
      }
    }
    if (!typed || $code.value !== text) $code.value = text;
    auto = true;
    changed();
  }

  /** 글이 바뀌었다: 그림을 다시 그리고 자동 저장을 건다. */
  function changed() {
    draw();
    scheduleSave();
    onChange();
  }

  function scheduleSave(wait = SAVE_MS) {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(flush, wait);
  }

  /** 자동 저장. 저장소를 읽지 못한 채로는 적지 않는다(읽지 못한 내용을 덮어쓰지 않게). */
  async function flush() {
    clearTimeout(saveTimer);
    saveTimer = 0;
    if (unread) return;
    const now = $code.value;
    if (now === stored) return;
    try {
      await invoke('erd_write', { id, text: now });
      stored = now;
    } catch (err) {
      if (!disposed) {
        setStatus(`ERD를 저장하지 못했습니다: ${err} · 잠시 뒤 다시 저장합니다.`);
        scheduleSave(SAVE_RETRY_MS);
      }
    }
  }

  /** 밖에서(같은 그룹의 Claude 세션이 파일을 직접 고친 경우 등) 저장소의 글이 바뀌었으면 읽어 들인다. */
  async function reload() {
    if (disposed) return;
    const base = stored;
    if (!unread && $code.value !== base) return; // 여기서 고치는 중인 글은 덮어쓰지 않는다.
    let found = null;
    try { found = await invoke('erd_read', { id }); } catch { return; }
    if (disposed || found == null) return;
    if (unread) {
      unread = false;
      stored = found;
      if (found !== $code.value) replaceText(found);
      return setStatus('ERD의 내용을 다시 읽었습니다.');
    }
    if (found === base || stored !== base || $code.value !== base) return;
    stored = found;
    replaceText(found);
  }

  async function exportAs(kind) {
    const fresh = parseErd($code.value);
    if (!fresh.entities.length) return setStatus('내보낼 그림이 없습니다. 개체를 하나 이상 적어 주세요.');
    const theme = readTheme();
    const made = standalone(fresh, layoutErd(fresh, measure), theme, measure);
    const name = title();
    const headers = (ext) => ({ 'x-name': encodeURIComponent(name), 'x-ext': ext, 'x-start': encodeURIComponent(item.cwd || ''), 'x-title': encodeURIComponent('ERD 내보내기') });
    try {
      setStatus('내보내는 중…');
      let saved = null;
      if (kind === 'svg') {
        const xml = `<?xml version="1.0" encoding="UTF-8"?>\n${new XMLSerializer().serializeToString(made.svg)}\n`;
        saved = await invoke('mindmap_export', new TextEncoder().encode(xml), { headers: headers('svg') });
      } else if (kind === 'png') {
        saved = await invoke('mindmap_export', await svgToPng(made.svg, made.width, made.height), { headers: headers('png') });
      } else {
        const image = await invoke('image_store', await svgToPng(made.svg, made.width, made.height), {
          headers: { 'x-name': encodeURIComponent(name), 'x-ext': 'png' },
        });
        await api.addImages([image], { near: item, source: `ERD: ${name}` });
        return setStatus('이 항목과 같은 묶음에 이미지 항목으로 넣었습니다.');
      }
      setStatus(saved ? `내보냈습니다: ${saved}` : '');
    } catch (err) {
      setStatus(`내보내지 못했습니다: ${err}`);
    }
  }

  /** 사이드바에 보일 기본 이름: 글의 title, 없으면 ERD. */
  const title = () => clip(model.title.trim(), 40) || 'ERD';

  // ── 코드 칸 ──
  $code.addEventListener('input', () => { drawSoon(); scheduleSave(); });
  $code.addEventListener('keydown', (ev) => {
    if (ev.key === 'Tab' && !ev.ctrlKey && !ev.altKey) {
      // Tab은 초점을 옮기지 않고 들여쓰기 네 칸을 넣는다.
      ev.preventDefault();
      document.execCommand('insertText', false, '    ');
    }
  });

  // ── 그림: 끌어서 옮기기, 휠로 옮기기, Ctrl+휠로 확대·축소 ──
  const stagePoint = (ev) => { const rect = $stage.getBoundingClientRect(); return { x: ev.clientX - rect.left, y: ev.clientY - rect.top }; };
  $stage.addEventListener('mousedown', (ev) => {
    if (ev.button !== 0 && ev.button !== 1) return;
    drag = { x: ev.clientX, y: ev.clientY, startX: view.x, startY: view.y, moved: false, target: ev.target };
    $stage.focus();
    ev.preventDefault();
  });
  const onMove = (ev) => {
    if (!drag) return;
    const dx = ev.clientX - drag.x, dy = ev.clientY - drag.y;
    if (!drag.moved && Math.hypot(dx, dy) < 4) return;
    drag.moved = true;
    $stage.classList.add('grabbing');
    view.x = drag.startX + dx;
    view.y = drag.startY + dy;
    auto = false;
    applyView();
  };
  const onUp = () => {
    if (!drag) return;
    const { moved, target } = drag;
    drag = null;
    $stage.classList.remove('grabbing');
    if (moved) return;
    // 그냥 누른 것: 그 개체·속성·관계를 적은 줄로 간다(코드 칸이 열려 있을 때).
    const line = Number(target.closest?.('[data-line]')?.dataset.line);
    if (line && codeShown) goToLine(line);
  };
  document.addEventListener('mousemove', onMove);
  document.addEventListener('mouseup', onUp);
  $stage.addEventListener('wheel', (ev) => {
    ev.preventDefault();
    if (ev.ctrlKey) return zoomBy(ev.deltaY < 0 ? 1.12 : 1 / 1.12, stagePoint(ev));
    view.x -= ev.shiftKey ? ev.deltaY : ev.deltaX;
    view.y -= ev.shiftKey ? 0 : ev.deltaY;
    auto = false;
    applyView();
  }, { passive: false });
  // 개체에 마우스를 올리면 그 개체의 관계선을 도드라지게 한다.
  $svg.addEventListener('mouseover', (ev) => {
    const hot = ev.target.closest?.('.erd-entity')?.dataset.id;
    for (const node of $svg.querySelectorAll('.erd-rel')) node.classList.toggle('hot', hot != null && (node.dataset.from === hot || node.dataset.to === hot));
    for (const node of $svg.querySelectorAll('.erd-entity')) node.classList.toggle('hot', node.dataset.id === hot);
  });
  $svg.addEventListener('mouseleave', () => { for (const node of $svg.querySelectorAll('.hot')) node.classList.remove('hot'); });
  box.addEventListener('keydown', (ev) => {
    if (!ev.ctrlKey || ev.altKey || ev.shiftKey) return;
    const key = ev.key.toLowerCase();
    if (key === 'e') { ev.preventDefault(); ev.stopPropagation(); showCode(!codeShown); }
    else if (key === '0') { ev.preventDefault(); fit(); }
    else if (ev.target !== $code && (key === '=' || key === '+')) { ev.preventDefault(); zoomBy(1.2); }
    else if (ev.target !== $code && key === '-') { ev.preventDefault(); zoomBy(1 / 1.2); }
  });

  const resize = new ResizeObserver(() => { if (auto) fit(); });
  resize.observe($stage);
  const retheme = () => { if (!disposed) draw(); }; // 테마·글꼴이 바뀌면 색과 글자 너비가 달라진다.
  window.addEventListener('cypher-look', retheme);

  $codeView.hidden = !codeShown;
  $toggle.classList.toggle('on', codeShown);
  const ready = (async () => {
    // 처음 넣을 내용이 주어졌으면(Claude 세션이 만든 것, 그룹 틀) 그것으로, 아니면 자동 저장소에서 읽는다.
    const given = typeof saved.text === 'string' ? saved.text : null;
    let kept = null;
    if (given == null) {
      try {
        kept = await invoke('erd_read', { id }); // 아직 적어 둔 것이 없으면 null, 읽지 못하면 오류다.
      } catch (err) {
        unread = true;
        setStatus(`ERD의 내용을 읽지 못했습니다: ${err} · 이 항목을 다시 보면 다시 읽습니다(그때까지 자동 저장하지 않습니다).`);
      }
    }
    $code.value = given ?? kept ?? SAMPLE;
    stored = kept ?? '';
    draw();
    if ($code.value !== stored) await flush();
    onChange();
  })();

  return {
    ready,
    focus: () => { if (codeShown) $code.focus(); else $stage.focus(); if (auto) fit(); reload(); },
    title,
    state: () => ({ erd: id, code: codeShown }),
    /** 저장할 것을 바로 저장한다(앱이 꺼지기 직전). */
    flush,
    /** erDiagram 글. */
    text: () => $code.value,
    /** 내용을 통째로 바꾼다. 같은 그룹의 Claude 세션이 고칠 때 쓴다(초점을 옮기지 않는다). */
    setText: (source) => { unread = false; replaceText(String(source ?? '')); return true; },
    /** 글자를 커서 자리에 넣는다(끌어다 놓기 등). */
    insert: (text) => { $code.focus(); if (document.activeElement === $code) document.execCommand('insertText', false, text); },
    /** 지금 글의 오류: [{ line, message, fix }] */
    problems: () => model.errors.map((error) => ({ ...error })),
    /** 지금 그림에 든 것(자동 검증용). */
    summary: () => ({ entities: model.entities.length, relations: model.relations.length, drawn: $view.querySelectorAll('.erd-entity').length }),
    setCode: (shown) => showCode(!!shown),
    /** 자동 저장소의 번호(파일 이름). */
    store: () => id,
    close: async () => { await flush(); return true; },
    dispose: () => {
      disposed = true;
      clearTimeout(saveTimer);
      clearTimeout(drawTimer);
      resize.disconnect();
      window.removeEventListener('cypher-look', retheme);
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      // 닫은 항목의 내용은 바로 지우지 않고 보관해 둔다(30일).
      invoke('erd_remove', { id, keep: true }).catch(() => {});
    },
  };
}
