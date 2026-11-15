// 터미널이 글자의 칸 수를 Windows 내장 가상 터미널(conhost)과 똑같이 세게 하는 표.
//
// 세션의 출력은 프로그램 → conhost → 이 화면(xterm.js) 순으로 온다. conhost는 자기 화면을 따로 갖고 있다가
// 바뀐 칸만 다시 보내는데, 이때 "한 줄이 꽉 차면 다음 글자는 알아서 다음 줄로 넘어간다"거나 "커서는 이미 그 자리에 있다"고
// 믿고 줄바꿈·커서 이동을 생략한다. 그래서 둘이 글자의 칸 수를 다르게 세면 줄이 한 칸씩 밀리고, 그 뒤로는
// 구분선 조각이 남거나 입력한 글자가 엉뚱한 줄에 찍힌다(Claude Code 화면이 어긋나 보이는 원인).
//
// xterm.js의 기본 표(유니코드 11)와 conhost(Windows 11 24H2·25H2에 내장된 것)를 글자마다 실제로 재어 보면 다른 곳은 둘이다.
// - 앞 글자에 붙는 글자(결합 부호, 이모지 변형 선택자 U+FE0F, 너비 없는 이음표 U+200D …): xterm.js는 0칸, conhost는 1칸.
//   ✔️·⚠️·✈️처럼 변형 선택자가 붙은 이모지가 여기에 걸린다(conhost 2칸, xterm.js 1칸).
// - 유니코드 12 이후에 생긴 이모지와 국기 글자: xterm.js는 1칸, conhost는 2칸.
// 이 표는 conhost 쪽에 맞춘다. 화면을 실제로 배치하는 쪽이 conhost이기 때문이다.

/** conhost가 두 칸으로 세는데 유니코드 11 표에는 한 칸(또는 0칸)으로 적힌 구간. */
const WIDE = [
  [0x302A, 0x302D], [0x3099, 0x309A], [0x31BB, 0x31BF],
  [0x1F1E6, 0x1F1FF], [0x1F6D6, 0x1F6D7], [0x1F6DC, 0x1F6DF], [0x1F6FB, 0x1F6FC], [0x1F7F0, 0x1F7F0],
  [0x1F90C, 0x1F90C], [0x1F972, 0x1F972], [0x1F977, 0x1F979], [0x1F9A3, 0x1F9A4], [0x1F9AB, 0x1F9AD],
  [0x1F9CB, 0x1F9CC], [0x1FA74, 0x1FA77], [0x1FA7B, 0x1FA7C], [0x1FA83, 0x1FA88], [0x1FA96, 0x1FABD],
  [0x1FABF, 0x1FAC5], [0x1FACE, 0x1FADB], [0x1FAE0, 0x1FAE8], [0x1FAF0, 0x1FAF8],
];
/** conhost가 한 칸으로 세는데 유니코드 11 표에는 두 칸으로 적힌 글자. */
const NARROW = [0x1F93B, 0x1F946];

const inRanges = (cp, ranges) => ranges.some(([from, to]) => cp >= from && cp <= to);

/** xterm.js에 든 유니코드 11 표를 꺼낸다(애드온이 터미널에 등록하려는 것을 가로챈다). */
function unicode11() {
  let found = null;
  new Unicode11Addon.Unicode11Addon().activate({ unicode: { register: (provider) => { found = provider; } } });
  return found;
}

/** 사람 피부색 바꾸개(🏻~🏿): 앞의 이모지에 붙어 한 글자가 되지만 conhost는 따로 두 칸으로 센다. */
const TONES = [0x1F3FB, 0x1F3FF];
/** 국기를 이루는 글자(둘이 모여 국기 하나): conhost는 글자마다 두 칸으로 센다. */
const FLAGS = [0x1F1E6, 0x1F1FF];
const VARIATION_16 = 0xFE0F; // 앞 글자를 이모지 모양으로 그리게 하는 변형 선택자

let lookupShared = null;

/** 글자 하나의 칸 수를 찾는 함수(낮은 두 비트가 칸 수, 0x80이 켜져 있으면 앞 글자에 붙는 글자). 유니코드 11 표를 꺼내지 못하면 null. */
function conhostLookup() {
  if (lookupShared) return lookupShared;
  const base = unicode11();
  if (!base) return null;
  // 0: 유니코드 11 표 그대로, 1·2: 그 칸 수, 3: 앞 글자에 붙는 글자(conhost에서는 한 칸).
  const kind = (cp) => {
    if (cp < 0xA0) return 0; // 제어 문자와 ASCII는 건드리지 않는다.
    if (inRanges(cp, WIDE)) return 2;
    if (NARROW.includes(cp)) return 1;
    return base.wcwidth(cp) === 0 ? 3 : 0;
  };
  // 자주 쓰는 구간(U+0000 ~ U+FFFF)은 미리 계산해 둔다.
  const table = new Uint8Array(0x10000);
  for (let cp = 0; cp < table.length; cp++) {
    const k = kind(cp);
    table[cp] = k === 0 ? base.wcwidth(cp) : k === 3 ? 0x81 : k;
  }
  lookupShared = (cp) => {
    if (cp < 0x10000) return table[cp];
    const k = kind(cp);
    return k === 0 ? base.wcwidth(cp) : k === 3 ? 0x81 : k;
  };
  return lookupShared;
}

