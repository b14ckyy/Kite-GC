# Render-Pfad im Webview — Messung

Gemessen am 19.08.2026 auf `marc-5300` in einem echten `WebKit2.WebView` (webkit2gtk 2.52.5,
Debian 13, Intel UHD 620). Aufgabe: **60 fps bei 1920×1080 auf den Schirm bringen**, streng auf
16,67 ms getaktet. CPU = Summe über den gesamten Prozessbaum (UI + WebKitWebProcess + GPU-Process).

## Ergebnis

| Modus | fps | zu spät | CPU | netto über Idle |
|---|---|---|---|---|
| `idle` (Standbild, kein Repaint) | – | 0 | 2 % | Baseline |
| **`img.src = blobURL`** (JPEG pro Frame) | 60,0 | 0 | 136 % | **+134 %** |
| `createImageBitmap` → `ctx2d.drawImage` | 59,9 | 9 | 98 % | **+96 %** |
| **`createImageBitmap` → `bitmaprenderer.transferFromImageBitmap`** | 59,9 | 6 | 90 % | **+88 %** |
| `VideoDecoder` (H.264) → `ctx2d.drawImage` | 59,4 | 5 | 173 % | **+171 %** |
| `VideoDecoder` (H.264) → WebGL `texImage2D` | – | – | 136 % | **+134 %** |

Engine-Caps: `bitmaprenderer: true`, `webgl: true`, `createImageBitmap: function`,
`VideoDecoder: function`, **`ImageDecoder: undefined`**.

## Befund 1 — WebCodecs ist der teuerste Pfad, nicht der billigste

+171 % gegen +88 %: H.264-Hardware-Decode ist im Webview *schlechter* als JPEG.
Ursache im WebKit-Quellcode, 2.52:

```cpp
allowedSinkCaps = gst_caps_from_string("video/x-raw");
// FIXME: Add DMABuf and GL caps here. See also
// https://bugs.webkit.org/show_bug.cgi?id=288625
```

Der von der iGPU dekodierte Frame wird in den System-RAM heruntergeladen und beim Zeichnen
wieder hochgeladen. Bei 1080p60 ist dieser Roundtrip teurer als der komplette JPEG-Decode.

Zusätzlich gemessen: WebKitGTKs WebCodecs-Decoder gibt den ersten Frame erst nach dem
**4. `decode()`** heraus und hält diesen Versatz durch (`pipelineDelay_p50 = 61 ms` bei 60 fps).
Das ist **nicht** vom Bitstrom abhängig — `ffprobe` meldet `has_b_frames=0` (Constrained
Baseline, `-bf 0`) auf allen Testclips, und weder `optimizeForLatency: true` noch
`prefer-hardware`/`prefer-software` noch `bitstream-restriction=1` ändern etwas. Der Hold sitzt
in WebKits GStreamer-Decoder-Harness.

**Konsequenz: H.264 im Webview ist in beiden Dimensionen schlechter als MJPEG** — +66 ms Latenz
bei 60 fps und +83 Prozentpunkte CPU. Damit auch als Sekundäroption raus.

## Befund 2 — Der Render-Pfad bringt ~34 % ohne Backend-Änderung

`bitmaprenderer` statt `img.src`: **136 % → 90 %**. Gleicher Stream, gleiche Latenz, gleiche
ffmpeg-Zeile. Nur der Weg vom JPEG zum Pixel ist ein anderer:

```js
const cv  = document.querySelector('canvas');
const bmr = cv.getContext('bitmaprenderer');       // NICHT '2d'

// pro Frame aus dem MJPEG-Strom:
const bmp = await createImageBitmap(blob);          // decodiert off-main-thread
bmr.transferFromImageBitmap(bmp);                   // Übergabe ohne Kopie
// kein drawImage, kein bmp.close() — transferFromImageBitmap konsumiert das Bitmap
```

`transferFromImageBitmap` übergibt das Bitmap direkt an den Canvas-Backing-Store, statt es wie
`drawImage` in einen bestehenden Puffer zu blitten. Das spart eine vollständige 1080p-Kopie pro
Frame: ~8 Prozentpunkte gegenüber `drawImage`, ~46 gegenüber dem `<img>`-Pfad.

