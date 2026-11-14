// Menu factories (Round 4G): one small function per kind of object, returning the items the
// one context-menu host renders. A factory takes its actions as arguments, so what a menu
// offers is tested without rendering anything, and every surface showing that object (a
// card, a sidebar row, the palette) offers the same things under the same labels.

export { DIVIDER, icon, labels, tidy, type MenuItem } from "./parts";
export { albumMenu, albumPageMenu, collectionMenu, type AlbumMenuActions, type AlbumPageMenuActions, type CollectionMenuActions } from "./album";
export { deviceMenu, type DeviceMenuActions } from "./device";
export { photoMenu, type PhotoMenuActions, type PhotoMenuContext } from "./photo";
export {
  faceMenu,
  importRowMenu,
  memoryMenu,
  personMenu,
  petBoxMenu,
  petMenu,
  setMenu,
  slideshowMenu,
  type FaceMenuActions,
  type ImportRowMenuActions,
  type MemoryMenuActions,
  type PersonMenuActions,
  type PetBoxMenuActions,
  type PetMenuActions,
  type SetMenuActions,
  type SlideshowMenuActions,
} from "./library";
export { photoTextMenu, type PhotoTextMenuActions } from "./photo-text";
export { viewerMenu, type ViewerMenuActions, type ViewerMenuContext } from "./viewer";
export { developCanvasMenu, type DevelopCanvasActions, type DevelopCanvasContext, type DevelopZoom } from "./develop";
export { connectedMenu, mapMenu, type ConnectedKind, type ConnectedMenuActions, type MapMenuActions } from "./connected";
export { pluginPresetMenu, presetsMenu, type PresetsMenuActions, type PresetsMenuContext } from "./presets";
export { pluginMenu, type PluginMenuActions, type PluginMenuContext } from "./plugins";
