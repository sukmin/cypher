// 코드 보기의 문법 강조: 글을 줄 단위 조각(토큰)으로 나눈다. 바깥 라이브러리 없이 직접 읽는다.
//
// 세 가지 읽는 법이 있다.
// - 일반 언어(Java, Rust, JavaScript/TypeScript, Kotlin, Go, Python, C 계열, SQL, 셸, JSON·YAML·TOML …): 언어마다 예약어와
//   주석·문자열 모양만 다르게 주고 같은 방식으로 읽는다(clike).
// - 마크업(HTML, XML, Vue): 태그·속성을 읽고, <script>와 <style> 안은 JavaScript·CSS로 넘겨 읽는다.
// - CSS(SCSS, Less): 선택자와 속성·값을 가려 읽는다.
//
// 결과는 줄마다의 조각 목록이다: [시작 칸, 끝 칸, 종류]. 종류가 없는 글자(기본 색)는 조각으로 만들지 않는다.
// 종류: kw(예약어) ty(자료형·클래스 이름) def(정의하는 이름) call(부르는 함수) cn(상수) num str com doc(문서 주석) ann(애너테이션·속성)
//       mac(매크로) lt(수명) tag attr prop(속성 이름·키) sel(CSS 선택자) var(변수 표기) op esc(이스케이프) head(제목) em(강조)

const words = (text) => new Set(text.trim().split(/\s+/));