**Worker:** `createImageBitmap` läuft in einem Web Worker, `ImageBitmap` ist transferierbar.
Decode im Worker, `postMessage(bmp, [bmp])` zum Main-Thread, dort nur noch
`transferFromImageBitmap`. Gesamt-CPU bleibt ähnlich, aber der UI-Thread wird frei — relevant
neben Karte, Telemetrie und Cesium-3D-View.

**Canvas-Auflösung:** Im Test war der Canvas 1920×1080 und wurde per CSS auf 960×540 skaliert;
die Skalierung kostet mit. Backing-Size auf die tatsächliche Anzeigegröße setzen
(`canvas.width = clientWidth * devicePixelRatio`) und ffmpeg direkt in dieser Auflösung
encodieren lassen. Ein 960 breites Videofenster mit 1080p-JPEG zu füttern ist doppelt bezahlte
Arbeit — im Encoder *und* im Decoder.

## Einordnung

Die ~30 % CPU sind der Preis dafür, 60 Bilder/s überhaupt durch einen Webview zu schieben.
Kein Codec-Wechsel ändert daran etwas: günstigster und teuerster Pfad liegen bei +88 % und
+171 % *eines Kerns*, und beide machen dieselbe Arbeit — ein komprimiertes Bild dekodieren und
in einen Compositor legen.

Nach Nutzen sortiert:

1. **`bitmaprenderer` statt `img`/`drawImage`** — ~34 % weniger CPU im Frontend.
2. **Decode in einen Worker** — Main-Thread wird frei.
3. **In Anzeigeauflösung encodieren und rendern** — spart proportional zur Pixelzahl.
4. **`mjpeg_vaapi` im Backend** — 137 % → 22 % eines Kerns bei 1080p60 im Echtzeittakt
   (siehe unten).
5. **Eigener hardwarebeschleunigter Renderer / nativer GStreamer-Layer** — der einzige Weg, der
   den Webview-Anteil auf ~0 bringt *und* die Latenz unter 120 ms drückt. Langfristiges Ziel,
   nicht für den Initial Release.

## Nebenmessung — `mjpeg_vaapi` im Backend

1080p60 H.264 High → MJPEG, Ausgabe nach `/dev/null`, Intel UHD 620:

| Pfad | Durchsatz | Echtzeittakt (`-re`) |
|---|---|---|
| CPU-Decode + CPU-JPEG | 586 % eines Kerns | – |
| GPU-Decode + CPU-JPEG | 369 % | **137 %** |
| **GPU-Decode + GPU-JPEG** | **117 %** | **22 %** |

Funktionierender Aufruf — **kein Filter** zwischen HW-Decode und Encode:

```sh
ffmpeg -hwaccel vaapi -hwaccel_device /dev/dri/renderD128 -hwaccel_output_format vaapi \
       -i <quelle> -c:v mjpeg_vaapi -global_quality 80 -f mjpeg -
```

Ohne HW-Decode davor:

```sh
ffmpeg -vaapi_device /dev/dri/renderD128 -i <quelle> \
       -vf "format=nv12,hwupload" -c:v mjpeg_vaapi -global_quality 80 -f mjpeg -
```

Zwei Fallen: **`mjpeg_qsv` funktioniert auf dieser Hardware nicht** (`-22 Invalid argument`), und
ein `scale_vaapi`-Filter zwischen HW-Decode und `mjpeg_vaapi` bricht ebenfalls mit `-22` ab.
`-global_quality` statt `-q:v` (VAAPI kennt `-q:v` nicht); bei 80 war der Frame ~20 % größer als
Software-`-q:v 5`.

Zur Laufzeit probieren, nicht annehmen: `/dev/dri/renderD128` fehlt in VMs, bei NVIDIA-Only-Setups
und ohne `render`-Gruppenmitgliedschaft; AMD kann JPEG-Encode je nach VCN-Generation, NVENC gar
nicht. Einen Frame probeweise encodieren und bei Fehler auf Software zurückfallen — dasselbe
Muster wie das `use_ffmpeg`-Flag in `src-tauri/src/video/mediamtx.rs`.

## Messgrundlage

