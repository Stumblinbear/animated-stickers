/// <reference types="types-for-adobe/AfterEffects/18.0"/>

/**
 * @param {Layer} layer
 * @returns {boolean}
 */
function isDuikBone(layer) {
    return layer.name.indexOf('B < ') !== -1;
}

/**
 * @param {Layer} layer
 * @returns {boolean}
 */
function isDuikIK(layer) {
    return layer.name.indexOf('IK < ') !== -1;
}

/**
 * @param {Layer} layer
 * @returns {boolean}
 */
function isDuikLayer(layer) {
    return isDuikIK(layer) || layer.name.indexOf('C < ') !== -1 || layer.name.indexOf('N < ') !== -1 || isDuikBone(layer);
}

/**
 * @param {Layer} layer
 * @param {PropertyGroup} group
 * @returns {boolean}
 */
function selectExpressionProps(layer, group) {
    var didSelectAtLeastOne = false;

    for (var k = 1; k <= group.numProperties; k++) {
        var prop = group.property(k);

        if (prop instanceof PropertyGroup || prop instanceof MaskPropertyGroup) {
            if (selectExpressionProps(layer, prop)) {
                didSelectAtLeastOne = true;
            }
        } else if (prop.canSetExpression && prop.expressionEnabled) {
            prop.selected = true;

            didSelectAtLeastOne = true;
        }
    }

    return didSelectAtLeastOne;
}

/**
 * Copies the significant frames of the expression to the target property. This attempts to create the fewest number of
 * keyframes possible.
 *
 * @param {Property} fromProp
 * @param {Property} toProp
 * @param {number} startFrame
 * @param {number} endFrame
 * @param {number} step
 * @param {number} threshold
 */
function copyExpression(fromProp, toProp, startFrame, endFrame, step, threshold) {
    if (startFrame == endFrame || startFrame > endFrame || endFrame - startFrame < step) {
        return;
    }

    // Create a keyframe at the startFrame if it doesn't exist
    if (toProp.numKeys == 0 || toProp.keyTime(toProp.nearestKeyIndex(startFrame)) != startFrame) {
        toProp.setValueAtTime(startFrame, fromProp.valueAtTime(startFrame, false));
    }

    // Create a keyframe at the endFrame if it doesn't exist
    if (toProp.numKeys == 0 || toProp.keyTime(toProp.nearestKeyIndex(endFrame)) != endFrame) {
        toProp.setValueAtTime(endFrame, fromProp.valueAtTime(endFrame, false));
    }

    // Check each frame between startFrame and endFrame to see if the value strays from the expression
    for (var keyTime = startFrame; keyTime <= endFrame; keyTime += step) {
        var fromValue = fromProp.valueAtTime(keyTime, false);
        var toValue = toProp.valueAtTime(keyTime, false);

        // If the value is significantly different from the expression, we create a keyframe at the halfway point
        // and do a copyExpression on both halves.
        if (Math.abs(fromValue - toValue) > threshold) {
            var midFrame = startFrame + (endFrame - startFrame) / 2;

            // Make sure the midFrame is on a step boundary
            midFrame = Math.round(midFrame / step) * step;

            toProp.setValueAtTime(midFrame, fromProp.valueAtTime(midFrame, false));

            copyExpression(fromProp, toProp, startFrame, midFrame, step, threshold);
            copyExpression(fromProp, toProp, midFrame, endFrame, step, threshold);

            break;
        }
    }
}

/**
 * @param {CompItem} comp
 * @param {Property} fromProp
 * @param {Property} toProp
 * @param {number} threshold
 */
