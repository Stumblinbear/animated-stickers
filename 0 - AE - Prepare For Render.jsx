/// <reference types="types-for-adobe/AfterEffects/18.0"/>

// app.findMenuCommandId("Create Shapes from Vector Layer");
// app.executeCommand(0000);

var proj = app.project;

if(proj) {
    var DESELECT_ALL = app.findMenuCommandId("Deselect All");
    var CREATE_SHAPES_FROM_VECTOR_LAYER = app.findMenuCommandId("Create Shapes from Vector Layer");
    var CONVERT_EXPRESSION_TO_KEYFRAMES = app.findMenuCommandId("Convert Expression to Keyframes");

    var properties = ["anchorPoint", "position", "rotation", "scale", "opacity"];

    /**
     * @type {[CompItem]} */
    var targets = [];
    var allTargets = false;

    if(confirm('Apply to all compositions?')) {
        allTargets = true;

        for(var i = 1; i <= app.project.numItems; i++) {
            var comp = app.project.item(i);
    
            if(comp != null && comp instanceof CompItem) {
                targets.push(comp);
            }
        }
    }else{
        if(app.project.activeItem != null && app.project.activeItem instanceof CompItem) {
            targets = [ app.project.activeItem ];
        }else {
            alert("Please select an active comp to use this script", "Parent Fill Layers");
        }
    }

    if(targets.length > 0) {
        app.beginUndoGroup("Prepare for Render");
    
        for(var d = 0; d < targets.length; d++) {
            var comp = targets[d];

            comp.openInViewer();

            app.executeCommand(DESELECT_ALL);

            /**
             * @param {PropertyGroup} group 
             */
            function selectExpressionProps(group){
                for(var k = 1; k <= group.numProperties; k++) {
                    var prop = group.property(k);
                    if (prop instanceof PropertyGroup || prop instanceof MaskPropertyGroup){
                        selectExpressionProps(prop);
                    } else if (prop.canSetExpression && prop.expressionEnabled) {
                        prop.selected = true;
                    }
                }
            }

            // Convert all expressions to keyframes
            for(var j = 1; j <= comp.numLayers; j++) {
                var layer = comp.layer(j);

                var isMask = layer.name.indexOf(' Mask') !== -1;

                if(!isMask && !layer.enabled) continue;

                selectExpressionProps(layer);
            }

            app.executeCommand(CONVERT_EXPRESSION_TO_KEYFRAMES);

            // Set the correct framerate
            comp.frameRate = (comp.frameRate != 30 && comp.frameRate != 60 ? 30 : comp.frameRate);

            app.executeCommand(DESELECT_ALL);

            var layerData = [];

            // Copy keyframes from parented Duik structure props
            for(var j = 1; j <= comp.numLayers; j++) {
                var layer = comp.layer(j);

                if(!layer.parent) continue;

                if((layer.name.indexOf('S | ') !== -1 || layer.parent.name.indexOf('S | ') === -1) && layer.parent.name.indexOf('C | ') === -1) continue;

                var basePosition = layer.transform.position.value; // Dirty hack
                var baseRotation = layer.transform.rotation.value; // Dirty hack
                
                var duikStruct = layer.parent;

                var layerDatum = { layer: layer, position: [], rotation: [], scale: [] };

                var applyPosition = false;

                if(!!layer.parent.parent) {
                    if(layer.parent.parent.name.indexOf('S | ') !== -1)
                        layer.parent = comp.layer(layer.parent.parent.name.split('S | ')[1]);
                    else if(layer.parent.parent.name.indexOf('C | ') !== -1)
                        layer.parent = comp.layer(layer.parent.parent.name.split('C | ')[1]);
                    else{
                        layer.parent = comp.layer(layer.parent.parent.name);

                        applyPosition = true;
                    }
                }else{
                    layer.parent = null;

                    applyPosition = true;
                }

                if(applyPosition) {
                    var prop = layer.transform.position;
                    var parentProp = duikStruct.transform.position;

                    for(var l = 1; l <= parentProp.numKeys; l++) {
                        layerDatum.position.push([ parentProp.keyTime(l), parentProp.keyValue(l) ]);
                    }

                    var flatten = true;
                    for(var k = 1; k < layerDatum.position.length; k++) {
                        if(layerDatum.position[k][1][0] != layerDatum.position[k - 1][1][0] || layerDatum.position[k][1][1] != layerDatum.position[k - 1][1][1]) {
                            flatten = false;
                            break;
                        }
                    }

                    if(flatten) {
                        layerDatum.position.splice(1, layerDatum.position.length - 1);
                    }
                }

                {
                    var prop = layer.transform.rotation;
                    var parentProp = duikStruct.transform.rotation;

                    for(var l = 1; l <= parentProp.numKeys; l++) {
                        var val = parentProp.keyValue(l);

                        if(layerDatum.rotation.length > 0) {
                            var val2 = layerDatum.rotation[layerDatum.rotation.length - 1];

                            if(Math.sqrt(Math.pow(val, 2) + Math.pow(val2, 2)) > 180) {
                                if(val2 > val) {
                                    val -= 360;
                                }else{
                                    val += 360;
                                }
                            }
                        }

                        layerDatum.rotation.push([ parentProp.keyTime(l), val ]);
                    }

                    var flatten = true;
                    for(var k = 1; k < layerDatum.rotation.length; k++) {
                        if(layerDatum.rotation[k][1] != layerDatum.rotation[k - 1][1]) {
                            flatten = false;
                            break;
                        }
                    }

                    if(flatten) {
                        layerDatum.rotation.splice(1, layerDatum.rotation.length - 1);
                    }
                }

                {
                    var prop = layer.transform.scale;
                    var parentProp = duikStruct.transform.scale;

                    for(var l = 1; l <= parentProp.numKeys; l++) {
                        layerDatum.scale.push([ parentProp.keyTime(l), parentProp.keyValue(l) ]);
                    }

                    var flatten = true;
                    for(var k = 1; k < layerDatum.scale.length; k++) {
                        if(layerDatum.scale[k][1][0] != layerDatum.scale[k - 1][1][0] || layerDatum.scale[k][1][1] != layerDatum.scale[k - 1][1][1]) {
                            flatten = false;
                            break;
                        }
                    }
                    
                    if(flatten) {
                        layerDatum.scale.splice(1, layerDatum.scale.length - 1);
                    }
                }

                layerData.push(layerDatum);
            }

            for(var j = 0; j < layerData.length; j++) {
                var layerDatum = layerData[j];

                var layer = layerDatum.layer;
                var positions = layerDatum.position;
                var rotations = layerDatum.rotation;
                var scales = layerDatum.scale;

                for(var l = 0; l < positions.length; l++) {
                    layer.transform.position.setValueAtTime(positions[l][0], positions[l][1]);
                }

                for(var l = 0; l < rotations.length; l++) {
                    layer.transform.rotation.setValueAtTime(rotations[l][0], rotations[l][1]);
                }

                for(var l = 0; l < scales.length; l++) {
                    layer.transform.scale.setValueAtTime(scales[l][0], scales[l][1]);
                }
            }
            /**
             * TODO: Is this actually necessary? Are keyframe indexes already sorted on time?
             * @param {Property} prop
             * @returns {[number]}
             */
             function getSortedKeyframeIndexes(prop){
                var keyFrameMap = [];
                if (prop.numKeys == 0){
                    return [];
                } else if (prop.numKeys == 1){
                    return [1];
                }
                // Their array, 1-indexed...
                for (var i = 1; i <= prop.numKeys; i++){
                    keyFrameMap.push({
                        keyIndex: i,
                        time: prop.keyTime(i)
                    });
                }
                keyFrameMap.sort(function (a, b){
                    return a.time - b.time;
                });
                var sortedKeys = [];
                for (var i = 0; i < keyFrameMap.length; i++){
                    sortedKeys.push(a.keyIndex);
                }
                return sortedKeys;
            }
            /**
             * @param {PropertyGroup} group 
             */
            function removeDuplicateKeyframes(group){
                // Their array, 1-indexed...
                for(var k = 1; k <= group.numProperties; k++) {
                    var prop = group.property(k);
                    if (prop instanceof PropertyGroup || prop instanceof MaskPropertyGroup){
                        removeDuplicateKeyframes(prop);
                        continue;
                    }
                    // skip if no or only one keyframe 
                    if(prop.numKeys <= 1) {
                        continue;
                    }
                    // If these keyframes weren't generated by "Convert Expression to KeyFrame",
                    // skip it.
                    if (!prop.canSetExpression || !prop.expression){
                        continue;
                    }
                    var sortedKeyIndexes = getSortedKeyframeIndexes(prop);
                    var lastKeyFrame = null;
                    var lastKeyIndex = null;
                    var keyIndexesToDelete = [];
                    var keysSinceDeletion = 0;
                    // Our own array, 0-indexed...
                    for (var n = 0; n < sortedKeyIndexes.length; n++){
                        var keyIndex = sortedKeyIndexes[n];
                        var keyFrame = prop.keyValue(keyIndex);
                        if (lastKeyFrame == null){
                            lastKeyFrame = keyFrame;
                            lastKeyIndex = keyIndex;
                        // found a duplicate keyframe, delete after this...
                        } else if (lastKeyFrame.toString() == keyFrame.toString()){
                            keyIndexesToDelete.push(keyIndex);
                            keysSinceDeletion++;
                        // non duplicate keyframe
                        } else {
                            // Setting HOLD prevents interpolation between the deleted frames
                            // Don't do it if these are sequential keyframes.
                            if (keysSinceDeletion > 0){
                                // TODO: Should we instead bring back the last deleted key and do it on that?
                                // Difference would only be noticible on low framerates.
                                // lastKeyIndex = keyIndexesToDelete.pop();
                                var lastKeyInType = prop.keyInInterpolationType(lastKeyIndex);
                                var thisKeyOutType = prop.keyOutInterpolationType(keyIndex);
    
                                prop.setInterpolationTypeAtKey(lastKeyIndex, lastKeyInType, KeyframeInterpolationType.HOLD);
                                prop.setInterpolationTypeAtKey(keyIndex, KeyframeInterpolationType.HOLD, thisKeyOutType);
                            }
                            keysSinceDeletion = 0;
                            lastKeyFrame = keyFrame;
                            lastKeyIndex = keyIndex;
                        }
                    }
                    // Have to delete highest index first, because key indexes change
                    // upon deleting.
                    keyIndexesToDelete = keyIndexesToDelete.sort(function(a, b){
                        return b - a;
                    });
                    for (var n = 0; n < keyIndexesToDelete.length; n++){
                        prop.removeKey(keyIndexesToDelete[n]);
                    }
                }
            }

            // go through all properties in layer, remove duplicate keyframes
            for(var j = 1; j <= comp.numLayers; j++) {
                var layer = comp.layer(j);
                
                var isMask = layer.name.indexOf(' Mask') !== -1;
                if(!isMask && !layer.enabled) continue;

                removeDuplicateKeyframes(layer);
            }

            for(var j = 1; j <= comp.numLayers; j++) {
                var layer = comp.layer(j);

                if(layer.name.indexOf('S | ') !== -1 || layer.name.indexOf('C | ') !== -1) {
                    layer.remove();
                    j--;
                }
            }

            // If it has a layer called "full mask" then we need to do some fancy shit to make the border hella swick
            if(!!comp.layer('Full Mask') && comp.layer('Full Mask').enabled) {
                var mainComp = comp.duplicate();
                mainComp.name = comp.name + ' Main';

                mainComp.openInViewer();

                {
                    var fullMaskIndex = mainComp.layer('Full Mask').index;

                    // Remove the layers above the full mask from the main comp
                    for(var i = 1; i <= mainComp.numLayers; i++) {
                        var layer = mainComp.layer(i);

                        if(i < fullMaskIndex) {
                            fullMaskIndex--;
                            layer.remove();
                            i--;
                        }
                    }

                    mainComp.layer('Full Mask').remove();
                }

                var fillComp = mainComp.duplicate();
                fillComp.name = comp.name + ' Border Fill';

                // Open the fill comp so the shapes command works correctly
                fillComp.openInViewer();

                // Replace the original sources of the layers with the fill sources. This has the effect of keeping
                // animations, properties, etc in sync with the main, but makes it render the fill layers instead.
                for(var i = 1; i <= fillComp.numLayers; i++) {
                    var layer = fillComp.layer(i);

                    if(layer.name.indexOf(' Fill') === -1) {
                        var originalFill = comp.layer(layer.name + ' Fill');

                        if(!!originalFill && !!layer.source) {
                            layer.name += ' Fill';

                            // Replace the source
                            layer.replaceSource(originalFill.source, false);

                            // Convert to shapes and delete the layer
                            layer.selected = true;
                        }else if(!layer.nullLayer) {
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
                    
                    if(!layer.parent) continue;
                    
                    if(!layer.parent.nullLayer && !(layer.parent instanceof ShapeLayer))
                        layer.parent = fillComp.layer(layer.parent.name + ' Outlines');
                }
                
                app.executeCommand(DESELECT_ALL);
                
                // Remove any non-shape layers
                for(var i = 1; i <= fillComp.numLayers; i++) {
                    var layer = fillComp.layer(i);
                    
                    if(!layer.nullLayer && !(layer instanceof ShapeLayer || layer instanceof TextLayer)) {
                        layer.remove();
                        i--;
                    }
                }

                // Create a list of layers which attach to bordered layers
                var attachesToBorder = { };

                for(var i = 1; i <= fillComp.numLayers; i++) {
                    var fillLayer = fillComp.layer(i);

                    if(fillLayer.nullLayer) continue;

                    for(var j = 1; j <= fillLayer.content.numProperties; j++) {
                        var fillGroup = fillLayer.content.property(j);
                        var fillFill = fillGroup.content.property('Fill 1');

                        if(!fillFill) continue;

                        var color = fillFill.color.value;

                        if(color[0] + color[1] + color[2] >= 3)
                            continue;

                        var attach = fillLayer;

                        while(!!attach) {
                            attachesToBorder[attach.name] = true;

                            attach = attach.parent;
                        }
                    }
                }

                // Remove bordered layers from the main comp
                for(var i = 1; i <= mainComp.numLayers; i++) {
                    var layer = mainComp.layer(i);
                    
                    // The main comp has not been processed, yet, so add " Outlines" to the layer name
                    if(!layer.nullLayer && !!attachesToBorder[layer.name + ' Outlines']) {
                        layer.remove();
                        i--;
                    }
                }

                // Remove unbordered layers from the fill comp
                for(var i = 1; i <= fillComp.numLayers; i++) {
                    var layer = fillComp.layer(i);
                    
                    if(!layer.nullLayer && !attachesToBorder[layer.name]) {
                        layer.remove();
                        i--;
                    }
                }
                
                // Duplicate the fill comp so we can make the border layers
                var borderComp = fillComp.duplicate();
                borderComp.name = comp.name + ' Border';

                // Since the fill and border comps are identical, we can do both with one pass.
                for(var i = 1; i <= fillComp.numLayers; i++) {
                    var fillLayer = fillComp.layer(i);

                    if(fillLayer.nullLayer) continue;

                    var borderLayer = borderComp.layer(i);

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
                            borderGroup.content.property('Path 1').remove();
                            continue;
                        }

                        borderFill.color.setValue(fillFill.color.value);

                        fillFill.color.setValue([ 1, 1, 1 ]);
                    }
                }

                // Remove any unnecessary limbs
                var found;

                do {
                    found = false;

                    for(var i = 1; i <= borderComp.numLayers; i++) {
                        var borderLayer = borderComp.layer(i);

                        // If the layer is helping create a border
                        if(!!attachesToBorder[borderLayer.name]) continue;

                        // Delete the layer
                        borderLayer.remove();
                        i--;

                        found = true;
                    }
                } while(found);

                var finalComp = comp;

                // We duplicated the comp, now remove the layers there and add the main, border, and outline comps.
                finalComp.openInViewer();

                var mask = finalComp.layer('Full Mask');

                // We don't want the mask at the very top, because there may be some
                // objects we don't want to mask out ABOVE the full mask layer
                // mask.moveToBeginning();

                mask.selected = true;
                app.executeCommand(CREATE_SHAPES_FROM_VECTOR_LAYER);
                mask.remove();

                mask = finalComp.layer('Full Mask Outlines');
                mask.name = 'Full Mask';

                // Remove any layer below the mask layer
                while(finalComp.numLayers > mask.index) {
                    finalComp.layer(mask.index + 1).remove();
                }

                var mainCompLayer = finalComp.layers.add(mainComp);
                mainCompLayer.moveAfter(mask);

                var mainCompMask = mask.duplicate();
                mainCompMask.moveBefore(mainCompLayer);
                mainCompMask.name = mainCompLayer.name + ' Mask';
                mainCompMask.scale.setValue([ 100, 100 ]);
                mainCompLayer.trackMatteType = TrackMatteType.ALPHA_INVERTED;


                var borderCompLayer = finalComp.layers.add(borderComp);
                borderCompLayer.moveAfter(mainCompLayer);

                var borderCompMask = mask.duplicate();
                borderCompMask.moveBefore(borderCompLayer);
                borderCompMask.name = borderCompLayer.name + ' Mask';
                // If the scale hasn't been set, use a default scale. This lets us adjust the border size by setting the scale of the mask.
                if(borderCompMask.scale.value[0] == 100)
                    borderCompMask.scale.setValue([ 102, 102 ]);
                borderCompLayer.trackMatteType = TrackMatteType.ALPHA_INVERTED;


                var fillCompLayer = finalComp.layers.add(fillComp);
                fillCompLayer.moveAfter(borderCompLayer);

                var fillCompMask = mask.duplicate();
                fillCompMask.moveBefore(fillCompLayer);
                fillCompMask.name = fillCompLayer.name + ' Mask';
                fillCompMask.scale.setValue([ 104, 104 ]);
                fillCompLayer.trackMatteType = TrackMatteType.ALPHA_INVERTED;

                mask.remove();

                // Add the compositions to the beginning, as they're already fully processed, and skip them.
                targets.unshift(fillComp);
                targets.unshift(borderComp);
                d += 2;
                
                // Add the "main" comp to the targets list so it can be processed as normal
                targets.push(mainComp);
                
                comp.openInViewer();
            }

            var maskCount = 0;

            // Convert layers
            for(var j = 1; j <= comp.numLayers; j++) {
                var layer = comp.layer(j);

                if(layer.trackMatteType != TrackMatteType.NO_TRACK_MATTE) {
                    maskCount++;
                }

                var isMask = layer.name.indexOf(' Mask') !== -1;

                if(isMask) {
                    if(layer instanceof ShapeLayer) {
                        layer.enabled = false;

                        continue;
                    }
                }else if(!layer.enabled) {
                    continue;
                }

                if(layer.source instanceof CompItem) {
                    // Add sub comps to the targets list if we aren't already doing them all
                    if(!allTargets)
                        targets.push(layer.source);
                    continue;
                }
            
                layer.selected = true;
                
                app.executeCommand(CREATE_SHAPES_FROM_VECTOR_LAYER);
                
                layer.selected = false;

                var layerOutline = comp.layer(layer.name + ' Outlines');

                if(!!layerOutline) {
                    if(layerOutline.trackMatteType != layer.trackMatteType) {
                        layerOutline.trackMatteType = layer.trackMatteType;
                    }
                }

                if(isMask) {
                    layer.enabled = false;

                    layer.remove();

                    j--;
                }
            }

            if(maskCount >= 15) {
                alert('Telegram limits masks to 15 per sticker! "' + comp.name + '" has ' + maskCount);
            }

            app.executeCommand(DESELECT_ALL);

            // Fix parenting
            for(var j = 1; j <= comp.numLayers; j++) {
                var layer = comp.layer(j);

                if(!layer.parent) continue;

                var convParent = comp.layer(layer.parent.name + ' Outlines');

                if(!convParent) continue;

                layer.parent = convParent;
            }

            // Remove unnecessary layers
            for(var j = 1; j <= comp.numLayers; j++) {
                var layer = comp.layer(j);

                if(layer.name.indexOf(' Outlines') !== -1) continue;

                if(layer instanceof ShapeLayer) continue;
                
                if(layer.source instanceof CompItem) continue;
                
                if(layer.nullLayer) continue;

                layer.remove();

                j--;
            }
        }

    
        // Obfuscation, baby. Randomize all layer names. >:3
        if(confirm('Obfuscate?')) {
            var c = 0;
            
            for(var d = 0; d < targets.length; d++) {
                var comp = targets[d];
    
                for(var j = 1; j <= comp.numLayers; j++) {
                    var layer = comp.layer(j);
    
                    layer.name = c.toString(16);
    
                    c++;
                }
            }
        }

        app.endUndoGroup();
    }
    
    app.executeCommand(app.findMenuCommandId("Bodymovin for Telegram Stickers"));
}
