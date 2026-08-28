// Point the download buttons at the actual latest release assets, and show
// the current tag, without baking a version into the page. The buttons keep a
// working fallback href (the releases/latest page) if this fetch fails or JS
// is off.
(function () {
  const REPO = "issinoho/loadbearer";

  fetch(`https://api.github.com/repos/${REPO}/releases/latest`)
    .then((r) => {
      if (!r.ok) throw new Error(`GitHub API ${r.status}`);
      return r.json();
    })
    .then((release) => {
      const assets = release.assets || [];
      const tag = release.tag_name || "";
      const pick = (re) => assets.find((a) => re.test(a.name));

      const win = pick(/windows.*\.zip$/i);
      const lin = pick(/linux.*\.tar\.gz$/i);

      if (win) {
        document
          .querySelectorAll('[data-dl="windows"]')
          .forEach((el) => (el.href = win.browser_download_url));
      }
      if (lin) {
        document
          .querySelectorAll('[data-dl="linux"]')
          .forEach((el) => (el.href = lin.browser_download_url));
      }

      const hero = document.getElementById("dl-hero");
      if (hero && tag) hero.textContent = `Download for Windows (${tag})`;

      document.querySelectorAll(".rel-tag").forEach((el) => {
        if (tag) el.textContent = tag;
      });
    })
    .catch((err) => {
      console.warn("Could not fetch the latest loadbearer release:", err);
    });
})();
