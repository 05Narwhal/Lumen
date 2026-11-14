import { invoke } from "@/lib/ipc";
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
export const loadDevPlugin = (folder: string) => invoke<PluginInfo>("load_dev_plugin", { folder });
export const reloadDevPlugin = (id: string) => invoke<PluginInfo>("reload_dev_plugin", { id });
export const pluginLog = (id: string) => invoke<string[]>("plugin_log", { id });
export const takePendingPluginPackage = () => invoke<string | null>("take_pending_plugin_package");
