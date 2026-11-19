// 코드 보기의 "정의로 가기" 해석기: 이름만 보지 않고, 받는 쪽의 자료형을 따라가 정의를 고른다.
//
// 언어 서버가 없으니 글자의 모양으로 할 수 있는 만큼만 한다:
// - `orderMapper.insert(…)`: `orderMapper`의 선언(지역 변수·인자·필드)에서 자료형을 읽고, 그 자료형(과 상위 자료형)의 `insert`로 간다.
// - `a.b().c()`: `b`의 정의에서 반환형을 읽어 이어 간다. `self.config.globs.matched()`처럼 필드를 거치는 것도 같다.
// - 자료형 이름은 import(Java)·use(Rust)·import … from(JS)·from … import(Python)이 가리키는 파일의 것을 고른다.
//   import가 이 폴더 밖(java.util, std)을 가리키면 엉뚱한 곳으로 가지 않고 "이 폴더 밖의 것"이라고 알린다.
// - 확실할 때만 바로 간다. 자료형을 끝내 알 수 없으면 예전처럼 이름이 같은 정의를 가까운 순서로 늘어놓아 고르게 한다.
//
// "자료형"은 이름의 묶음(`{ names: ['Arc', 'Mutex', 'Config'] }`)으로 다룬다. 묶음은 보통 "닫혀" 있다: 맨 바깥 자료형의 멤버만 찾는다
// (`Vec<Buffer>`의 `len`은 Vec의 것이지 Buffer의 것이 아니다). 안의 것을 꺼내는 일(`get`·`unwrap`·`iter`, `[i]`, `?`, for·람다의 원소)을
// 거치면 "열린" 묶음이 되어 안쪽 자료형에서도 찾는다. 가리키기만 하는 자료형(Box·Arc·Rc …)은 닫힌 묶음에서도 건너뛴다.

const TYPE_KINDS = ['class', 'interface', 'enum', 'struct', 'trait'];
const CALLABLE = ['function', 'method', 'constructor', 'macro'];
const VALUE_KINDS = ['field', 'constant', 'variable', 'variant'];
/** 받은 것과 다른 자료형을 내놓는 이름들: 이 뒤로는 자료형을 따라가지 않는다(틀린 곳으로 가느니 고르게 한다). */
const TRANSFORMERS = new Set(['map', 'flatMap', 'flat_map', 'and_then', 'then', 'collect', 'into', 'try_into', 'parse', 'map_or', 'map_or_else', 'fold', 'reduce', 'zip',
  'filter_map', 'mapToObj', 'thenApply', 'thenCompose', 'transform', 'convert', 'from', 'try_from', 'unzip', 'groupingBy', 'map_err', 'ok_or', 'ok_or_else', 'join', 'format']);
/** 받은 것(이나 그 안의 것)을 그대로 내놓는 이름들: 그 자료형에 정의가 없어도 묶음을 넘긴다. 밖의 자료형(Option·List)에서 부르면 묶음이 열린다. */
const TRANSPARENT = new Set(['clone', 'cloned', 'copied', 'to_owned', 'borrow', 'borrow_mut', 'as_ref', 'as_mut', 'as_deref', 'unwrap', 'expect', 'unwrap_or', 'unwrap_or_else',
  'unwrap_or_default', 'lock', 'read', 'write', 'get', 'get_mut', 'deref', 'iter', 'iter_mut', 'into_iter', 'first', 'last', 'orElse', 'orElseThrow', 'orElseGet', 'get_or_insert_with',
  'take', 'peek', 'next', 'stream', 'values', 'findFirst', 'findAny', 'filter', 'sorted', 'distinct', 'limit', 'skip', 'rev', 'upgrade', 'downgrade', 'into_inner']);
/** 원소를 하나씩 넘겨주는 이름들: 그 람다·클로저의 인자는 받는 쪽 묶음의 원소다. */
const ITERATORS = new Set(['forEach', 'for_each', 'map', 'filter', 'anyMatch', 'allMatch', 'noneMatch', 'removeIf', 'any', 'all', 'find', 'position', 'filter_map', 'flat_map',
  'flatMap', 'inspect', 'retain', 'peek', 'takeWhile', 'dropWhile', 'take_while', 'skip_while', 'some', 'every', 'findIndex', 'ifPresent', 'sort_by_key', 'max_by_key', 'min_by_key']);
/**
 * 안의 것을 꺼내야 쓸 수 있는 묶음 자료형(Option·Vec·HashMap·Mutex …): 그 멤버는 안쪽 자료형의 것이 아니다.
 * Rust에서 그 밖의 밖의 자료형(Box·Arc·MutexGuard, tauri의 `State<T>` …)은 안의 것을 그대로 내놓는 감싸개로 본다(Deref).
 */
const CONTAINER = /^(?:Option|Result|Optional|Either|Poll|Future|Promise|Task|Mono|Flux|Page|Slice|Entry|Mutex|RwLock|RefCell|Cell|OnceCell|OnceLock)$|(?:Vec|Map|Set|List|Queue|Deque|Stack|Heap|Iter|Iterator|Stream|Collection|Array|Buf|Buffer|Cache|Sender|Receiver|Channel)$/;
/** Java의 모든 자료형이 물려받는 메서드(java.lang.Object). */
const OBJECT_METHODS = ['getClass', 'hashCode', 'equals', 'toString', 'notify', 'notifyAll', 'wait', 'clone', 'finalize'];
/** 같은 이름의 정의가 여러 언어에 있으면 지금 파일의 언어 것만 보는 언어들(프런트와 백엔드가 같은 이름을 쓰는 저장소). */
const TYPED = ['java', 'kotlin', 'rust', 'go', 'csharp', 'python', 'js', 'vue', 'c', 'groovy'];
/** 이 폴더에 같은 이름의 별칭이 있어도 따라가지 않는 이름들(크레이트마다 `type Result<T> = …`를 둔다). */
const PRELUDE = new Set(['Result', 'Option', 'Box', 'Vec', 'String', 'Arc', 'Rc']);
/** 받는 쪽의 자료형을 아는데 거기에 없으면 "없다"·"밖의 것"이라고 알리는 언어. 그 밖의 언어(확장 함수, 동적 언어)는 이름으로 찾아본다. */
const STRICT = ['java', 'rust'];
/** 클래스 하나가 파일 하나에 통째로 든 언어: 그 자료형의 멤버는 그 자료형을 정의한 파일에만 있다. */
const ONE_FILE = ['java', 'kotlin', 'csharp', 'groovy', 'js', 'vue', 'python'];
const NOT_TYPES = new Set(['return', 'new', 'throw', 'else', 'case', 'yield', 'await', 'delete', 'goto', 'in', 'of', 'instanceof', 'typeof', 'var', 'val', 'let', 'const', 'auto', 'final',
  'mut', 'ref', 'static', 'public', 'private', 'protected', 'void', 'if', 'for', 'while', 'do', 'break', 'continue', 'import', 'package', 'assert']);
const ARRAY = '[]';   // 묶음 안에서 배열·슬라이스를 가리키는 표시(그 멤버는 이 폴더의 것이 아니다)
const TTL = 1500;     // 찾아 둔 정의를 다시 쓰는 시간(Claude 세션이 파일을 계속 고친다)
const MAX_LINE = 4000; // 이보다 긴 줄에서는 선언을 찾지 않는다

const esc = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const lowerFirst = (text) => text.slice(0, 1).toLowerCase() + text.slice(1);
const isUpper = (name) => /^[A-Z]/.test(name);
/** 경로가 붙은 이름(`anyhow::Result`, `Outer.Inner`)의 마지막 이름. */
const plain = (name) => name.slice(Math.max(name.lastIndexOf(':'), name.lastIndexOf('.')) + 1);
/** 받는 쪽의 자료형을 모를 때, 이 폴더에 같은 이름의 정의가 하나 있어도 그것이라고 믿기 어려운 흔한 이름들(표준 라이브러리의 것일 때가 많다). */
const COMMON = new Set(['is_none', 'is_some', 'is_ok', 'is_err', 'len', 'is_empty', 'unwrap', 'expect', 'clone', 'to_string', 'to_owned', 'iter', 'into_iter', 'push', 'pop', 'get',
  'get_mut', 'contains', 'insert', 'remove', 'extend', 'map', 'filter', 'collect', 'as_ref', 'as_mut', 'as_str', 'as_bytes', 'into', 'from', 'default', 'fmt', 'eq', 'cmp', 'hash',
  'toString', 'equals', 'hashCode', 'size', 'isEmpty', 'add', 'addAll', 'put', 'containsKey', 'stream', 'forEach', 'length', 'charAt', 'substring', 'trim', 'split', 'join', 'format',
  'valueOf', 'getClass', 'name', 'ordinal', 'values', 'next', 'hasNext', 'close', 'write', 'read', 'flush', 'lock', 'send', 'recv', 'poll', 'keys', 'entries', 'then', 'catch', 'find']);
const isGeneric = (name) => /^[A-Z]\d?$/.test(name);

/** 닫는 괄호(`text[close]`)와 짝이 맞는 여는 괄호의 자리. 없으면 -1. */
function openOf(text, close) {
  const closer = text[close];
  const opener = { ')': '(', ']': '[', '>': '<', '}': '{' }[closer];
  let depth = 0;
  for (let i = close; i >= 0; i--) {
    const ch = text[i];
    if (ch === closer && !(closer === '>' && (text[i - 1] === '-' || text[i - 1] === '='))) depth += 1;
    else if (ch === opener) { depth -= 1; if (depth === 0) return i; }
  }
  return -1;
}

/**
 * 자료형을 적은 글에서 자료형 이름들을 바깥 것부터 고른다: `&mut Arc<Mutex<Config>>` → Arc, Mutex, Config.
 * 소문자 경로가 붙은 이름은 경로째 둔다(`anyhow::Result` — 어느 모듈의 것인지 알아야 이 폴더의 것인지 가릴 수 있다).
 */
