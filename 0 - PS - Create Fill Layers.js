var doc = app.activeDocument;

doc.suspendHistory('FixKwikLines', 'main()');

function main() {
    var ignores = [
        'Eye', 'Eyelid', 'Iris', 'Brow', 'Mouf', 'Blush', 'Balls', 'Mask', 'Border'
    ]
    
    var length = doc.artLayers.length;

    loop:
    for(var i = 0; i < length; i++) {
        var layer = doc.artLayers[i];

        if(!layer.visible) continue;
        
        // Ignore some layers
        for(var j in ignores) {
            if(layer.name.indexOf(ignores[j]) !== -1)
                continue loop;
        }

        // Set the layer as active
        doc.activeLayer = layer;

        // Fancy magic to select non-transparent pixels
        (ref1 = new ActionReference()).putProperty(c = stringIDToTypeID('channel'), stringIDToTypeID('selection'));
        (dsc = new ActionDescriptor()).putReference(stringIDToTypeID('null'), ref1);
        (ref2 = new ActionReference()).putEnumerated(c, c, stringIDToTypeID('transparencyEnum'))
        dsc.putReference(stringIDToTypeID('to'), ref2), executeAction(stringIDToTypeID('set'), dsc);
        
        // Create a new layer
        var newLayerRef = app.activeDocument.artLayers.add();
        newLayerRef.name = layer.name + ' Fill';

        // Set the new layer as active
        doc.activeLayer = newLayerRef;

        // Move the new layer to the bottom of the list
        newLayerRef.move(doc.artLayers[doc.artLayers.length - 1], ElementPlacement.PLACEAFTER);

        // Fill the selection with black
        var fillColor = new SolidColor();
        fillColor.rgb.red = fillColor.rgb.green = fillColor.rgb.blue = 0;
        doc.selection.fill(fillColor);
    }

    // Merge depth fix fills
    for(var i = length; i < doc.artLayers.length; i++) {
        var layer = doc.artLayers[i];

        if(!layer.visible) continue;

        if(layer.name.indexOf('Depth Fix Fill') !== -1) {
            // Put the Depth Fix layer above the correct layer
            layer.move(doc.artLayers.getByName(layer.name.split('Depth Fix Fill')[0] + 'Fill'), ElementPlacement.PLACEBEFORE);

            // Merge them
            layer.merge();

            i--;
        } else if(layer.name.indexOf('DF Fill') !== -1) {
            // Put the Depth Fix layer above the correct layer
            layer.move(doc.artLayers.getByName(layer.name.split('DF Fill')[0] + 'Fill'), ElementPlacement.PLACEBEFORE);

            // Merge them
            layer.merge();

            i--;
        }
    }
}