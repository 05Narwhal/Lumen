import { invoke } from "@/lib/ipc";
import type { DetectStatus } from "@/types/generated/DetectStatus";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import type { PluginInspect } from "@/types/generated/PluginInspect";

export const listPlugins = () => invoke<PluginInfo[]>("list_plugins");
export const inspectPluginPackage = (path: string) => invoke<PluginInspect>("inspect_plugin_package", { path });
export const installPlugin = (path: string) => invoke<PluginInfo>("install_plugin", { path });
export const setPluginEnabled = (id: string, on: boolean) => invoke<void>("set_plugin_enabled", { id, on });
/** Returns the token `undoRemovePlugin` takes within this session. */
export const removePlugin = (id: string) => invoke<string>("remove_plugin", { id });
export const undoRemovePlugin = (token: string) => invoke<void>("undo_remove_plugin", { token });
export const setPluginSetting = (id: string, key: string, value: unknown) => invoke<void>("set_plugin_setting", { id, key, value });
export const setPluginDeveloperMode = (on: boolean) => invoke<void>("set_plugin_developer_mode", { on });
export const getPluginDeveloperMode = () => invoke<boolean>("get_plugin_developer_mode");
export const pluginReadme = (id: string) => invoke<string | null>("plugin_readme", { id });
export const loadDevPlugin = (folder: string) => invoke<PluginInfo>("load_dev_plugin", { folder });
export const reloadDevPlugin = (id: string) => invoke<PluginInfo>("reload_dev_plugin", { id });
/** A `button` control (API 1.2): runs the plugin's `ui.action`; the text is the outcome line. */
export const pluginUiAction = (id: string, action: string) => invoke<string>("plugin_ui_action", { id, action });
/** A `stat` (plain text) or `list` (JSON array of strings or `{text, detail?}`) control's value. */
export const pluginUiValue = (id: string, key: string) => invoke<string>("plugin_ui_value", { id, key });
export const pluginLog = (id: string) => invoke<string[]>("plugin_log", { id });
export const takePendingPluginPackage = () => invoke<string | null>("take_pending_plugin_package");

// Round 24 lane D: the apps a plugin declares (§8.2).
export const detectPluginApps = (pluginId: string) => invoke<DetectStatus[]>("plugin_detect", { pluginId });
export const locatePluginApp = (pluginId: string, detectId: string, exe: string) =>
  invoke<DetectStatus>("plugin_locate", { pluginId, detectId, exe });
