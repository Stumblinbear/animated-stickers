#target photoshop

var doc = app.activeDocument;

doc.suspendHistory('Fix Thin Lines', 'main()');

function main() {
    var dlg = new Window('dialog', 'Fix Thin Lines');
    dlg.alignChildren ='left';

    var sizeGroup = dlg.add('group');
    sizeGroup.orientation = 'row';
    sizeGroup.add('statictext', undefined, 'Minimum size');

    var pixelsInput = sizeGroup.add('edittext', undefined, 0.25);
    {
        pixelsInput.preferredSize = [20, 20];

        pixelsInput.onChanging = function() {
            if (parseFloat(pixelsInput.text) <= 0.05){
                pixelsInput.text = 0.05;
            }
        }
    }

    var btnPnl = dlg.add('group');
    {
        btnPnl.alignment = 'right';
        btnPnl.okBtn = btnPnl.add('button', undefined, 'OK', { name:'ok' });
        btnPnl.okBtn.active = true;
        btnPnl.cancelBtn = btnPnl.add('button', undefined, 'Cancel', { name:'cancel' });
    }

    if(dlg.show() == 2) return;

    var pixels = parseFloat(pixelsInput.text);

    if(pixels <= 0) {
        return alert('Value must be greater than zero.');
    }

    var size = [ doc.width, doc.height ];

    // It doesn't allow fraction-pixels, so upscale the image
    if(pixels < 1) {
        doc.resizeImage(size[0] * (1 / pixels), size[1] * (1 / pixels));
    }

    for(var i = 0; i < doc.artLayers.length; i++) {
        var layer = doc.artLayers[i];
    
        if(!layer.visible) continue;
    
        if(layer.name.indexOf('Fill') !== -1) continue;
    
        layer.applyMinimum(Math.ceil(pixels));
    }

    // Return the image to its original size
    doc.resizeImage(size[0], size[1]);
}