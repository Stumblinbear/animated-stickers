var proj = app.project;

function fullPathName(item) {
    return (!!item.parentFolder && item.parentFolder.name !== 'Root' ? item.parentFolder.name + ' ' : '' ) + item.name.replace(/%20/g, ' ');
}

function importFolder(fileTypes, searchFolder, parentFolder, prefix) {
    prefix = prefix || '';

    var files = searchFolder.getFiles();

    loop:
    for(var i in files) {
        if(files[i] instanceof Folder) {
            var name = files[i].name.replace(/%20/g, ' ');

            if(name.indexOf('Auto-Save') !== -1) continue;

            var folder = null;

            // Find an existing folder
            for(var j = 1; j <= app.project.numItems; j++) {
                var item = app.project.item(j);

                if(!(item instanceof FolderItem)) continue;

                if(prefix + name == fullPathName(item)) {
                    folder = item;
                    break;
                }
            }
            
            if(!folder) {
                folder = app.project.items.addFolder(files[i].name);

                folder.name = name;
            }

            importFolder(fileTypes, files[i], folder, prefix + name + ' ');
        }else if(files[i] instanceof File) {
            var ext = null;

            for(var j = 0; j < fileTypes.length; j++) {
                if(files[i].name.toLowerCase().indexOf('.' + fileTypes[j]) !== -1) {
                    ext = fileTypes[j];
                    break;
                }
            }

            if(!ext) continue;

            try {
                var name = files[i].name.split('.', 2)[0].replace(/%20/g, ' ');
    
                // Don't reimport existing items
                for(var j = 1; j <= app.project.numItems; j++) {
                    var item = app.project.item(j);
                    
                    if(!(item instanceof FolderItem)) continue;

                    if(prefix + name == item.name) {
                        continue loop;
                    }
                }

                var importOptions = new ImportOptions(files[i]);

                importOptions.importAs = ImportAsType.COMP;
    
                var imported = app.project.importFile(importOptions);

                var childFolder = app.project.items.addFolder(prefix + name);

                if(!!parentFolder)
                    childFolder.parentFolder = parentFolder;
                
                for(var j = 1; j <= app.project.numItems; j++) {
                    var item = app.project.item(j);

                    if(item instanceof FolderItem && item.name == imported.name + ' Layers') {
                        item.name = prefix + name + ' Layers';
                        
                        item.parentFolder = childFolder;

                        break;
                    }
                }

                {
                    imported.name = prefix + name;

                    imported.parentFolder = childFolder;

                    // TODO: Fix layer durations if this isn't 3
                    
                    imported.duration = 3;

                    if(ext == "ai") {
                        imported.frameRate = 30;
                        imported.width = imported.height = 512;

                        // Loop through the layers and set the center point to the middle, to account for larger import sizes
                        for(var i = 1; i <= imported.numLayers; i++) {
                            var layer = imported.layer(i);
                            
                            layer.transform.position.setValue([ 256, 256, ]);
                        }
                    }else if(ext == "psd") {
                        imported.frameRate = 60;
                    }
                }
            } catch (error) {
                alert(error.toString() + importOptions.file.fsName, "Set Up Project");
            }
        }
    }
}

if(proj) {
    var mainFolder = Folder(app.project.file.path);

    app.project.setDefaultImportFolder(mainFolder);

    app.beginUndoGroup("Set Up Project");

    var fileTypes = [];

    if(confirm('Import .ai files?')) fileTypes.push('ai');
    if(confirm('Import .psd files?')) fileTypes.push('psd');

    importFolder(fileTypes, mainFolder);

    app.endUndoGroup();
}else{
    alert("Please open a project first to use this script.", "Set Up Project");
}