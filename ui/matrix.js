// 매트릭스 테마의 바탕: 녹색 글자가 위에서 아래로 흐른다.
//
// 본창 맨 뒤에 깔리는 캔버스 하나에 그린다(look.js가 테마에 맞춰 켜고 끈다). 그 위의 화면(사이드바, 창, 터미널)은
// 테마가 반투명한 바탕을 써서 글자가 은은하게 비친다. 입력은 받지 않는다(pointer-events: none).
// 창이 가려지거나 최소화되면 브라우저가 그리기를 멈추므로 따로 멈추지 않아도 된다.

const GLYPHS = 'ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789:.=*+-<>|';
const CELL = 16;      // 글자 한 칸의 크기(픽셀)
const STEP_MS = 50;   // 한 걸음의 간격. 줄기마다 빠르기가 달라 실제로는 이보다 느리게 내려온다.
const FADE = 0.035;    // 걸음마다 지난 글자를 이만큼 어둡게 덮는다(꼬리의 길이)
const HEAD = '#d6ffe2';  // 맨 앞 글자
const BODY = '#00ff66';  // 지나간 자리

let canvas = null;
let ctx = null;
let streams = [];     // 줄기마다 { y: 맨 앞 글자의 줄(소수), speed: 걸음마다 내려가는 줄 수, row: 마지막으로 그린 줄, glyph: 그 줄의 글자 }
let frame = 0;
let lastStep = 0;

const glyph = () => GLYPHS[Math.floor(Math.random() * GLYPHS.length)];

/** 줄기를 화면 위 어딘가에서 새로 시작시킨다. `spread`이면(처음 켤 때) 화면 곳곳에 흩어 놓는다. */
function fresh(rows, spread) {
  return { y: spread ? Math.random() * rows * 1.4 - rows * 0.4 : -Math.random() * rows * 0.3, speed: 0.35 + Math.random() * 0.65, row: -1, glyph: '' };
}

function resize() {
  if (!canvas) return;
  const ratio = window.devicePixelRatio || 1;
  const width = window.innerWidth, height = window.innerHeight;
  canvas.width = Math.max(1, Math.round(width * ratio));
  canvas.height = Math.max(1, Math.round(height * ratio));
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  ctx.font = `${CELL - 2}px "MS Gothic", "Cascadia Mono", Consolas, monospace`;
  ctx.textBaseline = 'top';
  ctx.fillStyle = '#000';
  ctx.fillRect(0, 0, width, height);
  const columns = Math.ceil(width / CELL), rows = Math.ceil(height / CELL);
  streams = Array.from({ length: columns }, (_, i) => streams[i] || fresh(rows, true));
}

function step() {
  const width = window.innerWidth, height = window.innerHeight;
  const rows = Math.ceil(height / CELL);
  // 지난 글자를 조금씩 어둡게 덮어 꼬리를 만든다.
  ctx.fillStyle = `rgba(0, 0, 0, ${FADE})`;
  ctx.fillRect(0, 0, width, height);
  streams.forEach((stream, column) => {
    stream.y += stream.speed;
    const row = Math.floor(stream.y);
    if (row !== stream.row) {
      const x = column * CELL;
      // 방금까지 맨 앞이던 글자는 녹색으로 다시 그리고, 새 맨 앞 글자는 밝게 그린다.
      if (stream.glyph && stream.row >= 0) {
        ctx.fillStyle = '#000';
        ctx.fillRect(x, stream.row * CELL, CELL, CELL);
        ctx.fillStyle = BODY;
        ctx.fillText(stream.glyph, x + 1, stream.row * CELL);
      }
      stream.row = row;
      stream.glyph = glyph();
      if (row >= 0) {
        ctx.fillStyle = HEAD;
        ctx.fillText(stream.glyph, x + 1, row * CELL);
      }
    }
    // 화면 아래로 사라진 줄기는 조금 있다가 위에서 다시 시작한다.
    if (row > rows && Math.random() > 0.9) Object.assign(stream, fresh(rows, false));
  });
}

function loop(now) {
  frame = requestAnimationFrame(loop);
  if (now - lastStep < STEP_MS) return;
  lastStep = now;
  step();
}

/** 흐르는 글자를 켜거나 끈다. */
export function setRain(on) {
  if (on === !!canvas) return;
  if (!on) {
    cancelAnimationFrame(frame);
    window.removeEventListener('resize', resize);
    canvas.remove();
    canvas = ctx = null;
    streams = [];
    return;
  }
  canvas = document.createElement('canvas');
  canvas.id = 'matrix-rain';
  canvas.setAttribute('aria-hidden', 'true');
  ctx = canvas.getContext('2d', { alpha: false });
  document.body.prepend(canvas);
  window.addEventListener('resize', resize);
  resize();
  lastStep = 0;
  frame = requestAnimationFrame(loop);
}
