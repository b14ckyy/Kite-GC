// Sidebar: keep the first navigation section ("Getting started") expanded. Material has no
// per-section default, so this checks the section's toggle after every page load — including
// navigation.instant loads, which replace the sidebar (document$ fires for each).
document$.subscribe(function () {
  var toggle = document.querySelector(
    ".md-nav--primary > .md-nav__list > .md-nav__item--nested > .md-nav__toggle"
  );
  if (toggle && !toggle.checked) {
    toggle.checked = true;
    toggle.dispatchEvent(new Event("change", { bubbles: true }));
  }
});