Skript `/tmp/pb2.py` auf marc-5300: 30 verschiedene 1080p-Frames als JPEG-Blobs und als
H.264-AVCC-Access-Units, je 6 s pro Modus, streng auf 60 fps getaktet, CPU-Ticks aus
`/proc/<pid>/stat` über den gesamten Prozessbaum. `idle` = Standbild ohne Repaint als Nullpunkt
für den Compositor-Anteil. Latenz-/Decoder-Hold-Messung in `/tmp/wcb3.py`.

**Wiedervorlage:** WebKitGTK **2.54** hängt DMABuf- und GL-Caps an den WebCodecs-Sink
(`main`-Branch). Damit entfällt der GPU→CPU-Roundtrip und die CPU-Rechnung für H.264 könnte
kippen — die +66 ms Latenz bleiben aber. Beide Skripte laufen unverändert wieder.

## Quellen

* [webkit#288625 — DMABuf/GL-Caps fehlen in der WebCodecs-Sink-Konfiguration](https://bugs.webkit.org/show_bug.cgi?id=288625)
* [VideoDecoderGStreamer.cpp @ webkitglib/2.52](https://raw.githubusercontent.com/WebKit/WebKit/webkitglib/2.52/Source/WebCore/platform/graphics/gstreamer/VideoDecoderGStreamer.cpp)
* [MDN: ImageBitmapRenderingContext.transferFromImageBitmap](https://developer.mozilla.org/en-US/docs/Web/API/ImageBitmapRenderingContext/transferFromImageBitmap)
* [FFmpeg VAAPI](https://trac.ffmpeg.org/wiki/Hardware/VAAPI)

---

## Nachtrag (19.08.2026) — Hebel 1 in Kite gemessen: kein Gewinn, verworfen

`bitmaprenderer` wurde in Kites Main-Thread-Zeichenpfad eingebaut (Branch
`feat/mjpeg-bitmaprenderer`) und im laufenden Dev-Build per `git stash`-Wechsel alternierend
gegen den bestehenden 2d-`drawImage`-Pfad gemessen — gleiche Quelle (UAV-Link Pi, 720p60,
VAAPI-Transcode), `WebKitWebProcess`-CPU über je 12 s aus `/proc`:

| 12-s-Sample | bitmaprenderer | 2d drawImage |
|---|---|---|
| Runde 1 | 169,9 % | 147,9 % |
| Runde 2 | 154,1 % | 151,3 % |

Die Streuung **innerhalb** desselben Pfads (A: 169,9 → 154,1) ist größer als jede Differenz
zwischen den Pfaden; Hauptstörgröße ist die Szenenabhängigkeit der JPEG-Decode-Kosten (das
Testvideo wechselt zwischen sehr unterschiedlich komplexen Szenen, sichtbar auch an 7–11 %
ffmpeg-Schwankung).

**Warum der Tabellenwert nicht ankommt:** die „~34 %" oben vergleichen gegen den `img.src`-Pfad
(136 %) — den benutzt Kite im Worker-Modus gar nicht. Kite sitzt bereits auf
`createImageBitmap` + `drawImage` (98 %); zur Transfer-Variante (90 %) sind es laut eigener
Messung oben nur **~8 Punkte**, und die verschwinden in der Szenen-Varianz. Der große Sprung der
Tabelle war in Kite schon eingebaut.

Dazu kommt realer Umbau-Preis: `transferFromImageBitmap` **konsumiert** das Bitmap (danach 0×0,
Wiederverwendung wirft `InvalidStateError`, im WebView verifiziert) — bei mehreren sichtbaren
Surfaces braucht jede vor der letzten einen Klon, und die Kontextwahl pro Canvas ist endgültig
(`getContext('2d')` liefert danach `null`). Entscheidung: **nicht gebaut**, Branch verworfen,
2d-`drawImage` bleibt.

**Wiedervorlage Hebel „in Anzeigeauflösung encodieren":** kollidiert derzeit mit dem primären
VAAPI-Pfad — `scale_vaapi` zwischen HW-Decode und `mjpeg_vaapi` bricht auf dieser Hardware mit
`-22` ab (siehe Nebenmessung oben), Herunterskalieren ginge also nur im Software-Fallback. Dazu
mehrere Sinks verschiedener Größe und Encoder-Neuverhandlung beim Fenster-Resize. Erst wieder
anfassen, wenn eine dieser Randbedingungen fällt.
