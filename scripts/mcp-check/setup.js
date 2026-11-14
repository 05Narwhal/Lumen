(async () => {
  const inv = window.__TAURI_INTERNALS__.invoke;
  await inv("create_library", { path: "C:/Users/Sample/AppData/Local/Temp/claude/d340f0a98b1451a1/d2615125-ce2a-42b9-a533-ccc7335811f7/scratchpad/d1/library/Lumen Test" }); const s = await inv("get_settings");
  s.aiAssistants.enabled = true;
  s.aiAssistants.port = 47879;
  s.smart.enabled = true;
  s.smart.experimental.aiMasks = true;
  const saved = await inv("update_settings", { settings: s });
  inv("fetch_models").catch(() => {});
  const batch = await inv("start_folder_import", {
    path: "C:/Users/Sample/AppData/Local/Temp/claude/d340f0a98b1451a1/d2615125-ce2a-42b9-a533-ccc7335811f7/scratchpad/d1/photos",
    recursive: false,
    limitBytes: null,
  });
  return { batch, token: saved.aiAssistants.token, port: saved.aiAssistants.port, url: location.href };
})()


