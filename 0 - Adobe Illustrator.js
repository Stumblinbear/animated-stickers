/// <reference types="types-for-adobe/Illustrator/2022"/>

// ============================================================================
// Illustrator sticker prep — green-key revision
//
// New in this revision — automated background keying:
//   [K3] The rasterize anti-aliasing method is configurable
//        (CONFIG.antiAliasing) to test whether edge fringe originates in
//        rasterization AA or in Image Trace itself. NOTE: soft alpha in the
//        SOURCE PNG still blends into the key during compositing —
//        scripting cannot edit pixels, so true alpha-thresholding requires
//        an external pre-pass (ImageMagick) if fringe proves objectionable.
//
// [verify on first run] tags mark DOM behaviors to confirm on one document
// before trusting a batch.
// ============================================================================

var CONFIG = {
  // The key color used for transparency, since Image Trace cannot handle it.
  // We remove the key color after tracing to restore transparency.
  keyColor: { red: 0, green: 255, blue: 0 },

  // Detection thresholds for deleting key paths after tracing.
  //
  // `keyMin` is the minimum green channel value for a fill to be considered the
  // key color, while the remaining channels must be below `otherMax`.
  keyDetect: {
    keyMin: 200, // green channel must be at least this
    otherMax: 100, // red and blue must each be at most this
  },

  // Rasterize settings for the flatten step.
  rasterizeResolution: 72,
  antiAliasNone: true, // true → AntiAliasingMethod.None (fringe test)
  // false → ARTOPTIMIZED (Illustrator default-ish)

  // Trace settle-check patience.
  settleMaxAttempts: 10,

  // Layer-name suffix stripped by renameLayers.
  imageSuffix: " Image",

  // Fill-layer white outline width (unchanged from original).
  fillStrokeWidth: 11,
};

/**
 * @type {string[]}
 */
var runWarnings = [];

function getArtboardBounds(artboard) {
  var bounds = artboard.artboardRect,
    left = bounds[0],
    top = bounds[1],
    right = bounds[2],
    bottom = bounds[3],
    width = right - left,
    height = top - bottom;

  return { left: left, top: top, width: width, height: height };
}

function newRect(x, y, width, height) {
  var rect = [];
  rect[0] = x; // left
  rect[1] = -y; // top
  rect[2] = width + x; // right
  rect[3] = -(height - rect[1]); // bottom
  return rect;
}

function makeKeyColor() {
  var c = new RGBColor();
  c.red = CONFIG.keyColor.red;
  c.green = CONFIG.keyColor.green;
  c.blue = CONFIG.keyColor.blue;
  return c;
}

function isKeyFill(fillColor) {
  return (
    fillColor.green >= CONFIG.keyDetect.keyMin &&
    fillColor.red <= CONFIG.keyDetect.otherMax &&
    fillColor.blue <= CONFIG.keyDetect.otherMax
  );
}

function main() {
  var dlg = new Window("dialog", "Conversion Settings");
  dlg.alignChildren = "left";

  var allDocuments,
    shouldRenameLayers,
    shouldConvertLayers,
    shouldResizeLayers,
    doesHavePadding;

  var group = dlg.add("group");
  {
    group.orientation = "column";
    group.alignChildren = "left";

    allDocuments = group.add("checkbox", undefined, "All Documents");
    allDocuments.value = true;

    shouldRenameLayers = group.add("checkbox", undefined, "Rename Layers");
    shouldRenameLayers.value = true;

    shouldConvertLayers = group.add("checkbox", undefined, "Convert Layers");
    shouldConvertLayers.value = true;
  }

  var resizeGroup = dlg.add("panel", undefined, "Resizing");
  {
    resizeGroup.orientation = "column";
    resizeGroup.alignChildren = "left";

    shouldResizeLayers = resizeGroup.add(
      "checkbox",
      undefined,
      "Resize document to 512x512",
    );
    shouldResizeLayers.value = false;

    doesHavePadding = resizeGroup.add(
      "checkbox",
      undefined,
      "Document has 2x padding",
    );
    doesHavePadding.value = false;
  }

  var btnPnl = dlg.add("group");
  {
    btnPnl.alignment = "right";
    btnPnl.okBtn = btnPnl.add("button", undefined, "OK", { name: "ok" });
    btnPnl.okBtn.active = true;
    btnPnl.cancelBtn = btnPnl.add("button", undefined, "Cancel", {
      name: "cancel",
    });
  }

  if (dlg.show() == 2) return;

  var targets = [];

  if (allDocuments.value) {
    targets = app.documents;
  } else {
    targets = [app.activeDocument];
  }

  for (var d = 0; d < targets.length; d++) {
    var doc = targets[d];

    doc.activate();
    app.redraw();

    if (shouldRenameLayers.value) {
      renameLayers(doc);
    }

    if (shouldResizeLayers.value) {
      resizeLayers(doc, doesHavePadding.value);
    }

    if (shouldConvertLayers.value) {
      convertLayers(doc);
    }
  }

  if (runWarnings.length > 0) {
    alert(
      "Completed with " +
        runWarnings.length +
        " warning(s):\n\n" +
        runWarnings.join("\n"),
    );
  }
}

