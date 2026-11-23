document.addEventListener('keydown', (e) => {
  if (shortcutsOff) return;
  if (e.key === 'j') next();
});
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') close();
});
document.addEventListener('keydown', (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key === 's') save();
});