const C_COMMON = 'if else for while do switch case default break continue return goto sizeof struct union enum typedef static extern const volatile inline';
const SPECS = {
  rust: {
    kw: words('as async await break const continue crate dyn else enum extern fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait type unsafe use where while union macro_rules try yield'),
    types: words('bool char str u8 u16 u32 u64 u128 usize i8 i16 i32 i64 i128 isize f32 f64 String Vec Option Result Box Rc Arc HashMap HashSet BTreeMap Path PathBuf'),
    consts: words('true false None Some Ok Err'),
    defAfter: words('fn struct enum trait type mod union const static macro_rules'),
    line: ['//'], doc: ['///', '//!'], block: ['/*', '*/'], nested: true,
    quotes: { '"': 'multi', "'": 'char' }, rust: true,
  },
  java: {
    kw: words('abstract assert break case catch class continue default do else enum extends final finally for goto if implements import instanceof interface native new package private protected public return static strictfp super switch synchronized this throw throws transient try volatile while var record sealed permits non-sealed yield module requires exports'),
    types: words('boolean byte char short int long float double void String Object Integer Long Double Boolean List Map Set Optional'),
    consts: words('true false null'),
    defAfter: words('class interface enum record'),
    line: ['//'], doc: ['/**'], block: ['/*', '*/'], quotes: { '"': 'line', "'": 'line' }, triple: '"""', at: true,
  },
  kotlin: {
    kw: words('as break by catch class companion const constructor continue crossinline data do else enum external final finally for fun get if import in infix init inline inner interface internal is lateinit noinline object open operator out override package private protected public reified return sealed set super suspend tailrec this throw try typealias val var vararg when where while'),
    types: words('Any Boolean Byte Char Short Int Long Float Double Unit Nothing String List Map Set Array'),
    consts: words('true false null'),
    defAfter: words('class interface object fun val var typealias'),
    line: ['//'], doc: ['/**'], block: ['/*', '*/'], nested: true, quotes: { '"': 'line', "'": 'line' }, triple: '"""', at: true,
  },
  js: {
    kw: words('async await break case catch class const continue debugger default delete do else export extends finally for from function get if import in instanceof let new of return set static super switch this throw try typeof var void while with yield as type interface enum implements declare namespace abstract readonly public private protected keyof infer is satisfies override'),
    types: words('string number boolean any unknown never void object symbol bigint Array Promise Map Set Date RegExp Error Record Partial'),
    consts: words('true false null undefined NaN Infinity'),
    defAfter: words('function class interface type enum namespace const let var'),
    line: ['//'], doc: ['/**'], block: ['/*', '*/'], quotes: { '"': 'line', "'": 'line', '`': 'template' }, at: true, regex: true, dollar: true,
  },
  go: {
    kw: words('break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var'),
    types: words('bool byte rune string int int8 int16 int32 int64 uint uint8 uint16 uint32 uint64 uintptr float32 float64 complex64 complex128 error any'),
    consts: words('true false nil iota'),
    defAfter: words('func type var const'),
    line: ['//'], block: ['/*', '*/'], quotes: { '"': 'line', "'": 'line', '`': 'multi' }, raw: '`', capsCall: true,
  },
  python: {
    kw: words('and as assert async await break class continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return try while with yield match case'),
    types: words('int float str bool bytes list dict set tuple object'),
    consts: words('True False None self cls'),
    defAfter: words('def class'),
    line: ['#'], quotes: { '"': 'line', "'": 'line' }, triple: ['"""', "'''"], at: true,
  },
  c: {
    kw: words(`${C_COMMON} auto register signed unsigned class namespace template typename public private protected virtual override final new delete this throw try catch using operator friend constexpr noexcept nullptr explicit mutable decltype static_cast dynamic_cast const_cast reinterpret_cast`),
    types: words('void char short int long float double bool size_t int8_t int16_t int32_t int64_t uint8_t uint16_t uint32_t uint64_t string wchar_t'),
    consts: words('true false NULL nullptr'),
    defAfter: words('class struct enum namespace union'),
    line: ['//'], doc: ['/**', '///'], block: ['/*', '*/'], quotes: { '"': 'line', "'": 'line' }, hash: true,
  },
  csharp: {
    kw: words('abstract as async await base break case catch checked class const continue default delegate do else enum event explicit extern finally fixed for foreach get goto if implicit in init interface internal is lock namespace new operator out override params partial private protected public readonly record ref return sealed set sizeof stackalloc static struct switch this throw try typeof unchecked unsafe using var virtual volatile when where while yield'),
    types: words('bool byte char decimal double float int long object sbyte short string uint ulong ushort void dynamic String List Dictionary Task'),
    consts: words('true false null'),
    defAfter: words('class interface enum struct record namespace'),
    line: ['//'], doc: ['///'], block: ['/*', '*/'], quotes: { '"': 'line', "'": 'line' }, hash: true,
  },
  sql: {
    kw: words('select from where and or not in is null as join left right inner outer full cross on group by order having limit offset insert into values update set delete create alter drop table index view database schema primary key foreign references unique constraint default check union all distinct case when then else end exists between like asc desc begin commit rollback transaction if add column with returning truncate grant revoke trigger function procedure declare'),
    types: words('int integer bigint smallint tinyint decimal numeric float double real char varchar text date time timestamp datetime boolean bool blob json jsonb uuid serial'),
    consts: words('true false null'),
    line: ['--'], block: ['/*', '*/'], quotes: { "'": 'multi', '"': 'line', '`': 'line' }, ignoreCase: true,
  },
  shell: {
    kw: words('if then else elif fi for while until do done case esac in function select return exit break continue local export readonly declare unset shift source alias set echo cd'),
    consts: words('true false'),
    line: ['#'], quotes: { '"': 'multi', "'": 'multi' }, raw: "'", dollarVar: true,
  },
  // 배치 파일: 작은따옴표는 문자열이 아니고(REM don't …), REM·::로 시작하는 줄이 주석이다.
  batch: {
    kw: words('echo set if else for in do goto call exit setlocal endlocal not exist defined errorlevel pause shift start cd pushd popd'),
    line: ['::'], quotes: { '"': 'line' }, raw: '"', ignoreCase: true, rem: true,
  },
  powershell: {
    kw: words('if else elseif switch foreach for while do until break continue return function filter param begin process end try catch finally throw trap in class enum using'),
    consts: words('$true $false $null'),
    line: ['#'], block: ['<#', '#>'], quotes: { '"': 'multi', "'": 'multi' }, raw: '"\'', dollarVar: true, ignoreCase: true,
  },
  json: { consts: words('true false null'), line: ['//'], block: ['/*', '*/'], quotes: { '"': 'line' }, keys: ':' },
  yaml: { consts: words('true false null yes no on off ~'), hashAlone: true, line: ['#'], quotes: { '"': 'line', "'": 'line' }, keys: ':', bareKeys: true },
  toml: { consts: words('true false'), line: ['#'], quotes: { '"': 'line', "'": 'line' }, triple: ['"""', "'''"], keys: '=', bareKeys: true, sections: true },
  ini: { line: ['#', ';'], quotes: { '"': 'line' }, keys: '=', bareKeys: true, sections: true },
  groovy: {
    kw: words('abstract as assert break case catch class continue def default do else enum extends final finally for if implements import in instanceof interface new package private protected public return static super switch this throw throws trait try while'),
    types: words('boolean byte char short int long float double void String Object List Map'),
    consts: words('true false null'),
    defAfter: words('class interface enum def trait'),
    line: ['//'], doc: ['/**'], block: ['/*', '*/'], quotes: { '"': 'line', "'": 'line' }, triple: ['"""', "'''"], at: true,
  },
  docker: {
    kw: words('FROM RUN CMD LABEL MAINTAINER EXPOSE ENV ADD COPY ENTRYPOINT VOLUME USER WORKDIR ARG ONBUILD STOPSIGNAL HEALTHCHECK SHELL AS'),
    line: ['#'], quotes: { '"': 'line', "'": 'line' }, dollarVar: true,
  },
  plain: {},
};