function renameLayers(doc) {
  var SUFFIX = CONFIG.imageSuffix;

  for (var i = 0; i < doc.layers.length; i++) {
    var layer = doc.layers[i];

    if (
      layer.name.length > SUFFIX.length &&
      layer.name.substring(layer.name.length - SUFFIX.length) === SUFFIX
    ) {
      layer.name = layer.name.substring(0, layer.name.length - SUFFIX.length);
    }
  }
}

function resizeLayers(doc, hasPadding) {
  // Scale files that are a multiple of 512. This allows us to prep files at
  // a higher resolution than 512, for better quality results.

  var artboard = doc.artboards[0];
  var bounds = getArtboardBounds(artboard);

  if (bounds.width == 512) return;

  if (hasPadding) {
    // Only handle sizes that are a multiple of 512; with padding, half
    // the size is assumed to be the rendered area.
    if (Math.round(bounds.width / 512) != bounds.width / 512) return;
  }

  var scale = 1 / (bounds.width / 512 / (hasPadding ? 2 : 1));
  var offset = -1 * (bounds.left + bounds.width / 2);

  var items = doc.pageItems;
  for (var i = 0; i < items.length; i++) items[i].selected = true;

  var selection = doc.selection;

  if (selection.length > 0) {
    for (i = 0; i < selection.length; i++) {
      selection[i].translate(offset, offset, true, true, true, true);
      selection[i].resize(
        scale * 100,
        scale * 100,
        true,
        true,
        true,
        true,
        scale * 100,
        Transformation.DOCUMENTORIGIN,
      );
    }
  }

  var scaledArtboardRect = newRect(
    (-bounds.width / 2) * scale,
    (-bounds.height / 2) * scale,
    bounds.width * scale,
    bounds.height * scale,
  );

  artboard.artboardRect = scaledArtboardRect;
}

// Flatten one layer's raster over a key-green background. Returns the
// flattened RasterItem, or null on failure (original left untouched).
/**
 *
 * @param {Document} doc
 * @param {Layer} layer
 * @returns {RasterItem|null}
 */
function flattenRasterOverKey(doc, layer) {
  var rasterItem = layer.rasterItems[0];

  try {
    // Key rectangle exactly under the raster's footprint.
    // geometricBounds = [left, top, right, bottom]. [verify on first run]
    var gb = rasterItem.geometricBounds;
    var left = gb[0],
      top = gb[1],
      right = gb[2],
      bottom = gb[3];

    var keyRect = layer.pathItems.rectangle(
      top,
      left,
      right - left,
      top - bottom,
    );
    keyRect.filled = true;
    keyRect.fillColor = makeKeyColor();
    keyRect.stroked = false;

    // Behind the raster.
    keyRect.move(rasterItem, ElementPlacement.PLACEAFTER);

    // Group raster + key so rasterize consumes exactly these two.
    var grp = layer.groupItems.add();
    rasterItem.move(grp, ElementPlacement.PLACEATBEGINNING);
    keyRect.move(grp, ElementPlacement.PLACEATEND);

    var opts = new RasterizeOptions();
    opts.transparency = false; // composite alpha onto the key — the point
    opts.resolution = CONFIG.rasterizeResolution;
    opts.antiAliasingMethod = CONFIG.antiAliasNone
      ? AntiAliasingMethod.None
      : AntiAliasingMethod.ARTOPTIMIZED;
    // colorModel left at default (document color model, RGB expected).

    // Rasterize the group in place; source art is consumed and replaced
    // by the returned RasterItem. [verify on first run — if the group
    // survives, remove it manually here.]
    var flat = doc.rasterize(grp, grp.geometricBounds, opts);

    return flat;
  } catch (e) {
    runWarnings.push(
      '"' +
        doc.name +
        '" layer "' +
        layer.name +
        '": key-flatten failed (' +
        e +
        ") — traced with original raster (background will be white).",
    );
    return null;
  }
}

