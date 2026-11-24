# Google Takeout for Lumen

Imports a **Google Takeout** of Google Photos into your Lumen library. Choose the folder with your
Takeout (or the folder you unpacked it into; `takeout-*.zip` archives are read in place) and Lumen
copies each photo and video in, the way it does for any import.

Google keeps what it knows about a photo in a JSON file next to it. This plugin reads those files and gives
Lumen:

- the **date** the photo was taken (not the date you downloaded it),
- the **place**, when Google has one,
- the **title** and **description**,
- **favourites**, and
- **albums**, from Takeout's folders (switch this off in the plugin's page if you want the photos only).

Nothing is uploaded or sent anywhere: the plugin
has one permission, `import.read-chosen`, which lets it read below the folder you picked and nothing else.

Run it on a **copy** of the Takeout the first time, in a test library, and look at the result before you
import for real. The *Import* page shows the plugin as *From Google Takeout*.

MIT. Third-party code linked into the plugin is listed in `NOTICES`.