function copyKeyframes(comp, fromProp, toProp, threshold) {
    // If it's set by an expression, sample the value for each frame in the work area
    if (fromProp.canSetExpression && fromProp.expressionEnabled) {
        copyExpression(fromProp, toProp, comp.workAreaStart, comp.workAreaStart + comp.workAreaDuration, comp.frameDuration, threshold);
    } else {
        // Otherwise, we can copy the keyframes
        for (var l = 1; l <= fromProp.numKeys; l++) {
            var keyId = toProp.addKey(fromProp.keyTime(l));

            toProp.setValueAtKey(keyId, fromProp.keyValue(l));
            toProp.setInterpolationTypeAtKey(keyId, fromProp.keyInInterpolationType(l), fromProp.keyOutInterpolationType(l));

            var inTemporalEase = fromProp.keyInTemporalEase(l);
            var outTemporalEase = fromProp.keyOutTemporalEase(l);

            if (inTemporalEase.length == 1 && outTemporalEase.length == 1) {
                toProp.setTemporalEaseAtKey(keyId, inTemporalEase, outTemporalEase);
            } else if (inTemporalEase.length == 2 && outTemporalEase.length == 2) {
                toProp.setTemporalEaseAtKey(keyId, inTemporalEase, outTemporalEase);
            } else if (inTemporalEase.length == 3 && outTemporalEase.length == 3) {
                toProp.setTemporalEaseAtKey(keyId, inTemporalEase, outTemporalEase);
            } else {
                throw new Error('Temporal ease length mismatch');
            }
        }
    }
}

/**
 * @param {Property} prop
 * @returns {number[]}
 */
function getSortedKeyframeIndexes(prop) {
    /**
     * @type {{ keyIndex: number, time: number }[]}
     */
    var keyFrameMap = [];

    if (prop.numKeys == 0) {
        return [];
    } else if (prop.numKeys == 1) {
        return [1];
    }

    // Their array, 1-indexed...
    for (var i = 1; i <= prop.numKeys; i++) {
        keyFrameMap.push({
            keyIndex: i,
            time: prop.keyTime(i)
        });
    }

    keyFrameMap.sort(function (a, b) {
        return a.time - b.time;
    });

    /**
     * @type {number[]}
     */
    var sortedKeys = [];

    for (var i = 0; i < keyFrameMap.length; i++) {
        sortedKeys.push(keyFrameMap[i].keyIndex);
    }

    return sortedKeys;
}

/**
 * @param {CompItem} comp
 * @param {Layer} layer
 * @param {PropertyGroup} group
 */
function removeUnnecessaryKeyframes(comp, layer, group) {
    for (var k = 1; k <= group.numProperties; k++) {
        var prop = group.property(k);

        if (prop instanceof PropertyGroup || prop instanceof MaskPropertyGroup) {
            // Duik has some properties we will never care about. Skip them.
            if (isDuikLayer(layer)) {
                if (prop.name == 'Contents' || prop.name == 'Effects') {
                    continue;
                }
            }

            removeUnnecessaryKeyframes(comp, layer, prop);

            continue;
        }

        var sortedKeyIndexes = getSortedKeyframeIndexes(prop);
        var keyIndexesToDelete = [];

        // If there's only one keyframe, it shouldn't be necessary to keep.
        if (sortedKeyIndexes.length == 1) {
            prop.removeKey(sortedKeyIndexes[0]);

            continue;
        }

        // Find the first frame that is within the work area
        for (var n = 0; n < sortedKeyIndexes.length; n++) {
            var keyIndex = sortedKeyIndexes[n];

            var keyTime = prop.keyTime(keyIndex);

            if (keyTime < comp.workAreaStart) {
                continue;
            }

            // Remove all frames before this one
            for (var l = 0; l < n - 1; l++) {
                keyIndexesToDelete.push(sortedKeyIndexes[l]);
            }

            break;
        }

        var workAreaEnd = comp.workAreaStart + comp.workAreaDuration;

        // Find the last frame that is within the work area
        for (var n = 0; n < sortedKeyIndexes.length; n++) {
            var keyIndex = sortedKeyIndexes[n];

            var keyTime = prop.keyTime(keyIndex);

            if (keyTime < workAreaEnd) {
                continue;
            }

            // Remove all frames after this one
            for (var l = n + 1; l < sortedKeyIndexes.length; l++) {
                keyIndexesToDelete.push(sortedKeyIndexes[l]);
            }

            break;
        }

        // Have to delete highest index first, because key indexes change
        // upon deleting.
        keyIndexesToDelete = keyIndexesToDelete.sort(function (a, b) {
            return b - a;
        });

        for (var n = 0; n < keyIndexesToDelete.length; n++) {
            prop.removeKey(keyIndexesToDelete[n]);
        }
    }
}

