var COLORS = [ 'red', 'green', 'blue' ];

function getArtboardBounds(artboard) {
    var bounds = artboard.artboardRect,
        left   = bounds[0],
        top    = bounds[1],
        right  = bounds[2],
        bottom = bounds[3],
  
        width  = right - left,
        height = top - bottom,
  
        props  = {
          left   : left,
          top    : top,
          width  : width,
          height : height
        };

    return props;
}
  
function newRect(x, y, width, height)  {
    var l = 0;
    var t = 1;
    var r = 2;
    var b = 3;

    var rect = [];

    rect[l] = x;
    rect[t] = -y;
    rect[r] = width + x;
    rect[b] = -(height - rect[t]);

    return rect;
}

function main() {
    var dlg = new Window('dialog', 'Conversion Settings');
    dlg.alignChildren = 'left';

    var allDocuments, shouldRenameLayers, shouldConvertLayers, shouldResizeLayers, doesHavePadding, whiteColors = { }, shouldSimplify;

    var group = dlg.add('group');
    {
        group.orientation = 'column';
        group.alignChildren = 'left';

        allDocuments = group.add('checkbox', undefined, 'All Documents');
        allDocuments.value = true;

        shouldRenameLayers = group.add('checkbox', undefined, 'Rename Layers');
        shouldRenameLayers.value = true;

        shouldConvertLayers = group.add('checkbox', undefined, 'Convert Layers');
        shouldConvertLayers.value = true;
        
        // shouldSimplify = group.add('checkbox', undefined, 'Simplify');
        // shouldSimplify.value = true;
    }

    var resizeGroup = dlg.add('panel', undefined, 'Resizing');
    {
        resizeGroup.orientation = 'column';
        resizeGroup.alignChildren = 'left';

        shouldResizeLayers = resizeGroup.add('checkbox', undefined, 'Resize document to 512x512');
        shouldResizeLayers.value = false;
            
        doesHavePadding = resizeGroup.add('checkbox', undefined, 'Document has 2x padding');
        doesHavePadding.value = false;
    }

    var whiteColorGroup = dlg.add('panel', undefined, 'Replace Color');
    {
        whiteColorGroup.orientation = 'row';

        whiteColors['red'] = whiteColorGroup.add('checkbox', undefined, 'Red');
        whiteColors['red'].value = false;

        whiteColors['green'] = whiteColorGroup.add('checkbox', undefined, 'Green');
        whiteColors['green'].value = true;

        whiteColors['blue'] = whiteColorGroup.add('checkbox', undefined, 'Blue');
        whiteColors['blue'].value = false;
    }

    var btnPnl = dlg.add('group');
    {
        btnPnl.alignment = 'right';
        btnPnl.okBtn = btnPnl.add('button', undefined, 'OK', { name: 'ok' });
        btnPnl.okBtn.active = true;
        btnPnl.cancelBtn = btnPnl.add('button', undefined, 'Cancel', { name: 'cancel' });
    }

    if(dlg.show() == 2) return;

    var targets = [];

    if(allDocuments.value) {
        targets = app.documents;
    }else{
        targets = [ app.activeDocument ];
    }

    var whiteReplaceColors = [];

    for(var c in whiteColors) {
        if(whiteColors[c].value) whiteReplaceColors.push(c);
    }

    for(var d = 0; d < targets.length; d++) {
        var doc = targets[d];

        app.open(doc.fullName);

        // For some reason, if we don't "pause" the process after each document, AI goes fucky and just skips some steps
        alert(doc.name);

        if(shouldRenameLayers.value) {
            renameLayers(doc);
        }

        if(shouldResizeLayers.value) {
            resizeLayers(doc, doesHavePadding.value);
        }

        if(shouldConvertLayers.value) {
            convertLayers(doc);
        }
            
        if(whiteReplaceColors.length > 0) {
            replaceColors(doc, whiteReplaceColors);
        }

        /*if(shouldSimplify.value)
            simplifyLayers(targets[d]);*/
    }
}

function renameLayers(doc) {
    for(var i = 0; i < doc.layers.length; i++) {
        var layer = doc.layers[i];

        // Remove the "Image" at the end of the name
        layer.name = layer.name.substring(0, layer.name.length - 6);
    }
}