// xterm.js에 돌려주는 값의 모양: `기억해 둘 것 << 3 | 칸 수 << 1 | 앞 글자에 붙임`. "기억해 둘 것"은 다음 글자를 물을 때 되돌아온다.
const PLAIN = 1 << 3; // 앞 글자가 이모지가 아닌 보통 글자(숫자·영문·한글 …)였다

/**
 * conhost와 같은 칸 수를 주는 표(xterm.js의 `unicode.register`에 넘기는 모양). 유니코드 11 표를 꺼내지 못하면 null.
 * `onSkew`를 주면, Claude Code와 conhost가 서로 다르게 세는 글자가 찍힐 때마다 부른다(화면이 어긋날 수 있다는 신호):
 * 국기 글자, 피부색 바꾸개, 그리고 앞 글자에 붙는 글자 가운데 "한 칸짜리 이모지 뒤의 변형 선택자"가 아닌 것
 * (결합 부호, 이음표, 키캡, 두 칸짜리 이모지 뒤의 변형 선택자). 그 밖의 글자는 둘이 같게 센다.
 */
export function conhostWidths(onSkew = null) {
  const lookup = conhostLookup();
  if (!lookup) return null;
  const inRange = (cp, [from, to]) => cp >= from && cp <= to;
  return {
    version: 'conhost',
    wcwidth: (cp) => lookup(cp) & 3,
    /**
     * xterm.js가 글자마다 묻는 것: 칸 수와 앞 글자에 붙일지.
     * 붙는 글자가 한 칸짜리 글자 바로 뒤에 오면 그 칸에 붙이고 묶음을 두 칸으로 넓힌다(⚠ + U+FE0F → 두 칸짜리 ⚠️).
     * 그 밖에는(줄 맨 앞, 이미 두 칸인 글자 뒤) 제 칸 하나를 차지한다. 어느 쪽이든 conhost가 센 칸 수와 같다.
     */
    charProperties(cp, preceding) {
      const found = lookup(cp);
      const joins = (found & 0x80) !== 0;
      const afterNarrow = preceding !== 0 && ((preceding >> 1) & 3) === 1;
      if (onSkew && cp >= 0x300) {
        const emojiStyle = cp === VARIATION_16 && afterNarrow && !(preceding & PLAIN);
        if ((joins && !emojiStyle) || inRange(cp, FLAGS) || inRange(cp, TONES)) onSkew();
      }
      if (joins && afterNarrow) return (2 << 1) | 1;
      return ((found & 3) << 1) | (cp < 0x2000 || (cp >= 0xAC00 && cp <= 0xD7A3) ? PLAIN : 0);
    },
  };
}

// ───────────────────────── 세션이 붙는 가상 터미널 ─────────────────────────

/** 이 빌드부터는 내장 conhost가 새 방식(출력을 거의 그대로 넘김)일 수 있어 재어 보지 않은 표를 쓰지 않는다. */
const UNMEASURED_BUILD = 27000;

/** 앱이 알려 준 가상 터미널 정보: Windows 빌드 번호(모르면 0)와, 실행 파일 옆에 둔 ConPTY를 쓰는지. */
let pty = { build: 0, bundled: false };

export function setPty(info) {
  pty = { build: Number(info?.build) || 0, bundled: !!info?.bundled };
}

/**
 * 터미널을 만들 때 넘기는 `windowsPty` 값. 이것을 주면 xterm.js가 Windows 가상 터미널의 동작에 맞춘다:
 * 줄 수가 늘 때 지난 출력을 화면으로 도로 끌어내리지 않고 아래에 빈 줄을 더한다(conhost가 그렇게 한다).
 * 주지 않으면 줄 수가 늘 때마다 지난 출력 몇 줄이 conhost가 다시 그린 화면에 덮여 사라진다.
 */
export function windowsPty() {
  return { backend: 'conpty', buildNumber: pty.build || 19045 };
}

/**
 * 터미널이 글자의 칸 수를 세션의 가상 터미널과 같게 세게 한다(유니코드 11 애드온을 붙인 뒤에 부른다).
 * 내장 conhost(재어 본 빌드까지)면 conhost의 표를 쓰고, 그 밖에는 유니코드 11 표를 그대로 둔다.
 * `onSkew`: 화면이 어긋날 수 있는 글자가 찍힐 때 부를 함수(conhostWidths 참고).
 */
export function matchWidths(term, onSkew = null) {
  if (pty.bundled || pty.build >= UNMEASURED_BUILD) return;
  const widths = conhostWidths(onSkew);
  if (!widths) return;
  term.unicode.register(widths);
  term.unicode.activeVersion = widths.version;
}
