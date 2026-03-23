const DEFAULT_PAGE = Object.freeze({
  id: "item-47470773",
  location: "news.ycombinator.com/item?id=47470773",
  path: "/proxy/hn/item?id=47470773",
  subtitle: "Regular Hacker News comments, proxied into the local shell for deterministic demo recording.",
  title: "Tinybox – Offline AI device 120B parameters",
});

function render(root) {
  root.innerHTML = `
    <div class="hn-mobile-app">
      <iframe
        class="hn-page-frame"
        loading="eager"
        referrerpolicy="origin"
        src="${DEFAULT_PAGE.path}"
        title="Regular Hacker News page"
      ></iframe>
    </div>
  `;
}

export function mountApp(root) {
  render(root);
}