function resizeLayers(doc, hasPadding) {
    // Scale files that are a multiple of 512. This allows us to prep files at a higher resolution than 512, for better quality results

    var artboard = doc.artboards[0];
    var bounds = getArtboardBounds(artboard);

    if(bounds.width == 512) return;

    if(hasPadding) {
        // Ignore it if the size is not a multiple of 512
        // If it's a multiple of 512, we assume that half of its size is the "rendered area"
        if(Math.round(bounds.width / 512) != bounds.width / 512)
        return;
    }
    
    var scale = 1 / (bounds.width / 512 / (hasPadding ? 2 : 1));
    var offset = -1 * (bounds.left + bounds.width / 2);

    var items = doc.pageItems;
    for(var i = 0; i < items.length; i++)
        items[i].selected = true;
    
    var selection = doc.selection;
    
    // Translate artwork to bring artboard-center at document center, and then apply scale.
    if(selection.length > 0) {
        for (i = 0; i < selection.length; i++) {
            selection[i].translate(offset, offset, true, true, true, true);
            selection[i].resize(scale * 100, scale * 100, true, true, true, true, scale * 100, Transformation.DOCUMENTORIGIN);
        }
    } 
    
    var scaledArtboardRect = newRect(-bounds.width / 2 * scale, -bounds.height / 2 * scale, bounds.width * scale, bounds.height * scale);

    artboard.artboardRect = scaledArtboardRect;

    // var newAB = doc.artboards.add(scaledArtboardRect);
    // artboard.remove();
    
    // doc.fitArtboardToSelectedArt(doc.artboards.getActiveArtboardIndex());
    // app.executeMenuCommand("fitall");
}

function convertLayers(doc) {
    for(var i = 0; i < doc.layers.length; i++) {
        var layer = doc.layers[i];

        if(layer.rasterItems.length == 0) continue;

        // Set the layer as active
        doc.activeLayer = layer;

        var rasterItem = layer.rasterItems[0];

        rasterItem.trace().tracing.tracingOptions.loadFromPreset(layer.name.indexOf('Fill') !== -1 ? 'Sticker Fill' : 'Sticker');
    }

    for(var i = 0; i < doc.layers.length; i++) {
        var layer = doc.layers[i];

        if(layer.pluginItems.length == 0) continue;

        for(var j = layer.pluginItems.length - 1; j >= 0; j--) {
            var pluginItem = layer.pluginItems[j];

            if(pluginItem.tracing) {
                pluginItem.tracing.expandTracing();
            }else{
                pluginItem.remove();
            }
        }
    }

    for(var i = 0; i < doc.layers.length; i++) {
        var layer = doc.layers[i];

        // Cleanup
        if(layer.groupItems.length > 0) {
            var group = layer.groupItems[0];

            for(var j = group.pathItems.length - 1; j >= 0; j--) {
                var pathItem = group.pathItems[j];
                
                if(pathItem.fillColor.red >= 245
                        && pathItem.fillColor.green >= 245
                        && pathItem.fillColor.blue >= 245) {
                    pathItem.remove();
                }else if(layer.name.indexOf(' Fill') !== -1) {
                    if(pathItem.fillColor.red <= 5 && pathItem.fillColor.green <= 5 && pathItem.fillColor.blue <= 5)
                        pathItem.fillColor.red = pathItem.fillColor.green = pathItem.fillColor.blue = 255;
                }
            }
        }
    }
    
    for(var i = 0; i < doc.layers.length; i++) {
        var layer = doc.layers[i];

        if(layer.name.indexOf(' Fill') === -1)
            continue;

        if(layer.groupItems.length > 0) {
            var group = layer.groupItems[0];

            for(var j = group.pathItems.length - 1; j >= 0; j--) {
                var pathItem = group.pathItems[j];

                pathItem.stroked = true;
                pathItem.strokeColor.red = pathItem.strokeColor.green = pathItem.strokeColor.blue = 255;
                pathItem.strokeWidth = 11;
            }
        }
    }
}

function replaceColors(doc, colors) {
    for(var c in colors) {
        var color = colors[c];
        
        for(var i = 0; i < doc.layers.length; i++) {
            var layer = doc.layers[i];

            if(layer.groupItems.length > 0) {
                var group = layer.groupItems[0];

                for(var j = group.pathItems.length - 1; j >= 0; j--) {
                    var pathItem = group.pathItems[j];

                    if(pathItem.fillColor[color] >= 245) {
                        var fail = false;

                        // Make sure the other colors are low, otherwise we replace the wrong colors
                        for(var cc in COLORS) {
                            var lowColor = COLORS[cc];

                            if(color == lowColor) continue;
                            
                            if(pathItem.fillColor[lowColor] >= 100) {
                                fail = true;
                                break;
                            }
                        }

                        if(fail) continue;

                        pathItem.fillColor.red = pathItem.fillColor.green = pathItem.fillColor.blue = 255;
                    }
                }
            }
        }
    }
}