function convertLayers(doc) {
  // Flatten each raster over key green, then queue Image Traces. Image Tracing
  // is async, so we follow-up by pumping the event loop until every traced layer
  // has completed its tracing operation.
  var tracedLayerIndexes = [];

  for (var i = 0; i < doc.layers.length; i++) {
    var layer = doc.layers[i];

    if (layer.rasterItems.length == 0) continue;

    doc.activeLayer = layer;

    var flat = flattenRasterOverKey(doc, layer);
    var toTrace = flat !== null ? flat : layer.rasterItems[0];

    toTrace
      .trace()
      .tracing.tracingOptions.loadFromPreset(
        layer.name.indexOf("Fill") !== -1 ? "Sticker Fill" : "Sticker",
      );

    tracedLayerIndexes.push(i);
  }

  if (tracedLayerIndexes.length > 0) {
    var settled = false;

    for (
      var attempt = 1;
      attempt <= CONFIG.settleMaxAttempts && !settled;
      attempt++
    ) {
      app.redraw();

      settled = true;
      for (var t = 0; t < tracedLayerIndexes.length; t++) {
        if (doc.layers[tracedLayerIndexes[t]].pluginItems.length == 0) {
          settled = false;
          break;
        }
      }
    }

    if (!settled) {
      for (var t = 0; t < tracedLayerIndexes.length; t++) {
        var lyr = doc.layers[tracedLayerIndexes[t]];
        if (lyr.pluginItems.length == 0) {
          runWarnings.push(
            '"' +
              doc.name +
              '" layer "' +
              lyr.name +
              '": trace never settled after ' +
              CONFIG.settleMaxAttempts +
              " redraws — not expanded. Re-run on this document.",
          );
        }
      }
    }
  }

  // Now we can expand the traced layers.
  for (var i = 0; i < doc.layers.length; i++) {
    var layer = doc.layers[i];

    if (layer.pluginItems.length == 0) continue;

    for (var j = layer.pluginItems.length - 1; j >= 0; j--) {
      var pluginItem = layer.pluginItems[j];

      if (pluginItem.tracing) {
        try {
          pluginItem.tracing.expandTracing();
        } catch (e) {
          runWarnings.push(
            '"' +
              doc.name +
              '" layer "' +
              layer.name +
              '": expandTracing failed (' +
              e +
              ").",
          );
        }
      } else {
        pluginItem.remove();
      }
    }
  }

  // Delete the key color that we used as a fill-in for transparency.
  for (var i = 0; i < doc.layers.length; i++) {
    var layer = doc.layers[i];

    if (layer.groupItems.length > 0) {
      var group = layer.groupItems[0];

      for (var j = group.pathItems.length - 1; j >= 0; j--) {
        var pathItem = group.pathItems[j];

        if (isKeyFill(pathItem.fillColor)) {
          pathItem.remove();
        } else if (layer.name.indexOf(" Fill") !== -1) {
          // Fill layers: silhouette black flips to white (unchanged).
          if (
            pathItem.fillColor.red <= 5 &&
            pathItem.fillColor.green <= 5 &&
            pathItem.fillColor.blue <= 5
          )
            pathItem.fillColor.red =
              pathItem.fillColor.green =
              pathItem.fillColor.blue =
                255;
        }
      }
    }
  }

  // --- Pass 4: give Fill-layer paths their white stroke (unchanged).
  for (var i = 0; i < doc.layers.length; i++) {
    var layer = doc.layers[i];

    if (layer.name.indexOf(" Fill") === -1) continue;

    if (layer.groupItems.length > 0) {
      var group = layer.groupItems[0];

      for (var j = group.pathItems.length - 1; j >= 0; j--) {
        var pathItem = group.pathItems[j];

        pathItem.stroked = true;
        pathItem.strokeColor.red =
          pathItem.strokeColor.green =
          pathItem.strokeColor.blue =
            255;
        pathItem.strokeWidth = CONFIG.fillStrokeWidth;
      }
    }
  }
}

main();
