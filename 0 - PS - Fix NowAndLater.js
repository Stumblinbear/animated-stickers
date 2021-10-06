var doc = app.activeDocument;

doc.suspendHistory('Fix NowAndLater', 'main()');

function main() {
    for(var i = 0; i < doc.artLayers.length; i++) {
        var layer = doc.artLayers[i];
    
        if(!layer.visible) continue;
    
        if(layer.name.indexOf('Fill') !== -1) continue;
        
        layer.applySmartBlur(5, 25, SmartBlurQuality.LOW, SmartBlurMode.NORMAL);
    }
}