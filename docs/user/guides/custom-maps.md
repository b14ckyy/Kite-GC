# Your own maps

Besides the built-in maps, Kite can show map imagery you bring yourself — in two ways:

| | Custom online providers | Custom maps from a folder |
|---|---|---|
| **Source** | An online tile service, added by its URL template | Tile files in a folder on your computer |
| **Platforms** | Windows, Linux, macOS, Android, iPhone and iPad | Windows, Linux and macOS |
| **Internet** | Needed; viewed tiles are cached like those of the built-in maps | Not needed |
| **Set up in** | Settings → Map → Providers → **Custom Online Provider** | Settings → Map → Providers → **Custom Offline Map Location** |
| **In the map selector** | Group **Custom online maps**, after the built-in maps | Group **Custom offline maps**, at the end, marked **▦** |

Either kind replaces the imagery on the 2D map and the 3D globe, and you pick it in **Map Provider**
like any built-in map.

## Custom online providers

Many map services deliver their tiles under a URL pattern such as
`https://tiles.example.com/{z}/{x}/{y}.png`, often with an API key from your account at that service.
Enter that pattern in Kite and the service becomes one of your maps, on every platform. An API key is
simply part of the URL.

!!! note "The service's terms are yours to keep"
    Kite fetches what you configure. Whether you may use a service this way — in a ground control
    station, with its tiles kept in Kite's tile cache — and how many requests your plan allows is up
    to the service's terms, and keeping them is your responsibility. Kite does not track a request
    quota.

### Adding a provider