/**
 * @param {PropertyGroup} group
 */
function removeDuplicateKeyframes(layer, group) {
    // Their array, 1-indexed...
    for (var k = 1; k <= group.numProperties; k++) {
        var prop = group.property(k);

        if (prop instanceof PropertyGroup || prop instanceof MaskPropertyGroup) {
            // Duik has some properties we will never care about. Skip them.
            if (isDuikLayer(layer)) {
                if (prop.name == 'Contents' || prop.name == 'Effects') {
                    continue;
                }
            }

            removeDuplicateKeyframes(layer, prop);

            continue;
        }

        // Skip if no or only one keyframe
        if (prop.numKeys <= 1) {
            continue;
        }

        // If these keyframes weren't generated by "Convert Expression to KeyFrame",
        // skip it.
        if (!prop.canSetExpression || !prop.expression) {
            continue;
        }

        var sortedKeyIndexes = getSortedKeyframeIndexes(prop);
        /**
         * @type {{frame: number, index: number} | null}
         */
        var lastKeyFrame = null;
        var keyIndexesToDelete = [];
        var keysSinceDeletion = 0;

        // Our own array, 0-indexed...
        for (var n = 0; n < sortedKeyIndexes.length; n++) {
            var keyIndex = sortedKeyIndexes[n];
            var keyFrame = prop.keyValue(keyIndex);

            if (lastKeyFrame == null) {
                lastKeyFrame = { frame: keyFrame, index: keyIndex };
                // found a duplicate keyframe, delete after this...
            } else if (lastKeyFrame.frame.toString() == keyFrame.toString()) {
                keyIndexesToDelete.push(keyIndex);
                keysSinceDeletion++;
                // non duplicate keyframe
            } else {
                // Setting HOLD prevents interpolation between the deleted frames
                // Don't do it if these are sequential keyframes.
                if (keysSinceDeletion > 0) {
                    // TODO: Should we instead bring back the last deleted key and do it on that?
                    // Difference would only be noticible on low framerates.
                    // lastKeyIndex = keyIndexesToDelete.pop();
                    var lastKeyInType = prop.keyInInterpolationType(lastKeyFrame.index);
                    var thisKeyOutType = prop.keyOutInterpolationType(keyIndex);

                    prop.setInterpolationTypeAtKey(lastKeyFrame.index, lastKeyInType, KeyframeInterpolationType.HOLD);
                    prop.setInterpolationTypeAtKey(keyIndex, KeyframeInterpolationType.HOLD, thisKeyOutType);
                }

                keysSinceDeletion = 0;
                lastKeyFrame = { frame: keyFrame, index: keyIndex }
            }
        }

        // Have to delete highest index first, because key indexes change
        // upon deleting.
        keyIndexesToDelete = keyIndexesToDelete.sort(function (a, b) {
            return b - a;
        });

        for (var n = 0; n < keyIndexesToDelete.length; n++) {
            prop.removeKey(keyIndexesToDelete[n]);
        }
    }
}

/**
 *
 * @param {PropertyGroup | Property} item
 */
function obfuscate(index, item) {
    if((item instanceof AVLayer || item instanceof ShapeLayer) || (item.parentProperty != null && item.parentProperty.propertyType == PropertyType.INDEXED_GROUP)) {
        item.name = index.toString(26 + 10);
    }

    if(item instanceof AVLayer || item instanceof ShapeLayer || item instanceof PropertyGroup) {
        // Set the name of all of its properties to an obfuscated name
        for (var k = 1; k <= item.numProperties; k++) {
            var prop = item.property(k);

            obfuscate(k - 1, prop);
        }
    }
}

var proj = app.project;