/** 파일 이름(확장자)으로 강조에 쓸 언어를 정한다. 모르는 파일은 'plain'(강조 없음). */
export function langOf(path) {
  const name = String(path).split(/[\\/]/).pop().toLowerCase();
  if (name === 'dockerfile' || name.startsWith('dockerfile.')) return 'docker';
  if (name === 'makefile' || name === '.gitignore' || name === '.dockerignore' || name === '.editorconfig' || name.startsWith('.env')) return name === '.editorconfig' || name.startsWith('.env') ? 'ini' : 'shell';
  const ext = name.includes('.') ? name.slice(name.lastIndexOf('.') + 1) : '';
  return {
    rs: 'rust', java: 'java', kt: 'kotlin', kts: 'kotlin', gradle: 'groovy', groovy: 'groovy',
    js: 'js', mjs: 'js', cjs: 'js', jsx: 'js', ts: 'js', tsx: 'js', mts: 'js', cts: 'js',
    vue: 'vue', html: 'html', htm: 'html', xhtml: 'html', svelte: 'vue', xml: 'xml', svg: 'xml', pom: 'xml', xsd: 'xml', xsl: 'xml', plist: 'xml', fxml: 'xml', jsp: 'html',
    css: 'css', scss: 'css', sass: 'css', less: 'css',
    go: 'go', py: 'python', pyi: 'python',
    c: 'c', h: 'c', cc: 'c', cpp: 'c', cxx: 'c', hpp: 'c', hh: 'c', m: 'c', mm: 'c', cs: 'csharp',
    sql: 'sql', sh: 'shell', bash: 'shell', zsh: 'shell', ps1: 'powershell', psm1: 'powershell', psd1: 'powershell', bat: 'batch', cmd: 'batch',
    json: 'json', jsonc: 'json', json5: 'json', yaml: 'yaml', yml: 'yaml', toml: 'toml', lock: 'toml', ini: 'ini', cfg: 'ini', conf: 'ini', properties: 'ini',
    md: 'markdown', markdown: 'markdown', mdx: 'markdown',
  }[ext] || 'plain';
}

/** 사람이 읽는 언어 이름(상태 줄에 보인다). */
export const LANG_NAMES = {
  rust: 'Rust', java: 'Java', kotlin: 'Kotlin', js: 'JavaScript/TypeScript', vue: 'Vue', html: 'HTML', xml: 'XML', css: 'CSS', go: 'Go', python: 'Python',
  c: 'C/C++', csharp: 'C#', sql: 'SQL', shell: '셸', batch: '배치 파일', powershell: 'PowerShell', json: 'JSON', yaml: 'YAML', toml: 'TOML', ini: '설정', groovy: 'Groovy',
  docker: 'Dockerfile', markdown: 'Markdown', plain: '글',
};

const IDENT = /[A-Za-z_ -￿][\w -￿]*/y;
const IDENT_DOLLAR = /[A-Za-z_$ -￿][\w$ -￿]*/y;
const NUMBER = /0[xX][\da-fA-F_]+[uUlLn]*|0[bB][01_]+[uUlLn]*|0[oO][0-7_]+|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?(?:[fFdDlLuUn]|[iu](?:8|16|32|64|128|size)|f(?:32|64))*/y;
const SPACE = /\s+/y;
/** 이보다 긴 줄은 강조하지 않는다(한 줄로 줄인 CSS·JSON처럼 수십만 자인 줄에서 화면이 멈추지 않게). */
const MAX_LINE = 5000;

