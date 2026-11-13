// 고른 테마를 화면이 그려지기 전에 적용한다(어두운 화면이 잠깐 비치지 않게). 나머지 모양 설정은 look.js가 맡는다.
try {
  document.documentElement.dataset.theme = JSON.parse(localStorage.getItem('cypher.look') || '{}').theme || 'dark';
} catch (e) {
  document.documentElement.dataset.theme = 'dark';
}
