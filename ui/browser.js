// 브라우저 창의 주소 줄. 아래쪽 웹 페이지와는 별개의 화면이며, 이 화면만 앱 명령을 부를 수 있다.
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const $address = $('address');

$('back').onclick = () => invoke('browser_back');
$('forward').onclick = () => invoke('browser_forward');
$('reload').onclick = () => invoke('browser_reload');
$('handoff').onclick = () => invoke('browser_handoff', { fresh: false });
$('handoff-new').onclick = () => invoke('browser_handoff', { fresh: true });

$address.addEventListener('keydown', async (ev) => {
  if (ev.key !== 'Enter') return;
  try {
    $address.value = await invoke('browser_navigate', { url: $address.value });
    $address.blur();
  } catch (err) {
    $address.value = String(err);
    $address.select();
  }
});
$address.addEventListener('focus', () => $address.select());

// 페이지가 스스로 주소를 바꾸는 경우까지 따라가도록 주기적으로 확인한다.
async function refresh() {
  if (document.activeElement === $address) return;
  const url = await invoke('browser_current').catch(() => null);
  if (url && url !== $address.value) $address.value = url === 'about:blank' ? '' : url;
}
setInterval(refresh, 700);
refresh();

document.addEventListener('contextmenu', (ev) => ev.preventDefault());