/** 일반 언어 한 줄을 읽는다. `state`는 앞줄에서 이어진 것(여러 줄 주석·문자열)이고, 읽고 난 상태를 그 자리에 고쳐 둔다. */
function clikeLine(spec, line, state, out, base = 0, end = line.length) {
  const push = (from, to, kind) => { if (to > from) out.push([from, to, kind]); };
  const ident = spec.dollar ? IDENT_DOLLAR : IDENT;
  let i = base;
  let prevWord = '';     // 바로 앞의 낱말(그 뒤의 이름이 정의인지 가리는 데 쓴다)
  let prevSignificant = ''; // 바로 앞의 뜻 있는 글자(정규식 리터럴이 올 수 있는 자리인지 가리는 데 쓴다)

  // 여러 줄 주석·문자열이 이 줄에서도 이어지는지 본다. 끝나면 그 뒤부터 평소대로 읽는다.
  const continueBlock = () => {
    if (state.block) {
      const [open, close] = spec.block;
      let depth = state.block, j = i;
      while (j < end && depth) {
        if (line.startsWith(close, j)) { depth -= 1; j += close.length; }
        else if (spec.nested && line.startsWith(open, j)) { depth += 1; j += open.length; }
        else j += 1;
      }
      push(i, j, state.doc ? 'doc' : 'com');
      state.block = depth;
      if (!depth) state.doc = false;
      i = j;
    } else if (state.str) {
      i = readString(state.str, i);
    }
  };

  /** 문자열을 읽는다. `quote`는 닫는 글자(들). 줄 끝까지 닫히지 않으면 여러 줄 문자열일 때만 상태에 남긴다. 읽은 다음 칸을 돌려준다. */
  function readString(str, from) {
    const { close, raw, multi, template } = str;
    let j = from, start = from;
    while (j < end) {
      if (!raw && line[j] === '\\' && j + 1 < end) {
        push(start, j, 'str');
        push(j, j + 2, 'esc');
        j += 2;
        start = j;
        continue;
      }
      if (template && line[j] === '$' && line[j + 1] === '{') {
        // `${ … }` 안은 코드다. 같은 줄에서 닫히는 것만 코드로 읽는다.
        let depth = 1, k = j + 2;
        while (k < end && depth) { if (line[k] === '{') depth += 1; else if (line[k] === '}') depth -= 1; k += 1; }
        push(start, j, 'str');
        push(j, j + 2, 'esc');
        if (!depth) { clikeLine(spec, line, { block: 0, str: null }, out, j + 2, k - 1); push(k - 1, k, 'esc'); }
        j = k;
        start = j;
        continue;
      }
      if (line.startsWith(close, j)) {
        j += close.length;
        push(start, j, 'str');
        state.str = null;
        return j;
      }
      j += 1;
    }
    push(start, end, 'str');
    state.str = multi ? str : null;
    return end;
  }

  continueBlock();
  while (i < end) {
    const ch = line[i];
    SPACE.lastIndex = i;
    if (SPACE.test(line)) { i = SPACE.lastIndex; continue; }

    // 주석
    const doc = spec.doc?.find((mark) => line.startsWith(mark, i));
    if (doc && !(doc === '/**' && line.startsWith('/**/', i))) {
      if (doc.startsWith('/*')) { state.block = 1; state.doc = true; continueBlockFrom(doc.length); continue; }
      push(i, end, 'doc');
      return;
    }
    if (spec.line?.some((mark) => line.startsWith(mark, i) && (mark !== '#' || !(spec.dollarVar || spec.hashAlone) || i === base || /\s/.test(line[i - 1])))) {
      push(i, end, 'com');
      return;
    }
    if (spec.block && line.startsWith(spec.block[0], i)) {
      state.block = 1;
      state.doc = false;
      continueBlockFrom(spec.block[0].length);
      continue;
    }

    // 문자열
    const triple = spec.triple && [].concat(spec.triple).find((mark) => line.startsWith(mark, i));
    if (triple) { push(i, i + triple.length, 'str'); i = readString({ close: triple, multi: true }, i + triple.length); prevSignificant = '"'; continue; }
    if (spec.rust && (ch === 'r' || ch === 'b')) {
      // 날 문자열 r"…", r#"…"#, 바이트 문자열 b"…", br#"…"#
      const m = /^(?:br|r)(#*)"/.exec(line.slice(i, i + 12));
      if (m) { push(i, i + m[0].length, 'str'); i = readString({ close: `"${m[1]}`, raw: true, multi: true }, i + m[0].length); prevSignificant = '"'; continue; }
      if (line.startsWith('b"', i)) { push(i, i + 2, 'str'); i = readString({ close: '"', multi: true }, i + 2); prevSignificant = '"'; continue; }
    }
    if (spec.quotes?.[ch]) {
      const mode = spec.quotes[ch];
      if (mode === 'char') {
        // Rust: 'a'·'\n'는 글자, 'a(닫는 따옴표 없음)는 수명이다.
        const m = /^'(?:\\(?:x[\da-fA-F]{2}|u\{[\da-fA-F_]+\}|.)|[^\\'])'/.exec(line.slice(i, i + 14));
        if (m) { push(i, i + m[0].length, 'str'); i += m[0].length; prevSignificant = '"'; continue; }
        IDENT.lastIndex = i + 1;
        if (IDENT.test(line)) { push(i, IDENT.lastIndex, 'lt'); i = IDENT.lastIndex; prevSignificant = 'a'; continue; }
        i += 1;
        continue;
      }
      push(i, i + 1, 'str');
      i = readString({ close: ch, multi: mode === 'multi' || mode === 'template', template: mode === 'template', raw: !!spec.raw?.includes(ch) }, i + 1);
      prevSignificant = '"';
      continue;
    }
    // JavaScript의 정규식 리터럴: 값이 올 자리의 `/`만 정규식으로 본다.
    // (`a++ / 2`의 `/`는 나눗셈이고, JSX의 닫는 태그 `</b>`의 `/`도 정규식이 아니다. `x => /re/`의 `>` 뒤는 정규식일 수 있다.)
    const before = line.slice(Math.max(base, i - 3), i).trimEnd();
    const valueSpot = prevSignificant === '' || /[=(,:;!&|?{\[+\-*%~^]/.test(prevSignificant) || (prevSignificant === '>' && before.endsWith('=>'));
    if (spec.regex && ch === '/' && !before.endsWith('++') && !before.endsWith('--') && (valueSpot || /^(return|typeof|case|in|of|delete|void|throw|new|else|do)$/.test(prevWord))) {
      const m = /^\/(?![/*])(?:\\.|\[(?:\\.|[^\]\\])*\]|[^/\\\n[])+\/[dgimsuvy]*/.exec(line.slice(i, end));
      if (m) { push(i, i + m[0].length, 'str'); i += m[0].length; prevSignificant = '"'; prevWord = ''; continue; }
    }

    // 애너테이션·속성·전처리
    if (spec.at && ch === '@') {
      IDENT.lastIndex = i + 1;
      if (IDENT.test(line)) {
        let j = IDENT.lastIndex;
        while (line[j] === '.') { IDENT.lastIndex = j + 1; if (!IDENT.test(line)) break; j = IDENT.lastIndex; }
        push(i, j, 'ann');
        i = j;
        prevSignificant = 'a';
        continue;
      }
    }
    if (spec.rust && ch === '#' && (line[i + 1] === '[' || (line[i + 1] === '!' && line[i + 2] === '['))) {
      // #[derive(Debug)] — 같은 줄에서 닫히는 데까지.
      let depth = 0, j = i;
      while (j < end) { if (line[j] === '[') depth += 1; else if (line[j] === ']') { depth -= 1; if (!depth) { j += 1; break; } } j += 1; }
      push(i, j, 'ann');
      i = j;
      continue;
    }
    if (spec.hash && ch === '#' && /^\s*$/.test(line.slice(base, i))) {
      const m = /^#\s*\w+/.exec(line.slice(i, end));
      if (m) { push(i, i + m[0].length, 'ann'); i += m[0].length; continue; }
    }
    if (spec.dollarVar && ch === '$') {
      const m = /^\$(?:\{[^}]*\}|[\w:]+|[?@#*!$0-9])/.exec(line.slice(i, end));
      if (m) {
        const constant = spec.consts?.has(spec.ignoreCase ? m[0].toLowerCase() : m[0]);
        push(i, i + m[0].length, constant ? 'cn' : 'var');
        i += m[0].length;
        prevSignificant = 'a';
        continue;
      }
    }
    // [구역] 머리(TOML, INI)
    if (spec.sections && ch === '[' && /^\s*$/.test(line.slice(base, i))) {
      const close = line.lastIndexOf(']', end);
      if (close > i) { push(i, close + 1, 'tag'); i = close + 1; continue; }
    }

    // 숫자
    if ((ch >= '0' && ch <= '9') || (ch === '.' && line[i + 1] >= '0' && line[i + 1] <= '9' && !/[\w)\]]/.test(prevSignificant))) {
      NUMBER.lastIndex = ch === '.' ? i + 1 : i;
      if (NUMBER.test(line)) { push(i, NUMBER.lastIndex, 'num'); i = NUMBER.lastIndex; prevSignificant = '0'; prevWord = ''; continue; }
    }

    // 따옴표 없는 키(YAML·TOML·INI): 줄 맨 앞의 `이름:`·`이름 =`. 이름에 `-`·`.`이 들어도 한 덩어리다.
    if (spec.bareKeys && /^\s*(?:-\s+)?$/.test(line.slice(base, i))) {
      const m = (spec.keys === ':' ? /^[\w.\-\/]+(?=\s*:(?:\s|$))/ : /^[\w.\-]+(?=\s*=)/).exec(line.slice(i, end));
      if (m) { push(i, i + m[0].length, 'prop'); i += m[0].length; prevSignificant = 'a'; prevWord = ''; continue; }
    }

    // 낱말
    ident.lastIndex = i;
    if (ident.test(line)) {
      const j = ident.lastIndex;
      const word = line.slice(i, j);
      const key = spec.ignoreCase ? word.toLowerCase() : word;
      let k = j;
      while (line[k] === ' ' || line[k] === '\t') k += 1;
      const next = line[k];
      const afterDot = prevSignificant === '.';
      // 배치 파일의 REM: 줄의 나머지가 주석이다.
      if (spec.rem && key === 'rem' && /^[\s@]*$/.test(line.slice(base, i))) { push(i, end, 'com'); return; }
      let kind = '';
      if (!afterDot && spec.kw?.has(key)) kind = 'kw';
      else if (!afterDot && spec.consts?.has(key)) kind = 'cn';
      else if (spec.defAfter?.has(prevWord) && !afterDot) kind = 'def';
      else if (spec.rust && next === '!' && line[k + 1] !== '=') kind = 'mac';
      else if (spec.types?.has(key)) kind = 'ty';
      else if (next === '(') kind = /^[A-Z]/.test(word) && !spec.capsCall ? 'ty' : 'call'; // Go는 밖에 내놓는 함수 이름이 대문자로 시작한다
      else if (/^[A-Z][A-Z0-9_]+$/.test(word) && word.length > 1) kind = 'cn';
      else if (/^[A-Z]/.test(word)) kind = 'ty';
      if (kind) push(i, j, kind);
      prevWord = kind === 'kw' || kind === '' ? word : '';
      prevSignificant = 'a';
      i = j;
      continue;
    }

    // JSON·YAML의 따옴표 없는 값, 그 밖의 기호
    prevSignificant = ch;
    prevWord = '';
    i += 1;
  }

  /** 여는 표시를 지난 뒤 여러 줄 주석을 이어 읽는다. */
  function continueBlockFrom(openLength) {
    const start = i;
    i += openLength;
    const kind = state.doc ? 'doc' : 'com';
    const [open, close] = spec.block;
    let depth = 1, j = i;
    while (j < end && depth) {
      if (line.startsWith(close, j)) { depth -= 1; j += close.length; }
      else if (spec.nested && line.startsWith(open, j)) { depth += 1; j += open.length; }
      else j += 1;
    }
    push(start, j, kind);
    state.block = depth;
    if (!depth) state.doc = false;
    i = j;
  }
}

/** JSON처럼 따옴표로 감싼 키("이름": 값)를 속성 이름 색으로 바꾼다. */
function markQuotedKeys(line, tokens) {
  for (let index = 0; index < tokens.length; index++) {
    const [from, , kind] = tokens[index];
    if (kind !== 'str' || line[from] !== '"') continue;
    // 문자열은 여는 따옴표·내용·닫는 따옴표가 이어진 조각들이다. 닫히는 데까지 묶어 본다.
    let last = index;
    while (last + 1 < tokens.length && tokens[last + 1][0] === tokens[last][1] && (tokens[last + 1][2] === 'str' || tokens[last + 1][2] === 'esc')) last += 1;
    const after = line.slice(tokens[last][1]);
    if (/^\s*:/.test(after) && /^\s*[{,]?\s*$/.test(line.slice(0, from).replace(/.*[{,]/, ''))) {
      for (let k = index; k <= last; k++) if (tokens[k][2] === 'str') tokens[k][2] = 'prop';
    }
    index = last;
  }
}

// ───────────────────────── CSS ─────────────────────────

/** CSS 한 줄. state: { block(주석 중), depth(중괄호 깊이), value(속성 값을 읽는 중) } */
function cssLine(line, state, out, base = 0, end = line.length) {
  const push = (from, to, kind) => { if (to > from) out.push([from, to, kind]); };
  let i = base;
  while (i < end) {
    if (state.block) {
      const close = line.indexOf('*/', i);
      const j = close < 0 || close + 2 > end ? end : close + 2;
      push(i, j, 'com');
      if (close >= 0 && close + 2 <= end) state.block = 0;
      i = j;
      continue;
    }
    const ch = line[i];
    if (line.startsWith('/*', i)) {
      const close = line.indexOf('*/', i + 2);
      const j = close < 0 || close + 2 > end ? end : close + 2;
      push(i, j, 'com');
      state.block = close < 0 || close + 2 > end ? 1 : 0;
      i = j;
      continue;
    }
    if (line.startsWith('//', i) && (i === base || /\s/.test(line[i - 1]))) { push(i, end, 'com'); return; }
    if (ch === '"' || ch === "'") {
      let j = i + 1;
      while (j < end && line[j] !== ch) j += line[j] === '\\' ? 2 : 1;
      push(i, Math.min(end, j + 1), 'str');
      i = Math.min(end, j + 1);
      continue;
    }
    if (ch === '{') { state.depth += 1; state.value = false; i += 1; continue; }
    if (ch === '}') { state.depth = Math.max(0, state.depth - 1); state.value = false; i += 1; continue; }
    if (ch === ';') { state.value = false; i += 1; continue; }
    const rest = line.slice(i, end);
    let m;
    if ((m = /^@[\w-]+/.exec(rest))) { push(i, i + m[0].length, 'kw'); i += m[0].length; continue; }
    if ((m = /^(?:--[\w-]+|\$[\w-]+)/.exec(rest))) {
      push(i, i + m[0].length, 'var');
      i += m[0].length;
      // 변수를 정하는 줄(`--색: 값;`, `$이름: 값;`)은 그 뒤가 값이다.
      const colon = /^\s*:/.exec(line.slice(i, end));
      if (colon && !state.value) { i += colon[0].length; state.value = true; }
      continue;
    }
    if (state.value) {
      if ((m = /^#[\da-fA-F]{3,8}\b/.exec(rest))) { push(i, i + m[0].length, 'num'); i += m[0].length; continue; }
      if ((m = /^-?(?:\d+\.?\d*|\.\d+)(?:%|[a-zA-Z]+)?/.exec(rest))) { push(i, i + m[0].length, 'num'); i += m[0].length; continue; }
      if ((m = /^!important\b/.exec(rest))) { push(i, i + m[0].length, 'kw'); i += m[0].length; continue; }
      if ((m = /^[\w-]+(?=\()/.exec(rest))) { push(i, i + m[0].length, 'call'); i += m[0].length; continue; }
      if ((m = /^[\w-]+/.exec(rest))) { i += m[0].length; continue; }
      i += 1;
      continue;
    }
    // 속성 이름: 중괄호 안에서 `이름:` 뒤에 값이 오고 그 줄에 `{`가 없을 때(중첩 선택자 `a:hover {`와 가린다).
    if (state.depth > 0 && (m = /^[\w-]+(?=\s*:(?!:))/.exec(rest)) && !/\{\s*$/.test(line.slice(i, end)) && !/^[\w-]+\s*:[\w-]+[^;]*\{/.test(rest)) {
      push(i, i + m[0].length, 'prop');
      i += m[0].length;
      const colon = line.indexOf(':', i);
      i = colon + 1;
      state.value = true;
      continue;
    }
    // 선택자
    if ((m = /^-?(?:\d+\.?\d*|\.\d+)%?/.exec(rest))) { push(i, i + m[0].length, 'num'); i += m[0].length; continue; } // @keyframes의 50.5%
    if ((m = /^[.#][\w-]+/.exec(rest))) { push(i, i + m[0].length, 'sel'); i += m[0].length; continue; }
    if ((m = /^::?[\w-]+/.exec(rest))) { push(i, i + m[0].length, 'ann'); i += m[0].length; continue; }
    if ((m = /^[\w-]+/.exec(rest))) { push(i, i + m[0].length, 'tag'); i += m[0].length; continue; }
    i += 1;
  }
}

// ───────────────────────── 마크업(HTML, XML, Vue) ─────────────────────────

/**
 * 마크업 한 줄. state: { mode: 'text' | 'comment' | 'tag' | 'script' | 'style', quote(속성 값의 따옴표), tagName, inner(안쪽 언어의 상태) }
 * `vue`이면 글 안의 {{ … }}를 코드로 읽는다.
 */
function markupLine(line, state, out, vue) {
  const push = (from, to, kind) => { if (to > from) out.push([from, to, kind]); };
  let i = 0;
  const end = line.length;
  while (i < end) {
    if (state.mode === 'comment') {
      const close = line.indexOf('-->', i);
      const j = close < 0 ? end : close + 3;
      push(i, j, 'com');
      if (close >= 0) state.mode = 'text';
      i = j;
      continue;
    }
    if (state.mode === 'script' || state.mode === 'style') {
      const closer = state.mode === 'script' ? /<\/script\s*>/i : /<\/style\s*>/i;
      const m = closer.exec(line.slice(i));
      const stop = m ? i + m.index : end;
      if (state.mode === 'script') clikeLine(SPECS.js, line, state.inner, out, i, stop);
      else cssLine(line, state.inner, out, i, stop);
      if (!m) return;
      i = stop;
      state.mode = 'text';
      continue;
    }
    if (state.mode === 'tag') {
      if (state.quote) {
        const close = line.indexOf(state.quote, i);
        const j = close < 0 ? end : close + 1;
        push(i, j, 'str');
        if (close >= 0) state.quote = '';
        i = j;
        continue;
      }
      const rest = line.slice(i);
      let m;
      if ((m = /^\s+/.exec(rest))) { i += m[0].length; continue; }
      if ((m = /^\/?>/.exec(rest))) {
        push(i, i + m[0].length, 'tag');
        i += m[0].length;
        const name = state.tagName;
        state.tagName = '';
        if (m[0] === '>' && name === 'script') { state.mode = 'script'; state.inner = { block: 0, str: null }; }
        else if (m[0] === '>' && name === 'style') { state.mode = 'style'; state.inner = { block: 0, depth: 0, value: false }; }
        else state.mode = 'text';
        continue;
      }
      if (rest[0] === '"' || rest[0] === "'") { state.quote = rest[0]; state.eq = false; push(i, i + 1, 'str'); i += 1; continue; }
      if (rest[0] === '=') { state.eq = true; i += 1; continue; }
      // `=` 바로 뒤의 따옴표 없는 글은 속성의 값이다.
      if ((m = /^[^\s=>"'/]+/.exec(rest))) { push(i, i + m[0].length, state.eq ? 'str' : 'attr'); state.eq = false; i += m[0].length; continue; }
      i += 1;
      continue;
    }
    // 글
    if (line.startsWith('<!--', i)) { state.mode = 'comment'; continue; }
    const rest = line.slice(i);
    let m;
    if ((m = /^<!\[CDATA\[.*?(?:\]\]>|$)/.exec(rest)) || (m = /^<[!?][^>]*>?/.exec(rest))) { push(i, i + m[0].length, 'ann'); i += m[0].length; continue; }
    if ((m = /^<\/?([A-Za-z][\w:.-]*)/.exec(rest))) {
      push(i, i + m[0].length, 'tag');
      i += m[0].length;
      state.mode = 'tag';
      state.tagName = m[0].startsWith('</') ? '' : m[1].toLowerCase();
      continue;
    }
    if ((m = /^&[#\w]+;/.exec(rest))) { push(i, i + m[0].length, 'esc'); i += m[0].length; continue; }
    if (vue && line.startsWith('{{', i)) {
      const close = line.indexOf('}}', i + 2);
      const stop = close < 0 ? end : close;
      push(i, i + 2, 'esc');
      clikeLine(SPECS.js, line, { block: 0, str: null }, out, i + 2, stop);
      if (close >= 0) push(close, close + 2, 'esc');
      i = close < 0 ? end : close + 2;
      continue;
    }
    const next = line.slice(i + 1).search(vue ? /<|&|\{\{/ : /<|&/);
    i = next < 0 ? end : i + 1 + next;
  }
}

// ───────────────────────── Markdown ─────────────────────────

function markdownLine(line, state, out) {
  const push = (from, to, kind) => { if (to > from) out.push([from, to, kind]); };
  const fence = /^\s*(```+|~~~+)(.*)$/.exec(line);
  if (fence && (!state.fence || (fence[1].startsWith(state.fence) && !fence[2].trim()))) { state.fence = state.fence ? '' : fence[1]; push(0, line.length, 'com'); return; }
  if (state.fence) { push(0, line.length, 'str'); return; }
  if (/^\s{0,3}#{1,6}\s/.test(line)) { push(0, line.length, 'head'); return; }
  if (/^\s{0,3}>/.test(line)) { push(0, line.length, 'com'); return; }
  let m = /^\s*(?:[-*+]|\d+[.)])\s+(?:\[[ xX]\]\s+)?/.exec(line);
  if (m) push(0, m[0].length, 'kw');
  const inline = /`[^`]+`|\*\*[^*]+\*\*|__[^_]+__|!?\[[^\]]*\]\([^)]*\)|<https?:[^>]+>/g;
  while ((m = inline.exec(line))) push(m.index, m.index + m[0].length, m[0][0] === '`' ? 'str' : m[0][0] === '*' || m[0][0] === '_' ? 'em' : 'tag');
}

// ───────────────────────── 전체 ─────────────────────────

/**
 * 글 전체를 줄마다의 조각 목록으로 나눈다. `lines`는 줄들의 배열.
 * 긴 파일도 한 번에 읽는다(글자 하나씩 훑는 정도라 수만 줄도 금방이다).
 */
export function tokenize(lang, lines) {
  const result = new Array(lines.length);
  if (lang === 'plain') return result.fill(null);
  const tooLong = (n) => lines[n].length > MAX_LINE && ((result[n] = null), true);
  if (lang === 'html' || lang === 'xml' || lang === 'vue') {
    const state = { mode: 'text', quote: '', tagName: '', inner: null };
    for (let n = 0; n < lines.length; n++) { if (tooLong(n)) continue; const out = []; markupLine(lines[n], state, out, lang === 'vue'); result[n] = sorted(out); }
    return result;
  }
  if (lang === 'css') {
    const state = { block: 0, depth: 0, value: false };
    for (let n = 0; n < lines.length; n++) { if (tooLong(n)) continue; const out = []; cssLine(lines[n], state, out); result[n] = sorted(out); }
    return result;
  }
  if (lang === 'markdown') {
    const state = { fence: '' };
    for (let n = 0; n < lines.length; n++) { if (tooLong(n)) continue; const out = []; markdownLine(lines[n], state, out); result[n] = sorted(out); }
    return result;
  }
  const spec = SPECS[lang] || SPECS.plain;
  const state = { block: 0, doc: false, str: null };
  for (let n = 0; n < lines.length; n++) {
    if (tooLong(n)) continue;
    const out = [];
    clikeLine(spec, lines[n], state, out);
    if (spec.keys === ':' && !spec.bareKeys) markQuotedKeys(lines[n], out);
    result[n] = sorted(out);
  }
  return result;
}

/** 조각을 시작 칸 순으로 늘어놓고 겹친 것은 뒤의 것을 버린다(그릴 때 겹치면 글자가 두 번 나온다). */
function sorted(tokens) {
  if (tokens.length < 2) return tokens;
  tokens.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const kept = [];
  let edge = 0;
  for (const token of tokens) {
    if (token[0] < edge) continue;
    kept.push(token);
    edge = token[1];
  }
  return kept;
}

/** 그 언어에서 이름(식별자)을 이루는 글자인지. CSS·HTML·Vue에서는 `-`도 이름의 일부다(class 이름, 속성 이름). */
export function wordPattern(lang) {
  return lang === 'css' || lang === 'html' || lang === 'vue' || lang === 'xml' ? /[\w$ -￿-]/ : /[\w$ -￿]/;
}