1. Open **[Settings → Map → Providers](../reference/settings.md#providers)**. The row **Custom Online
   Provider** shows how many you have added (**None**, **1 provider**, **2 providers**, …).
2. Press **Manage**. The **Custom Online Providers** dialog lists your providers, each with its name
   and the service's host name.
3. Press **Add provider** and fill in the form (**New online provider**) — see the fields below.
4. Press **Test** to check the provider with one tile, then **Save**. **Cancel** leaves the form
   without saving.
5. **Close** the dialog and pick the map in **Map Provider**, under **Custom online maps**.

The form has these fields:

| Field | What it is |
|---|---|
| **Name** | The name in the map selector. Required; two of your providers cannot have the same name (upper and lower case count as the same). |
| **URL template** | The tile address with placeholders — see the next section. |
| **Max zoom** | The highest zoom level in the service's URLs (the highest `{z}`), 1 to 22. A new provider starts at 19. See [Max zoom](#max-zoom-and-zooming-in-further). |
| **Tile size** | The pixel size of one tile as the service delivers it: **256 px** (the start value), **512 px** or **128 px**. See [Tile size](#tile-size). |
| **Attribution (optional)** | The credit line the service asks for, for example `© Provider name`. Plain text: Kite shows it exactly as typed, without links. |

Spaces at the start and end of the name, the URL and the attribution are removed when you save.

### The URL template

Kite fills in these placeholders for every tile, written exactly as shown, in lower case:

| Placeholder | Filled with |
|---|---|
| `{z}` `{x}` `{y}` | Zoom level, column and row of the tile — **required**. Rows are counted from the top (the north), the usual XYZ scheme. |
| `{-y}` | The row counted from the bottom (the south), for a service in **TMS** order — use it **instead of** `{y}`. |
| `{s}` | A subdomain: `a`, `b` or `c`, changing from tile to tile. |
| `{r}` | `@2x` on a high-resolution screen, otherwise nothing — 2D map only; the 3D globe always leaves it empty. Optional. |

Two generic examples (`tiles.example.com` and `YOUR_KEY` stand for the service's host and your key):

```text
https://tiles.example.com/v1/{z}/{x}/{y}.png?key=YOUR_KEY
https://{s}.tiles.example.com/YOUR_KEY/{z}/{x}/{-y}{r}.jpg
```

- Take the pattern and the place of the key from the service's own documentation. Kite sends the
  key only as part of the URL; a service that wants its key in a request header cannot be used.
- `{s}` always means `a`, `b` or `c`. For a service with other subdomain names, write one of them
  into the URL instead.
- Use `{r}` only if the service offers `@2x` tiles under that name.

#### When Kite refuses the template

Kite checks the form when you press **Save** (all fields) or **Test** (the URL only). From then on the
messages under the fields update as you type:

| Message | Cause |
|---|---|
| **Enter a name.** | The name is empty. |
| **A provider with this name already exists.** | Another of your providers has this name. |
| **Enter an http:// or https:// URL.** | The template is not a web address starting with `http://` or `https://`. |
| **The URL needs the zoom, column and row placeholders.** | `{z}` or `{x}` is missing, or neither `{y}` nor `{-y}` is in the template. |
| **Unknown placeholder {…}. Allowed: {z} {x} {y} {-y} {s} {r}.** | The template contains a `{…}` Kite does not fill in — for example `{Z}`, or a placeholder the service expects you to replace by hand, such as `{apikey}`: put your key there instead. |

### Testing a provider

**Test** loads one tile through the same path the map uses and tells you what came back. The tile
is the one under the **centre of the 2D map**, at zoom 12 — or at the **Max zoom** if that is lower.
The message names the tile as `zoom/column/row`:

- **OK: tile … loaded, … × … px** — the service delivered an image; the size is its pixel size.
- **Tile …: the server answered, but not with an image (wrong key or URL?).** — the service sent
  something else, often an error page or message.
- **Tile … failed: …** — the request failed. The reason is the service's HTTP status (for example
  `tile HTTP 403 Forbidden`) or a network error. It never contains your URL: where the URL would
  appear, Kite shows only the host name.

When the test tile is square and its size does not match **Tile size**, Test sets the field to the
size it measured and says so: **Tile size set to … px to match the test tile.** A tile of another
size counts as the nearest of the three — a 300 px tile as 256 px, a 1024 px tile as 512 px. Like
every other field, it is stored only when you press **Save**, and you can set it back by hand.

!!! tip "Test where the service has tiles"
    A regional service — national aerial imagery, for example — has no tiles outside its area, and the
    test fails there even with a correct URL and key. Move the 2D map into the service's area first.

The tested tile is not stored in the tile cache. A test result disappears as soon as you change the
URL or the max zoom.

### Tile size

Kite needs to know how large the service's tiles are to place them at the right zoom level:

- **256 px** — the usual size; at map zoom 15 Kite loads the service's level 15.
- **512 px** — Kite loads **one level lower** than the map zoom (level 14 at map zoom 15) and shows
  each tile at 512 pixels. One such tile covers what four 256 px tiles do, so the same view needs
  **a quarter as many tiles and requests**, and labels appear at the size the service drew them.
- **128 px** — the opposite: Kite loads one level higher than the map zoom, and the same view needs
  four times as many tiles.

With the wrong setting the map still lines up, but the tiles are drawn at the wrong scale: 512 px
tiles set as 256 px show half-size labels and cost four times the requests. The 3D globe uses the
tile size as well: it draws a 512 px service from one level lower.

### Max zoom and zooming in further

Set **Max zoom** to the highest level in the service's URLs — the highest `{z}` it delivers — whatever
the tile size:

- **2D map** — it loads tiles up to the max zoom and shows them at full resolution up to the matching
  map zoom level; with **512 px** tiles that is one map zoom level further, with **128 px** tiles one
  level less. Zoomed in further, it **enlarges the tiles of the highest level**, so a service that
  ends at, say, level 16 does not stop the map there: the 2D map zooms to at least level 20.
- **3D globe** — it loads tiles up to the max zoom and no further; closer in, the globe shows no
  more detail.

Kite does not detect a missing level by itself. If the max zoom is set higher than the service
delivers, the tiles of the missing levels fail: the 2D map has nothing to show there, while the 3D
globe keeps the last level that loaded.

### Editing and deleting

In the **Custom Online Providers** dialog:

- **Edit** opens the form (**Edit online provider**) with the provider's values. After **Save** the
  2D map and the 3D globe use the changed provider at once; if it is the selected map, it stays
  selected, also after a rename.
- **Delete** turns into **Really delete?** — press it again to delete the provider.

When you delete the provider that is selected, the map shows **OpenStreetMap**, and so does **Map
Provider**. Pick another map there. Adding the same provider again creates a new entry, which you
select again.

**Escape** takes you from the form back to the list, and closes the dialog from the list. While the
form holds changes you have not saved, Escape does nothing — **Cancel** discards them. A click
outside the dialog closes the list, but not a form you are filling in.

### Cache, keys and privacy

- **The tile cache.** Tiles of your providers go into Kite's
  **[Tile Cache](../reference/settings.md#cache)** like those of the built-in maps (unless
  the cache is set to off), so an area you have viewed also works offline. The cache stores each tile
  under its full URL — key included. Tiles stay in the cache after you delete a provider or change
  its key or URL, until the cache drops the least recently used tiles to stay within its size or you
  press **Clear**. A tile already in the cache is shown without a new request.
- **Where the key is kept.** The URL template, key included, is stored in plain text in Kite's
  settings on your device.
- **The diagnostics log.** Kite does not write the URL to the diagnostics log. Error messages show
  the service's host name only, and the dialog's list shows the host name, not the full URL.
- **Keys restricted to websites.** Kite requests tiles from its own backend, like a standalone
  program — not from a web page, so the request carries no website origin or referrer. A key that the
  service only accepts from certain *allowed origins* or *referrers* is therefore likely refused,
  usually with an HTTP error in **Test**. Create the key without such a restriction.
- **Attribution.** The attribution text is shown in the credits line at the bottom of the 2D map. The
  3D globe shows no credits.
- **Who sees the requests.** The service receives the requests for the tiles you view — see
  [Privacy](../privacy.md#third-party-services-the-app-contacts).

## Custom maps from a folder

On the desktop — Windows, Linux and macOS — Kite can show map tiles from a **folder on your computer**
instead of an online tile provider. A custom map needs no internet connection: Kite reads the tiles
straight from disk, on the 2D map and on the 3D globe. You can keep several maps side by side and pick
one like any other tile provider.

Custom maps are not available on Android, iPhone and iPad; the selector entries, the Settings row and
the map frames are not shown there.

!!! note "Your tiles, your responsibility"
    Kite does not download tiles for a custom map — you put them into the folder yourself. You are
    responsible for having the right to use the tiles you store. The terms of many map providers do
    not allow downloading their tiles in bulk. Kite shows no attribution line for a custom map.

### The map folder

Kite reads all custom maps from one folder, the **custom map location**. By default that is a
`custom_maps` folder in Kite's app data folder:

| Windows (installed) | Linux (installed) | macOS (installed) | Portable |
|---|---|---|---|
| `%APPDATA%\kite-gc\custom_maps\` | `~/.local/share/kite-gc/custom_maps/` | `~/Library/Application Support/kite-gc/custom_maps/` | `<app>/data/custom_maps/` |

Kite creates the default folder if it does not exist. Moving the **Flight log database** to another
folder in Settings does not move it. To use a different folder — another drive, a USB stick, a network
share — pick it in Settings (see [below](#choosing-a-custom-map)).

#### Layout

Every folder directly inside the location is **one map**, and its folder name is the name the map gets
in the selector. Inside a map folder the tiles follow the usual `Z/X/Y` layout: one folder per zoom
level, in it one folder per tile column (X), in that one image file per tile, named after its row (Y):

```text
custom_maps/
├── Home field/
│   ├── 15/
│   │   ├── 17000/
│   │   │   ├── 11000.png
│   │   │   └── 11001.png
│   │   └── 17001/
│   │       └── 11000.png
│   └── 16/
│       └── …
└── Coast survey/
    └── …
```

- **XYZ order, rows counted from the top** — the scheme OpenStreetMap uses: row 0 is the northern edge
  of the world. A tile set in **TMS** order (rows counted from the bottom) is not converted and lands
  in the wrong place on the map.
- The **zoom-level folders sit directly in the map folder**, with no extra folder in between. Zoom
  levels 0 to 30 are read.
- Folder and file names are **plain numbers**: `12`, not `012`.
- Tile files are **`.png`, `.jpg`, `.jpeg` or `.webp`**, in upper or lower case; one map may mix them.
  Other files are ignored.
- **One tile size per map**: 256 × 256 pixels, the usual size, or 512 × 512 or 128 × 128. Kite detects
  the size by itself — see [The map's tile size](#the-maps-tile-size).
- A map does not need every zoom level. It can start and end at any level and leave levels out.

Folders that Kite does **not** list as a map:

- a folder whose name starts with a dot (hidden folders),
- a folder whose name contains a colon `:` or a control character,
- a folder without any tile in the layout above (for example, tiles inside an extra subfolder).

Files lying directly in the location are ignored.

### Choosing a custom map

Two settings are involved:

- **Map Provider** (**[Settings → Interface → Display](../reference/settings.md#display)**) — the maps
  Kite found are at the **end of the list**, under **Custom offline maps**, each marked with **▦**
  and sorted by name. Pick one and it replaces the imagery on the 2D map and the 3D globe.
- **Custom Offline Map Location** (**[Settings → Map → Providers](../reference/settings.md#providers)**)
  — the folder in use. **Choose** opens a folder picker; Kite reads the new folder at once and the
  selector lists its maps. **Use Default** goes back to the default folder (it is greyed out while the
  default is in use). Hover over the label for a short reminder of the layout.

Below the location Kite shows how many maps it found (**Scanning…** while it reads), plus two buttons:

- **Show map area** (only while a custom map is selected) moves the map to the area the map covers
  that is **nearest to the centre of the 2D map**. On the 2D map the follow mode switches to **Free**
  for this. In 3D the camera flies there in **Free** mode; in Follow, Orbit and FPV it stays with the
  aircraft. If the map holds no tiles, or its tiles cannot be read, a message appears under the row
  instead.
- **Rescan** reads the location again — see [Rescan](#after-adding-tiles-rescan).

Kite remembers the selected map by its **folder name**. When you change the location and the new one
has a map of the same name, Kite shows that one; otherwise the map shows **OpenStreetMap** until the
map is back or you pick another one.

### How a custom map is drawn

#### The map's tile size

A custom map has no tile-size setting. When Kite reads the location, it reads the pixel size of one
tile per map: the first tile it finds in the map's **highest zoom level**. If that tile's size cannot be
read, Kite tries the first tile of the next lower level, and so on. The size counts as the nearest of
**128, 256 and 512 px** — 100 px tiles count as 128 px, 300 px tiles as 256 px, 1024 px tiles as 512 px;
for a tile that is not square, the longer side decides. If no tile's size can be read, the map counts as
256 px.

The whole map is then drawn with that size, the same way as a
[custom online provider](#tile-size):

- **256 px** — the folder's level 15 is shown at map zoom 15.
- **512 px** — each tile is shown at 512 pixels, the folder's level 14 at map zoom 15. The same view
  needs a quarter as many tiles, and labels appear at the size they were drawn at.
- **128 px** — the opposite: each tile is shown at 128 pixels, the folder's level 16 at map zoom 15,
  and the same view needs four times as many tiles.

Tiles between the three sizes are stretched or shrunk a little to fit: 100 px tiles fill 128 pixels.

**One size per map.** The size Kite detects applies to every tile of the map; a folder that mixes tile
sizes is not supported. Tiles of another size still land in the right place, but squeezed or stretched
to the detected size, so their labels and details appear too small or too large. The size is detected
again whenever Kite reads the location — press [Rescan](#after-adding-tiles-rescan) after replacing a
map's tiles with ones of another size.

The 3D globe uses the tile size as well: it draws a 512 px map from one level lower.

#### Missing tiles and zoom levels

Below, a **level** is one of the map's zoom-level folders. With 256 px tiles it is shown at the map zoom
of the same number; with 512 px tiles one map zoom higher, with 128 px tiles one lower.

- **Gaps are filled from lower levels.** Where a tile is missing, the 2D map enlarges the nearest
  lower-level tile that exists — however many levels down that is. A map with a coarse overview and a
  detailed patch therefore shows no holes around the patch.
- **One level below the lowest level** the 2D map still shows the map: Kite builds each tile from the
  (up to four) tiles of the lowest level, scaled down. Zoomed out further, the map is empty.
- **Outside the tiles the map is empty.** No online map fills in around a custom map.
- **Above the highest level** the 2D map enlarges the highest-level tiles and keeps zooming — up to map
  zoom 20, or two map zooms past the one that shows the highest level, if that is higher.
- **In 3D** the globe uses the same tiles up to the map's highest level. Where a tile is missing, the
  lower level stays visible. Below the map's lowest level the globe shows none of it — the "one level
  below" step is 2D only.

For example, a map with the levels 8 to 13 on the 2D map:

| Tile size | Levels 8–13 at full resolution | One level below (built from level 8) | Empty at map zoom | Highest level enlarged from map zoom |
|---|---|---|---|---|
| 256 px | map zoom 8–13 | map zoom 7 | 6 and lower | 14 |
| 512 px | map zoom 9–14 | map zoom 8 | 7 and lower | 15 |
| 128 px | map zoom 7–12 | map zoom 6 | 5 and lower | 13 |

In all three cases the 2D map zooms in to map zoom 20.

#### Frames on the 2D map

A detailed patch of a few hundred metres is easy to lose on a large map. So while a custom map is
selected, the 2D map frames every **area** it covers (see the next section) with a thin blue line. The
frames are always shown, are not clickable and change with the zoom:

- **Dashed rectangle** — centred on the area and **four times its width and height**, shown while that
  rectangle fits into the map view. It points you to the area from further out.
- **Solid outline** — once you zoom in so far that the dashed rectangle would be wider or taller than
  the view, the frame turns into the outline of the area itself: exactly where its highest-resolution
  tiles are.
- **Small circle** — when you zoom out so far that the dashed rectangle would be only a few pixels
  across.

The 3D globe shows no frames.

#### What counts as an area

An area is a patch of connected tiles. Kite looks at the map's **highest zoom level first**: every
separate patch of tiles there is an area. Each lower level adds areas only where it covers ground that
no higher level covers. In practice:

- A full pyramid around one airfield — from an overview of the whole world down to the detail level at
  the airfield — is **one** area: the airfield, not the world.
- Separate patches, such as two flying sites, are **separate areas**, each with its own frame, even when
  they end at different zoom levels.
- Small gaps or notches in the highest level (tiles a tool left out, for instance) do not get a frame of
  their own.
- Kite keeps at most **64 areas**, highest levels first. Further areas get no frame, and **Show map
  area** cannot jump to them; the diagnostics log then notes it.
- For very large or ragged areas the outline is drawn on a coarser grid. The frames are a help for
  finding the tiles, not an exact map of every tile.

### Practical notes

#### The index file in the map folder

To draw the frames, Kite scans a map the first time it needs its areas. The scan only lists folders —
it never opens a tile — and its result goes into a small file, **`.kite-coverage.json`**, in the map's
folder. Kite reuses it as long as the map's folders are unchanged; adding or removing tiles changes the
folders, and Kite scans again. On a large map the first scan can take a moment; the tiles show
meanwhile and the frames appear when it is done.

When the map folder is **read-only** (a write-protected SD card, a read-only network share), the file
goes into a `custom_maps_index` folder in Kite's app data folder instead — for example
`%APPDATA%\kite-gc\custom_maps_index\` on Windows, or `<app>/data/custom_maps_index/` in portable mode.
You can delete the index file at any time; Kite builds it again.

#### After adding tiles: Rescan

Kite reads the list of maps, their zoom levels and their tile sizes when it needs them, not
continuously. Press **Rescan** after you:

- add a new map folder or new zoom levels, or replace a map's tiles with ones of another size,
- connect the drive that holds the maps — a **USB stick** or **SD card** plugged in after Kite started,
- notice that new or changed tiles do not show up.

Rescan reads the location again, redraws the selected custom map and rebuilds its index file from
scratch.

#### When the folder or the drive disappears

- If the drive goes away while a custom map is shown, tiles already on screen stay and new ones stay
  empty.
- The next time Kite reads the location (**Rescan**, **Choose**, or the next start) and the chosen
  folder is missing, an error appears under the row, the selector lists no custom maps and the map
  shows **OpenStreetMap**. When only the selected map's folder is gone, the other maps stay in the
  selector and the map shows OpenStreetMap as well. A missing default folder is simply created again.
- Your choice is kept: once the folder is back and Kite has read it again — **Rescan** or a restart —
  the custom map returns.

#### Network folders

A location on a network share works, but every tile is a separate file read over the network, so the
map fills in more slowly than from a local disk. Some network shares do not report changes to their
folders; there the frames can stay out of date after you add tiles — press **Rescan**.

#### Not part of the tile cache

Custom-map tiles are read from disk every time and never copied into Kite's **Tile Cache**. The cache
size and **Clear** in **Settings → Map → Cache** do not affect them, and a custom map does not use up
cache space.

## Where to go next

- Every map setting: **[Settings → Map](../reference/settings.md#map)**, and the **Map Provider** under
  **[Interface → Display](../reference/settings.md#display)**.
- The 3D globe and its terrain: **[3D map](map-3d.md)**.
- Offline use of the online maps: **[FAQ](../faq.md#can-i-use-the-maps-offline)**.