if (proj) {
    var DESELECT_ALL = app.findMenuCommandId("Deselect All");
    var UNLOCK_ALL_LAYERS = app.findMenuCommandId("Unlock All Layers");
    var CREATE_SHAPES_FROM_VECTOR_LAYER = app.findMenuCommandId("Create Shapes from Vector Layer");
    var CONVERT_EXPRESSION_TO_KEYFRAMES = app.findMenuCommandId("Convert Expression to Keyframes");

    /**
     * @type {CompItem[]} */
    var targets = [];
    var allTargets = false;

    if (confirm('Apply to all compositions?')) {
        allTargets = true;

        for (var i = 1; i <= app.project.numItems; i++) {
            var item = app.project.item(i);

            if (item != null && item instanceof CompItem) {
                targets.push(item);
            }
        }
    } else {
        if (app.project.activeItem != null && app.project.activeItem instanceof CompItem) {
            targets = [app.project.activeItem];
        } else {
            alert("Please select an active comp to use this script", "Parent Fill Layers");
        }
    }

    if (targets.length > 0) {
        app.beginUndoGroup("Prepare for Render");

        for (var d = 0; d < targets.length; d++) {
            var comp = targets[d];

            comp.openInViewer();

            app.executeCommand(DESELECT_ALL);

            // Make sure all Duik layers are unlocked and enabled
            for (var j = 1; j <= comp.numLayers; j++) {
                var layer = comp.layer(j);

                if (isDuikLayer(layer)) {
                    layer.locked = false;
                    layer.enabled = true;
                }
            }

            {
                var needsKeyframeCleanup = [];

                // Convert all expressions to keyframes
                for (var j = 1; j <= comp.numLayers; j++) {
                    var layer = comp.layer(j);

                    var isMask = layer.name.indexOf(' Mask') !== -1;

                    if (!isMask && !layer.enabled) continue;

                    if (isDuikLayer(layer)) continue;

                    if (selectExpressionProps(layer, layer)) {
                        needsKeyframeCleanup.push(layer);
                    }
                }

                app.executeCommand(CONVERT_EXPRESSION_TO_KEYFRAMES);

                // Go through all properties in all layers to remove frames outside of the work area, as well as duplicate keyframes
                for (var j = 0; j < needsKeyframeCleanup.length; j++) {
                    var layer = needsKeyframeCleanup[j];

                    removeDuplicateKeyframes(layer, layer);
                }
            }

            // Set the correct framerate
            comp.frameRate = (comp.frameRate != 30 && comp.frameRate != 60 ? 30 : comp.frameRate);

            app.executeCommand(DESELECT_ALL);

            /**
             * Tracks which layers connect to which duik layers. This lets us re-parent our actual layers
             * back to our other actual layers after converting the Duik transformations to keyframes,
             * letting us remove all Duik structures.
             *
             * @type {{ [key: string]: string }}
             */
            var nonDuikToDuik = {};

            /**
             * @type {{ [key: string]: string }}
             */
            var duikToNonDuik = {};

            /**
             * @type {string[]}
             */
            var parentsToAdjust = [];

            // Build up a mapping between Duik and non Duik layers
            for (var j = 1; j <= comp.numLayers; j++) {
                var layer = comp.layer(j);

                if (isDuikLayer(layer)) continue;

                if (!layer.parent) continue;

                var layerParent = layer.parent;

                // If the parent isn't a duik layer, skip it
                if (!isDuikLayer(layerParent)) continue;

                nonDuikToDuik[layer.name] = layerParent.name;
                duikToNonDuik[layerParent.name] = layer.name;
                parentsToAdjust.push(layer.name);
            }

            {
                var didChangeAtLeastOne = true;

                while (didChangeAtLeastOne) {
                    didChangeAtLeastOne = false;

                    for (var j = 0; j < parentsToAdjust.length; j++) {
                        var nonDuikLayerName = parentsToAdjust[j];
                        var duikLayerName = nonDuikToDuik[nonDuikLayerName];

                        var nonDuikLayer = comp.layer(nonDuikLayerName);
                        var duikLayer = comp.layer(duikLayerName);

                        /**
                         * @type {Layer | null | undefined}
                         */
                        var newParentLayer = undefined;

                        // If the Duik layer is not parented to another Duik layer, we can take the parent directly
                        if (!duikLayer.parent || !isDuikLayer(duikLayer.parent)) {
                            newParentLayer = duikLayer.parent;
                        } else {
                            // // Grab the non-Duik layer mapping and attach to that layer instead, if possible
                            var nonDuikAttachmentLayerName = duikToNonDuik[duikLayer.parent.name];

                            if (!nonDuikAttachmentLayerName) {
                                // If there's no valid parent (this can happen in the case of feet where rotation is not inherited), null the parent
                                newParentLayer = null;
                            } else {
                                var nonDuikAttachmentLayer = comp.layer(nonDuikAttachmentLayerName);

                                // If the layer is not parented to a Duik layer, we can take that
                                if (!nonDuikAttachmentLayer.parent || !isDuikLayer(nonDuikAttachmentLayer.parent)) {
                                    newParentLayer = nonDuikAttachmentLayer;
                                }
                            }
                        }

                        if (newParentLayer !== undefined) {
                            nonDuikLayer.parent = newParentLayer;

                            parentsToAdjust.splice(j, 1);
                            j--;

                            didChangeAtLeastOne = true;
                        }
                    }
                }

                if (parentsToAdjust.length) {
                    alert('Unable to adjust parent chain: ' + JSON.stringify(parentsToAdjust));
                }
            }

            // Copy position, rotation, and scale from the original Duik layer
            for (var nonDuikLayerName in nonDuikToDuik) {
                var nonDuikLayer = comp.layer(nonDuikLayerName);
                var duikLayer = comp.layer(nonDuikToDuik[nonDuikLayerName]);

                /**
                 * @type {Layer | null}
                 */
                var duikPositionLayer = duikLayer;

                /**
                 * @type {Layer}
                 */
                var duikRotationLayer = duikLayer;

                /**
                 * @type {Layer | null}
                 */
                var duikScaleLayer = duikLayer;

                // If the duik layer is parented to an IK handle, use the IK handle properties instead. This handles the case of feet.
                //
                // A more robust solution would be to go up the parent chain until we find the non-Duik layer we'll be attaching to,
                // and take the cumulative transform of all the layers in between.
                if (duikLayer.parent && isDuikIK(duikLayer.parent)) {
                    var duikIkHandle = duikLayer.parent;

                    // The parent chain should be valid, since that's how duik sets up its layers.
                    duikPositionLayer = duikIkHandle.parent.parent;
                    // This may have to also support `duikIkHandle.parent` instead of just `duikIkHandle.parent.parent`.
                    duikRotationLayer = duikIkHandle.parent.parent;
                    duikScaleLayer = duikIkHandle.parent.parent;

                    if(duikPositionLayer == null) {
                        throw new Error('Duik position layer should not be null');
                    }

                    // Something is up with this, but I'm unsure what. It can place the layer in the wrong place,
                    // which is (currently) fixed by returning the time to the first frame.

                    var baseNonDuikPosition = nonDuikLayer.transform.position.valueAtTime(0, true);
                    var baseDuikPosition = duikPositionLayer.transform.position.valueAtTime(0, true);

                    var offset = [
                        baseNonDuikPosition[0] - baseDuikPosition[0],
                        baseNonDuikPosition[1] - baseDuikPosition[1],
                    ];

                    var baseNonDuikAnchorPoint = nonDuikLayer.transform.anchorPoint.valueAtTime(0, true);

                    // Set the anchor point to the duik position
                    nonDuikLayer.transform.anchorPoint.setValue([
                        baseNonDuikAnchorPoint[0] - offset[0],
                        baseNonDuikAnchorPoint[1] - offset[1],
                    ]);

                    nonDuikLayer.transform.position.setValue([
                        baseNonDuikPosition[0] - offset[0],
                        baseNonDuikPosition[1] - offset[1],
                    ]);
                } else {
                    // So, the position applier thing doesn't seem to work for non IK handle parented things (feet). Fix this later if necessary.
                    duikPositionLayer = null;
                }

                if (duikPositionLayer) {
                    copyKeyframes(comp, duikPositionLayer.transform.position, nonDuikLayer.transform.position, 0.15);
                }

                copyKeyframes(comp, duikRotationLayer.transform.rotation, nonDuikLayer.transform.rotation, 0.15);

                copyKeyframes(comp, duikScaleLayer.transform.scale, nonDuikLayer.transform.scale, 0.25);
            }

            // Remove all Duik layers
            for (var j = 1; j <= comp.numLayers; j++) {
                var layer = comp.layer(j);

                if (isDuikLayer(layer)) {
                    layer.remove();
                    j--;
                }
            }

            for (var j = 1; j <= comp.numLayers; j++) {
                var layer = comp.layer(j);

                removeUnnecessaryKeyframes(comp, layer, layer);
            }

            // If it has a layer called "full mask" then we need to do some fancy shit to make the border hella swick
            if (!!comp.layer('Full Mask') && comp.layer('Full Mask').enabled) {
                var mainComp = comp.duplicate();
                mainComp.name = comp.name + ' Main';

                mainComp.openInViewer();

                {
                    var fullMaskIndex = mainComp.layer('Full Mask').index;

                    // Remove the layers above the full mask from the main comp
                    for (var i = 1; i <= mainComp.numLayers; i++) {
                        var layer = mainComp.layer(i);

                        if (i < fullMaskIndex) {
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
                for (var i = 1; i <= fillComp.numLayers; i++) {
                    var layer = fillComp.layer(i);

                    if (layer.name.indexOf(' Fill') === -1) {
                        var originalFill = comp.layer(layer.name + ' Fill');

                        if ((!!originalFill && originalFill instanceof AVLayer) && (layer instanceof AVLayer && !!layer.source)) {
                            layer.name += ' Fill';

                            // Replace the source
                            layer.replaceSource(originalFill.source, false);

                            // Convert to shapes and delete the layer
                            layer.selected = true;
                        } else if (!layer.nullLayer) {
                            layer.remove();
                            i--;
                        }
                    } else {
                        layer.remove();
                        i--;
                    }
                }

                // After this, only the new shape layers will be marked as selected.
                app.executeCommand(CREATE_SHAPES_FROM_VECTOR_LAYER);

                // Fix the layer's parents
                for (var i = 1; i <= fillComp.numLayers; i++) {
                    var layer = fillComp.layer(i);

                    if (!layer.parent) continue;

                    if (!layer.parent.nullLayer && !(layer.parent instanceof ShapeLayer)) {
                        layer.parent = fillComp.layer(layer.parent.name + ' Outlines');
                    }
                }

                app.executeCommand(DESELECT_ALL);

                // Remove any non-shape layers
                for (var i = 1; i <= fillComp.numLayers; i++) {
                    var layer = fillComp.layer(i);

                    if (!layer.nullLayer && !(layer instanceof ShapeLayer || layer instanceof TextLayer)) {
                        layer.remove();
                        i--;
                    }
                }

                // Create a list of layers which attach to bordered layers
                var attachesToBorder = {};

                for (var i = 1; i <= fillComp.numLayers; i++) {
                    var fillLayer = fillComp.layer(i);

                    if (fillLayer.nullLayer) continue;

                    /* @ts-ignore */
                    var fillLayerContent = /** @type {PropertyGroup} */ (fillLayer.content);

                    for (var j = 1; j <= fillLayerContent.numProperties; j++) {
                        var fillGroup = fillLayerContent.property(j);
                        var fillFill = fillLayerContent.property('Fill 1');

                        if (!fillFill) continue;

                        /* @ts-ignore */
                        var color = /** @type {ColorValue} */ (fillFill.color.value);

                        if (color[0] + color[1] + color[2] >= 3)
                            continue;

                        /**
                         * @type {Layer | null}
                         */
                        var attach = fillLayer;

                        while (!!attach) {
                            attachesToBorder[attach.name] = true;

                            attach = attach.parent;
                        }
                    }
                }

                // Remove bordered layers from the main comp
                for (var i = 1; i <= mainComp.numLayers; i++) {
                    var layer = mainComp.layer(i);

                    // The main comp has not been processed, yet, so add " Outlines" to the layer name
                    if (!layer.nullLayer && !!attachesToBorder[layer.name + ' Outlines']) {
                        layer.remove();
                        i--;
                    }
                }

                // Remove unbordered layers from the fill comp
                for (var i = 1; i <= fillComp.numLayers; i++) {
                    var layer = fillComp.layer(i);

                    if (!layer.nullLayer && !attachesToBorder[layer.name]) {
                        layer.remove();
                        i--;
                    }
                }

                // Duplicate the fill comp so we can make the border layers
                var borderComp = fillComp.duplicate();
                borderComp.name = comp.name + ' Border';

                // Since the fill and border comps are identical, we can do both with one pass.
                for (var i = 1; i <= fillComp.numLayers; i++) {
                    var fillLayer = fillComp.layer(i);

                    if (fillLayer.nullLayer) continue;

                    var borderLayer = borderComp.layer(i);

                    /* @ts-ignore */
                    var fillLayerContent = /** @type {PropertyGroup} */ (fillLayer.content);

                    /* @ts-ignore */
                    var borderLayerContent = /** @type {PropertyGroup} */ (borderLayer.content);

                    for (var j = 1; j <= fillLayerContent.numProperties; j++) {
                        var fillGroup = fillLayerContent.property(j);
                        var borderGroup = borderLayerContent.property(j);

                        var borderStroke = borderLayerContent.property('Stroke 1');

                        if (!!borderStroke) {
                            borderStroke.remove();
                        }

                        /* @ts-ignore */
                        var fillGroupContent = /** @type {PropertyGroup} */ (fillGroup.content);

                        /* @ts-ignore */
                        var borderGroupContent = /** @type {PropertyGroup} */ (borderGroup.content);

                        var fillFill = fillGroupContent.property('Fill 1');
                        var borderFill = borderGroupContent.property('Fill 1');

                        if (!fillFill) continue;

                        /* @ts-ignore */
                        var color = /** @type {ColorValue} */ (fillFill.color.value);

                        if (color[0] + color[1] + color[2] >= 3) {
                            borderFill.remove();
                            borderGroupContent.property('Path 1').remove();

                            continue;
                        }

                        /* @ts-ignore */
                        borderFill.color.setValue(/** @type {ColorValue} */(fillFill.color.value));

                        /* @ts-ignore */
                        fillFill.color.setValue([1, 1, 1]);
                    }
                }

                // Remove any unnecessary limbs
                var found;

                do {
                    found = false;

                    for (var i = 1; i <= borderComp.numLayers; i++) {
                        var borderLayer = borderComp.layer(i);

                        // If the layer is helping create a border
                        if (!!attachesToBorder[borderLayer.name]) continue;

                        // Delete the layer
                        borderLayer.remove();
                        i--;

                        found = true;
                    }
                } while (found);

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
                while (finalComp.numLayers > mask.index) {
                    finalComp.layer(mask.index + 1).remove();
                }

                var mainCompLayer = finalComp.layers.add(mainComp);
                mainCompLayer.moveAfter(mask);

                var mainCompMask = mask.duplicate();
                mainCompMask.moveBefore(mainCompLayer);
                mainCompMask.name = mainCompLayer.name + ' Mask';
                mainCompMask.scale.setValue([100, 100]);
                mainCompLayer.trackMatteType = TrackMatteType.ALPHA_INVERTED;


                var borderCompLayer = finalComp.layers.add(borderComp);
                borderCompLayer.moveAfter(mainCompLayer);

                var borderCompMask = mask.duplicate();
                borderCompMask.moveBefore(borderCompLayer);
                borderCompMask.name = borderCompLayer.name + ' Mask';
                // If the scale hasn't been set, use a default scale. This lets us adjust the border size by setting the scale of the mask.
                if (borderCompMask.scale.value[0] == 100)
                    borderCompMask.scale.setValue([102, 102]);
                borderCompLayer.trackMatteType = TrackMatteType.ALPHA_INVERTED;


                var fillCompLayer = finalComp.layers.add(fillComp);
                fillCompLayer.moveAfter(borderCompLayer);

                var fillCompMask = mask.duplicate();
                fillCompMask.moveBefore(fillCompLayer);
                fillCompMask.name = fillCompLayer.name + ' Mask';
                fillCompMask.scale.setValue([104, 104]);
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
            for (var j = 1; j <= comp.numLayers; j++) {
                var layer = comp.layer(j);

                if (layer instanceof AVLayer && layer.trackMatteType != TrackMatteType.NO_TRACK_MATTE) {
                    maskCount++;
                }

                var isMask = layer.name.indexOf(' Mask') !== -1;

                if (isMask) {
                    if (layer instanceof ShapeLayer) {
                        layer.enabled = false;

                        continue;
                    }
                } else if (!layer.enabled) {
                    continue;
                }

                if (layer instanceof AVLayer && layer.source instanceof CompItem) {
                    // Add sub comps to the targets list if we aren't already doing them all
                    if (!allTargets)
                        targets.push(layer.source);
                    continue;
                }

                layer.selected = true;

                app.executeCommand(CREATE_SHAPES_FROM_VECTOR_LAYER);

                layer.selected = false;

                if (layer instanceof AVLayer) {
                    var layerOutline = comp.layer(layer.name + ' Outlines');

                    if (!!layerOutline && layerOutline instanceof AVLayer && layerOutline.trackMatteType != layer.trackMatteType) {
                        layerOutline.trackMatteType = layer.trackMatteType;
                    }
                }

                if (isMask) {
                    layer.enabled = false;

                    layer.remove();

                    j--;
                }
            }

            if (maskCount >= 15) {
                alert('Telegram limits masks to 15 per sticker! "' + comp.name + '" has ' + maskCount);
            }

            app.executeCommand(DESELECT_ALL);

            // Fix parenting
            for (var j = 1; j <= comp.numLayers; j++) {
                var layer = comp.layer(j);

                if (!layer.parent) continue;

                var convParent = comp.layer(layer.parent.name + ' Outlines');

                if (!convParent) continue;

                layer.parent = convParent;
            }

            // Remove unnecessary layers
            for (var j = 1; j <= comp.numLayers; j++) {
                var layer = comp.layer(j);

                if (layer.name.indexOf(' Outlines') !== -1) continue;

                if (layer instanceof ShapeLayer) continue;

                if (layer instanceof AVLayer && layer.source instanceof CompItem) continue;

                if (layer.nullLayer) continue;

                layer.remove();

                j--;
            }

            // Remove the Outlines suffix from all layers
            for (var j = 1; j <= comp.numLayers; j++) {
                var layer = comp.layer(j);

                if (layer.name.indexOf(' Outlines') !== -1) {
                    layer.name = layer.name.replace(' Outlines', '');
                }
            }
        }

        // No longer necessary, as the changes to the Bodymovin plugin now strip out all layer names.
        // // Obfuscation, baby. Randomize all layer names. >:3
        // if (confirm('Obfuscate?')) {
        //     var c = 0;

        //     for (var d = 0; d < targets.length; d++) {
        //         var comp = targets[d];

        //         app.executeCommand(UNLOCK_ALL_LAYERS);

        //         for (var j = 1; j <= comp.numLayers; j++) {
        //             var layer = comp.layer(j);

        //             obfuscate(c, layer);

        //             c++;
        //         }
        //     }
        // }

        app.endUndoGroup();
    }

    app.executeCommand(app.findMenuCommandId("Bodymovin for Telegram Stickers"));
}
