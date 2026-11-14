import { createBrowserRouter, redirect, RouterProvider, type RouteObject } from "react-router";
import { Boot } from "./boot";
import { DuplicatesRoute } from "./duplicates";
import { HomeRoute } from "./home";
import { LibraryRoute } from "./library";
import { RootLayout } from "./root-layout";
import { SetupRoute } from "./setup";

/**
 * Every page but the shell, Home, Library, Setup and Duplicates (Home shows its card) loads on
 * first visit. Imported up front, the whole app — MapLibre, Develop, the slideshow player —
 * had to load before the splash could paint; in `tauri dev` that is a request per module.
 */
const page = <M,>(load: () => Promise<M>, pick: (m: M) => RouteObject["Component"]): Pick<RouteObject, "lazy"> => ({
  lazy: async () => ({ Component: pick(await load()) }),
});

const albums = page(() => import("./albums"), (m) => m.AlbumsRoute);
const map = page(() => import("./map"), (m) => m.MapRoute);
const memories = page(() => import("./memories"), (m) => m.MemoriesRoute);
const people = page(() => import("./people"), (m) => m.PeopleRoute);
const pets = page(() => import("./pets"), (m) => m.PetsRoute);
const slideshows = page(() => import("./slideshows"), (m) => m.SlideshowsRoute);
const hidden = page(() => import("./hidden"), (m) => m.HiddenRoute);
const importPage = page(() => import("./import"), (m) => m.ImportRoute);
const develop = page(() => import("./develop"), (m) => m.DevelopRoute);
const connected = page(() => import("./connected"), (m) => m.ConnectedRoute);
const trash = page(() => import("./trash"), (m) => m.TrashRoute);
const search = page(() => import("./search"), (m) => m.SearchRoute);
const settings = page(() => import("./settings"), (m) => m.SettingsRoute);

export const routes: RouteObject[] = [
  { path: "/setup", Component: SetupRoute },
  {
    path: "/",
    Component: RootLayout,
    children: [
      { index: true, Component: HomeRoute },
      { path: "library", Component: LibraryRoute },
      { path: "library/:deviceId", Component: LibraryRoute },
      { path: "library/group/:group", Component: LibraryRoute },
      { path: "albums", ...albums },
      { path: "albums/:id", ...albums },
      { path: "map", ...map },
      { path: "memories", ...memories },
      { path: "memories/:id", ...memories },
      { path: "people", ...people },
      { path: "people/:id", ...people },
      // Round 20: the Pets list is a tab of People & Pets, the Slideshows list a tab of Memories.
      { path: "pets", loader: () => redirect("/people?show=pets") },
      { path: "pets/:id", ...pets },
      { path: "slideshows", loader: () => redirect("/memories?tab=slideshows") },
      { path: "slideshows/:id", ...slideshows },
      { path: "hidden", ...hidden },
      { path: "import", ...importPage },
      { path: "develop", ...develop },
      { path: "develop/:id", ...develop },
      { path: "connected/:kind/:id", ...connected },
      { path: "trash", ...trash },
      { path: "duplicates", Component: DuplicatesRoute },
      { path: "search", ...search },
      { path: "settings", ...settings },
      ...(import.meta.env.DEV
        ? [
            { path: "dev/transport", ...page(() => import("./dev-transport"), (m) => m.DevTransportRoute) },
            { path: "dev/ui", ...page(() => import("./dev-ui"), (m) => m.DevUiRoute) },
          ]
        : []),
    ],
  },
];

const router = createBrowserRouter(routes);

export function AppRouter() {
  return (
    <Boot resume={(to) => void router.navigate(to, { replace: true })}>
      <RouterProvider router={router} />
    </Boot>
  );
}