export function typeNames(text) {
  const out = [];
  for (const m of String(text || '').matchAll(/(?:[A-Za-z_$][\w$]*\s*(?:::|\.(?!\.))\s*)*[A-Za-z_$][\w$]*/g)) {
    const parts = m[0].split(/::|\./).map((part) => part.trim());
    const last = parts[parts.length - 1];
    if (!isUpper(last)) continue;
    // 소문자 경로(`anyhow::Result`)는 경로째, 안쪽 자료형(`Outer.Inner`)은 바깥 자료형과 함께 둔다.
    const name = m[0].includes('::') ? (parts.length > 1 && !isUpper(parts[0]) ? parts.join('::') : last)
      : parts.length > 1 && isUpper(parts[parts.length - 2]) ? `${parts[parts.length - 2]}.${last}` : last;
    if (!out.includes(name)) out.push(name);
  }
  return out;
}
/** 묶음에 넣을 이름들: 배열·슬라이스(`[T]`, `T[]`)면 맨 앞에 배열 표시를 둔다. */
const bagNames = (text) => (/\[/.test(text) ? [ARRAY, ...typeNames(text)] : typeNames(text));

/** 이름 앞에 적힌 자료형(Java·C 계열: `final List<Foo> name`). 없으면 빈 글. */
export function typeBefore(text, col) {
  let i = col;
  while (i > 0 && /\s/.test(text[i - 1])) i -= 1;
  const end = i;
  let depth = 0;
  while (i > 0) {
    const ch = text[i - 1];
    if (ch === '>' || ch === ']') depth += 1;
    else if (ch === '<' || ch === '[') { if (depth === 0) break; depth -= 1; }
    else if (depth === 0 && !/[\w$.?]/.test(ch)) break;
    i -= 1;
  }
  const type = text.slice(i, end).trim().replace(/\.{3}$/, '');
  const head = /^[A-Za-z_$][\w$]*/.exec(type)?.[0] || '';
  return !head || NOT_TYPES.has(head) ? '' : type;
}

/** 이름 뒤에 적힌 자료형(Rust·TypeScript·Kotlin·Python: `name: &mut Foo<Bar>`). 없으면 빈 글. */
export function typeAfter(text, from) {
  const m = /^\s*\??\s*:(?!:)\s*/.exec(text.slice(from));
  if (!m) return '';
  let i = from + m[0].length, depth = 0;
  const start = i;
  for (; i < text.length; i++) {
    const ch = text[i];
    if (ch === '>' && (text[i - 1] === '-' || text[i - 1] === '=')) continue;
    if ('<(['.includes(ch)) depth += 1;
    else if ('>)]'.includes(ch)) { if (depth === 0) break; depth -= 1; }
    else if (depth === 0 && ',=;|{}'.includes(ch)) break;
  }
  return text.slice(start, i).trim();
}

/** 괄호 안의 인자(또는 서명의 인자)를 맨 바깥 쉼표로 나눈다. */
function splitArgs(inner, angles) {
  const parts = [];
  let depth = 0, from = 0;
  for (let i = 0; i < inner.length; i++) {
    const ch = inner[i];
    if ('([{'.includes(ch) || (angles && ch === '<')) depth += 1;
    else if (')]}'.includes(ch) || (angles && ch === '>' && inner[i - 1] !== '-' && inner[i - 1] !== '=')) depth = Math.max(0, depth - 1);
    else if (ch === ',' && depth === 0) { parts.push(inner.slice(from, i)); from = i + 1; }
  }
  parts.push(inner.slice(from));
  return parts.map((part) => part.trim()).filter(Boolean);
}

/** 정의의 서명에서 인자 수의 범위 [가장 적게, 가장 많이]. 읽지 못하면 null. */
function arity(def) {
  const at = new RegExp(`(?<![\\w$])${esc(def.name)}\\s*(?:<[^()]*>)?\\s*\\(`).exec(def.sig || '');
  if (!at) return null;
  const open = at.index + at[0].length - 1;
  let depth = 0, close = -1;
  for (let i = open; i < def.sig.length; i++) {
    if (def.sig[i] === '(') depth += 1;
    else if (def.sig[i] === ')') { depth -= 1; if (depth === 0) { close = i; break; } }
  }
  if (close < 0) return null;
  let params = splitArgs(def.sig.slice(open + 1, close), true);
  // Rust의 self, Python의 self·cls는 부르는 쪽이 적지 않는다.
  if (params.length && /^(?:&\s*(?:'\w+\s+)?)?(?:mut\s+)?(?:self|cls)\b/.test(params[0])) params = params.slice(1);
  const variadic = params.some((param) => /\.\.\./.test(param) || /^\*/.test(param));
  const optional = params.filter((param) => /=/.test(param) || /\?\s*:/.test(param)).length;
  return [params.length - optional - (variadic ? 1 : 0), variadic ? Infinity : params.length];
}

/**
 * `use` 문 하나(세미콜론 앞까지의 글)를 풀어 이름 → 경로를 적는다: `a::b::{C, d::E as F, self}`.
 * `names`: 이름 → 경로의 조각들(마지막이 진짜 이름), `globs`: `a::b::*`의 경로들.
 */
function parseUse(text, prefix, names, globs) {
  for (const item of splitArgs(text, false)) {
    const brace = item.indexOf('{');
    if (brace >= 0) {
      const head = item.slice(0, brace).split('::').map((part) => part.trim()).filter(Boolean);
      parseUse(item.slice(brace + 1, item.lastIndexOf('}')), [...prefix, ...head], names, globs);
      continue;
    }
    const [path, alias] = item.split(/\s+as\s+/).map((part) => part.trim());
    const parts = [...prefix, ...path.split('::').map((part) => part.trim()).filter(Boolean)];
    if (!parts.length) continue;
    const last = parts[parts.length - 1];
    if (last === '*') globs.push(parts.slice(0, -1));
    else if (last === 'self') names.set(alias || parts[parts.length - 2] || '', parts.slice(0, -1));
    else if (alias !== '_') names.set(alias || last, parts);
  }
}

/**
 * 파일이 가져다 쓰는 이름들. `lines`는 원래 글(문자열이 그대로 있다).
 * `names`: 파일 맨 바깥에서 가져온 이름, `scoped`: 함수·블록 안에서 가져온 이름(Rust의 들여 쓴 `use` — 그 함수 안에서만 통한다).
 */
export function importsOf(lang, lines) {
  const found = { names: new Map(), scoped: [], globs: [], statics: [], pack: '' };
  if (lang === 'java' || lang === 'kotlin' || lang === 'groovy') {
    for (const line of lines) {
      let m = /^\s*package\s+([\w.]+)/.exec(line);
      if (m) { found.pack = m[1]; continue; }
      m = /^\s*import\s+(static\s+)?([\w.]+?)(\.\*)?\s*(?:as\s+(\w+)\s*)?;?\s*$/.exec(line);
      if (!m) continue;
      const parts = m[2].split('.');
      if (m[3]) (m[1] ? found.statics : found.globs).push(parts);
      else found.names.set(m[4] || parts[parts.length - 1], { parts, isStatic: !!m[1] });
    }
  } else if (lang === 'rust') {
    for (let i = 0; i < lines.length; i++) {
      const m = /^(\s*)(?:pub(?:\s*\([^)]*\))?\s+)?use\s+(.*)$/.exec(lines[i]);
      if (!m) continue;
      let text = m[2];
      for (let more = 1; !text.includes(';') && more < 60 && i + more < lines.length; more++) text += ` ${lines[i + more].replace(/\/\/.*$/, '')}`;
      const names = new Map();
      const globs = [];
      parseUse(text.split(';')[0], [], names, globs);
      for (const [name, parts] of names) {
        if (!name) continue;
        if (m[1]) found.scoped.push({ name, parts, line: i + 1 });
        else found.names.set(name, { parts });
      }
      if (!m[1]) found.globs.push(...globs);
    }
  } else if (lang === 'js' || lang === 'vue') {
    for (const line of lines) {
      let m = /^\s*import\s+(?:type\s+)?(.+?)\s+from\s+['"]([^'"]+)['"]/.exec(line);
      if (m) {
        const from = m[2];
        const braces = /\{([^}]*)\}/.exec(m[1]);
        for (const item of (braces ? braces[1] : '').split(',')) {
          const [real, alias] = item.trim().replace(/^type\s+/, '').split(/\s+as\s+/);
          if (real) found.names.set((alias || real).trim(), { from, real: real.trim() });
        }
        const head = m[1].replace(/\{[^}]*\}/, '').split(',').map((part) => part.trim()).filter(Boolean);
        for (const item of head) {
          const star = /^\*\s+as\s+(\w+)/.exec(item);
          found.names.set(star ? star[1] : item, { from, real: '', whole: true });
        }
        continue;
      }
      m = /^\s*(?:const|let|var)\s+(\{[^}]*\}|[\w$]+)\s*=\s*require\(\s*['"]([^'"]+)['"]/.exec(line);
      if (m) {
        for (const item of m[1].replace(/[{}]/g, '').split(',')) {
          const [real, alias] = item.split(':').map((part) => part.trim());
          if (real) found.names.set(alias || real, { from: m[2], real: m[1].startsWith('{') ? real : '', whole: !m[1].startsWith('{') });
        }
      }
    }
  } else if (lang === 'python') {
    for (const line of lines) {
      let m = /^\s*from\s+([\w.]+)\s+import\s+(.+)$/.exec(line);
      if (m) {
        for (const item of m[2].replace(/[()\\]/g, '').split(',')) {
          const [real, alias] = item.trim().split(/\s+as\s+/);
          if (real && real !== '*') found.names.set((alias || real).trim(), { parts: [...m[1].split('.').filter(Boolean), real.trim()], real: real.trim() });
        }
        continue;
      }
      m = /^\s*import\s+([\w.]+)(?:\s+as\s+(\w+))?\s*$/.exec(line);
      if (m) found.names.set(m[2] || m[1].split('.').pop(), { parts: m[1].split('.'), real: '', whole: true });
    }
  }
  return found;
}

/**
 * 해석기를 만든다. `env`:
 * - `invoke`, `root()`, `tab()`(보고 있는 파일: path, lines, tokens, lang, outline, modified)
 * - `full(프로젝트 기준 경로)`, `relative(전체 경로)`, `lower`, `dirOf`, `nameOf`, `langOf(파일)`
 * - `enclosing(tab, 줄)`(그 줄을 감싼 정의들), `localDeclaration(이름, from, until, skip)`, `rank(정의들, found)`
 * - `files()`(프로젝트의 파일 목록), `lines(전체 경로)`(그 파일의 줄들)
 */
export function createNav(env) {
  const cache = new Map(); // 열쇠 → { at, value(약속) }
  const cached = (key, load, empty = []) => {
    const now = Date.now();
    const hit = cache.get(key);
    if (hit && now - hit.at < TTL) return hit.value;
    if (cache.size > 400) cache.clear();
    // 실패한 것은 담아 두지 않는다(다음에 다시 묻는다).
    const value = Promise.resolve().then(load).catch(() => { cache.delete(key); return empty; });
    cache.set(key, { at: now, value });
    return value;
  };
  /** 그 이름의 정의들. `within`을 주면 감싼 이름이 그 가운데 하나인 것만. */
  const defsOf = (name, within = null) => cached(`d|${env.root()}|${name}|${within ? within.join(',') : ''}`,
    () => env.invoke('code_defs', { root: env.root(), name, within, fresh: false }));
  const subtypesOf = (name) => cached(`s|${env.root()}|${name}`, () => env.invoke('code_subtypes', { root: env.root(), name }));
  const langOfDef = (def) => env.langOf(def.file);
  const typeAfterLang = (lang) => ['rust', 'js', 'vue', 'kotlin', 'python'].includes(lang);

  /** 지금 답을 찾고 있는 파일과 누른 줄(기다리는 사이 화면이 다른 파일로 가도 이것으로 끝까지 찾는다). */
  let active = null, clickLine = 0;

  /** 그 줄의 글에서 주석은 지우고 문자열은 여는 따옴표만 남기고 빈칸으로 바꾼 것(길이는 그대로). */
  function codeLine(tab, line) {
    const text = tab.lines[line - 1] ?? '';
    const tokens = tab.tokens?.[line - 1];
    if (!tokens?.length) return text;
    let out = text;
    for (const [from, to, kind] of tokens) {
      if (kind === 'com' || kind === 'doc' || kind === 'esc') out = out.slice(0, from) + ' '.repeat(to - from) + out.slice(to);
      else if (kind === 'str') out = `${out.slice(0, from)}"${' '.repeat(Math.max(0, to - from - 1))}${out.slice(to)}`;
    }
    return out;
  }

  /** 그 자리 앞의 글: 앞의 몇 줄을 이어 붙인다(여러 줄에 걸친 `.메서드()` 사슬의 받는 쪽을 읽으려고). */
  function before(tab, line, col) {
    const parts = [];
    for (let n = Math.max(1, line - 6); n < line; n++) parts.push(codeLine(tab, n).trim());
    parts.push(codeLine(tab, line).slice(0, col));
    return parts.join(' ');
  }

  /**
   * `end` 앞에서 끝나는 식을 거꾸로 읽어 조각으로 나눈다: `self.config.globs.matched(path)` →
   * [{name:'self'}, {name:'config'}, {name:'globs'}, {name:'matched', call:true}]. 읽지 못하면 null.
   * `arrow`: `a->b`를 멤버 접근으로 볼지(C·C++만. Java·Kotlin의 람다, Rust의 반환형 화살표는 멤버 접근이 아니다).
   */
  function chainBefore(s, end, arrow) {
    const segs = [];
    let i = end;
    for (;;) {
      while (i > 0 && /\s/.test(s[i - 1])) i -= 1;
      // 뒤에 붙는 것: Rust의 `?`(감싼 것에서 꺼낸다), TypeScript·Kotlin의 `!`
      let unwrap = false;
      while (i > 0 && (s[i - 1] === '?' || s[i - 1] === '!')) { if (s[i - 1] === '?') unwrap = true; i -= 1; while (i > 0 && /\s/.test(s[i - 1])) i -= 1; }
      let call = false;
      let inner = '';
      if (s[i - 1] === ')') {
        const open = openOf(s, i - 1);
        if (open < 0) return null;
        call = true;
        inner = s.slice(open + 1, i - 1);
        i = open;
        while (i > 0 && /\s/.test(s[i - 1])) i -= 1;
        if (s[i - 1] === '!') return null; // 매크로를 부른 값(`ready!(…)`)의 자료형은 알 수 없다
        // Rust의 `이름::<T>(…)`
        if (s[i - 1] === '>') {
          const lt = openOf(s, i - 1);
          if (lt > 1 && s.slice(lt - 2, lt) === '::') i = lt - 2; else return null;
        }
      } else if (s[i - 1] === ']') {
        const open = openOf(s, i - 1);
        if (open < 0) return null;
        segs.push({ index: true });
        i = open;
        continue;
      }
      let j = i;
      while (j > 0 && /[\w$]/.test(s[j - 1])) j -= 1;
      const name = s.slice(j, i);
      if (!name && call) {
        // (식).이름: 괄호 안이 `*self`·`&mut x.y`처럼 간단한 식이면 그것이 받는 쪽이다.
        const sub = /^[\s*&]*(?:mut\s+)?[\w$.:\s]+$/.test(inner) ? chainBefore(inner, inner.length, arrow) : null;
        if (!sub) return null;
        segs.push(...sub.reverse());
        break;
      }
      if (!name || /^\d/.test(name)) return null; // 숫자, 읽을 수 없는 식
      // 부르는 조각의 인자 수(오버로드를 가리는 데 쓴다). 제네릭 인자·클로저의 쉼표와 가릴 수 없으면 세지 않는다.
      const argc = call && !/[\w$]\s*<|\|/.test(inner) ? splitArgs(inner, false).length : null;
      const seg = { name, call, unwrap, argc };
      segs.push(seg);
      i = j;
      while (i > 0 && /\s/.test(s[i - 1])) i -= 1;
      if (s[i - 1] === '.' && s[i - 2] !== '.') { i -= 1; if (s[i - 1] === '?') i -= 1; continue; }
      if (s[i - 1] === ':' && s[i - 2] === ':') { i -= 2; seg.path = true; continue; }
      if (arrow && s[i - 1] === '>' && s[i - 2] === '-') { i -= 2; continue; }
      if (call && /(?:^|[^\w$.])new\s*$/.test(s.slice(Math.max(0, i - 6), i))) seg.isNew = true;
      break;
    }
    return segs.reverse();
  }

  /** 그 줄을 감싼 자료형의 이름들(안쪽 것부터): 클래스, impl의 자료형. */
  function ownerTypes(tab, line) {
    const out = [];
    for (const def of [...env.enclosing(tab, line)].reverse()) {
      const name = TYPE_KINDS.includes(def.kind) ? def.name : def.container;
      if (name && !out.includes(name)) out.push(name);
    }
    return out;
  }
  const enclosingFunction = (tab, line) => env.enclosing(tab, line).filter((def) => ['function', 'method', 'constructor'].includes(def.kind)).pop() || null;

  /**
   * 함수의 인자 가운데 그 이름을 선언한 자리. 인자를 줄마다 하나씩 적은 서명에서는 `localDeclaration`이 찾지 못한다.
   * 인자 목록(함수 이름 뒤의 여는 괄호부터 짝이 맞는 닫는 괄호까지)만 보고, 맨 바깥 쉼표로 나눈 조각마다 인자의 이름을 읽는다
   * (자료형 안의 이름 — `(A, B)`, `Map<K, V>` — 이나 반환형을 인자로 보지 않는다).
   */
  function paramDeclaration(tab, name, around, skip) {
    const chars = []; // [글자, 줄, 칸, 깊이]
    let depth = 0, started = false, closed = false;
    for (let line = around.line; line <= Math.min(tab.lines.length, around.line + 40) && !closed; line++) {
      const text = codeLine(tab, line);
      if (text.length > MAX_LINE) return null;
      for (let col = line === around.line ? around.col : 0; col < text.length; col++) {
        const ch = text[col];
        if (!started) {
          if (ch === '(') { started = true; depth = 1; } else if (ch === '{' || ch === ';') return null;
          continue;
        }
        if ('([{<'.includes(ch)) depth += 1;
        else if (')]}'.includes(ch) || (ch === '>' && text[col - 1] !== '-' && text[col - 1] !== '=')) {
          depth -= 1;
          if (depth <= 0) { closed = true; break; }
        }
        chars.push([ch, line, col, depth]);
      }
      if (started && !closed) chars.push([' ', line, text.length, depth]);
    }
    if (!closed) return null;
    const pieces = [[]];
    for (const item of chars) {
      if (item[0] === ',' && item[3] === 1) pieces.push([]); else pieces[pieces.length - 1].push(item);
    }
    const after = typeAfterLang(tab.lang);
    for (const piece of pieces) {
      const text = piece.map((item) => item[0]).join('');
      let at = -1;
      if (after) {
        const m = /^\s*(?:(?:mut|pub|public|private|protected|readonly|override|val|var|vararg|ref)\s+|&\s*(?:'\w+\s+)?|\.{3}|\*{1,2}|@[\w.]+(?:\([^)]*\))?\s+)*([A-Za-z_$][\w$]*)\s*(\??\s*:(?!:)|=|$)/d.exec(text);
        // Rust의 인자는 늘 `이름: 자료형`이다.
        if (m && m[1] === name && (tab.lang !== 'rust' || m[2].includes(':'))) at = m.indices[1][0];
      } else {
        const m = /([A-Za-z_$][\w$]*)\s*(?:\[\s*\]\s*)*(?:=.*)?$/d.exec(text.replace(/\s+$/, ''));
        if (m && m[1] === name && /[\w$>\]?]\s+$|[*&]\s*$|\.{3}\s*$/.test(text.slice(0, m.indices[1][0]))) at = m.indices[1][0];
      }
      if (at < 0) continue;
      const [, line, col] = piece[at];
      if (skip && skip.line === line && skip.col === col) continue;
      return { line, col, len: name.length };
    }
    return null;
  }
  /** 그 줄에서 보이는 지역 선언(지역 변수, 인자). `from`번째 줄부터 위로 찾는다. */
  function localOf(tab, name, from, skip) {
    const around = enclosingFunction(tab, from);
    if (!around) return null;
    const local = env.localDeclaration(name, from, around.line, skip) || paramDeclaration(tab, name, around, skip);
    if (local || tab.lang !== 'go') return local;
    // Go의 메서드를 받는 쪽: `func (b *Bank) Deposit(…)`
    const m = /^func\s*\(\s*([\w]+)\s/d.exec(codeLine(tab, around.line));
    return m && m[1] === name && !(skip && skip.line === around.line && skip.col === m.indices[1][0]) ? { line: around.line, col: m.indices[1][0], len: name.length } : null;
  }

  // ── import·use ──

  /** 그 파일이 가져다 쓰는 이름들(파일이 바뀌면 다시 읽는다). */
  function importsFor(tab) {
    if (!tab.imports || tab.imports.stamp !== tab.modified) tab.imports = { stamp: tab.modified, ...importsOf(tab.lang, tab.lines) };
    return tab.imports;
  }
  /**
   * 이름을 풀 자리: 그 이름을 적은 파일의 언어·경로·import. `src`는 그 이름을 읽은 파일(프로젝트 기준 경로)이고, 비었으면 보고 있는 파일이다.
   * 다른 파일의 서명에서 읽은 자료형 이름은 그 파일의 import로 풀어야 한다(상위 클래스의 `Log logger`).
   */
  async function ctxOf(src) {
    const tab = active;
    if (!src || env.lower(env.full(src)) === env.lower(tab.path)) return { lang: tab.lang, path: tab.path, imports: importsFor(tab), tab, line: clickLine };
    const lang = env.langOf(src);
    const imports = await cached(`i|${env.root()}|${src}`, async () => importsOf(lang, await env.lines(env.full(src))), null);
    return { lang, path: env.full(src), imports: imports || importsOf(lang, []), tab: null, line: 0 };
  }
  /** 그 자리에서 그 이름을 가져온 import. 함수 안의 `use`는 그 함수 안에서만, 그리고 파일 맨 바깥의 것보다 먼저 통한다. */
  function importHit(ctx, name) {
    if (ctx.tab && ctx.line && ctx.imports.scoped.length) {
      const around = env.enclosing(ctx.tab, ctx.line).find((def) => ['function', 'method', 'constructor'].includes(def.kind));
      const inside = around ? ctx.imports.scoped.filter((each) => each.name === name && each.line >= around.line && each.line <= (around.end ?? around.line)) : [];
      if (inside.length) return inside[inside.length - 1];
    }
    return ctx.imports.names.get(name);
  }
  /** 가져온 이름을 경로째 적는다(Rust: `Result` → `anyhow::Result`, 다른 이름으로 가져온 것은 진짜 이름으로). */
  function qualified(ctx, name) {
    const hit = importHit(ctx, name);
    if (!hit) return name;
    if (ctx.lang === 'rust') return hit.parts.length > 1 ? hit.parts.join('::') : name;
    return hit.real || name;
  }
  /** 모듈 경로의 첫 조각이 가져온 이름이면 그 경로로 푼다(`io::Error` ← `use std::io`). */
  function expandMods(ctx, mods) {
    const first = mods[0] && importHit(ctx, mods[0]);
    if (first?.parts) return [...first.parts, ...mods.slice(1)];
    if (first?.from) return [...first.from.split('/').filter((part) => part && part !== '.' && part !== '..'), ...mods.slice(1)];
    return mods;
  }

  let cratesOf = { files: null, names: null };
  /** 이 폴더의 크레이트 이름들(Cargo.toml의 [package]·[lib]의 name. `-`는 `_`로). */
  async function cratesHere() {
    const files = await env.files();
    if (cratesOf.files === files && cratesOf.names) return cratesOf.names;
    const names = new Set();
    await Promise.all(files.filter((file) => /(?:^|\/)Cargo\.toml$/i.test(file)).slice(0, 120).map(async (file) => {
      let section = '';
      for (const line of await env.lines(env.full(file)).catch(() => [])) {
        const head = /^\s*\[([^\]]+)\]/.exec(line);
        if (head) { section = head[1].trim(); continue; }
        const m = /^\s*name\s*=\s*"([^"]+)"/.exec(line);
        if (m && (section === 'package' || section === 'lib')) names.add(m[1].toLowerCase().replace(/-/g, '_'));
      }
    }));
    cratesOf = { files, names };
    return names;
  }

  let partsOf = { files: null, parts: null };
  /** 모듈 경로(`std::io`, `crate::search`, `serde_json`)가 이 폴더의 것인지: 파일·폴더 이름이나 모듈 정의에 그 이름이 있는지 본다. */
  async function moduleHere(ctx, mods) {
    const full = expandMods(ctx, mods);
    if (['std', 'core', 'alloc', 'proc_macro'].includes(full[0])) return false;
    const seg = full.find((each) => !['crate', 'self', 'super'].includes(each));
    if (!seg) return true;
    if (ctx.lang === 'rust' && seg === full[0]) {
      // Rust에서 맨 앞에 적은 이름은 크레이트이거나, 이 파일에서 선언한 모듈이다(가져온 이름은 위에서 풀었다).
      // 파일 이름이 우연히 같은 밖의 크레이트(`tracing::` ↔ `src/tracing.rs`)를 이 폴더의 것으로 보지 않는다.
      if ((await cratesHere()).has(seg.toLowerCase().replace(/-/g, '_'))) return true;
      if (ctx.tab) return (ctx.tab.outline || []).some((def) => def.kind === 'module' && def.name === seg);
    }
    const files = await env.files();
    if (partsOf.files !== files) {
      const parts = new Set();
      for (const file of files) for (const part of file.toLowerCase().replace(/\.[^./]+$/, '').replace(/_/g, '-').split('/')) parts.add(part);
      partsOf = { files, parts };
    }
    const wanted = seg.toLowerCase().replace(/_/g, '-');
    if (partsOf.parts.has(wanted)) return true;
    for (const part of partsOf.parts) if (part.length > 2 && wanted.endsWith(`-${part}`)) return true; // 크레이트 grep_searcher ↔ 폴더 searcher
    return (await defsOf(seg)).some((def) => def.kind === 'module');
  }

  /** 경로의 조각들(모듈 이름)이 파일 경로에 든 정의를 앞에 둔다. 그런 것이 있으면 그것들만. */
  function moduleFirst(found, mods) {
    const wanted = mods.filter((each) => !['crate', 'self', 'super'].includes(each)).map((each) => each.toLowerCase().replace(/_/g, '-'));
    if (!wanted.length) return found;
    const scored = found.map((def) => {
      const parts = def.file.toLowerCase().replace(/\.[^./]+$/, '').replace(/_/g, '-').split('/');
      let score = 0;
      for (const each of wanted) if (parts.includes(each) || parts.some((part) => part.length > 2 && each.endsWith(part))) score += 1;
      // 끝에서부터 차례로 맞으면 더 쳐 준다(`a::b::name` ↔ `…/a/b.rs`).
      const tail = parts.filter((part) => !['src', 'mod', 'lib', 'main', 'index'].includes(part));
      for (let k = 1; k <= wanted.length && tail[tail.length - k] === wanted[wanted.length - k]; k++) score += 2;
      return { def, score };
    });
    const best = Math.max(0, ...scored.map((each) => each.score));
    return best ? scored.filter((each) => each.score === best).map((each) => each.def) : found;
  }

  /** 그 파일이 든 크레이트의 폴더(`src` 앞까지. `src`가 없으면 그 파일의 폴더). */
  const crateOf = (file) => { const parts = file.split('/'); const at = parts.indexOf('src'); return (at >= 0 ? parts.slice(0, at) : parts.slice(0, -1)).join('/'); };
  /** `use` 문 안에서 그 자리 앞에 적힌 경로: `use std::{ffi::OsStr, path::{` → [std, path]. */
  function usePrefix(ahead) {
    const groups = [];
    let i = ahead.length;
    const own = /((?:[\w$]+\s*::\s*)*)$/.exec(ahead)[1];
    groups.unshift(own);
    i -= own.length;
    let depth = 0;
    for (i -= 1; i >= 0; i--) {
      const ch = ahead[i];
      if (ch === '}') depth += 1;
      else if (ch === ';') break;
      else if (ch === '{') {
        if (depth > 0) { depth -= 1; continue; }
        const m = /((?:[\w$]+\s*::\s*)+)$/.exec(ahead.slice(0, i));
        if (!m) break;
        groups.unshift(m[1]);
        i -= m[1].length;
      }
    }
    return groups.join('').split('::').map((part) => part.trim()).filter(Boolean);
  }

  /** 그 경로(확장자 없이 적었을 수 있다)의 파일이 프로젝트에 있으면 전체 경로. */
  async function fileAt(relative) {
    const files = await env.files();
    const wanted = relative.toLowerCase();
    const hit = files.find((file) => {
      const lower = file.toLowerCase();
      return lower === wanted || lower.replace(/\.[^./]+$/, '') === wanted || lower.replace(/\/(?:index|__init__|mod)\.[^./]+$/, '') === wanted;
    });
    return hit ? env.full(hit) : null;
  }

  /** import가 그 이름을 어디서 가져오는지로 정의를 고른다. { defs } | { external } | { file } | null(가릴 수 없다) */
  async function byImport(ctx, name, found) {
    const { lang, imports } = ctx;
    const dots = (def) => def.file.replace(/\.[^./]+$/, '').split('/');
    const endsWith = (parts, tail) => tail.length > 0 && tail.length <= parts.length && tail.every((each, k) => parts[parts.length - tail.length + k] === each);
    if (lang === 'java' || lang === 'kotlin' || lang === 'groovy') {
      const hit = importHit(ctx, name);
      if (hit) {
        const { parts } = hit;
        const last = parts.length - 1;
        // 경로가 곧 파일이다. 안쪽 자료형·static 멤버면 그 앞의 어느 조각이 파일이고(`a.b.Outer.Mid.Inner`), 감싼 이름이 바로 앞 조각이다.
        const exact = found.filter((def) => {
          const at = dots(def);
          if (!def.container && endsWith(at, parts)) return true;
          for (let cut = last; cut >= Math.max(1, last - 3); cut--) if (def.container === parts[last - 1] && endsWith(at, parts.slice(0, cut))) return true;
          // 파일 이름이 클래스 이름과 다른 맨 바깥 자료형(Kotlin, Java의 public 아닌 클래스): 패키지의 폴더가 맞으면 된다.
          return !def.container && endsWith(at.slice(0, -1), parts.slice(0, -1));
        });
        if (exact.length) return { defs: exact };
        // Kotlin은 패키지와 폴더가 달라도 된다: 같은 이름이 이 폴더에 있으면 밖이라고 하지 않는다.
        return found.length && lang !== 'java' ? null : { external: parts.join('.') };
      }
      const here = env.dirOf(env.lower(ctx.path));
      const same = found.filter((def) => !def.container && env.dirOf(env.lower(env.full(def.file))) === here);
      if (same.length) return { defs: same };
      for (const pack of imports.globs) {
        const inPack = found.filter((def) => !def.container && endsWith(dots(def).slice(0, -1), pack));
        if (inPack.length) return { defs: inPack };
      }
      for (const owner of imports.statics) {
        const inType = found.filter((def) => def.container === owner[owner.length - 1] && endsWith(dots(def), owner));
        if (inType.length) return { defs: inType };
      }
      // import 없이 쓰는 대문자 이름은 java.lang의 것이거나 이 패키지의 것이다. 이 폴더에 그런 이름이 하나도 없으면 밖의 것이다(자료형 인자 `T`는 빼고).
      if (!found.length && isUpper(name) && !isGeneric(name)) {
        const packs = imports.globs.map((pack) => `${pack.join('.')}.*`).filter((pack) => /^(?:java|javax|jakarta|kotlin|org|com\.google|com\.fasterxml)\./.test(pack));
        return { external: packs.length ? `${packs.slice(0, 3).join(', ')} 또는 java.lang` : lang === 'java' ? 'java.lang 또는 * 로 가져온 라이브러리' : '' };
      }
      return null;
    }
    if (lang === 'rust') {
      const hit = importHit(ctx, name);
      if (!hit) return null;
      const { parts } = hit;
      const real = parts[parts.length - 1];
      // 표준 라이브러리이거나 이 폴더에 없는 크레이트에서 가져온 이름이면, 이 폴더에 같은 이름이 있어도 그것이 아니다.
      if (!(await moduleHere({ ...ctx, imports: { ...imports, names: new Map(), scoped: [] } }, parts.slice(0, -1)))) return { external: parts.join('::') };
      let all = real === name ? found : await defsOf(real);
      if (['crate', 'self', 'super'].includes(parts[0])) {
        const mine = crateOf(env.relative(ctx.path));
        const same = all.filter((def) => crateOf(def.file) === mine);
        if (same.length) all = same;
      }
      if (!all.length) return null;
      const picked = moduleFirst(all.filter((def) => !def.container || isUpper(real)), parts.slice(0, -1));
      return picked.length ? { defs: picked, loose: picked.length === all.length } : null;
    }
    if (lang === 'js' || lang === 'vue') {
      const hit = importHit(ctx, name);
      if (!hit) return null;
      const real = hit.real || name;
      const all = real === name ? found : await defsOf(real);
      if (!/^[./]/.test(hit.from)) return all.length ? null : { external: hit.from };
      // 상대 경로: 이 파일의 폴더에서 풀어 본다(확장자와 /index는 적지 않을 수 있다).
      const base = env.relative(env.dirOf(ctx.path)).split('/').filter(Boolean);
      for (const part of hit.from.split('/')) {
        if (part === '..') base.pop(); else if (part !== '.' && part) base.push(part);
      }
      const target = base.join('/').toLowerCase().replace(/\.(?:m?[jt]sx?|vue)$/, '');
      const exact = all.filter((def) => { const path = def.file.toLowerCase().replace(/\.[^./]+$/, ''); return path === target || path === `${target}/index`; });
      if (exact.length) return { defs: exact };
      // 통째로 가져온 것(`import x from './x'`, `import * as x`)이면 그 파일로 간다.
      const file = hit.whole ? await fileAt(base.join('/')) : null;
      return file ? { file } : null;
    }
    if (lang === 'python') {
      const hit = importHit(ctx, name);
      if (!hit) return null;
      const real = hit.real || name;
      const all = real === name ? found : await defsOf(real);
      const mods = hit.real ? hit.parts.slice(0, -1) : hit.parts;
      const exact = hit.real ? all.filter((def) => { const parts = dots(def); return endsWith(parts, mods) || endsWith(parts, [...mods, '__init__']); }) : [];
      if (exact.length) return { defs: exact };
      // 모듈을 가져온 것(`from pkg import other`, `import pkg.other`)이면 그 파일로 간다.
      const tail = hit.parts.join('/');
      const here = env.relative(env.dirOf(ctx.path));
      const file = await fileAt(tail) || await fileAt(`${here}/${tail}`) || (await env.files()).map((each) => each).filter((each) => each.toLowerCase().replace(/\.py$/, '').endsWith(`/${tail.toLowerCase()}`)).map((each) => env.full(each))[0] || null;
      if (file) return { file };
      return all.length ? null : { external: hit.parts.join('.') };
    }
    return null;
  }

  /** 그 자리의 파일이 그 자료형 이름을 이 폴더 밖에서 가져오는지. */
  async function importedFromOutside(ctx, type) {
    if (!importHit(ctx, type)) return false;
    const types = (await defsOf(type)).filter((def) => TYPE_KINDS.includes(def.kind) || def.kind === 'type');
    return !!(await byImport(ctx, type, types))?.external;
  }

  /**
   * 그 자리에서 그 이름이 가리키는 자료형을 정의한 파일들. 그런 자료형이 이 폴더에 없으면 undefined, 여럿인데 가릴 수 없으면 null.
   */
  async function homesOf(ctx, full) {
    const type = plain(full);
    let types = (await defsOf(type)).filter((def) => TYPE_KINDS.includes(def.kind) || def.kind === 'type');
    if (!types.length) return undefined;
    const files = (defs) => [...new Set(defs.map((def) => def.file))];
    // 안쪽 자료형(`Outer.Inner`): 바깥 자료형의 파일에 든 것
    if (full.includes('.')) {
      const outer = full.slice(0, full.indexOf('.'));
      const outerFiles = (await homesOf(ctx, outer)) || files((await defsOf(outer)).filter((def) => TYPE_KINDS.includes(def.kind)));
      const inner = types.filter((def) => outerFiles.includes(def.file));
      if (inner.length) return files(inner);
    }
    const mine = env.lower(ctx.path);
    const here = types.filter((def) => env.lower(env.full(def.file)) === mine);
    if (here.length) return files(here);
    if (full.includes('::')) types = moduleFirst(types, expandMods(ctx, full.split('::').slice(0, -1)));
    const picked = await byImport(ctx, type, types);
    if (picked?.defs?.length) return files(picked.defs);
    return types.length === 1 ? files(types) : null;
  }

  /**
   * 찾은 멤버 가운데 그 자리에서 그 이름이 가리키는 자료형의 것만 남긴다. 이름이 같은 자료형이 여럿이면(패키지·크레이트마다 있는 `Config`)
   * 이 파일에서 정의했거나 import한 것의 멤버만 맞다. 어느 것인지 가릴 수 없으면 모두 남긴다(고르게 된다).
   */
  async function homeOnly(ctx, full, found) {
    if (!found.length) return found;
    const files = await homesOf(ctx, full);
    if (!files) return found; // 정의는 없고 impl만 있는 자료형(밖의 자료형에 붙인 확장)이거나, 어느 것인지 가릴 수 없다
    const same = found.filter((def) => files.includes(def.file));
    if (same.length || ONE_FILE.includes(env.langOf(files[0]))) return same;
    // Rust·C·Go: impl이 다른 파일에 있을 수 있다. 같은 크레이트(`src` 앞까지가 같은 폴더) 안의 것만 맞다.
    const roots = files.map(crateOf);
    return found.filter((def) => roots.includes(crateOf(def.file)));
  }

  /**
   * 자료형과 그 상위 자료형들을 층마다: [[자료형], [그 상위 자료형들], …]. 이 폴더에 정의가 없으면 `defined`가 거짓.
   * `foreign`: 이 폴더에 정의가 없는 상위 자료형(라이브러리의 것 — 거기서 물려받은 멤버는 이 폴더에 없다).
   */
  const lineage = (type) => cached(`l|${env.root()}|${type}`, async () => {
    const levels = [[type]];
    const seen = new Set([type]);
    const foreign = [];
    let defined = false;
    for (let depth = 0; depth < 6; depth++) {
      const next = [];
      for (const each of levels[depth]) {
        const all = await defsOf(each);
        const types = all.filter((def) => TYPE_KINDS.includes(def.kind));
        // 별칭(`type Range = Match;`): 가리키는 자료형(맨 바깥 것 하나)의 것을 그대로 쓴다.
        const alias = types.length || PRELUDE.has(each) ? [] : all.filter((def) => def.kind === 'type')
          .map((def) => typeNames((def.sig || def.text || '').split('=').slice(1).join('=')).map(plain).find((name) => !isGeneric(name))).filter(Boolean);
        if (depth === 0 && (types.length || alias.length)) defined = true;
        if (depth > 0 && !types.length && !alias.length) foreign.push(each);
        for (const parent of [...types.flatMap((def) => def.supers || []), ...alias]) if (!seen.has(parent)) { seen.add(parent); next.push(parent); }
      }
      if (!next.length) break;
      levels.push(next);
    }
    return { levels, defined, foreign };
  }, { levels: [[type]], defined: false, foreign: [] });

  const preferKind = (found, call) => {
    const callable = found.filter((def) => CALLABLE.includes(def.kind));
    const others = found.filter((def) => !CALLABLE.includes(def.kind));
    // 부르는 자리가 아니면 생성자를 가리킬 수 없다(`extends Base`의 Base는 클래스다).
    return call ? (callable.length ? callable : others) : (others.length ? others : callable.filter((def) => def.kind !== 'constructor'));
  };
  const fits = (def, argc) => { const range = arity(def); return !range || (argc >= range[0] && argc <= range[1]); };
  const narrowArity = (found, argc) => {
    if (found.length < 2 || argc == null) return found;
    const fit = found.filter((def) => fits(def, argc));
    return fit.length ? fit : found;
  };

  /**
   * 묶음의 자료형(과 그 상위 자료형)에서 그 이름의 멤버를 찾는다.
   * - `known`: 받는 쪽 자료형이 이 폴더의 것이고 그 계보가 모두 이 폴더에 있다(그런데 없으면 정말 없는 것이다).
   * - `outside`: 그 이름을 가졌을 수 있는 이 폴더 밖의 자료형(맨 바깥 자료형이거나 라이브러리의 상위 자료형).
   * - `fieldOnly`: 필드를 읽는 자리(`a.b`)라 메서드는 답이 아니다.
   */
  async function findMember(receiver, name, { call = false, argc = null, accessor = true, fieldOnly = false } = {}) {
    const ctx = await ctxOf(receiver.src);
    const tried = [];
    let outside = receiver.outside || '';
    let misfit = null; // 이름은 맞는데 인자 수가 맞지 않은 것(다른 데서 못 찾으면 이것이라도 내놓는다)
    const pick = (found) => {
      const sameLang = found.filter((def) => env.langOf(def.file) === ctx.lang);
      let list = preferKind(sameLang.length ? sameLang : found, call);
      if (fieldOnly) list = list.filter((def) => !CALLABLE.includes(def.kind));
      if (argc == null || !list.length) return list;
      const fit = list.filter((def) => fits(def, argc));
      if (!fit.length) misfit ||= list;
      return fit;
    };
    for (const full of receiver.outside ? [] : receiver.names) {
      const type = plain(full);
      let here = full !== ARRAY;
      // 경로가 붙은 이름은 그 모듈이, 가져온 이름은 그 import가 이 폴더 밖을 가리키면 이 폴더에 같은 이름의 자료형이 있어도 그것이 아니다.
      if (here && full.includes('::')) here = await moduleHere(ctx, full.split('::').slice(0, -1));
      else if (here && await importedFromOutside(ctx, full.includes('.') ? full.slice(0, full.indexOf('.')) : type)) here = false;
      if (here) {
        // 그 자료형에 바로 있는지(정의는 없고 impl만 있는 자료형, 밖의 자료형에 붙인 확장도 여기서 걸린다)
        const own = await homeOnly(ctx, full, pick(await defsOf(name, [type])));
        if (own.length) return { defs: own, owner: type, known: true };
        const family = await lineage(type);
        if (family.defined) {
          tried.push(type);
          // 상위 자료형의 것: 이 자료형과 같은 파일에 든 것이 있으면 그것이다(파일마다 있는 안쪽 클래스 `Criteria extends GeneratedCriteria`).
          const homes = family.levels.length > 1 ? await homesOf(ctx, full) : null;
          for (const level of family.levels.slice(1)) {
            const found = pick(await defsOf(name, level));
            const near = homes ? found.filter((def) => homes.includes(def.file)) : [];
            if (found.length) return { defs: near.length ? near : found, owner: type, known: true };
          }
          if (family.foreign.length && !outside) outside = family.foreign[0];
          // 닫힌 묶음: 이 자료형의 멤버가 아니면 안쪽 자료형의 것도 아니다(`Page<Order>`의 `getId`).
          if (!receiver.open) break;
          continue;
        }
      }
      if (!outside) outside = full === ARRAY ? '배열' : type;
      // 닫힌 묶음에서 안쪽으로 내려가는 것은 Rust의 감싸개(안의 것을 그대로 내놓는 밖의 자료형)뿐이다.
      if (!receiver.open && !(ctx.lang === 'rust' && full !== ARRAY && !CONTAINER.test(type))) break;
    }
    if (tried.length && accessor && call) {
      // Lombok·record: `getName()`·`setName(…)`·`isOpen()`은 정의가 없고 필드만 있다. record의 구성 요소는 `point.x()`로 읽는다.
      const m = /^(?:get|set|is)([A-Z][\w$]*)$/.exec(name);
      for (const field of m ? [lowerFirst(m[1]), name] : [name]) {
        const found = await findMember(receiver, field, { call: false, accessor: false, fieldOnly: true });
        if (found.defs.length && found.defs.every((def) => def.kind === 'field')) return { ...found, accessor: field !== name };
      }
    }
    if (misfit) return { defs: misfit, owner: tried[0] || '', known: true };
    return { defs: [], known: tried.length > 0 && !outside, tried, outside };
  }

  // ── 자료형 읽기 ──

  /** 한 글자짜리 자료형 인자(`M`)를 그 제약(`M: Matcher`, `T extends Base`)으로 바꾼다. 제약을 못 찾으면 뺀다. */
  function bounded(tab, line, names) {
    if (!names.some(isGeneric)) return names;
    const around = enclosingFunction(tab, line);
    let from = around ? around.line : line;
    // impl·클래스의 머리까지 올라가 본다.
    for (let n = from; n >= Math.max(1, from - 400); n--) {
      if (/^\s*(?:unsafe\s+)?impl\b|\b(?:class|interface|trait|struct|enum)\s+\w/.test(tab.lines[n - 1] || '')) { from = n; break; }
    }
    const text = [];
    for (let n = from; n <= line && text.length < 400; n++) text.push(codeLine(tab, n));
    const joined = text.join(' ');
    const out = [];
    for (const name of names) {
      if (!isGeneric(name)) { if (!out.includes(name)) out.push(name); continue; }
      const m = new RegExp(`(?<![\\w$])${name}\\s*(?::(?!:)|\\bextends\\b)\\s*([^,>{()=;]+(?:<[^<>]*>)?[^,>{()=;]*)`).exec(joined);
      for (const each of m ? typeNames(m[1]).filter((each) => !isGeneric(each)) : []) if (!out.includes(each)) out.push(each);
    }
    return out;
  }
  /** 이 파일에 적힌 자료형 글의 묶음. 자료형 인자뿐이라 알 수 없으면 null, 이름이 없으면(기본 자료형) 빈 묶음. */
  function bag(tab, line, text) {
    const all = bagNames(text);
    const ctx = { lang: tab.lang, path: tab.path, imports: importsFor(tab), tab, line };
    const names = bounded(tab, line, all).map((each) => (each.includes('::') || each === ARRAY ? each : qualified(ctx, each)));
    if (names.length) return { names };
    // 소문자 이름을 자료형으로 쓰는 언어(C·Go)에서는 이름을 못 읽은 것이지 기본 자료형이라는 뜻이 아니다.
    return all.length || ['c', 'go'].includes(tab.lang) ? null : { names: [] };
  }

  /** Go: 이름 뒤에 적힌 자료형(`store *Store`, `items []Item`). 넣는 값(`:=`)이면 null. */
  function goType(after, src) {
    const m = /^[ \t]+((?:\[\]|\*|map\[[^\]]*\]|chan\s+)*)([\w.]+)/.exec(after);
    if (!m || /^(?:func|struct|interface|range|return)$/.test(m[2])) return null;
    const name = m[2].split('.').pop();
    return { names: /\[\]|map\[|chan/.test(m[1]) ? [ARRAY, name] : [name], ...(src ? { src } : {}) };
  }

  /** 선언한 자리(줄·칸)에서 그 이름의 자료형을 읽는다: 적어 둔 자료형, 없으면 넣는 값의 자료형. */
  async function typeAtDecl(tab, line, col, name, depth) {
    const text = codeLine(tab, line);
    if (text.length > MAX_LINE) return null;
    const after = text.slice(col + name.length);
    const lang = tab.lang;
    if (lang === 'go') {
      const type = goType(after);
      if (type) return type;
    } else if (typeAfterLang(lang)) {
      const type = typeAfter(text, col + name.length);
      if (type && type !== '_') return bag(tab, line, type);
    } else {
      const type = typeBefore(text, col);
      if (type && !NOT_TYPES.has(type)) return bag(tab, line, type);
    }
    const opened = (type) => (type ? { ...type, open: true } : null);
    // 람다·클로저의 인자(`items.forEach(item -> …)`, `items.iter().map(|item| …)`): 그것을 넘겨받는 호출의 받는 쪽 묶음의 원소.
    // 원소를 넘겨주는 호출이거나 받는 쪽이 이 폴더 밖의 묶음(List·Vec)일 때만 그렇게 본다(`bus.subscribe(event -> …)`의 event는 bus가 아니다).
    const head = before(tab, line, col);
    const lambda = lang === 'rust' ? /\|[^|()]*$/.test(head) && /^\s*(?::[^|]*)?(?:,[^|]*)?\|/.test(after) : /^\s*(?:,[^()]*)?\)?\s*(?:->|=>)/.test(after) && !/[\w$>\]]\s+$/.test(head);
    if (lambda) {
      let at = head.length - 1, level = 0, open = -1;
      const outward = () => {
        for (; at >= 0; at--) {
          const ch = head[at];
          if (ch === ')') level += 1;
          else if (ch === '(') { if (level === 0) return at; level -= 1; }
        }
        return -1;
      };
      open = outward();
      // `(a, b) ->` 꼴이면 방금 찾은 괄호는 인자 목록이다: 한 번 더 밖으로 나간다.
      if (open >= 0 && /^\s*$/.test(head.slice(0, open).replace(/.*[(,]/, '')) && /\)\s*(?:->|=>)/.test(after) && lang !== 'rust') { at = open - 1; open = outward(); }
      const callee = open > 0 ? /(\.|::)\s*([\w$]+)\s*$/.exec(head.slice(0, open)) : null;
      if (!callee) return null;
      const receiver = await typeOfExpr(tab, line, head, callee.index, depth + 1, line);
      if (!receiver?.names?.length) return null;
      const outer = receiver.names[0];
      const wraps = outer === ARRAY || !(await lineage(plain(outer))).defined;
      return ITERATORS.has(callee[2]) || wraps ? opened(receiver) : null;
    }
    // Rust의 무늬에서 꺼낸 이름(`Printer::Standard(ref mut p) =>`): 그 열거 값이 품은 자료형
    if (lang === 'rust') {
      const pattern = /((?:[\w$]+::)*[A-Z][\w$]*)\s*\(((?:[^()]*,)?)\s*(?:ref\s+)?(?:mut\s+)?&?\s*$/.exec(text.slice(0, col));
      const path = pattern ? pattern[1].split('::') : [];
      const variant = path[path.length - 1];
      if (variant && !['Some', 'Ok', 'Err', 'Box', 'Arc', 'Rc'].includes(variant)) {
        let owner = path[path.length - 2] || '';
        if (owner === 'Self') owner = ownerTypes(tab, line)[0] || '';
        const found = (await defsOf(variant, owner ? [owner] : null)).filter((def) => def.kind === 'variant' || def.kind === 'struct');
        const inner = found.length === 1 ? /\(([^()]*(?:\([^()]*\)[^()]*)*)\)/.exec(found[0].sig || found[0].text || '')?.[1] : '';
        if (inner) {
          const part = splitArgs(inner, true)[splitArgs(pattern[2], true).length] || '';
          const names = bagNames(part).filter((each) => !isGeneric(each)).map((each) => (each === 'Self' ? found[0].container : each));
          if (names.length) return { names, src: found[0].file };
        }
      }
      // `Some(x) =>`·`Ok(v) =>`처럼 감싼 것에서 꺼낸 이름: 견주는 식(`match 식 {`)의 묶음을 열어 쓴다.
      if (pattern && !/(?<![=!<>])=(?![=>])/.test(after)) {
        const indent = /^\s*/.exec(text)[0].length;
        for (let n = line - 1; n >= Math.max(1, line - 80); n--) {
          const above = codeLine(tab, n);
          if (!above.trim() || /^\s*/.exec(above)[0].length >= indent) continue;
          const m = /\bmatch\s+(.+?)\s*\{\s*$/.exec(above);
          if (m) return opened(await typeOfExpr(tab, n, m[1], m[1].length, depth + 1, n));
          break;
        }
      }
    }
    // 넣는 값에서 읽는다: `let x = …;`, `var x = …;`, `if let Some(x) = … {`, `for x in … {`, `for (var x : …)`
    const assign = /^[^=;{]*?(?<![=!<>])=(?![=>])\s*/.exec(after);
    const each = assign ? null : /^[\s,)\]\w&]*\bin\s+/.exec(after) || /^\s*:\s*/.exec(after);
    let m = assign || each;
    if (!m) return null;
    const start = col + name.length + m[0].length;
    // 식이 끝나는 자리: 맨 바깥의 `;`. 다음 줄로 이어지는 것은 괄호가 열려 있거나, 줄이 연산자로 끝나거나, 다음 줄이 `.`으로 시작할 때뿐이다
    // (세미콜론을 쓰지 않는 언어에서 뒤따르는 문장을 이 식으로 읽지 않는다). `if let`·`for`는 몸통을 여는 `{` 앞.
    const block = /\b(?:if|while|for)\b/.test(text.slice(0, col));
    const depthOf = (s) => { let d = 0; for (const ch of s) { if ('([{'.includes(ch)) d += 1; else if (')]}'.includes(ch)) d -= 1; } return d; };
    let joined = text.slice(start);
    for (let n = line + 1; n <= line + 8 && n <= tab.lines.length && !block && !/;/.test(joined); n++) {
      const next = codeLine(tab, n).trim();
      if (!(depthOf(joined) > 0 || /[.,=+\-*/|&?(]\s*$/.test(joined) || /^(?:\.|\?\.)/.test(next))) break;
      joined += ` ${next}`;
    }
    let end = joined.length, level = 0;
    for (let i = 0; i < joined.length; i++) {
      const ch = joined[i];
      if ('([{'.includes(ch)) { if (ch === '{' && level === 0 && block) { end = i; break; } level += 1; }
      else if (')]}'.includes(ch)) { if (level === 0) { end = i; break; } level -= 1; }
      else if (ch === ';' && level === 0) { end = i; break; }
    }
    let value = joined.slice(0, end).trim().replace(/^(?:&\s*(?:mut\s+)?|\*|await\s+|mut\s+)+/, '');
    // 원소를 꺼내는 자리(`for x in …`, `for (var x : …)`, `if let Some(x) = …`)면 묶음을 연다.
    const element = !!each || /\b(?:Some|Ok|Err)\s*\(\s*(?:ref\s+)?(?:mut\s+)?&?\s*$/.test(text.slice(0, col));
    // 자료형을 바꿔 적은 값: `(Order) obj`, `obj as Order`
    m = /^\(\s*([A-Z][\w$.<>, ?[\]]*)\s*\)\s*[\w$(]/.exec(value) || /\bas\s+([\w$:<>&' ]+?)\s*$/.exec(value);
    if (m) return bag(tab, line, m[1]);
    // `Foo { … }`, `Foo::Bar { … }`: 구조체를 만드는 식
    m = /^((?:[\w$]+::)*[A-Z][\w$]*)\s*\{/.exec(value);
    if (m) {
      const ctx = { lang, path: tab.path, imports: importsFor(tab), tab, line };
      return { names: typeNames(m[1]).map(plain).map((each) => (each === 'Self' ? ownerTypes(tab, line)[0] || each : qualified(ctx, each))) };
    }
    if (/^(?:match|if|loop|while|unsafe|async|move|\||\[)/.test(value)) return null;
    // 글자·숫자 값과 표준 매크로(`vec![…]`, `format!(…)`): 이 폴더의 자료형이 아니다. 그 밖의 매크로가 내놓는 것은 알 수 없다.
    if (/^(?:vec!|Vec::)/.test(value) && !/\)\s*\./.test(value)) return { names: ['Vec'] };
    if (/^(?:format!|b?["'])/.test(value) && !/\)\s*\./.test(value)) return { names: ['String'] };
    if (/^\d/.test(value)) return { names: [] };
    if (/^[\w$:]+!\s*[[({]/.test(value)) return null;
    value = value.replace(/\s+$/, '');
    const type = await typeOfExpr(tab, line, value, value.length, depth + 1, line - 1);
    return element ? opened(type) : type;
  }

  /** 변수(지역 변수·인자·필드)의 자료형. 모르면 null. */
  async function varType(tab, name, line, depth, from = line) {
    const local = from >= 1 ? localOf(tab, name, from, null) : null;
    if (local) return typeAtDecl(tab, local.line, local.col, name, depth);
    // 이 파일에 선언한 필드(Java의 `this.` 없는 필드), 맨 바깥의 상수·변수
    const owners = ownerTypes(tab, line);
    const field = (tab.outline || []).find((def) => def.name === name && VALUE_KINDS.includes(def.kind) && (owners.includes(def.container) || !def.container));
    if (field) return typeAtDecl(tab, field.line, field.col, name, depth);
    // 물려받은 필드
    if (owners.length && tab.lang !== 'rust') {
      const found = await findMember({ names: [owners[0]] }, name, { call: false, fieldOnly: true });
      if (found.defs.length === 1) return declBag(found.defs[0]);
    }
    return null;
  }

  /** 서명에서 읽은 자료형 글의 묶음: 이름이 없으면(기본 자료형) 빈 묶음, 자료형 인자뿐이면 null. 그 이름들은 그 파일의 import로 푼다(`src`). */
  function sigBag(type, def) {
    const all = bagNames(type).map((each) => (each === 'Self' ? def.container : each));
    const names = all.filter((each) => !isGeneric(each));
    if (!names.length && all.length) return null;
    return { names, src: def.file };
  }

  /** 다른 파일의 정의(필드·상수)에서 그 자료형을 읽는다(서명으로). */
  function declBag(def) {
    if (def.kind === 'variant') return { names: [def.container], src: def.file };
    if (TYPE_KINDS.includes(def.kind)) return { names: [def.name], isStatic: true, src: def.file };
    const sig = def.sig || def.text || '';
    const at = new RegExp(`(?<![\\w$])${esc(def.name)}(?![\\w$])`).exec(sig);
    if (!at) return null;
    if (langOfDef(def) === 'go') return goType(sig.slice(at.index + def.name.length), def.file);
    const type = typeAfterLang(langOfDef(def)) ? typeAfter(sig, at.index + def.name.length) : typeBefore(sig, at.index);
    return type ? sigBag(type, def) : null;
  }

  /** 함수·메서드의 정의에서 돌려주는 자료형을 읽는다. `receiver`는 받는 쪽의 묶음(자료형 인자를 돌려주는 메서드면 그 안의 것을 꺼낸 것으로 본다). */
  function returnBag(def, receiver) {
    if (def.kind === 'constructor') return { names: [def.container], src: def.file };
    if (!CALLABLE.includes(def.kind)) return declBag(def);
    const sig = def.sig || '';
    const lang = langOfDef(def);
    let type = '';
    if (lang === 'go') {
      // func (s *Store) Get(key string) *Item — 인자 목록 뒤의 것
      const open = sig.indexOf('(', sig.search(new RegExp(`(?<![\\w$])${esc(def.name)}\\s*\\(`)));
      let level = 0, close = -1;
      for (let i = open; i >= 0 && i < sig.length; i++) { if (sig[i] === '(') level += 1; else if (sig[i] === ')') { level -= 1; if (level === 0) { close = i; break; } } }
      return close >= 0 ? goType(` ${sig.slice(close + 1).replace(/^\s*\(?\s*/, '')}`, def.file) : null;
    }
    if (lang === 'rust') type = /->\s*(.+?)\s*(?:\bwhere\b.*)?$/.exec(sig)?.[1] || '';
    else if (lang === 'js' || lang === 'vue' || lang === 'kotlin' || lang === 'python') {
      const close = sig.lastIndexOf(')');
      type = close >= 0 ? /^\s*(?::|->)\s*([^{=]+)/.exec(sig.slice(close + 1))?.[1] || '' : '';
    } else {
      const at = new RegExp(`(?<![\\w$])${esc(def.name)}\\s*\\(`).exec(sig);
      type = at ? typeBefore(sig, at.index) : '';
    }
    if (!type) return null;
    const read = sigBag(type, def);
    if (read) return read;
    // 자료형 인자만 돌려주면(`T getData()`), 받는 쪽의 묶음에서 이 자료형을 뺀 것(감싸인 자료형)을 연 것으로 본다.
    const rest = receiver ? receiver.names.filter((each) => plain(each) !== def.container) : [];
    return rest.length ? { ...receiver, names: rest, open: true } : null;
  }

  /** 여럿이 찾아졌을 때의 자료형: 모두 같은 자료형을 내놓을 때만 그것으로 본다. */
  function sameBag(defs, read) {
    const bags = defs.map(read);
    if (bags.some((each) => !each)) return null;
    const key = (each) => each.names.join(',');
    return bags.every((each) => key(each) === key(bags[0])) ? bags[0] : null;
  }

  /** 사슬의 첫 조각의 자료형. `pathAfter`: 그 뒤가 `::`인지(Rust에서 `::` 앞의 이름은 변수가 아니라 모듈·자료형이다). */
  async function baseType(tab, line, seg, depth, from, pathAfter) {
    const { name } = seg;
    const ctx = { lang: tab.lang, path: tab.path, imports: importsFor(tab), tab, line };
    const owners = ownerTypes(tab, line);
    if (name === 'this' || name === 'self') return owners.length ? { names: [owners[0]] } : null;
    if (name === 'Self') return owners.length ? { names: [owners[0]], isStatic: true } : null;
    if (name === 'super') {
      if (!owners.length) return null;
      const family = await lineage(owners[0]);
      return family.levels[1]?.length ? { names: family.levels[1] } : null;
    }
    if (name === 'crate') return { names: [], module: ['crate'] };
    if (seg.isNew) return { names: [qualified(ctx, name)] };
    if (seg.call) {
      // `foo(…)`: 이 자료형의 메서드이거나 함수. 대문자로 시작하면 자료형을 만드는 것으로 본다.
      const target = await unqualified(tab, line, name, { call: true, argc: seg.argc ?? null });
      if (target.length) return sameBag(target, (def) => returnBag(def, null));
      return isUpper(name) ? { names: [qualified(ctx, name)] } : null;
    }
    const path = pathAfter && tab.lang === 'rust';
    if (!path) {
      const declared = await varType(tab, name, line, depth, from);
      if (declared) return declared;
      // 대문자뿐인 이름(`FLAGS`, `LOGGER`)은 상수일 수 있다: 하나로 가려지면 그 상수의 자료형
      if (/^[A-Z][A-Z0-9_]+$/.test(name)) {
        const constants = (await defsOf(name)).filter((def) => VALUE_KINDS.includes(def.kind));
        const mine = constants.filter((def) => env.lower(env.full(def.file)) === env.lower(tab.path));
        const picked = mine.length ? mine : (await byImport(ctx, name, constants))?.defs || (constants.length === 1 ? constants : []);
        if (picked.length === 1) return declBag(picked[0]);
        if (constants.length) return null;
      }
    }
    if (isUpper(name)) return { names: [qualified(ctx, name)], isStatic: true };
    // 선언을 못 찾은 소문자 이름: 가져온 모듈(`use std::io` 뒤의 `io::…`, `import * as utils`)이거나 Rust의 경로(`grep::searcher::…`)면 모듈이다.
    // 그 밖에는 자료형을 알아내지 못한 변수다(물려받은 필드, Lombok의 `log`).
    if (importHit(ctx, name) || path) return { names: [], module: [name] };
    return null;
  }

  /** 사슬의 다음 조각의 자료형: 받는 쪽 묶음에서 그 멤버를 찾아 그 자료형을 읽는다. */
  async function memberType(tab, receiver, seg) {
    if (receiver.outside) return { ...receiver, far: true }; // 밖의 것을 거친 값: 어느 자료형인지는 모른다
    if (receiver.module) {
      const ctx = { lang: tab.lang, path: tab.path, imports: importsFor(tab), tab, line: clickLine };
      const full = expandMods(ctx, receiver.module);
      // 이 폴더에 없는 모듈의 것은 그 뒤로도 밖의 것이다(`std::fs::File::open`의 `File`은 이 폴더의 `File`이 아니다).
      if (tab.lang === 'rust' && !(await moduleHere(ctx, receiver.module))) return { names: isUpper(seg.name) ? [seg.name] : [], outside: [...full, seg.name].join('::') };
      if (isUpper(seg.name)) return { names: [[...receiver.module, seg.name].join('::')], isStatic: true };
      if (!seg.call) return { names: [], module: [...receiver.module, seg.name] };
      const found = moduleFirst((await defsOf(seg.name)).filter((def) => def.kind === 'function'), full);
      return found.length === 1 ? returnBag(found[0], null) : null;
    }
    // Rust의 `.await`: 기다린 값은 그 안의 것이다.
    if (seg.name === 'await' && !seg.call) return { ...receiver, open: true };
    const found = await findMember(receiver, seg.name, { call: seg.call, argc: seg.argc ?? null, fieldOnly: !seg.call && !seg.path && STRICT.includes(tab.lang) });
    if (found.defs.length) return sameBag(found.defs, (def) => returnBag(def, receiver));
    if (seg.call && TRANSFORMERS.has(seg.name)) return null;
    // 받은 것을 그대로 내놓는 메서드: 이 폴더의 자료형이면 그대로(`config.clone()`), 밖의 묶음이면 안의 것을 꺼낸 것으로(`list.get(0)`).
    if (seg.call && TRANSPARENT.has(seg.name)) return found.known ? receiver : { ...receiver, open: true };
    // `Outer.Inner`, `Enum::Variant`
    if (!seg.call && isUpper(seg.name) && receiver.isStatic) return { names: [seg.name], isStatic: true, src: receiver.src };
    // 받는 쪽이 이 폴더 밖의 것이면 그 메서드도 같은 묶음을 내놓는다고 본다(닫힌 채로: 그 멤버는 여전히 밖의 것이다).
    return found.known || !seg.call ? null : receiver;
  }

  /** 받는 쪽 식(`text`의 `end` 앞에서 끝나는 식)의 자료형. 모르면 null. */
  async function typeOfExpr(tab, line, text, end, depth = 0, from = line, tailPath = false) {
    if (depth > 4) return null;
    const segs = chainBefore(text, end, tab.lang === 'c');
    if (!segs) return null;
    let type = null;
    for (let k = 0; k < segs.length; k++) {
      const seg = segs[k];
      if (seg.index) { if (!type) return null; type = { ...type, open: true }; continue; }
      const pathAfter = k + 1 < segs.length ? !!segs[k + 1].path : tailPath;
      type = k === 0 || !type ? await baseType(tab, line, seg, depth, from, pathAfter) : await memberType(tab, type, seg);
      if (!type) return null;
      if (seg.unwrap && !type.module) type = { ...type, open: true };
    }
    return type;
  }

  /** 받는 쪽 없이 쓴 이름의 정의들(이 자료형의 멤버, import한 것). 가릴 수 없으면 빈 배열. */
  async function unqualified(tab, line, name, { call = false, argc = null } = {}) {
    // 이 자료형(과 바깥·상위 자료형)의 멤버. Rust의 메서드는 `self.` 없이 부를 수 없으니 보지 않는다.
    if (tab.lang !== 'rust') {
      for (const owner of ownerTypes(tab, line)) {
        const found = await findMember({ names: [owner] }, name, { call, argc, accessor: false });
        if (found.defs.length) return found.defs;
      }
    }
    const ctx = { lang: tab.lang, path: tab.path, imports: importsFor(tab), tab, line };
    const picked = await byImport(ctx, name, await defsOf(name));
    if (picked?.defs && !picked.loose) return narrowArity(preferKind(picked.defs, call), argc);
    return [];
  }

  /** 부르는 자리의 인자 수. 세지 못하면(제네릭 인자의 쉼표와 가릴 수 없으면) null. */
  function countArgs(tab, line, col) {
    let text = codeLine(tab, line).slice(col);
    const lead = /^\s*(?:::\s*<[^()]*>\s*)?\(/.exec(text);
    if (!lead) return null;
    text = text.slice(lead[0].length);
    for (let n = line + 1; n <= line + 12 && n <= tab.lines.length; n++) text += ` ${codeLine(tab, n)}`;
    let depth = 0, commas = 0, any = false, since = false, pipes = false;
    for (let i = 0; i < text.length; i++) {
      const ch = text[i];
      if (tab.lang === 'rust' && ch === '|' && depth === 0 && text[i + 1] !== '|' && text[i - 1] !== '|') { pipes = !pipes; any = since = true; continue; }
      if (pipes) continue;
      // 이름에 바로 붙은 `<`는 제네릭 인자일 수 있다(`new HashMap<String, Object>()`): 그 안의 쉼표와 가릴 수 없으니 세지 않는다.
      if (ch === '<' && depth === 0 && /[\w$]/.test(text[i - 1] || '') && /^<[\w$.,\s<>?[\]&']*>/.test(text.slice(i))) return null;
      if ('([{'.includes(ch)) { depth += 1; any = since = true; }
      else if (')]}'.includes(ch)) { if (depth === 0) return any ? commas + (since ? 1 : 0) : 0; depth -= 1; }
      else if (ch === ',' && depth === 0) { commas += 1; since = false; }
      else if (!/\s/.test(ch)) any = since = true;
    }
    return null;
  }

  /** `name`을 물려받거나 구현하는 것들을 아래로 따라가며 모은다(자료형 정의와 Rust의 impl). 묻는 수를 묶어 한 번에, 정해 둔 만큼만 묻는다. */
  async function subtypesDeep(name) {
    const out = [];
    const seen = new Set([name]);
    let level = [name], asked = 0;
    for (let depth = 0; depth < 4 && level.length && asked < 60; depth++) {
      const batch = level.slice(0, 60 - asked);
      asked += batch.length;
      const next = [];
      for (const list of await Promise.all(batch.map((each) => subtypesOf(each)))) {
        for (const def of list) {
          out.push(def);
          if (def.kind !== 'impl' && !seen.has(def.name)) { seen.add(def.name); next.push(def.name); }
        }
      }
      level = next;
    }
    return out;
  }

  /** 메서드의 구현들: 그것을 가진 자료형을 물려받거나 구현하는 자료형들의 같은 이름. */
  async function implementationsOf(def) {
    if (TYPE_KINDS.includes(def.kind)) return subtypesDeep(def.name);
    if (!def.container) return [];
    const owners = [...new Set((await subtypesDeep(def.container)).map((each) => each.name))];
    if (!owners.length) return [];
    return (await defsOf(def.name, owners)).filter((each) => CALLABLE.includes(each.kind) === CALLABLE.includes(def.kind));
  }

  /** Rust의 `mod name;`이 가리키는 파일(전체 경로). 없으면 null. */
  async function moduleFile(declaredIn, name) {
    const dir = env.dirOf(declaredIn);
    const stem = env.nameOf(declaredIn).replace(/\.[^.]+$/, '');
    const bases = ['mod', 'lib', 'main'].includes(stem) ? [dir] : [`${dir}\\${stem}`, dir];
    const files = new Set((await env.files()).map((file) => file.toLowerCase()));
    for (const base of bases) {
      for (const path of [`${base}\\${name}.rs`, `${base}\\${name}\\mod.rs`]) {
        if (files.has(env.relative(path).toLowerCase())) return path;
      }
    }
    return null;
  }
  const declaresModule = (def) => def.kind === 'module' && /\bmod\s+[\w$]+\s*$/.test((def.sig || '').trim()) && /;\s*(?:\/\/.*)?$/.test(def.text || '');

  /** 고른 정의들로 답을 만든다: 하나면 바로, 여럿이면 고르게. `mod name;`이면 그 파일로 이어 간다. */
  async function done(found, tab, where, note = '') {
    const sameLang = found.filter((def) => env.langOf(def.file) === tab.lang);
    if (sameLang.length && sameLang.length < found.length) found = sameLang;
    const defs = found.length > 1 ? env.rank(found, where).map((each) => each.def) : found;
    if (defs.length === 1 && declaresModule(defs[0])) {
      const file = await moduleFile(env.full(defs[0].file), defs[0].name);
      if (file) return { kind: 'file', path: file };
    }
    // 답이 누른 자리 그대로면(구조를 아직 읽지 못한 파일의 정의) 갈 곳이 없다: 쓰는 곳을 보여 준다.
    if (defs.length === 1 && defs[0].line === where.line && defs[0].col === where.from && env.lower(env.full(defs[0].file)) === env.lower(tab.path)) return { kind: 'usages' };
    return defs.length === 1 ? { kind: 'def', def: defs[0], note } : { kind: 'pick', defs, note };
  }

  /** 누른 자리가 바로 정의일 때: 구현(인터페이스·트레이트·추상 메서드), 짝(매퍼 XML ↔ 인터페이스), 아니면 쓰는 곳. */
  async function fromDefinition(tab, where, def) {
    if (def.kind === 'module' && tab.lang === 'rust' && /;\s*$/.test(codeLine(tab, where.line).trimEnd())) {
      const file = await moduleFile(tab.path, def.name);
      if (file) return { kind: 'file', path: file };
    }
    const mine = env.lower(tab.path);
    if (CALLABLE.includes(def.kind) && def.container) {
      // 구현을 찾는 것은 몸통이 없는 선언일 때만 한다(인터페이스·트레이트·abstract). 보통의 메서드는 곧바로 쓰는 곳을 보여 준다.
      const owner = (tab.outline || []).find((each) => each.name === def.container && TYPE_KINDS.includes(each.kind));
      const abstract = owner && (owner.kind === 'interface' || owner.kind === 'trait' || /\babstract\b/.test(codeLine(tab, owner.line)) || /\babstract\b/.test(codeLine(tab, def.line)));
      const impls = abstract ? await implementationsOf(def) : [];
      if (impls.length) return { ...(await done(impls, tab, where)), note: `"${def.name}"의 구현` };
      // 같은 자료형 이름 아래의 같은 이름이 다른 언어의 파일에 있으면 그것이 짝이다(MyBatis의 매퍼 XML ↔ 인터페이스). C는 선언(.h) ↔ 구현(.c).
      const twins = (await defsOf(def.name, [def.container])).filter((each) => env.lower(env.full(each.file)) !== mine && (env.langOf(each.file) !== tab.lang || tab.lang === 'c'));
      if (twins.length) return { ...(await done(twins, tab, where)), note: `"${def.container}.${def.name}"의 짝` };
    }
    // C·C++: 선언(.h)과 구현(.c)을 서로 오간다.
    if (tab.lang === 'c' && CALLABLE.includes(def.kind) && !def.container) {
      const others = (await defsOf(def.name)).filter((each) => env.lower(env.full(each.file)) !== mine && env.langOf(each.file) === 'c' && CALLABLE.includes(each.kind) && !each.container);
      if (others.length) return { ...(await done(others, tab, where)), note: `"${def.name}"의 선언·구현` };
    }
    // 자료형을 가리지 않는 언어(스타일·문서·스크립트)에서는 예전처럼 이름이 같은 다른 정의를 보여 준다.
    if (!['java', 'kotlin', 'rust', 'csharp', 'groovy', 'js', 'vue', 'python', 'go', 'c', 'xml'].includes(tab.lang)) {
      const others = (await defsOf(def.name)).filter((each) => !(env.lower(env.full(each.file)) === mine && each.line === def.line && each.col === def.col));
      if (others.length) return done(others, tab, where);
    }
    return { kind: 'usages' };
  }

  /**
   * 그 자리의 이름이 가리키는 정의를 찾는다. 답:
   * - { kind: 'local', at }: 이 파일 안의 선언(지역 변수·인자)
   * - { kind: 'def', def, note }: 정의 하나 · { kind: 'pick', defs, note }: 여럿(고르게 한다) · { kind: 'file', path }: 모듈의 파일
   * - { kind: 'usages' }: 누른 자리가 정의다(쓰는 곳을 보여 준다)
   * - { kind: 'external', what }: 이 폴더 밖의 것 · { kind: 'missing', owner }: 그 자료형에 정의가 없다 · { kind: 'none' }
   */
  async function resolve(where, keep = false) {
    const tab = env.tab();
    active = tab;
    clickLine = where.line;
    // 찾아 둔 정의는 한 번 누르는 동안만 다시 쓴다(그사이 파일이 고쳐졌으면 줄 번호가 묵는다).
    if (!keep) cache.clear();
    const name = where.word;
    const { line } = where;
    const text = codeLine(tab, line);
    const lang = tab.lang;
    const ctx = { lang, path: tab.path, imports: importsFor(tab), tab, line };
    const rest = text.slice(where.to).trimStart();
    const call = rest[0] === '(' || (lang === 'rust' && (rest[0] === '!' || /^::\s*</.test(rest)));
    const argc = call ? countArgs(tab, line, where.to) : null;
    const clicked = { line, col: where.from };
    // Rust에서 뒤가 `::`인 이름은 모듈·자료형이다(변수·필드·함수가 아니다).
    const pathSeg = lang === 'rust' && /^\s*::/.test(text.slice(where.to));

    const self = (tab.outline || []).find((def) => def.line === line && def.col === where.from && def.name === name);
    if (self) return fromDefinition(tab, where, self);
    // Rust의 속성 안의 낱말(`#[cfg(any(test, feature = "x"))]`)은 코드의 이름이 아니다.
    if (lang === 'rust' && /^\s*#!?\[/.test(text) && ['cfg', 'cfg_attr', 'any', 'all', 'not', 'test', 'feature', 'allow', 'deny', 'warn', 'inline', 'doc'].includes(name)) return { kind: 'none' };

    const ahead = before(tab, line, where.from).trimEnd();
    const sep = ahead.endsWith('::') || ahead.endsWith('?.') || (lang === 'c' && ahead.endsWith('->')) ? 2 : ahead.endsWith('.') && !ahead.endsWith('..') ? 1 : 0;
    // import·use 줄 위의 이름은 받는 쪽을 따지지 않는다(경로 자체가 답이다). Rust의 `use a::{`는 여러 줄에 걸친다.
    const importing = /^\s*(?:pub(?:\s*\([^)]*\))?\s+)?(?:import|use|from|package)\b/.test(text) || (lang === 'rust' && /(?:^|[\s;{}])use\s[^;]*$/.test(ahead));
    let mods = [];
    if (sep && !importing) {
      const receiver = await typeOfExpr(tab, line, ahead, ahead.length - sep, 0, line, ahead.endsWith('::'));
      if (receiver?.outside) return { kind: 'external', what: receiver.far ? `${receiver.outside} …의 ${name}` : `${receiver.outside}::${name}` };
      if (receiver?.module) {
        mods = expandMods(ctx, receiver.module);
        // 표준 라이브러리이거나 이 폴더에 없는 모듈의 것(`io::Error`, `serde_json::from_str`)이면 엉뚱한 곳으로 가지 않는다.
        if (lang === 'rust' && !(await moduleHere(ctx, receiver.module))) return { kind: 'external', what: [...mods, name].join('::') };
      } else if (receiver) {
        const found = await findMember(receiver, name, { call, argc, fieldOnly: !call && sep === 1 && STRICT.includes(lang) });
        if (found.defs.length) return done(found.defs, tab, where, found.accessor ? `${name}()의 정의가 없어 필드로 갔습니다(Lombok·record가 만드는 접근자)` : '');
        if (!found.known) {
          // 받는 쪽의 자료형(이나 그 상위 자료형)이 이 폴더 밖의 것이다. 자료형을 느슨하게 다루는 언어에서는 그래도 이름으로 찾아본다.
          if (STRICT.includes(lang) || !(await defsOf(name)).length) return { kind: 'external', what: found.outside ? `${found.outside}의 ${name}` : name };
          return byName();
        }
        if (!STRICT.includes(lang)) return byName();
        // Rust: 자료형은 알지만 거기에 없다 → 그 자료형이 구현한 트레이트의 기본 메서드일 수 있다.
        if (lang === 'rust') {
          const loose = [];
          const seen = new Map();
          for (const def of preferKind(await defsOf(name), call)) {
            if (!def.container || seen.size >= 12) continue;
            if (!seen.has(def.container)) {
              const trait = (await defsOf(def.container)).some((each) => each.kind === 'trait');
              seen.set(def.container, trait && (await subtypesOf(def.container)).some((each) => each.kind === 'impl' && found.tried.includes(each.name)));
            }
            if (seen.get(def.container)) loose.push(def);
          }
          if (loose.length) return done(narrowArity(loose, argc), tab, where);
        }
        if (lang === 'java' && OBJECT_METHODS.includes(name)) return { kind: 'external', what: `java.lang.Object의 ${name}` };
        return { kind: 'missing', owner: found.tried[0] || plain(receiver.names[0] || '') };
      }
    } else if (!importing && !pathSeg && !(call && lang === 'java')) {
      // 받는 쪽이 없다: 지역 변수·인자, 이 자료형의 멤버, import한 것. (Java에서 부르는 이름은 변수가 아니라 메서드다.)
      const local = localOf(tab, name, line, clicked);
      if (local) return { kind: 'local', at: local };
    }

    return byName();

    /** 자료형으로 가리지 못했을 때: 이 자료형의 멤버, import한 것, 그리고 이름이 같은 정의를 가까운 순서로. */
    async function byName() {
      let all = await defsOf(name);
      // 이 파일의 정의는 색인을 기다리지 않는다(방금 만든 파일은 색인에 아직 없을 수 있다): 화면이 읽어 둔 구조에서 더한다.
      const here = env.relative(tab.path);
      for (const def of tab.outline || []) {
        if (def.name !== name || (def.line === line && def.col === where.from) || ['heading', 'section'].includes(def.kind)) continue;
        if (!all.some((each) => each.file === here && each.line === def.line && each.col === def.col)) all = [...all, { ...def, file: here, text: (tab.lines[def.line - 1] || '').trim().slice(0, 200), sig: '', supers: [] }];
      }
      // 색인에 아직 없는 새 파일의 정의일 수 있다: 한 번은 색인을 맞추고 다시 찾는다.
      if (!all.length) all = await env.invoke('code_defs', { root: env.root(), name, within: null, fresh: true }).catch(() => []);
      // 같은 이름이 다른 언어의 파일에도 있으면 이 언어의 것만 본다.
      if (TYPED.includes(lang)) {
        const same = all.filter((def) => env.langOf(def.file) === lang);
        if (same.length) all = same;
      }
      // Rust의 use 줄: 앞에 적힌 경로가 표준 라이브러리나 이 폴더에 없는 크레이트면 그 조각도 밖의 것이다(`use std::sync::Arc`의 sync).
      let usePath = null;
      if (importing && lang === 'rust') {
        usePath = usePrefix(ahead);
        if (usePath.length && !(await moduleHere({ ...ctx, imports: { ...ctx.imports, names: new Map(), scoped: [] } }, usePath))) return { kind: 'external', what: [...usePath, name].join('::') };
      }
      if (!sep && !importing && lang !== 'rust') {
        for (const owner of ownerTypes(tab, line)) {
          const found = await findMember({ names: [owner] }, name, { call, argc, accessor: false });
          if (found.defs.length) return done(found.defs, tab, where);
        }
      }
      if (pathSeg) all = all.filter((def) => def.kind === 'module' || def.kind === 'type' || TYPE_KINDS.includes(def.kind));
      // `crate::…`·`self::…`·`super::…`로 적은 것은 이 크레이트의 것이다.
      const pathHead = /(?:^|[^\w:])((?:crate|self|super)(?:::[\w$]+)*)::$/.exec(ahead)?.[1];
      if (pathHead || ['crate', 'self', 'super'].includes(mods[0])) {
        const crate = (file) => { const parts = file.split('/'); const at = parts.indexOf('src'); return (at >= 0 ? parts.slice(0, at) : parts.slice(0, -1)).join('/'); };
        const mine = crate(env.relative(tab.path));
        const same = all.filter((def) => crate(def.file) === mine);
        if (same.length) all = same;
      }
      if (mods.length) all = moduleFirst(all, mods); // 경로를 적었으면(`util::time_alt`) 그 경로가 답이다. 같은 이름의 import보다 먼저다.
      else if (!sep || importing) {
        const picked = await byImport(ctx, name, all);
        if (picked?.external) return { kind: 'external', what: picked.external };
        if (picked?.file) return { kind: 'file', path: picked.file };
        if (picked?.defs) {
          const narrowed = narrowArity(preferKind(picked.defs, call), argc);
          if (!picked.loose || narrowed.length === 1) return done(narrowed, tab, where);
          all = narrowed;
        }
      }
      // import·use 줄: 적힌 경로로 고른다(`import a.b.Foo;`, `use crate::a::b::Foo;`).
      if (importing && all.length > 1) {
        const path = usePath?.length ? usePath : /[\w$.:]*$/.exec(ahead)?.[0].split(/\.|::/).filter(Boolean) || [];
        all = moduleFirst(all, path);
      }
      // Rust: 받는 쪽 없이 부르는 것은 메서드가 아니다.
      if (!sep && lang === 'rust' && call) all = all.filter((def) => def.kind !== 'method');
      // 받는 쪽 없이 쓴 이름은 이 파일에서 정의한 것(자료형, 맨 바깥의 함수·상수)이 먼저다: 같은 이름을 가져다 쓰면서 또 정의할 수는 없다.
      if (!sep && !importing) {
        const mine = env.lower(tab.path);
        const here = preferKind(all.filter((def) => env.lower(env.full(def.file)) === mine && (!def.container || TYPE_KINDS.includes(def.kind))), call);
        if (here.length === 1) return done(here, tab, where);
      }
      // 받는 쪽 변수의 이름과 자료형 이름이 닮은 것(`orderService` ↔ `OrderService`)을 먼저 본다.
      let narrowed = false; // 이름이 닮았다는 것만으로 추린 것은 바로 가지 않는다(고르는 창에 그것만 보인다)
      if (sep === 1) {
        const owner = /([A-Za-z_$][\w$]*)\s*\??\.$/.exec(ahead)?.[1]?.toLowerCase() || '';
        const alike = owner.length > 3 ? all.filter((def) => def.container && def.container.toLowerCase().endsWith(owner)) : [];
        // 이름이 똑같거나(`account` ↔ `Account`) 필드를 읽는 자리면 믿을 만하다. 끝만 닮은 호출(`queue.pop_front()` ↔ `RegistrationQueue`)은 표준 라이브러리의 것일 수 있다.
        if (alike.length && alike.length < all.length) { all = alike; narrowed = call && !alike.every((def) => def.container.toLowerCase() === owner); }
      }
      if (all.length) {
        const ranked = env.rank(narrowArity(preferKind(all, call), argc), where);
        // 받는 쪽이 있는데 그 자료형을 모른다: 같은 파일에 있다는 것만으로 바로 가지 않는다(하나뿐일 때만, 흔한 이름은 그때도 고르게 한다).
        const unsure = sep === 1 && !mods.length && !importing;
        const direct = unsure ? ranked.length === 1 && !narrowed && !(STRICT.includes(lang) && COMMON.has(name)) : ranked.length === 1 || ranked[0].value >= ranked[1].value + 40;
        if (direct) return done([ranked[0].def], tab, where);
        return { kind: 'pick', defs: ranked.map((each) => each.def), note: unsure ? `"${name}" — 받는 쪽의 자료형을 알 수 없어 이름으로 찾은 정의` : '' };
      }
      // 프로젝트에서 못 찾았으면 이 파일의 위쪽·아래쪽 어디서든 선언을 찾아본다(경로의 조각이나 `a.b`의 b는 지역 선언이 아니다).
      const local = sep || pathSeg ? null : env.localDeclaration(name, line, 1, clicked) || env.localDeclaration(name, line + 1, tab.lines.length, clicked);
      if (local) return { kind: 'local', at: local };
      return { kind: 'none' };
    }
  }

  /** 구현으로 가기: 자료형이면 물려받거나 구현하는 것들, 메서드면 그 구현들. { kind: 'def' | 'pick' | 'none', … } */
  async function implementations(where) {
    const tab = env.tab();
    active = tab;
    clickLine = where.line;
    cache.clear();
    let def = (tab.outline || []).find((each) => each.line === where.line && each.col === where.from && each.name === where.word);
    if (def) def = { ...def, file: env.relative(tab.path) };
    else {
      const target = await resolve(where, true);
      def = target.kind === 'def' ? target.def : target.kind === 'pick' ? target.defs[0] : null;
    }
    if (!def) return { kind: 'none' };
    const found = await implementationsOf(def);
    if (!found.length) return { kind: 'none', def };
    return { ...(await done(found, tab, where)), of: def };
  }

  return { resolve, implementations, typeOfExpr, forget: () => cache.clear() };
}
