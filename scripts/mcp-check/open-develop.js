(async () => {
  const inv = window.__TAURI_INTERNALS__.invoke;
  const ids = [];
  const names = {};
  for (let id = 1; id <= 12; id++) {
    try {
      const a = await inv("get_asset", { id });
      ids.push(id);
      names[id] = a.filename;
    } catch {}
  }
  const m = await import("/src/lib/develop-source.ts");
  m.developSourceStore.set((s) => ({ ...m.DEFAULT_DEVELOP_SOURCE, sort: s.sort, only: ids, label: "Selection" }));
  history.pushState({}, "", `/develop/${ids[0]}`);
  dispatchEvent(new PopStateEvent("popstate"));
  await new Promise((r) => setTimeout(r, 3000));
  return { ids, names, url: location.href, status: await inv("mcp_status") };
})()
