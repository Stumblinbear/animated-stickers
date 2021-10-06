// app.findMenuCommandId("Create Shapes from Vector Layer");
// app.executeCommand(0000);

var proj = app.project;

if(proj) {
    /*
    app.beginUndoGroup("Fix my fuckup");

    var targets = [];

    for(var i = 1; i <= app.project.numItems; i++) {
        var comp = app.project.item(i);

        if(comp != null && comp instanceof CompItem) {
            targets.push(comp);
        }
    }

    for(var k = 0; k < targets.length; k++) {
        var comp = targets[k];

        for(var i = 1; i <= comp.numLayers; i++) {
            var layer = comp.layer(i);

            if(layer.name.indexOf(' Fill Outlines') === -1 || layer.name.indexOf('Mask') !== -1) {
                continue;
            }

            for(var j = 1; j <= layer.content.numProperties; j++) {
                var group = layer.content.property(j);

                var fill = group.content.property('Fill 1');

                if(!fill) continue;

                fill.color.setValue([ 1, 1, 1 ]);
            }
        }
    }

    app.endUndoGroup();
    */

    var DESELECT_ALL = app.findMenuCommandId("Deselect All");
    var CREATE_SHAPES_FROM_VECTOR_LAYER = app.findMenuCommandId("Create Shapes from Vector Layer");

    if(app.project.activeItem == null || !(app.project.activeItem instanceof CompItem)) {
        alert("Please select an active comp to use this script", "Mask Layers");
    }else{
        var comp = app.project.activeItem;

/*        app.beginUndoGroup("Mask Layers");


        var mainComp = comp.duplicate();
        var fillComp = comp.duplicate();

        mainComp.layer('Full Mask').remove();

        mainComp.name = comp.name + ' Main';
        fillComp.name = comp.name + ' Fill';

        // Remove the fill layers
        for(var i = 1; i <= mainComp.numLayers; i++) {
            var layer = mainComp.layer(i);

            if(layer.name.indexOf(' Fill') !== -1) {
                layer.remove();
                i--;
            }
        }

        // Open the fill comp so the shapes command works correctly
        fillComp.openInViewer();

        // Remove the fill layers, and replace the original sources with the fill sources. This has the effect of
        // keeping animations, properties, etc in sync with the main, but makes it render the fill layers.
        for(var i = 1; i <= fillComp.numLayers; i++) {
            var layer = fillComp.layer(i);
            
            if(layer.name.indexOf(' Fill') === -1) {
                var originalFill = comp.layer(layer.name + ' Fill');

                if(!!originalFill) {
                    // Replace the source
                    layer.replaceSource(comp.layer(layer.name + ' Fill').source, false);

                    // Convert to shapes and delete the layer
                    layer.selected = true;
                }else{
                    layer.remove();
                    i--;
                }
            }else{
                layer.remove();
                i--;
            }
        }
        
        // After this, only the new shape layers will be marked as selected.
        app.executeCommand(CREATE_SHAPES_FROM_VECTOR_LAYER);
        
        // Fix the layer's parents
        for(var i = 1; i <= fillComp.numLayers; i++) {
            var layer = fillComp.layer(i);
            
            if(layer.selected) {
                if(!!layer.parent) {
                    layer.parent = fillComp.layer(layer.parent.name + ' Outlines');
                }
            }
        }
        
        // Remove any non-shape layers
        for(var i = 1; i <= fillComp.numLayers; i++) {
            var layer = fillComp.layer(i);
            
            if(!layer.selected) {
                layer.remove();
                i--;
            }else{
                layer.selected = false;
            }
        }
        
        // Duplicate the fill comp so we can make the border layers
        var borderComp = fillComp.duplicate();
        borderComp.name = comp.name + ' Border';

        // Since the fill and border comps are identical, we can do both with one pass.
        for(var i = 1; i < fillComp.numLayers; i++) {
            var fillLayer = fillComp.layer(i);
            var borderLayer = borderComp.layer(i);

            var borderColor = null;

            for(var j = 1; j <= fillLayer.content.numProperties; j++) {
                var fillGroup = fillLayer.content.property(j);
                var borderGroup = borderLayer.content.property(j);

                var borderStroke = borderGroup.content.property('Stroke 1');
                    
                if(!!borderStroke)
                    borderStroke.remove();

                var fillFill = fillGroup.content.property('Fill 1');
                var borderFill = borderGroup.content.property('Fill 1');

                if(!fillFill) continue;

                var color = fillFill.color.value;

                if(color[0] + color[1] + color[2] >= 3) {
                    borderFill.remove();
                    continue;
                }

                borderFill.color.setValue(fillFill.color.value);

                fillFill.color.setValue([ 1, 1, 1 ]);
            }
        }

        var finalComp = comp.duplicate();
        finalComp.name = comp.name + ' Final';

        // We duplicated the comp, now remove the layers there and add the main, border, and outline comps.
        finalComp.openInViewer();

        var mask = finalComp.layer('Full Mask');

        mask.moveToBeginning();

        while(finalComp.numLayers > 1) {
            finalComp.layer(2).remove();
        }

        var fillCompLayer = finalComp.layers.add(fillComp);
        var fillCompMask = mask.duplicate();
        fillCompMask.moveBefore(fillCompLayer);
        fillCompMask.name = fillCompLayer.name + ' Mask';
        fillCompMask.scale.setValue([ 104, 104 ]);
        fillCompLayer.trackMatteType = TrackMatteType.ALPHA_INVERTED;

        var borderCompLayer = finalComp.layers.add(borderComp);
        var borderCompMask = mask.duplicate();
        borderCompMask.moveBefore(borderCompLayer);
        borderCompMask.name = borderCompLayer.name + ' Mask';
        borderCompMask.scale.setValue([ 102, 102 ]);
        borderCompLayer.trackMatteType = TrackMatteType.ALPHA_INVERTED;

        var mainCompLayer = finalComp.layers.add(mainComp);
        var mainCompMask = mask.duplicate();
        mainCompMask.moveBefore(mainCompLayer);
        mainCompMask.name = mainCompLayer.name + ' Mask';
        mainCompMask.scale.setValue([ 100, 100 ]);
        mainCompLayer.trackMatteType = TrackMatteType.ALPHA_INVERTED;

        mask.remove();

        app.endUndoGroup();
        
*/

        comp.openInViewer();

        var mask = null;
        var targets = [];

        for(var i = 0; i <= comp.selectedLayers.length; i++) {
            var layer = comp.selectedLayers[i];

            if(mask == null) {
                mask = layer;

                if(mask.name == 'Full Mask')
                    break;
            }else if(!!layer) {
                targets.push(layer);
            }
        }
        
        var isFullMask = (mask.name == 'Full Mask');
        
        for(var i = 1; i <= comp.numLayers; i++) {
            var layer = comp.layer(i);

            if(layer.name.indexOf(' Fill') !== -1) {
                break;
            }
            
            if(isFullMask) {
                targets.push(layer);
            }
        }

        function getFirstFillLayer() {
            for(var i = 1; i <= comp.numLayers; i++) {
                var layer = comp.layer(i);
    
                if(layer.name.indexOf(' Fill') !== -1) {
                    return layer;
                }
            }
        }

        if(mask == null || targets.length == 0) {
            alert("Must select at least two layers.", "Mask Layers");
        }else{
            // app.beginUndoGroup("Mask Layers");
        
            // Hide the original mask object
            mask.enabled = false;

            // Deselect all layers
            app.executeCommand(DESELECT_ALL);

            // Select the mask and create the shape
            mask.selected = true;
            mask.name = mask.name;
            app.executeCommand(DESELECT_ALL);

            for(var i = 0; i < targets.length; i++) {
                var layer = targets[i];

                var fillLayer = comp.layer(layer.name + ' Fill');
                
                // If there is no corresponding fill layer, dont bother continuing.
                if(!fillLayer) {
                    continue;
                }

                var borderColor = null;

                {
                    // Shape-ify the fill layer.
                    fillLayer.selected = true;
                    app.executeCommand(CREATE_SHAPES_FROM_VECTOR_LAYER);
                    fillLayer.selected = false;
                    
                    var fillLayerShape = comp.selectedLayers[0];
                    fillLayerShape.selected = false;
                    
                    // app.executeCommand(DESELECT_ALL);

                    // Loop through the fill layer's groups. If the fill color is
                    // pure white, there's no need to mask out anything.
                    for(var j = 1; j <= fillLayerShape.content.numProperties; j++) {
                        var group = fillLayerShape.content.property(j);

                        var fill = group.content.property('Fill 1');

                        if(!fill) continue;

                        var color = fill.color.value;

                        if(color[0] + color[1] + color[2] >= 3) {
                            continue;
                        }

                        borderColor = color;

                        fill.color.setValue([ 1, 1, 1 ]);
                    }
                
                    if(borderColor == null) {
                        fillLayer.enabled = true;
                        fillLayerShape.remove();
                        continue;
                    }

                    // Remove the original fill layer
                    fillLayer.remove();

                    fillLayer = fillLayerShape;
                    fillLayer.shy = true;
                }

                // Copy the mask shape above the layer
                layer.selected = true;
                mask.copyToComp(comp);
                layer.selected = false;
                    
                var layerMask = comp.layer(layer.index - 1);
                layerMask.name = layer.name + ' Mask';
                layerMask.scale.setValue([ 100, 100 ]);
                layerMask.shy = true;

                layer.trackMatteType = TrackMatteType.ALPHA_INVERTED;
                
                app.executeCommand(DESELECT_ALL);

                {
                    // Copy the fill mask shape above the layer
                    fillLayer.selected = true;
                    mask.copyToComp(comp);
                    fillLayer.selected = false;
                    
                    var fillLayerMask = comp.layer(fillLayer.index - 1);
                    fillLayerMask.name = fillLayer.name + ' Mask';
                    fillLayerMask.scale.setValue([ 104, 104 ]);
                    fillLayerMask.shy = true;

                    fillLayer.trackMatteType = TrackMatteType.ALPHA_INVERTED;
                }

                {
                    var firstFillLayer = getFirstFillLayer();

                    firstFillLayer.selected = true;
                    fillLayer.copyToComp(comp);
                    firstFillLayer.selected = false;
                    
                    var borderLayer = comp.layer(firstFillLayer.index - 1);
                    borderLayer.name = layer.name + ' Border';
                    borderLayer.parent = layer;
                    borderLayer.shy = true;
                    borderLayer.selected = false;
                    
                    borderLayer.selected = true;
                    mask.copyToComp(comp);
                    borderLayer.selected = false;
                    
                    var borderLayerMask = comp.layer(borderLayer.index - 1);
                    borderLayerMask.name = layer.name + ' Border Mask';
                    borderLayerMask.scale.setValue([ 102, 102 ]);
                    borderLayerMask.shy = true;
                    borderLayerMask.selected = false;
                    
                    // Loop through the content groups
                    for(var j = 1; j <= borderLayer.content.numProperties; j++) {
                        var group = borderLayer.content.property(j);

                        var stroke = group.content.property('Stroke 1');
                        var fill = group.content.property('Fill 1');

                        if(!fill) {
                            group.remove();
                            
                            j--;
                        }else{
                            fill.color.setValue(borderColor);
                            
                            if(!!stroke)
                                stroke.remove();
                        }
                    }
                }
                
                app.executeCommand(DESELECT_ALL);
            }

            mask.remove();

            // app.endUndoGroup();
        }
    }
}