function simplifyLayers(doc) {

}

main();

/*for(var d = 0; d < targets.length; d++) {
    var doc = targets[d];

    var length = doc.layers.length;

    for(var i = 0; i < length; i++) {
        var layer = doc.layers[i];

        if(trace) {
            if(layer.rasterItems.length == 0) continue;

            // Set the layer as active
            doc.activeLayer = layer;

            // Remove the "Image" at the end of the name
            layer.name = layer.name.replace(' Image', '');

            var rasterItem = layer.rasterItems[0];

            rasterItem.trace().tracing.tracingOptions.loadFromPreset(layer.name.indexOf('Fill') !== -1 ? 'Sticker Fill' : 'Sticker');
        }
    }
}

for(var d = 0; d < targets.length; d++) {
    var doc = targets[d];

    var length = doc.layers.length;
    
    if(expand) {
        for(var i = 0; i < length; i++) {
            var layer = doc.layers[i];
    
            if(layer.pluginItems.length == 0) continue;

            for(var j = layer.pluginItems.length - 1; j >= 0; j--) {
                var pluginItem = layer.pluginItems[j];

                if(pluginItem.tracing) {
                    pluginItem.tracing.expandTracing();
                }else{
                    pluginItem.remove();
                }
            }
        }
    }
}

for(var d = 0; d < targets.length; d++) {
    var doc = targets[d];

    var length = doc.layers.length;
    
    if(cleanUp) {
        for(var i = 0; i < length; i++) {
            var layer = doc.layers[i];
    
            if(layer.groupItems.length > 0) {
                var group = layer.groupItems[0];

                for(var j = group.pathItems.length - 1; j >= 0; j--) {
                    var pathItem = group.pathItems[j];
                    
                    if(pathItem.fillColor.red >= 245
                            && pathItem.fillColor.green >= 245
                            && pathItem.fillColor.blue >= 245) {
                        pathItem.remove();
                    }else if(layer.name.indexOf(' Fill') !== -1) {
                        if(pathItem.fillColor.red <= 5 && pathItem.fillColor.green <= 5 && pathItem.fillColor.blue <= 5)
                            pathItem.fillColor.red = pathItem.fillColor.green = pathItem.fillColor.blue = 255;
                    }
                }
            }
        }
    }
}
    
for(var d = 0; d < targets.length; d++) {
    var doc = targets[d];

    var length = doc.layers.length;
    
    for(var c in replaceColors) {
        var color = replaceColors[c];
        
        for(var i = 0; i < length; i++) {
            var layer = doc.layers[i];

            if(layer.groupItems.length > 0) {
                var group = layer.groupItems[0];

                for(var j = group.pathItems.length - 1; j >= 0; j--) {
                    var pathItem = group.pathItems[j];

                    if(pathItem.fillColor[color] >= 245) {
                        var fail = false;
                        
                        // Make sure the other colors are low, otherwise we replace the wrong colors
                        for(var cc in colors) {
                            var lowColor = colors[cc];

                            if(color == lowColor) continue;
                            
                            if(pathItem.fillColor[lowColor] >= 100) {
                                fail = true;
                                break;
                            }
                        }

                        if(fail) continue;

                        pathItem.fillColor.red = pathItem.fillColor.green = pathItem.fillColor.blue = 255;
                    }
                }
            }
        }
    }
}

// Add border to fills
for(var d = 0; d < targets.length; d++) {
    var doc = targets[d];

    var length = doc.layers.length;
    
    for(var i = 0; i < length; i++) {
        var layer = doc.layers[i];

        if(layer.name.indexOf(' Fill') === -1)
            continue;

        if(layer.groupItems.length > 0) {
            var group = layer.groupItems[0];

            for(var j = group.pathItems.length - 1; j >= 0; j--) {
                var pathItem = group.pathItems[j];

                pathItem.stroked = true;
                pathItem.strokeColor.red = pathItem.strokeColor.green = pathItem.strokeColor.blue = 255;
                pathItem.strokeWidth = 11;
            }
        }
    }
}*/