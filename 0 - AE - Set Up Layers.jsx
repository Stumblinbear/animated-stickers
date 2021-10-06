var proj = app.project;

var parenting = {
    'Left Ear': 'Head',
    'L Ear': 'Head',
    'Right Ear': 'Head',
    'R Ear': 'Head',
    
    'Hair': 'Head',
    
    'Left Brow': 'Head',
    'L Brow': 'Head',
    'Right Brow': 'Head',
    'R Brow': 'Head',
    
    'Left Eyelid': 'Head',
    'L Eyelid': 'Head',
    'Right Eyelid': 'Head',
    'R Eyelid': 'Head',
    
    'Glint': 'Head',
    
    'Left Eye': 'Head',
    'L Eye': 'Head',
    'Right Eye': 'Head',
    'R Eye': 'Head',
    'Left Iris': 'Head',
    'L Iris': 'Head',
    'Right Iris': 'Head',
    'R Iris': 'Head',
    
    'Eyes': 'Head',
    'Balls': 'Head',

    'Snoot': 'Head',
    'Mouth': 'Head',
    'Mouf': 'Head',
    'Tongue': 'Head',
    'Jaw': 'Head',
    
    'Head': ['Necc', 'Body', 'Boi'],

    'Necc': ['Body', 'Boi'],
    
    'PP': ['Lower Body', 'Body', 'Boi'],
    
    'Pouch': ['Lower Body', 'Body', 'Boi'],
    
    'Tail': ['Lower Body', 'Body', 'Boi'],
    
    'Leg': ['Lower Body', 'Body', 'Boi'],

    'Right Leg': ['Lower Body', 'Body', 'Boi'],
    'R Leg': ['Lower Body', 'Body', 'Boi'],
    'Left Leg': ['Lower Body', 'Body', 'Boi'],
    'L Leg': ['Lower Body', 'Body', 'Boi'],
    'Right Thigh': ['Lower Body', 'Body', 'Boi'],
    'R Thigh': ['Lower Body', 'Body', 'Boi'],
    'Left Thigh': ['Lower Body', 'Body', 'Boi'],
    'L Thigh': ['Lower Body', 'Body', 'Boi'],
    
    'Left Calf': 'Left Thigh',
    'L Calf': 'L Thigh',
    'Right Calf': 'Right Thigh',
    'R Calf': 'R Thigh',

    'Left Foot': 'Left Calf',
    'L Foot': 'L Calf',
    'Right Foot': 'Right Calf',
    'R Foot': 'R Calf',

    'Arm': ['Body', 'Boi'],
    'Forearm': ['Arm'],
    'Hand': ['Forearm', 'Arm'],
    
    'Finger': 'Hand',
    'Fingers': 'Hand',
    'Thumb': 'Hand',

    'Right Arm': ['Body', 'Boi'],
    'R Arm': ['Body', 'Boi'],
    'Left Arm': ['Body', 'Boi'],
    'L Arm': ['Body', 'Boi'],

    'Right Forearm': 'Right Arm',
    'R Forearm': 'R Arm',
    'Left Forearm': 'Left Arm',
    'L Forearm': 'L Arm',

    'Right Hand': 'Right Forearm',
    'R Hand': 'R Forearm',
    'Left Hand': 'Left Forearm',
    'L Hand': 'L Forearm',

    'Right Finger': 'Right Hand',
    'R Finger': 'R Hand',
    'Right Fingers': 'Right Hand',
    'R Fingers': 'R Hand',
    'Right Thumb': 'Right Hand',
    'R Thumb': 'R Hand',

    'Left Finger': 'Right Hand',
    'L Finger': 'L Hand',
    'Left Fingers': 'Left Hand',
    'L Fingers': 'L Hand',
    'Left Thumb': 'Left Hand',
    'L Thumb': 'L Hand',

    'Depth Fix': '',
    'Border': '',
    'DF': '',
    'Inner': '',
    'Press': ''
};

var shyify = [
    'Fill',
    'Depth Fix',
    'DF',
    'Border',
    'Balls'
];

if(proj) {
    var targets = [];

    if(confirm('Apply to all compositions?')) {
        for(var i = 1; i <= app.project.numItems; i++){
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
        app.beginUndoGroup("Parent Fill Layers");

        for(var d = 0; d < targets.length; d++) {
            var comp = targets[d];

            for(var i = 1; i <= comp.numLayers; i++) {
                var layer = comp.layer(i);
                
                for(var s in shyify) {
                    if(layer.name.indexOf(shyify[s]) === -1) continue;
                    
                    layer.shy = true;
                }

                // If the layer is a fill layer
                if(layer.name.indexOf('Fill') !== -1) {
                    // Parent it to the normal layer
                    layer.parent = comp.layer(layer.name.split(' Fill')[0]);
                }else{
                    for(var match in parenting) {
                        if(layer.name.indexOf(match) === -1) continue;

                        var parts = layer.name.split(match);

                        if(parts.length == 1) {
                            parts = [ '', parts[0] ];
                        }

                        main:
                        while(match) {
                            var possibleParents = parenting[match];

                            if(!(possibleParents instanceof Array)) possibleParents = [ possibleParents ];

                            for(var j = 0; j < possibleParents.length; j++) {
                                layer.parent = comp.layer((parts[0] + possibleParents[j]).replace(/^\s+|\s+$/g, ''));
                            
                                // If we found a parent, bail.
                                if(layer.parent) break main;
                            }

                            if(!layer.parent)
                                match = possibleParents[0];
                        }
                    }
                }
            }
        }

        app.endUndoGroup();
    }
}else{
    alert("Please open a project first to use this script.", "Parent Fill Layers");
}